//! The voice: what the player hears said, in which order, and when a key
//! press cuts it short.
//!
//! Answers to keys and messages said "now" (menus, prompts) interrupt the
//! queue. Outside a game, a message said now holds the caller until it is
//! said or a key is pressed; inside the game loop nothing waits. News the
//! game volunteers waits in the queue, as the policy of its category says,
//! and is never lost: a message cut off goes back in the queue, newer news
//! about a subject joins the unheard news about it (a count adds what it
//! counts to it), and every message but answers is kept in a log. The
//! player can walk back through the log, one category at a time, or through
//! what they have not heard.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use soundrts_world::SquareId;
use soundrts_world::msg::{Msg, Phrase, comma, msg, nb2msg};

use crate::audio::DEFAULT_VOLUME;
use crate::clock::ClockRef;
use crate::input::{Input, InputPoint, Key, KeyDown};
use crate::log::Log;
use crate::message::{Category, News, Parts, Policy, Subject, translate_and_collapse};
use crate::sound::Audio;
use crate::sound_cache::{Sounds, Spoken};
use crate::tts::Tts;

/// The log keeps at most this many messages; past it, the oldest goes.
pub const LOG_LIMIT: usize = 1000;

/// Speaker volumes of a message that is not placed.
const CENTERED: (f64, f64) = (DEFAULT_VOLUME, DEFAULT_VOLUME);

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
    /// Said now; outside a game, keys pressed meanwhile are dropped.
    Alert,
    /// Said now; outside a game, until a key press, which is used up.
    Important,
    /// Said now; outside a game, until a key press, which stays queued.
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
            audio,
            sounds,
            clock,
            log,
            debug_mode,
            record,
        }
    }

    /// Starts saying `parts` at these speaker volumes, or only their
    /// recorded sounds, instead of what was being said.
    pub fn play(&mut self, parts: &Msg, (lv, rv): (f64, f64), kind: VoiceKind, which: Parts) {
        if self.debug_mode && which != Parts::Sounds {
            self.log.said(&self.words(parts));
        }
        let spoken = translate_and_collapse(parts, &self.sounds, which);
        if let Some(record) = &self.record {
            record.push(VoiceEvent::Started {
                time: self.clock.now(),
                kind,
                parts: spoken
                    .iter()
                    .map(|p| match p {
                        Spoken::Text(t) => UtterancePart::Text(t.clone()),
                        Spoken::Sound(s) => UtterancePart::Sound(s.key.clone()),
                    })
                    .collect(),
                lv,
                rv,
            });
        }
        self.silence();
        self.queue.clear();
        for part in spoken {
            self.queue.push_back((part, lv, rv));
        }
        self.update();
    }

    /// The words of a message as the speech device gets them, without its
    /// recorded sounds.
    pub fn words(&self, parts: &Msg) -> String {
        let texts: Vec<String> = translate_and_collapse(parts, &self.sounds, Parts::Texts)
            .into_iter()
            .filter_map(|s| match s {
                Spoken::Text(t) => Some(t),
                Spoken::Sound(_) => None,
            })
            .collect();
        texts.join(" ")
    }

    /// Whether a message has recorded sounds among its parts.
    pub fn has_sounds(&self, parts: &Msg) -> bool {
        !translate_and_collapse(parts, &self.sounds, Parts::Sounds).is_empty()
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

    /// Whether the player silenced the message at the screen reader.
    pub fn was_cancelled(&self) -> bool {
        self.tts.was_cancelled()
    }

    /// Starts the next part when the previous one is over. A message the
    /// player silenced at the screen reader is dismissed with its other
    /// parts.
    pub fn update(&mut self) {
        if self.audio.voice_busy() || self.tts.is_speaking() {
            return;
        }
        // Read after the end: a cancel reported in between still counts.
        if self.tts.was_cancelled() {
            self.queue.clear();
        }
        if let Some((part, lv, rv)) = self.queue.pop_front() {
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

/// Whether the player has heard a message of the log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Heard {
    /// Waits in the queue to be said, or with `sounds_only`, for its
    /// recorded sounds to be played.
    Waiting {
        sounds_only: bool,
    },
    /// Kept without its words being said.
    No,
    Yes,
}

/// What a message of the log is.
#[derive(Clone, Debug, PartialEq)]
enum Content {
    /// Said at once: a menu title, a prompt, a message outside a game.
    Now(Msg),
    News(News),
}

/// A message's place in the log, which neither replacing other messages
/// nor dropping the oldest changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct EntryId(u64);

/// A message of the log.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    id: EntryId,
    content: Content,
    heard: Heard,
}

impl Entry {
    pub fn parts(&self) -> &Msg {
        match &self.content {
            Content::Now(m) => m,
            Content::News(n) => &n.parts,
        }
    }

    /// The category of news, `None` for what was said at once.
    pub fn category(&self) -> Option<Category> {
        match &self.content {
            Content::Now(_) => None,
            Content::News(n) => Some(n.category),
        }
    }

    pub fn heard(&self) -> Heard {
        self.heard
    }

    fn subject(&self) -> Option<&Subject> {
        match &self.content {
            Content::Now(_) => None,
            Content::News(n) => n.subject(),
        }
    }

    fn is_waiting(&self) -> bool {
        matches!(self.heard, Heard::Waiting { .. })
    }

    /// News the player has not heard.
    fn is_unheard_news(&self) -> bool {
        self.category().is_some() && self.heard != Heard::Yes
    }
}

/// What "what did I miss" says next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Missed {
    /// How many unheard messages wait in each category.
    Count,
    /// An unheard message: the newest alert if `first`, else the oldest.
    Steps { first: bool },
}

