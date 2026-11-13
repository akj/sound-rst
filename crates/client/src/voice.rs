//! The voice: what the player hears said, in which order, and when a key
//! press cuts it short.
//!
//! Messages said "now" (menus, alerts) interrupt the queue; messages said
//! "sooner or later" (game information) wait in the queue and may expire.
//! Every message is kept in a history the player can walk back through.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use soundrts_world::msg::Msg;

use crate::audio::DEFAULT_VOLUME;
use crate::clock::ClockRef;
use crate::input::{Input, InputPoint};
use crate::log::Log;
use crate::message::{InfoOptions, Message};
use crate::sound::Audio;
use crate::sound_cache::{Sounds, Spoken};
use crate::tts::Tts;

/// The history keeps at most this many messages...
const HISTORY_LIMIT: usize = 200;
/// ...dropping this many of the oldest when it is full.
const HISTORY_DROP: usize = 100;

/// One part of an utterance as it started: words, or a recorded sound by
/// its name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UtterancePart {
    Text(String),
    Sound(String),
}

/// What one message became when the voice started saying it.
pub type Utterance = Vec<UtterancePart>;

/// How a message came to be said.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VoiceKind {
    /// Said now, not kept in the history.
    Item,
    /// Said now; keys pressed meanwhile are dropped.
    Alert,
    /// Said now until a key press, which is used up.
    Important,
    /// Said now until a key press, which stays queued.
    Confirmation,
    /// A menu title: as a confirmation.
    Menu,
    /// A queued message, said when its turn came.
    Info,
    /// A message of the history, said again.
    History,
}

impl VoiceKind {
    pub fn name(self) -> &'static str {
        match self {
            VoiceKind::Item => "item",
            VoiceKind::Alert => "alert",
            VoiceKind::Important => "important",
            VoiceKind::Confirmation => "confirmation",
            VoiceKind::Menu => "menu",
            VoiceKind::Info => "info",
            VoiceKind::History => "history",
        }
    }
}

/// Something the voice channel did.
#[derive(Clone, Debug, PartialEq)]
pub enum VoiceEvent {
    /// A message started, at `time`, with its speaker volumes.
    Started {
        time: f64,
        kind: VoiceKind,
        parts: Utterance,
        lv: f64,
        rv: f64,
    },
    /// Speech in progress was stopped other than by a new message, with
    /// `queued` parts of the message still to come.
    Cut { time: f64, queued: usize },
}

#[derive(Default)]
struct VoiceRecord {
    events: Vec<VoiceEvent>,
    last_cut: Option<f64>,
}

/// A record of what the voice channel did, in order. Clones share the
/// same record.
#[derive(Clone, Default)]
pub struct UtteranceLog(Rc<RefCell<VoiceRecord>>);

impl UtteranceLog {
    /// The utterances started, in order.
    pub fn utterances(&self) -> Vec<Utterance> {
        self.0
            .borrow()
            .events
            .iter()
            .filter_map(|e| match e {
                VoiceEvent::Started { parts, .. } => Some(parts.clone()),
                VoiceEvent::Cut { .. } => None,
            })
            .collect()
    }

    /// Every event from the `from`th on.
    pub fn events_from(&self, from: usize) -> Vec<VoiceEvent> {
        self.0.borrow().events[from.min(self.len())..].to_vec()
    }

    /// The number of events recorded.
    pub fn len(&self) -> usize {
        self.0.borrow().events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn clear(&self) {
        self.0.borrow_mut().events.clear();
    }

    fn push(&self, e: VoiceEvent) {
        self.0.borrow_mut().events.push(e);
    }

    /// Records a cut, once per instant.
    fn cut(&self, time: f64, queued: usize) {
        let mut r = self.0.borrow_mut();
        if r.last_cut != Some(time) {
            r.last_cut = Some(time);
            r.events.push(VoiceEvent::Cut { time, queued });
        }
    }
}

/// Says one message at a time: its texts through speech, its sounds on the
/// voice channel, in order.
pub struct VoiceChannel {
    tts: Tts,
    queue: VecDeque<(Spoken, f64, f64)>,
    starting_time: f64,
    audio: Audio,
    sounds: Sounds,
    clock: ClockRef,
    log: Log,
    debug_mode: bool,
    record: Option<UtteranceLog>,
}

impl VoiceChannel {
    pub fn new(
        tts: Tts,
        audio: Audio,
        sounds: Sounds,
        clock: ClockRef,
        log: Log,
        debug_mode: bool,
        record: Option<UtteranceLog>,
    ) -> Self {
        VoiceChannel {
            tts,
            queue: VecDeque::new(),
            starting_time: 0.0,
            audio,
            sounds,
            clock,
            log,
            debug_mode,
            record,
        }
    }