/// The category the player reviews, and the message of it said last.
#[derive(Clone, Copy, Debug)]
struct Review {
    category: Category,
    at: Option<EntryId>,
}

/// The words for a category.
pub fn category_title(c: Category) -> Msg {
    match c {
        Category::Alert => msg(Phrase::Alert),
        Category::Production => msg(Phrase::ProductionNews),
        Category::Economy => msg(Phrase::EconomyNews),
        Category::Scouting => msg(Phrase::ScoutingNews),
        Category::Chat => msg(Phrase::ChatNews),
        Category::System => msg(Phrase::SystemNews),
        Category::Story => msg(Phrase::StoryNews),
    }
}

/// The voice: the message queue and the log.
pub struct Voice {
    /// Said and unsaid messages, in the order they came.
    log: VecDeque<Entry>,
    next_id: u64,
    /// The first message of the game being played, or of the log.
    game_start: EntryId,
    /// A message of the log is being said (not an answer).
    active: bool,
    /// The player is walking through the log.
    history: bool,
    /// The index of the message being said; the next to say, or
    /// `log.len()`, when none is.
    current: usize,
    missed: Missed,
    review: Review,
    /// The message of the log whose words were said last, as said (newer
    /// news may replace it in the log).
    said_last: Option<Msg>,
    /// The message a review key said last, until an answer to another key
    /// or a message said now.
    reviewed: Option<Msg>,
    /// Inside the game loop: a message said now does not wait.
    in_game: bool,
    /// The counts waiting for the news of their moment to be complete,
    /// one for each subject, in the order they came.
    held: Vec<(News, Policy)>,
    channel: VoiceChannel,
    input: Input,
    clock: ClockRef,
}

impl Voice {
    pub fn new(channel: VoiceChannel, input: Input, clock: ClockRef) -> Voice {
        Voice {
            log: VecDeque::new(),
            next_id: 0,
            game_start: EntryId(0),
            active: false,
            history: false,
            current: 0,
            missed: Missed::Count,
            review: Review {
                category: Category::Alert,
                at: None,
            },
            said_last: None,
            reviewed: None,
            in_game: false,
            held: Vec::new(),
            channel,
            input,
            clock,
        }
    }

    /// Inside the game loop nothing said waits, so that the world, its
    /// sounds and the keys go on: a message said now starts, and the next
    /// key or message cuts it as it cuts an answer. Outside, a message said
    /// now holds the caller until it is said or a key is pressed.
    pub fn set_in_game(&mut self, in_game: bool) {
        self.in_game = in_game;
    }

    /// The index of the next message waiting to be said, or `log.len()`:
    /// the first one of the first category of [`Category::AHEAD`] that has
    /// one, else the first.
    fn next_waiting(&self) -> usize {
        Category::AHEAD
            .iter()
            .find_map(|&c| {
                self.log
                    .iter()
                    .position(|e| e.is_waiting() && e.category() == Some(c))
            })
            .or_else(|| self.log.iter().position(Entry::is_waiting))
            .unwrap_or(self.log.len())
    }

    fn waiting_exists(&self) -> bool {
        self.log.iter().any(Entry::is_waiting)
    }

    fn current_message_is_waiting(&self) -> bool {
        self.log.get(self.current).is_some_and(Entry::is_waiting)
    }