    pub fn play(&mut self, msg: &Message, kind: VoiceKind) {
        if self.debug_mode {
            let texts: Vec<String> = msg
                .translate_and_collapse(&self.sounds, true)
                .into_iter()
                .filter_map(|s| match s {
                    Spoken::Text(t) => Some(t),
                    Spoken::Sound(_) => None,
                })
                .collect();
            self.log.said(&texts.join(" "));
        }
        let parts = msg.translate_and_collapse(&self.sounds, false);
        if let Some(record) = &self.record {
            record.push(VoiceEvent::Started {
                time: self.clock.now(),
                kind,
                parts: parts
                    .iter()
                    .map(|p| match p {
                        Spoken::Text(t) => UtterancePart::Text(t.clone()),
                        Spoken::Sound(s) => UtterancePart::Sound(s.key.clone()),
                    })
                    .collect(),
                lv: msg.lv,
                rv: msg.rv,
            });
        }
        self.silence();
        self.queue.clear();
        for part in parts {
            self.queue.push_back((part, msg.lv, msg.rv));
        }
        self.update();
        self.starting_time = self.clock.now();
    }

    /// Whether the current message has been playing for over a second.
    pub fn is_almost_done(&self) -> bool {
        self.clock.now() - self.starting_time > 1.0
    }

    /// Stops the current message and forgets the rest of it.
    pub fn stop(&mut self) {
        self.note_cut();
        self.silence();
        self.queue.clear();
    }

    /// Stops the part being said; the rest of the message comes at the
    /// next update.
    pub fn stop_part(&mut self) {
        self.note_cut();
        self.silence();
    }

    fn silence(&mut self) {
        self.audio.voice_stop();
        self.tts.stop();
    }

    fn note_cut(&self) {
        if let Some(record) = &self.record
            && (self.tts.is_speaking() || self.audio.voice_busy())
        {
            record.cut(self.clock.now(), self.queue.len());
        }
    }

    /// Starts the next part when the previous one is over.
    pub fn update(&mut self) {
        if !self.audio.voice_busy()
            && !self.tts.is_speaking()
            && let Some((part, lv, rv)) = self.queue.pop_front()
        {
            match part {
                Spoken::Text(t) => self.tts.speak(&t),
                Spoken::Sound(s) => self.audio.voice_play(&s, lv, rv),
            }
        }
    }

    pub fn get_busy(&self) -> bool {
        self.audio.voice_busy() || !self.queue.is_empty() || self.tts.is_speaking()
    }
}

/// The voice: the message queue and history.
pub struct Voice {
    /// Said and unsaid messages, oldest first.
    msgs: Vec<Message>,
    /// A message from the queue is being said (not just an item).
    active: bool,
    /// The player is walking through the history.
    history: bool,
    /// The index of the message being said; `msgs.len()` if none.
    current: usize,
    channel: VoiceChannel,
    input: Input,
    clock: ClockRef,
}

impl Voice {
    pub fn new(channel: VoiceChannel, input: Input, clock: ClockRef) -> Voice {
        Voice {
            msgs: Vec::new(),
            active: false,
            history: false,
            current: 0,
            channel,
            input,
            clock,
        }
    }

    fn now(&self) -> f64 {
        self.clock.now()
    }