    /// The message being said is over. Under the sound policy, outside the
    /// history, only its sounds were played: its words stay unheard.
    fn mark_current_as_said(&mut self) {
        let history = self.history;
        if let Some(e) = self.log.get_mut(self.current) {
            e.heard = match e.heard {
                Heard::Waiting { sounds_only: true } if !history => Heard::No,
                _ => Heard::Yes,
            };
        }
    }

    fn go_to_next_unsaid(&mut self) {
        self.current = self.next_waiting();
    }

    /// The queued message on the voice channel counts as said if the
    /// player silenced it at the screen reader. Cut off by the game, it
    /// stays unsaid and is said again when the voice is free. A message
    /// still waiting is not touched.
    fn dismiss_current_if_cancelled(&mut self) {
        if self.active && self.current_message_is_waiting() && self.channel.was_cancelled() {
            self.mark_current_as_said();
        }
    }

    fn start_current(&mut self) {
        self.channel.stop();
        self.active = false;
        self.update();
    }

    /// Adds a message to the log.
    fn push(&mut self, content: Content, heard: Heard) {
        self.log.push_back(Entry {
            id: EntryId(self.next_id),
            content,
            heard,
        });
        self.next_id += 1;
        self.trim();
    }

    /// Drops the oldest messages past [`LOG_LIMIT`], unless the oldest is
    /// being said.
    fn trim(&mut self) {
        while self.log.len() > LOG_LIMIT && !(self.active && self.current == 0) {
            self.log.pop_front();
            self.current = self.current.saturating_sub(1);
        }
    }

    /// The index of the newest news about `subject` since the game
    /// started, if the player has not heard it and it is not being said.
    fn unheard_about(&self, subject: &Subject) -> Option<usize> {
        let i = self
            .log
            .iter()
            .rposition(|e| e.subject() == Some(subject))?;
        let being_said = self.active && self.current == i;
        let replaceable = self.log[i].heard != Heard::Yes && self.log[i].id >= self.game_start;
        (replaceable && !being_said).then_some(i)
    }

    /// Says the previous message of the log.
    pub fn previous(&mut self) {
        self.missed = Missed::Count;
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
            if self.current_message_is_waiting() {
                if history_only {
                    return;
                }
                self.mark_current_as_said();
            }
            self.current += 1;
            self.start_current();
        }
    }

    /// Gives up the messages `ids` that wait, which are kept in the log
    /// unheard, and stops what is being said: news from the queue is
    /// skipped as a key skips it, and a walk through the log ends. The next
    /// message waiting starts; none of `ids` does.
    pub fn skip_with(&mut self, ids: &[EntryId]) {
        for e in &mut self.log {
            if e.is_waiting() && ids.contains(&e.id) {
                e.heard = Heard::No;
            }
        }
        if self.active && self.current_message_is_waiting() {
            self.mark_current_as_said();
        }
        self.history = false;
        self.start_current();
    }

    /// Says now, and waits until the end; a key press does not interrupt.
    /// Key presses made meanwhile are dropped.
    pub fn alert(&mut self, msg: &Msg) {
        self.say_now(msg, VoiceKind::Alert, Content::Now(msg.clone()));
    }

    /// Says now, and waits until the end or a key press, which is used up.
    pub fn important(&mut self, msg: &Msg) {
        self.say_now(msg, VoiceKind::Important, Content::Now(msg.clone()));
    }

    /// Says now, and waits until the end or a key press, which stays in the
    /// queue for the caller.
    pub fn confirmation(&mut self, msg: &Msg) {
        self.say_now(msg, VoiceKind::Confirmation, Content::Now(msg.clone()));
    }

    /// Says a menu title: as [`Voice::confirmation`].
    pub fn menu(&mut self, msg: &Msg) {
        self.say_now(msg, VoiceKind::Menu, Content::Now(msg.clone()));
    }

    /// Tells a story now, outside a game (a campaign's cut scene), as its
    /// policy says: each line spoken as [`Voice::important`], or else taken
    /// as other news. A key skips the line being said; Escape skips the
    /// rest of the story, whose lines are kept in the log unheard.
    pub fn tell_now(&mut self, story: Vec<News>, policy: Policy) {
        let mut lines = story.into_iter();
        for news in lines.by_ref() {
            if policy != Policy::Speak {
                self.news(news, policy);
            } else if self.say_now(
                &news.parts.clone(),
                VoiceKind::Important,
                Content::News(news),
            ) {
                break;
            }
        }
        for news in lines.filter(|n| !n.parts.is_empty()) {
            self.push(Content::News(news), Heard::No);
        }
    }

    /// Takes news as `policy` says: queued to be said (or for its recorded
    /// sounds only), kept unheard, or dropped. It joins the unheard news
    /// about the same subject, as it says, in that news's turn. A count
    /// waits for [`Voice::news_complete`], joined with the other counts
    /// about its subject. The place of the news in the log, unless it was
    /// dropped or waits.
    pub fn news(&mut self, news: News, policy: Policy) -> Option<EntryId> {
        if news.rank().is_none() {
            return self.take(news, policy);
        }
        let held = news
            .subject()
            .and_then(|s| self.held.iter().position(|(n, _)| n.subject() == Some(s)));
        match held {
            Some(i) => {
                if let Some(news) = news.join(Some(&self.held[i].0)) {
                    self.held[i] = (news, policy);
                }
            }
            None => self.held.push((news, policy)),
        }
        None
    }

    /// The news of the moment is complete (a turn has ended, or a fight):
    /// the counts held since are taken, in the order of their ranks, each
    /// joined with the unheard news about its subject.
    pub fn news_complete(&mut self) {
        let mut held = std::mem::take(&mut self.held);
        held.sort_by_key(|(n, _)| n.rank());
        for (news, policy) in held {
            self.take(news, policy);
        }
    }

    /// The counts held are not news: what a replay's catch-up saw where it
    /// landed.
    pub fn give_up_held(&mut self) {
        self.held.clear();
    }

    /// The player hears another side, or the same side from another
    /// moment (a watcher moved to another player, a seat back after a lost
    /// connection, a replay's jump): the counts held are given up, and a
    /// newer count replaces the unheard news it would have counted on.
    pub fn new_side(&mut self) {
        self.give_up_held();
        for e in &mut self.log {
            if let Content::News(n) = &mut e.content {
                n.stop_counting();
            }
        }
    }

    fn take(&mut self, news: News, policy: Policy) -> Option<EntryId> {
        if news.parts.is_empty() {
            return None;
        }
        let older = news.subject().and_then(|s| self.unheard_about(s));
        let older_news = older.and_then(|i| match &self.log[i].content {
            Content::News(n) => Some(n),
            Content::Now(_) => None,
        });
        let news = news.join(older_news)?;
        let heard = match policy {
            Policy::Drop => return None,
            Policy::Speak => Heard::Waiting { sounds_only: false },
            Policy::Sound if self.channel.has_sounds(&news.parts) => {
                Heard::Waiting { sounds_only: true }
            }
            Policy::Sound | Policy::Log => Heard::No,
        };
        let id = match older {
            Some(i) => {
                let e = &mut self.log[i];
                e.content = Content::News(news);
                e.heard = heard;
                e.id
            }
            None => {
                self.push(Content::News(news), heard);
                EntryId(self.next_id - 1)
            }
        };
        self.update();
        Some(id)
    }

    /// Whether the message of the log at `id` waits to be said, or is
    /// being said from the queue.
    pub fn is_waiting(&self, id: EntryId) -> bool {
        self.log.iter().any(|e| e.id == id && e.is_waiting())
    }

    /// Says now, cutting the message being said (news cut off is said
    /// again in its turn). Outside a game, waits until the end, or until a
    /// key press unless it is an alert; the key press stays queued after a
    /// confirmation or a menu title. In a game, does not wait. Whether
    /// Escape cut it short.
    fn say_now(&mut self, msg: &Msg, kind: VoiceKind, content: Content) -> bool {
        if msg.is_empty() {
            return false;
        }
        self.dismiss_current_if_cancelled();
        self.channel.play(msg, CENTERED, kind, Parts::All);
        let mut escaped = false;
        if !self.in_game {
            let interruptible = kind != VoiceKind::Alert;
            let keep_key = matches!(kind, VoiceKind::Confirmation | VoiceKind::Menu);
            while self.channel.get_busy() {
                let keys = if interruptible {
                    self.keys_hit(keep_key)
                } else {
                    Vec::new()
                };
                if !keys.is_empty() {
                    escaped = keys.iter().any(|k| k.key == Key::Escape);
                    break;
                }
                self.clock.sleep(0.1);
                self.channel.update();
            }
            if !interruptible {
                self.input.clear_keydowns();
            }
        }
        self.push(content, Heard::Yes);
        self.said_last = Some(msg.clone());
        self.reviewed = None;
        self.go_to_next_unsaid();
        self.active = false;
        escaped
    }

    /// Says now without recording in the log: a menu item, a typed
    /// letter, an answer to a key.
    pub fn item(&mut self, msg: &Msg) {
        self.item_stereo(msg, DEFAULT_VOLUME, DEFAULT_VOLUME);
    }

    pub fn item_stereo(&mut self, msg: &Msg, lv: f64, rv: f64) {
        self.missed = Missed::Count;
        self.reviewed = None;
        self.answer(msg, (lv, rv));
    }

    fn answer(&mut self, msg: &Msg, volumes: (f64, f64)) {
        if !msg.is_empty() {
            self.dismiss_current_if_cancelled();
            self.go_to_next_unsaid();
            self.channel.play(msg, volumes, VoiceKind::Item, Parts::All);
            self.active = false;
            self.history = false;
        }
    }

    /// Says a message of the log again, now, and counts it as heard.
    fn say_entry(&mut self, i: usize) {
        self.dismiss_current_if_cancelled();
        let e = &mut self.log[i];
        e.heard = Heard::Yes;
        let parts = e.parts().clone();
        self.said_last = Some(parts.clone());
        self.reviewed = Some(parts.clone());
        self.go_to_next_unsaid();
        self.channel
            .play(&parts, CENTERED, VoiceKind::History, Parts::All);
        self.active = false;
        self.history = false;
    }

    /// What did I miss: says how many unheard messages wait in each
    /// category that has any. Pressed again, with no other answer between,
    /// says them one at a time: the newest alert first, then the others
    /// oldest first.
    pub fn unheard(&mut self) {
        match self.missed {
            Missed::Count => {
                let mut m = Msg::new();
                for c in Category::ALL {
                    let n = self
                        .log
                        .iter()
                        .filter(|e| e.category() == Some(c) && e.is_unheard_news())
                        .count();
                    if n > 0 {
                        if !m.is_empty() {
                            m.extend(comma());
                        }
                        m.extend(category_title(c));
                        m.extend(nb2msg(n as i64));
                    }
                }
                if m.is_empty() {
                    self.answer(&msg(Phrase::Nothing), CENTERED);
                } else {
                    self.answer(&m, CENTERED);
                    self.missed = Missed::Steps { first: true };
                }
            }
            Missed::Steps { first } => {
                let newest_alert = || {
                    self.log
                        .iter()
                        .rposition(|e| e.is_unheard_news() && e.category() == Some(Category::Alert))
                };
                let next = first
                    .then(newest_alert)
                    .flatten()
                    .or_else(|| self.log.iter().position(Entry::is_unheard_news));
                match next {
                    Some(i) => {
                        self.say_entry(i);
                        self.missed = Missed::Steps { first: false };
                    }
                    None => {
                        self.answer(&msg(Phrase::Nothing), CENTERED);
                        self.missed = Missed::Count;
                    }
                }
            }
        }
    }

    /// Says the message of the reviewed category before the one said last
    /// (the newest at first); the oldest one stays.
    pub fn review_previous(&mut self) {
        self.review(true);
    }

    /// Says the message of the reviewed category after the one said last
    /// (the newest at first); the newest one stays.
    pub fn review_next(&mut self) {
        self.review(false);
    }

    fn review(&mut self, back: bool) {
        self.missed = Missed::Count;
        let c = self.review.category;
        let in_category: Vec<usize> = (0..self.log.len())
            .filter(|&i| self.log[i].category() == Some(c))
            .collect();
        let Some(last) = in_category.len().checked_sub(1) else {
            self.answer(
                &[category_title(c), msg(Phrase::Nothing)].concat(),
                CENTERED,
            );
            return;
        };
        let at = self
            .review
            .at
            .and_then(|id| in_category.iter().position(|&i| self.log[i].id == id));
        let k = match at {
            None => last,
            Some(k) if back => k.saturating_sub(1),
            Some(k) => (k + 1).min(last),
        };
        let i = in_category[k];
        self.review.at = Some(self.log[i].id);
        self.say_entry(i);
    }

    /// Reviews the next category, and says which.
    pub fn review_category(&mut self) {
        self.missed = Missed::Count;
        self.review = Review {
            category: self.review.category.next(),
            at: None,
        };
        self.answer(&category_title(self.review.category), CENTERED);
    }

    /// The copy key: says "copied" and gives the words of the message a
    /// review key said last, or, when the player is not reviewing, of the
    /// message said last; says "nothing" when there are none. A review
    /// goes on after it.
    pub fn copy(&mut self) -> Option<String> {
        let copied = self
            .reviewed
            .as_ref()
            .or(self.said_last.as_ref())
            .map(|m| self.channel.words(m))
            .filter(|w| !w.is_empty());
        let answer = match copied {
            Some(_) => msg(Phrase::Copied),
            None => msg(Phrase::Nothing),
        };
        self.answer(&answer, CENTERED);
        copied
    }

    /// A game starts: news from now on does not replace the news of the
    /// log so far, and the counts held of an earlier game are given up.
    /// The place in the log of the game's first message.
    pub fn start_game(&mut self) -> EntryId {
        self.give_up_held();
        self.game_start = EntryId(self.next_id);
        self.game_start
    }

    /// The newest alert tied to a square, older than the alert `before`
    /// if given.
    pub fn alert_before(&self, before: Option<EntryId>) -> Option<(EntryId, SquareId)> {
        self.log
            .iter()
            .rev()
            .filter(|e| before.is_none_or(|b| e.id < b))
            .find_map(|e| match &e.content {
                Content::News(n) if n.category == Category::Alert => n.square.map(|s| (e.id, s)),
                _ => None,
            })
    }

    /// Moves the queue along: starts the next unsaid message when the
    /// voice is free. Call it often.
    pub fn update(&mut self) {
        if self.channel.get_busy() {
            self.channel.update();
            return;
        }
        if self.active {
            // A message from the queue has just finished.
            self.mark_current_as_said();
            self.current += 1;
            self.active = false;
        }
        self.trim();
        if !self.history {
            self.go_to_next_unsaid();
        }
        match self.log.get(self.current) {
            Some(e) => {
                let which = match e.heard {
                    Heard::Waiting { sounds_only: true } if !self.history => Parts::Sounds,
                    _ => Parts::All,
                };
                let kind = if self.history {
                    VoiceKind::History
                } else {
                    VoiceKind::Info
                };
                let parts = e.parts().clone();
                if which == Parts::All {
                    self.said_last = Some(parts.clone());
                }
                if self.history {
                    self.reviewed = Some(parts.clone());
                }
                self.channel.play(&parts, CENTERED, kind, which);
                self.active = true;
            }
            None => {
                self.active = false;
                self.history = false;
            }
        }
    }

    /// A game is over: the news about it waiting to be said is given up,
    /// and stays in the log unheard. The news said ahead of the others
    /// (what players say, news of the game itself, the story) is still
    /// said, since the end of a game is often what it tells; so is the
    /// message being said. A walk through the history ends with it.
    pub fn end_game(&mut self) {
        let being_said = self.active.then_some(self.current);
        if std::mem::take(&mut self.history)
            && let Some(e) = being_said.and_then(|i| self.log.get_mut(i))
            && e.heard == (Heard::Waiting { sounds_only: true })
        {
            // Said from the history with its words: heard once it ends.
            e.heard = Heard::Waiting { sounds_only: false };
        }
        for (i, e) in self.log.iter_mut().enumerate() {
            let ahead = e.category().is_some_and(|c| Category::AHEAD.contains(&c));
            if e.is_waiting() && !ahead && being_said != Some(i) {
                e.heard = Heard::No;
            }
        }
    }

    /// Stops talking and gives up the queue: the messages waiting stay in
    /// the log, unheard.
    pub fn silent_flush(&mut self) {
        self.channel.stop();
        self.active = false;
        self.current = self.log.len();
        for e in &mut self.log {
            if e.is_waiting() {
                e.heard = Heard::No;
            }
        }
    }

    /// Says every unsaid message, waiting. If `interruptible`, a key press
    /// skips to the next message, or ends the wait after the last one.
    pub fn flush(&mut self, interruptible: bool) {
        loop {
            self.update();
            if !(self.waiting_exists() || self.channel.get_busy()) {
                break;
            } else if interruptible && !self.keys_hit(true).is_empty() {
                if self.waiting_exists() {
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

    /// The keys pressed, put back at the end of the queue if `keep_key`.
    fn keys_hit(&mut self, keep_key: bool) -> Vec<KeyDown> {
        self.input.input_point(InputPoint::Voice, self.is_idle());
        let keys = self.input.get_keydowns();
        if keep_key {
            for k in &keys {
                self.input.post(crate::input::Event::KeyDown(k.clone()));
            }
        }
        keys
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
        !self.channel.get_busy() && !self.waiting_exists()
    }

    /// The log, oldest first.
    pub fn log(&self) -> impl Iterator<Item = &Entry> {
        self.log.iter()
    }
}