    /// The index of the first message never said, or `msgs.len()`.
    fn unsaid(&self) -> usize {
        self.msgs
            .iter()
            .position(|m| !m.said)
            .unwrap_or(self.msgs.len())
    }

    fn exists(&self, index: usize) -> bool {
        index < self.msgs.len()
    }

    fn unsaid_exists(&self) -> bool {
        self.exists(self.unsaid())
    }

    fn current_message_is_unsaid(&self) -> bool {
        self.msgs.get(self.current).is_some_and(|m| !m.said)
    }

    fn mark_current_as_said(&mut self) {
        if let Some(m) = self.msgs.get_mut(self.current) {
            m.said = true;
        }
    }

    fn go_to_next_unsaid(&mut self) {
        self.current = self.unsaid();
    }

    /// A message cut short after a second counts as said, so it does not
    /// come back again and again.
    fn give_up_current_if_partially_said(&mut self) {
        if self.current_message_is_unsaid() && self.channel.is_almost_done() {
            self.mark_current_as_said();
        }
    }

    fn start_current(&mut self) {
        self.channel.stop();
        self.active = false;
        self.update();
    }

    /// Says the previous message of the history.
    pub fn previous(&mut self) {
        self.history = true;
        if self.current > 0 {
            self.current -= 1;
        }
        self.start_current();
    }

    /// Skips to the next message. With `history_only`, an unsaid message
    /// is not skipped.
    pub fn say_next(&mut self, history_only: bool) {
        if self.active {
            if self.current_message_is_unsaid() {
                if history_only {
                    return;
                }
                self.mark_current_as_said();
            }
            self.current += 1;
            self.start_current();
        }
    }

    /// Says now, and waits until the end; a key press does not interrupt.
    /// Key presses made meanwhile are dropped.
    pub fn alert(&mut self, msg: &Msg) {
        self.say_now(msg, VoiceKind::Alert);
    }

    /// Says now, and waits until the end or a key press, which is used up.
    pub fn important(&mut self, msg: &Msg) {
        self.say_now(msg, VoiceKind::Important);
    }

    /// Says now, and waits until the end or a key press, which stays in the
    /// queue for the caller.
    pub fn confirmation(&mut self, msg: &Msg) {
        self.say_now(msg, VoiceKind::Confirmation);
    }

    /// Says a menu title: as [`Voice::confirmation`].
    pub fn menu(&mut self, msg: &Msg) {
        self.say_now(msg, VoiceKind::Menu);
    }

    /// Queues a message to say sooner or later.
    pub fn info(&mut self, msg: &Msg) {
        self.info_with(msg, InfoOptions::default());
    }

    pub fn info_with(&mut self, msg: &Msg, options: InfoOptions) {
        if !msg.is_empty() {
            let now = self.now();
            self.msgs
                .push(Message::new(msg.clone(), options, false, now));
            self.update();
        }
    }

    /// Says now (giving up the unsaid queue) until the end, or until a key
    /// press unless it is an alert. The key press stays queued after a
    /// confirmation or a menu title.
    fn say_now(&mut self, msg: &Msg, kind: VoiceKind) {
        if msg.is_empty() {
            return;
        }
        let interruptible = kind != VoiceKind::Alert;
        let keep_key = matches!(kind, VoiceKind::Confirmation | VoiceKind::Menu);
        let options = InfoOptions::default();
        self.give_up_current_if_partially_said();
        let now = self.now();
        self.channel.play(
            &Message::new(msg.clone(), options.clone(), false, now),
            kind,
        );
        while self.channel.get_busy() {
            if interruptible && self.key_hit(keep_key) {
                break;
            }
            self.clock.sleep(0.1);
            self.channel.update();
        }
        if !interruptible {
            self.input.clear_keydowns();
        }
        let now = self.now();
        self.msgs
            .push(Message::new(msg.clone(), options, true, now));
        self.go_to_next_unsaid();
        self.active = false;
    }

    /// Says now without recording in the history: a menu item, a typed
    /// letter.
    pub fn item(&mut self, msg: &Msg) {
        self.item_stereo(msg, DEFAULT_VOLUME, DEFAULT_VOLUME);
    }

    pub fn item_stereo(&mut self, msg: &Msg, lv: f64, rv: f64) {
        if !msg.is_empty() {
            self.give_up_current_if_partially_said();
            self.go_to_next_unsaid();
            let now = self.now();
            let options = InfoOptions {
                lv,
                rv,
                ..InfoOptions::default()
            };
            self.channel.play(
                &Message::new(msg.clone(), options, false, now),
                VoiceKind::Item,
            );
            self.active = false;
            self.history = false;
        }
    }

    fn expired(&self, index: usize) -> bool {
        let msg = &self.msgs[index];
        if msg.has_expired(self.now()) {
            return true;
        }
        let newer = &self.msgs[index + 1..];
        if msg.update_type.is_some() && newer.iter().any(|m| m.update_type == msg.update_type) {
            return true;
        }
        newer.iter().any(|m| m.parts == msg.parts)
    }

    fn mark_expired_messages_as_said(&mut self) {
        for i in 0..self.msgs.len() {
            if !self.msgs[i].said && self.expired(i) {
                self.msgs[i].said = true;
            }
        }
        if self.msgs.len() > HISTORY_LIMIT {
            self.msgs.drain(..HISTORY_DROP);
            self.current = self.current.saturating_sub(HISTORY_DROP);
        }
    }

    /// Moves the queue along: starts the next unsaid message when the
    /// voice is free. Call it often.
    pub fn update(&mut self) {
        if self.channel.get_busy() {
            self.channel.update();
            return;
        }
        self.mark_expired_messages_as_said();
        if self.active {
            // A message from the queue has just finished.
            self.mark_current_as_said();
            self.current += 1;
        }
        if !self.history {
            self.go_to_next_unsaid();
        }
        if self.exists(self.current) {
            let msg = self.msgs[self.current].clone();
            let kind = if self.history {
                VoiceKind::History
            } else {
                VoiceKind::Info
            };
            self.channel.play(&msg, kind);
            self.active = true;
        } else {
            self.active = false;
            self.history = false;
        }
    }

    /// Stops talking and marks every message as said.
    pub fn silent_flush(&mut self) {
        self.channel.stop();
        self.active = false;
        self.current = self.msgs.len();
        for m in &mut self.msgs {
            m.said = true;
        }
    }

    /// Says every unsaid message, waiting. If `interruptible`, a key press
    /// skips to the next message, or ends the wait after the last one.
    pub fn flush(&mut self, interruptible: bool) {
        loop {
            self.update();
            if !(self.unsaid_exists() || self.channel.get_busy()) {
                break;
            } else if interruptible && self.key_hit(true) {
                if self.unsaid_exists() {
                    self.say_next(false);
                    self.input.clear_keydowns();
                } else {
                    break;
                }
            }
            self.clock.sleep(0.1);
        }
        if !interruptible {
            self.input.clear_keydowns();
        }
    }

    /// Whether a key was pressed; the presses are put back at the end of
    /// the queue if `keep_key`.
    fn key_hit(&mut self, keep_key: bool) -> bool {
        self.input.input_point(InputPoint::Voice, self.is_idle());
        let keys = self.input.get_keydowns();
        let hit = !keys.is_empty();
        if keep_key {
            for k in keys {
                self.input.post(crate::input::Event::KeyDown(k));
            }
        }
        hit
    }

    /// Stops the part being said; the rest of the current message goes on.
    pub fn stop_speech(&mut self) {
        self.channel.stop_part();
    }

    /// Whether the voice is saying something.
    pub fn is_busy(&self) -> bool {
        self.channel.get_busy()
    }

    /// Whether the voice says nothing and has nothing left to say.
    pub fn is_idle(&self) -> bool {
        !self.channel.get_busy() && !self.unsaid_exists()
    }

    /// The history, oldest first, with whether each message has been said.
    pub fn history(&self) -> impl Iterator<Item = (&Msg, bool)> {
        self.msgs.iter().map(|m| (&m.parts, m.said))
    }
}
