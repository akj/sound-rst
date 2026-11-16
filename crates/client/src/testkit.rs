//! Test support: fake devices that record what the client would say and
//! play, a scripted keyboard, a manual clock, and a harness that builds a
//! complete client on the real game data with them.
//!
//! A test runs the real menus or game interface against [`Harness`] and
//! checks [`Harness::spoken`].

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};

use crate::audio::{AudioOutput, Channel, Sound};
use crate::client::{Client, Desktop, Devices, open_resources};
use crate::clock::{Clock, ManualClock};
use crate::config::Config;
use crate::game::GameInterface;
use crate::input::{InputSource, Key, KeyDown, KeyUp, Mods, RawInput};
use crate::log::Log;
use crate::net::conn::{ConnectError, Connection, Link, NetEvent, Opening, Outgoing};
use crate::net::{NetConfig, Redial, Seated};
use crate::paths::Paths;
use crate::session::Session;
use crate::tts::{Progress, SpeechBackend};
use crate::voice::{Utterance, UtteranceLog, UtterancePart};
use soundrts_protocol::game::{Role, RulesHash, SeatFrame, decode_command, decode_frame};
use soundrts_protocol::{
    ClientMsg, Computer, Faction, Income, LobbyPlayer, LobbySnapshot, Login, MapInfo,
    PROTOCOL_VERSION, RoomId, RoomState, SeatToken, ServerMsg, Slot, SlotPlayer, Speed,
    Whereabouts,
};
use soundrts_world::{Command, Controller, GameSetup, Host, PlayerId, TurnMessage};

/// The imported game data the tests read. Panics with instructions if it
/// is missing, or if another table of names made it.
pub fn game_root() -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/game-data");
    assert!(
        crate::paths::is_root(&root),
        "The imported game data is missing at {}, or is not this checkout's. Run \
         scripts/fetch-upstream.ps1.",
        root.display()
    );
    root
}

/// The checkout of the original, for the tests that read its own test
/// files. Panics with instructions if it is missing.
pub fn upstream_root() -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/soundrts-upstream");
    assert!(
        root.join("res/rules.txt").is_file(),
        "The original game is missing at {}. Run scripts/fetch-upstream.ps1 first.",
        root.display()
    );
    root
}

/// The upstream test data folder.
pub fn upstream_tests_dir() -> PathBuf {
    upstream_root().join("soundrts/tests")
}

/// The original's small test packages, imported: `res`, `res.zip`,
/// `jl1.txt` and what `single` holds, under the same names. They are
/// imported once for each test program, into a temporary folder.
pub fn imported_tests_dir() -> PathBuf {
    static DIR: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let dir = DIR.get_or_init(|| {
        let dir = tempfile::tempdir().expect("temporary folder");
        let from = upstream_tests_dir();
        std::fs::create_dir_all(dir.path().join("single")).expect("folder");
        for name in [
            "res",
            "res.zip",
            "jl1.txt",
            "single/map1",
            "single/map1.zip",
            "single/campaign1",
            "single/campaign1.zip",
        ] {
            soundrts_import::import(&from.join(name), &dir.path().join(name))
                .unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        dir
    });
    dir.path().to_path_buf()
}

/// A copy of upstream's `tests/res2` folder in a temporary folder, with the
/// empty `mods/mod2` and `mods/sound1` folders that `res2.zip` has and git
/// cannot keep.
pub fn upstream_res2_folder() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temporary folder");
    let res2 = dir.path().join("res2");
    let source = upstream_tests_dir().join("res2");
    for (from, to) in [
        ("readme.txt", "readme.txt"),
        ("mods/mod1/rules.txt", "mods/mod1/rules.txt"),
    ] {
        let to = res2.join(to);
        std::fs::create_dir_all(to.parent().expect("parent")).expect("folder");
        std::fs::copy(source.join(from), to).expect("copy");
    }
    for empty in ["mods/mod2", "mods/sound1"] {
        std::fs::create_dir_all(res2.join(empty)).expect("folder");
    }
    dir
}

/// What the speech backend was asked to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpeechEvent {
    Speak(String),
    Stop,
}

/// The record of a [`RecordingSpeech`].
#[derive(Clone, Default)]
pub struct SpeechLog {
    events: Rc<RefCell<Vec<SpeechEvent>>>,
    progress: Rc<Cell<Progress>>,
}

impl SpeechLog {
    pub fn events(&self) -> Vec<SpeechEvent> {
        self.events.borrow().clone()
    }

    /// The texts spoken, in order.
    pub fn spoken(&self) -> Vec<String> {
        self.events
            .borrow()
            .iter()
            .filter_map(|e| match e {
                SpeechEvent::Speak(t) => Some(t.clone()),
                SpeechEvent::Stop => None,
            })
            .collect()
    }

    pub fn clear(&self) {
        self.events.borrow_mut().clear();
    }

    /// Reports `progress` for the last utterance, as a screen reader that
    /// tells when speech ends would. Until a test reports something, the
    /// device cannot tell, and the end is estimated.
    pub fn report(&self, progress: Progress) {
        self.progress.set(progress);
    }
}

/// A speech backend that records every utterance and stop.
pub struct RecordingSpeech {
    log: SpeechLog,
}

impl RecordingSpeech {
    pub fn new() -> (RecordingSpeech, SpeechLog) {
        let log = SpeechLog::default();
        (RecordingSpeech { log: log.clone() }, log)
    }
}

impl SpeechBackend for RecordingSpeech {
    fn speak(&mut self, text: &str) -> Result<(), String> {
        self.log
            .events
            .borrow_mut()
            .push(SpeechEvent::Speak(text.to_owned()));
        self.log.progress.set(Progress::Unknown);
        Ok(())
    }

    fn stop(&mut self) -> Result<(), String> {
        self.log.events.borrow_mut().push(SpeechEvent::Stop);
        self.log.progress.set(Progress::Unknown);
        Ok(())
    }

    fn progress(&self) -> Progress {
        self.log.progress.get()
    }
}

/// What the audio output was asked to do.
#[derive(Clone, Debug, PartialEq)]
pub enum AudioEvent {
    /// A sound started, with the channel's gains and the sound's own volume.
    Play {
        channel: usize,
        sound: String,
        looping: bool,
        left: f32,
        right: f32,
        volume: f32,
    },
    Gains {
        channel: usize,
        left: f32,
        right: f32,
        behind: f32,
    },
    Stop {
        channel: usize,
    },
}

#[derive(Default)]
struct AudioRecord {
    events: Vec<AudioEvent>,
    gains: HashMap<usize, (f32, f32)>,
    /// When each busy channel ends (`None` for a loop).
    playing: HashMap<usize, Option<f64>>,
}

/// The record of a [`RecordingAudio`].
#[derive(Clone, Default)]
pub struct AudioLog(Rc<RefCell<AudioRecord>>);

impl AudioLog {
    pub fn events(&self) -> Vec<AudioEvent> {
        self.0.borrow().events.clone()
    }

    /// The names of the sounds started, in order, with their channels.
    pub fn played(&self) -> Vec<(usize, String)> {
        self.0
            .borrow()
            .events
            .iter()
            .filter_map(|e| match e {
                AudioEvent::Play { channel, sound, .. } => Some((*channel, sound.clone())),
                _ => None,
            })
            .collect()
    }

    pub fn clear(&self) {
        self.0.borrow_mut().events.clear();
    }
}

/// An audio output that records what would play. A sound keeps its
/// channel busy for its real duration on the test clock.
pub struct RecordingAudio {
    log: AudioLog,
    clock: Rc<dyn Clock>,
}

impl RecordingAudio {
    pub fn new(clock: Rc<dyn Clock>) -> (RecordingAudio, AudioLog) {
        let log = AudioLog::default();
        (
            RecordingAudio {
                log: log.clone(),
                clock,
            },
            log,
        )
    }
}

impl AudioOutput for RecordingAudio {
    fn play(&mut self, channel: Channel, sound: &Sound, looping: bool) {
        let mut r = self.log.0.borrow_mut();
        let ch = channel.index();
        let (left, right) = r.gains.get(&ch).copied().unwrap_or((1.0, 1.0));
        r.events.push(AudioEvent::Play {
            channel: ch,
            sound: sound.key.clone(),
            looping,
            left,
            right,
            volume: sound.volume(),
        });
        let end = (!looping).then(|| self.clock.now() + sound.duration());
        r.playing.insert(ch, end);
    }

    fn set_gains(&mut self, channel: Channel, left: f32, right: f32, behind: f32) {
        let mut r = self.log.0.borrow_mut();
        r.gains.insert(channel.index(), (left, right));
        r.events.push(AudioEvent::Gains {
            channel: channel.index(),
            left,
            right,
            behind,
        });
    }

    fn stop(&mut self, channel: Channel) {
        let mut r = self.log.0.borrow_mut();
        if r.playing.remove(&channel.index()).is_some() {
            r.events.push(AudioEvent::Stop {
                channel: channel.index(),
            });
        }
    }

    fn is_busy(&self, channel: Channel) -> bool {
        let r = self.log.0.borrow();
        match r.playing.get(&channel.index()) {
            Some(None) => true,
            Some(Some(end)) => *end > self.clock.now(),
            None => false,
        }
    }
}

/// A key press as the window would deliver it, typing what the key types
/// on a US layout.
pub fn down(key: Key) -> KeyDown {
    press(key, Mods::NONE)
}

/// A key press with modifiers. Letters typed with Shift are upper case.
pub fn press(key: Key, mods: Mods) -> KeyDown {
    let unicode = match key {
        Key::Return | Key::KpEnter => "\r".to_owned(),
        Key::Tab => "\t".to_owned(),
        Key::Escape => "\u{1b}".to_owned(),
        Key::Backspace => "\u{8}".to_owned(),
        Key::Space => " ".to_owned(),
        k => match us_char(k) {
            Some(c) if mods.shift => c.to_uppercase().collect(),
            Some(c) => c.to_string(),
            None => String::new(),
        },
    };
    KeyDown {
        key,
        physical: key,
        mods,
        unicode,
    }
}

fn us_char(key: Key) -> Option<char> {
    ('a'..='z')
        .chain('0'..='9')
        .chain("-=[]\\;'`,./".chars())
        .find(|&c| Key::from_char(c) == key)
}

/// Key presses scheduled while the client runs, by a game hook for
/// example. Clones share the same queue, kept in time order.
#[derive(Clone, Default)]
pub struct LiveKeys(Rc<RefCell<VecDeque<(f64, RawInput)>>>);

impl LiveKeys {
    fn insert(&self, time: f64, e: RawInput) {
        let mut q = self.0.borrow_mut();
        let i = q.iter().position(|(t, _)| *t > time).unwrap_or(q.len());
        q.insert(i, (time, e));
    }

    /// A press and release at `time`.
    pub fn tap_at(&self, time: f64, key: Key, mods: Mods) {
        self.down_at(time, key, mods);
        self.up_at(time, key);
    }

    /// A press at `time`; the key stays down until [`LiveKeys::up_at`].
    pub fn down_at(&self, time: f64, key: Key, mods: Mods) {
        self.insert(time, RawInput::KeyDown(press(key, mods)));
    }

    /// A release at `time`.
    pub fn up_at(&self, time: f64, key: Key) {
        self.insert(time, RawInput::KeyUp(KeyUp { key, physical: key }));
    }

    /// Types each character as a press of its key, `interval` seconds
    /// apart from `time` on.
    pub fn type_at(&self, time: f64, interval: f64, text: &str) {
        for (i, c) in text.chars().enumerate() {
            let t = time + i as f64 * interval;
            let key = Key::from_char(c);
            let mods = if c.is_ascii_uppercase() {
                Mods::SHIFT
            } else {
                Mods::NONE
            };
            let mut e = press(key, mods);
            e.unicode = c.to_string();
            self.insert(t, RawInput::KeyDown(e));
            self.insert(t, RawInput::KeyUp(KeyUp { key, physical: key }));
        }
    }

    /// When the next scheduled key comes, if any.
    pub fn next_time(&self) -> Option<f64> {
        self.0.borrow().front().map(|(t, _)| *t)
    }

    /// Whether every scheduled key has been delivered.
    pub fn is_empty(&self) -> bool {
        self.0.borrow().is_empty()
    }
}

/// A timed list of key presses and releases, and points where the keys
/// wait until something has been said.
#[derive(Clone, Default)]
pub struct Script {
    /// The keys, each in the part of the script after its number of waits.
    events: VecDeque<(usize, f64, RawInput)>,
    waits: Vec<Wait>,
    time: f64,
    repeat: Option<Repeat>,
    keep_open: bool,
    live: Option<LiveKeys>,
    deadline: Option<f64>,
    progress: Option<Arc<AtomicUsize>>,
}

/// A point of a script that waits until an utterance is said.
#[derive(Clone, Debug)]
struct Wait {
    text: String,
    /// The whole utterance, rendered, must be `text`; else it contains it.
    exact: bool,
}

/// How long, in real time, a script waits for an utterance before the
/// test fails. Tests that wait run against a real server.
pub const WAIT_LIMIT: std::time::Duration = std::time::Duration::from_secs(30);

/// How late a key after a wait may come before the keys after it are
/// delayed as much.
const LATE: f64 = 0.05;

impl Script {
    pub fn new() -> Script {
        Script::default()
    }

    /// The window stays open after the last event, for a test that posts
    /// its own key presses or steps a game itself.
    pub fn keep_open(mut self) -> Script {
        self.keep_open = true;
        self
    }

    /// Closes the window at `time` whatever happens, so that a stuck test
    /// ends instead of waiting forever.
    pub fn deadline(mut self, time: f64) -> Script {
        self.deadline = Some(time);
        self
    }

    /// Held keys repeat as a system repeats them: the last key pressed,
    /// `delay` seconds after it goes down, then every `interval` seconds
    /// until it is released. Without this, held keys do not repeat.
    pub fn repeat(mut self, delay: f64, interval: f64) -> Script {
        self.repeat = Some(Repeat { delay, interval });
        self
    }

    /// Also delivers the keys scheduled on `live`, in time order with the
    /// script's.
    pub fn with_live(mut self, live: LiveKeys) -> Script {
        self.live = Some(live);
        self
    }

    /// The following events happen at `time` seconds (on the test clock),
    /// counted from the last wait if there is one.
    pub fn at(mut self, time: f64) -> Script {
        self.time = time;
        self
    }

    /// The following keys wait until an utterance containing `text` (as
    /// [`render`] writes it) is said after the previous wait, at most
    /// [`WAIT_LIMIT`] of real time; their times count from then.
    pub fn wait_for(self, text: &str) -> Script {
        self.wait(text, false)
    }

    /// As [`Script::wait_for`], for an utterance that is exactly `text`.
    pub fn wait_for_exact(self, text: &str) -> Script {
        self.wait(text, true)
    }

    /// The number of waits so far: the value `progress` reaches when the
    /// last one is over.
    pub fn wait_count(&self) -> usize {
        self.waits.len()
    }

    /// Counts the waits that are over in `progress`, which another thread
    /// can read.
    pub fn progress(mut self, progress: Arc<AtomicUsize>) -> Script {
        self.progress = Some(progress);
        self
    }

    fn wait(mut self, text: &str, exact: bool) -> Script {
        self.waits.push(Wait {
            text: text.to_owned(),
            exact,
        });
        self.time = 0.0;
        self
    }

    fn push(&mut self, e: RawInput) {
        self.events.push_back((self.waits.len(), self.time, e));
    }

    pub fn key_down(mut self, key: Key) -> Script {
        self.push(RawInput::KeyDown(down(key)));
        self
    }

    pub fn key_up(mut self, key: Key) -> Script {
        self.push(RawInput::KeyUp(KeyUp { key, physical: key }));
        self
    }

    /// A press and release.
    pub fn tap(self, key: Key) -> Script {
        self.tap_with(key, Mods::NONE)
    }

    pub fn tap_with(mut self, key: Key, mods: Mods) -> Script {
        self.push(RawInput::KeyDown(press(key, mods)));
        self.key_up(key)
    }

    /// A press and release of a fully described key press (another layout,
    /// other typed text).
    pub fn tap_event(mut self, e: KeyDown) -> Script {
        let up = KeyUp {
            key: e.key,
            physical: e.physical,
        };
        self.push(RawInput::KeyDown(e));
        self.push(RawInput::KeyUp(up));
        self
    }

    /// Types each character as a press of its key.
    pub fn type_text(mut self, text: &str) -> Script {
        for c in text.chars() {
            let key = Key::from_char(c);
            let mods = if c.is_ascii_uppercase() {
                Mods::SHIFT
            } else {
                Mods::NONE
            };
            let mut e = press(key, mods);
            e.unicode = c.to_string();
            self = self.tap_event(e);
        }
        self
    }

    pub fn source(self) -> ScriptedInput {
        ScriptedInput {
            events: self.events,
            waits: self.waits,
            satisfied: 0,
            offsets: vec![0.0],
            waiting_since: None,
            heard_from: 0,
            utterances: None,
            progress: self.progress,
            live: self.live,
            deadline: self.deadline,
            last_time: 0.0,
            quit_sent: self.keep_open,
            repeat: self.repeat,
            held: None,
        }
    }
}

/// A system's key repeat.
#[derive(Clone, Copy, Debug)]
struct Repeat {
    delay: f64,
    interval: f64,
}

/// Plays a [`Script`]. Once it has run out, and time has moved past its
/// last event, the window "closes" so that every blocking loop ends.
pub struct ScriptedInput {
    events: VecDeque<(usize, f64, RawInput)>,
    waits: Vec<Wait>,
    /// How many waits are over.
    satisfied: usize,
    /// When each part of the script started: 0, then when each wait ended.
    offsets: Vec<f64>,
    waiting_since: Option<std::time::Instant>,
    /// The utterances before this one are not looked at by the next wait.
    heard_from: usize,
    utterances: Option<UtteranceLog>,
    progress: Option<Arc<AtomicUsize>>,
    live: Option<LiveKeys>,
    deadline: Option<f64>,
    last_time: f64,
    quit_sent: bool,
    repeat: Option<Repeat>,
    /// The key that repeats, and when it repeats next.
    held: Option<(KeyDown, f64)>,
}

impl ScriptedInput {
    /// Where the waits look for what was said.
    pub fn listen(&mut self, utterances: UtteranceLog) {
        self.utterances = Some(utterances);
    }

    /// Ends the waits that are met; panics on a wait that has taken too
    /// long.
    fn end_waits(&mut self, now: f64) {
        while self.satisfied < self.waits.len() {
            if self
                .events
                .front()
                .is_some_and(|(part, _, _)| *part <= self.satisfied)
            {
                return;
            }
            let wait = &self.waits[self.satisfied];
            let log = self
                .utterances
                .as_ref()
                .expect("a script that waits listens");
            let said: Vec<String> = log.utterances().iter().map(render).collect();
            let found = said.iter().skip(self.heard_from).position(|s| {
                if wait.exact {
                    *s == wait.text
                } else {
                    s.contains(&wait.text)
                }
            });
            match found {
                Some(i) => {
                    self.heard_from += i + 1;
                    self.satisfied += 1;
                    self.offsets.push(now);
                    self.last_time = self.last_time.max(now);
                    self.waiting_since = None;
                    if let Some(p) = &self.progress {
                        p.store(self.satisfied, std::sync::atomic::Ordering::SeqCst);
                    }
                }
                None => {
                    let since = *self
                        .waiting_since
                        .get_or_insert_with(std::time::Instant::now);
                    if since.elapsed() > WAIT_LIMIT {
                        let tail = &said[said.len().saturating_sub(30)..];
                        panic!(
                            "waited too long to hear {:?}; the last utterances: {tail:#?}",
                            wait.text
                        );
                    }
                    return;
                }
            }
        }
    }

    /// When the next key is due, if its part of the script has started.
    fn next_due(&self) -> Option<f64> {
        self.events
            .front()
            .filter(|(part, _, _)| *part <= self.satisfied)
            .map(|(part, t, _)| self.offsets[*part] + t)
    }

    /// The held key's repeat, if it is due by `now` and before the next
    /// key.
    fn next_repeat(&mut self, now: f64) -> Option<(f64, RawInput)> {
        let interval = self.repeat?.interval;
        let live = self.live.as_ref().and_then(LiveKeys::next_time);
        let next_key = [self.next_due(), live]
            .into_iter()
            .flatten()
            .reduce(f64::min);
        let (e, at) = self.held.as_mut()?;
        if *at > now || next_key.is_some_and(|t| t <= *at) {
            return None;
        }
        let time = *at;
        *at += interval;
        Some((time, RawInput::KeyRepeat(e.clone())))
    }

    /// Notes which key is held after `e`.
    fn hold(&mut self, time: f64, e: &RawInput) {
        match e {
            RawInput::KeyDown(k) => {
                self.held = self.repeat.map(|r| (k.clone(), time + r.delay));
            }
            RawInput::KeyUp(k) => {
                if self
                    .held
                    .as_ref()
                    .is_some_and(|(h, _)| h.physical == k.physical)
                {
                    self.held = None;
                }
            }
            RawInput::FocusLost | RawInput::Quit => self.held = None,
            RawInput::KeyRepeat(_) => {}
        }
    }

    /// The next scripted or live input due by `now`.
    fn next_key(&mut self, now: f64) -> Option<(f64, RawInput)> {
        let waiting = self.satisfied < self.waits.len();
        let script_first = self.next_due();
        if let Some(live) = &self.live {
            let mut q = live.0.borrow_mut();
            let live_first = q.front().map(|(t, _)| *t);
            if let Some(t) = live_first
                && t <= now
                && script_first.is_none_or(|s| t < s)
            {
                self.last_time = self.last_time.max(t);
                return q.pop_front();
            }
            if live_first.is_some() && script_first.is_none() {
                return None;
            }
        }
        match script_first {
            Some(t) if t <= now => {
                let (part, _, e) = self.events.pop_front().expect("a key is due");
                // After a wait, the clock may be real: a key that comes
                // late delays the keys after it, so that a stalled client
                // does not get them all at once (a menu drops keys that
                // pile up).
                if part > 0 && now - t > LATE {
                    self.offsets[part] += now - t;
                }
                self.last_time = t;
                Some((t, e))
            }
            Some(_) => None,
            None if self.events.is_empty()
                && !waiting
                && !self.quit_sent
                && now > self.last_time =>
            {
                self.quit_sent = true;
                Some((now, RawInput::Quit))
            }
            None => None,
        }
    }
}

impl InputSource for ScriptedInput {
    fn next(&mut self, now: f64) -> Option<(f64, RawInput)> {
        if self.deadline.is_some_and(|d| now > d) {
            self.deadline = None;
            self.events.clear();
            if let Some(live) = &self.live {
                live.0.borrow_mut().clear();
            }
            self.quit_sent = true;
            self.held = None;
            return Some((now, RawInput::Quit));
        }
        self.end_waits(now);
        if let Some(repeat) = self.next_repeat(now) {
            return Some(repeat);
        }
        let next = self.next_key(now);
        if let Some((time, e)) = &next {
            self.hold(*time, e);
        }
        next
    }
}

/// A desktop that records titles, opened folders and the clipboard. The
/// clipboard record can be shared with another thread; its last text is
/// what a paste gets.
#[derive(Default)]
pub struct RecordingDesktop {
    pub titles: RefCell<Vec<String>>,
    pub opened: RefCell<Vec<PathBuf>>,
    pub clipboard: Arc<Mutex<Vec<String>>>,
}

impl Desktop for RecordingDesktop {
    fn set_title(&self, title: &str) {
        self.titles.borrow_mut().push(title.to_owned());
    }

    fn open_folder(&self, path: &Path) {
        self.opened.borrow_mut().push(path.to_path_buf());
    }

    fn set_clipboard(&self, text: &str) {
        self.clipboard
            .lock()
            .expect("clipboard record")
            .push(text.to_owned());
    }

    fn clipboard_text(&self) -> Option<String> {
        self.clipboard
            .lock()
            .expect("clipboard record")
            .last()
            .cloned()
    }
}

/// A server that is only a script: it delivers messages at set times on
/// the test clock and records what the client sends. Clones share the
/// same server.
#[derive(Clone)]
pub struct FakeServer(Rc<RefCell<FakeState>>);

struct FakeState {
    clock: Rc<dyn Clock>,
    /// In time order; equal times keep their order.
    scheduled: VecDeque<(f64, NetEvent)>,
    sent: Vec<Outgoing>,
    /// What each connection the client opens again finds, in order: a
    /// server and its welcome, or why it cannot be reached. Once none is
    /// left, none can be.
    dials: VecDeque<Result<(FakeServer, LobbySnapshot), ConnectError>>,
}

struct FakeLink(FakeServer);

impl Link for FakeLink {
    fn send(&mut self, out: Outgoing) {
        self.0.0.borrow_mut().sent.push(out);
    }

    fn poll(&mut self) -> Option<NetEvent> {
        let mut s = self.0.0.borrow_mut();
        let now = s.clock.now();
        match s.scheduled.front() {
            Some((t, _)) if *t <= now => s.scheduled.pop_front().map(|(_, e)| e),
            _ => None,
        }
    }
}

impl FakeServer {
    /// A server, and the client's connection to it.
    pub fn new(clock: Rc<dyn Clock>) -> (FakeServer, Connection) {
        let server = FakeServer(Rc::new(RefCell::new(FakeState {
            clock,
            scheduled: VecDeque::new(),
            sent: Vec::new(),
            dials: VecDeque::new(),
        })));
        let conn = Connection::new(Box::new(FakeLink(server.clone())));
        (server, conn)
    }

    /// What opens the client's connection again: each time, the next of
    /// [`FakeServer::dial_to`] and [`FakeServer::dial_fails`].
    pub fn redial(&self) -> Redial {
        let server = self.clone();
        Box::new(move || {
            let next = server.0.borrow_mut().dials.pop_front();
            Opening::done(match next {
                Some(Ok((s, snapshot))) => Ok((Connection::new(Box::new(FakeLink(s))), snapshot)),
                Some(Err(e)) => Err(e),
                None => Err(ConnectError::Unreachable(
                    "no server, as the test has it".into(),
                )),
            })
        })
    }

    /// The next connection the client opens again reaches a new server,
    /// which welcomes it with `snapshot`: returned, to script what it
    /// sends.
    pub fn dial_to(&self, snapshot: LobbySnapshot) -> FakeServer {
        let clock = self.0.borrow().clock.clone();
        let (back, _) = FakeServer::new(clock);
        self.0
            .borrow_mut()
            .dials
            .push_back(Ok((back.clone(), snapshot)));
        back
    }

    /// The next connection the client opens again cannot reach the server.
    pub fn dial_fails(&self) {
        let e = ConnectError::Unreachable("refused, as the test has it".into());
        self.0.borrow_mut().dials.push_back(Err(e));
    }

    fn schedule(&self, time: f64, e: NetEvent) {
        let mut s = self.0.borrow_mut();
        let i = s
            .scheduled
            .iter()
            .position(|(t, _)| *t > time)
            .unwrap_or(s.scheduled.len());
        s.scheduled.insert(i, (time, e));
    }

    /// Sends `msg` at `time` on the test clock.
    pub fn at(&self, time: f64, msg: ServerMsg) {
        self.schedule(time, NetEvent::Msg(msg));
    }

    /// Sends a frame of a game's stream at `time`.
    pub fn frame_at(&self, time: f64, frame: SeatFrame) {
        self.schedule(time, NetEvent::Frame(Ok(Box::new(frame))));
    }

    /// Sends a binary frame at `time`, as the client's connection reads
    /// it.
    pub fn bytes_at(&self, time: f64, bytes: &[u8]) {
        self.schedule(time, NetEvent::Frame(decode_frame(bytes).map(Box::new)));
    }

    /// Closes the connection at `time`.
    pub fn close_at(&self, time: f64) {
        self.schedule(time, NetEvent::Closed("closed by the test".into()));
    }

    /// The messages the client has sent so far.
    pub fn sent(&self) -> Vec<ClientMsg> {
        self.0
            .borrow()
            .sent
            .iter()
            .filter_map(|o| match o {
                Outgoing::Msg(m) => Some(m.clone()),
                Outgoing::Command(_) => None,
            })
            .collect()
    }

    /// The commands the client has sent so far, with their bytes.
    pub fn commands(&self) -> Vec<(Command, usize)> {
        self.0
            .borrow()
            .sent
            .iter()
            .filter_map(|o| match o {
                Outgoing::Command(b) => Some((decode_command(b).expect("a command"), b.len())),
                Outgoing::Msg(_) => None,
            })
            .collect()
    }
}

/// How long a fake server says it holds a seat whose connection is lost.
pub const FAKE_HOLD: std::time::Duration = std::time::Duration::from_secs(30);

/// A game run here as a server runs one, for a fake server to send its
/// stream to the seat of slot `you`, or to a watcher following that slot's
/// player: the frames a server sends.
pub struct FakeStream {
    host: Host,
    /// The hash of the rules text the game was built from.
    rules: RulesHash,
    you: Role,
    seat: PlayerId,
    last: TurnMessage,
}

impl FakeStream {
    /// The game on the multiplayer map `map` with these slots, built as a
    /// server builds it from the texts the client would send.
    pub fn new(h: &mut Harness, map: &str, slots: &[Slot], you: usize, seed: i64) -> FakeStream {
        FakeStream::with_rules(h, map, slots, you, seed, |rules| rules)
    }

    /// As [`FakeStream::new`], for a game built from the client's rules
    /// text as `edit` changes it.
    pub fn with_rules(
        h: &mut Harness,
        map: &str,
        slots: &[Slot],
        you: usize,
        seed: i64,
        edit: impl FnOnce(String) -> String,
    ) -> FakeStream {
        let m = h.multiplayer_map(map);
        let texts = crate::multiplayer::game_texts(&mut h.client, &m).expect("the texts");
        let rules = edit(texts.rules.into_string());
        let setup = GameSetup::multiplayer(
            rules.clone(),
            texts.ai.into_string(),
            texts.map.into_string(),
            seed,
            slots.iter().map(Slot::controller).collect(),
        );
        let host = Host::new(&setup).expect("a game");
        let seat = host.seats()[you];
        let last = host.message(seat);
        FakeStream {
            host,
            rules: RulesHash::of(&rules),
            you: Role::Player(you),
            seat,
            last,
        }
    }

    /// The stream of a watcher following the player of slot `you`.
    pub fn watching(mut self) -> FakeStream {
        self.you = Role::Watcher;
        self
    }

    /// The watcher follows the player of `slot` from now on: what a
    /// server sends it, that player's whole message as it is now.
    pub fn follow(&mut self, slot: usize) -> SeatFrame {
        self.seat = self.host.seats()[slot];
        self.last = self.host.message(self.seat);
        SeatFrame::Whole(self.last.clone())
    }

    /// What a server sends the seat when the game starts: its seat, the
    /// start and its whole message.
    pub fn opening(&self, map: MapInfo, speed: Speed) -> Vec<SeatFrame> {
        vec![
            SeatFrame::Seated {
                you: self.you,
                map,
                speed,
                paused: false,
                hold: FAKE_HOLD,
                token: SeatToken::from_bits(1),
            },
            self.start(),
            SeatFrame::Whole(self.last.clone()),
        ]
    }

    /// Plays a turn, with the seat's `commands`, and gives the frames a
    /// server sends the seat for it.
    pub fn turn(&mut self, commands: Vec<Command>) -> Vec<SeatFrame> {
        for c in commands {
            let _ = self.host.queue(self.seat, c);
        }
        let turn = self
            .host
            .step_seats(&[self.seat])
            .pop()
            .expect("the seat's turn");
        let frames = turn
            .changes(&self.last, [])
            .into_iter()
            .map(SeatFrame::Change)
            .collect();
        self.last = (*turn.message).clone();
        frames
    }

    /// What the seat is sent when the game is over for it.
    /// The seat's message as of the last turn played.
    pub fn message(&self) -> &TurnMessage {
        &self.last
    }

    pub fn end(&self) -> SeatFrame {
        SeatFrame::End(self.host.game_end(self.seat))
    }

    pub fn host(&self) -> &Host {
        &self.host
    }

    /// The start of the game, as a server sends it.
    pub fn start(&self) -> SeatFrame {
        SeatFrame::Start {
            start: self.host.game_start().message(),
            rules: self.rules,
        }
    }
}

/// A slot of a room, ready, with a random faction.
pub fn slot(player: SlotPlayer, alliance: u8) -> Slot {
    Slot {
        player,
        faction: Faction::random(),
        alliance,
        ready: true,
        income: Income::FULL,
    }
}

/// A lobby where `you` has just arrived, with `others` there before.
pub fn snapshot(you: &str, others: &[&str]) -> LobbySnapshot {
    let player = |l: &str| LobbyPlayer {
        login: Login::new(l).expect("a login"),
        whereabouts: Whereabouts::Lobby,
    };
    LobbySnapshot {
        server_name: soundrts_protocol::ServerName::new("test").expect("a server name"),
        you: Login::new(you).expect("a login"),
        players: std::iter::once(you)
            .chain(others.iter().copied())
            .map(player)
            .collect(),
        rooms: Vec::new(),
        chat: Vec::new(),
    }
}

/// Room 1, hosted by the first human of `slots`, on `map`, whose game is
/// played.
pub fn fake_room(map: MapInfo, slots: Vec<Slot>) -> RoomState {
    let host = slots
        .iter()
        .find_map(|s| match &s.player {
            SlotPlayer::Human(l) => Some(l.clone()),
            SlotPlayer::Computer(_) => None,
        })
        .expect("a human");
    RoomState {
        id: RoomId(1),
        host,
        map,
        speed: Speed::Normal,
        open: true,
        content: soundrts_protocol::ContentHash::new("x").expect("a hash"),
        takes_watchers: true,
        watchers: Vec::new(),
        slots,
    }
}

/// The lobby a connection opened again by `you` is welcomed to: the game
/// of `room` runs, with `playing` still playing it, as far as the server
/// knows.
pub fn welcome_back(you: &str, room: &RoomState, playing: &[&str]) -> LobbySnapshot {
    let mut s = snapshot(you, playing);
    for p in &mut s.players {
        if playing.contains(&p.login.as_str()) {
            p.whereabouts = Whereabouts::Playing(room.id);
        }
    }
    s.rooms = vec![soundrts_protocol::RoomSummary {
        id: room.id,
        host: room.host.clone(),
        map: room.map.clone(),
        speed: room.speed,
        open: room.open,
        content: room.content.clone(),
        players: room.slots.iter().map(|s| s.player.clone()).collect(),
        started: true,
        takes_watchers: true,
    }];
    s
}

/// The settings that keep tests off the network: a hosted server listens
/// on 127.0.0.1 on a free port, with no router and no lists to fetch.
pub fn test_net_config() -> NetConfig {
    NetConfig {
        host_ip: std::net::Ipv4Addr::LOCALHOST.into(),
        host_port: 0,
        upnp: false,
        discovery_targets: Vec::new(),
        public_list_url: None,
        liveness: crate::net::conn::Liveness::default(),
        trust: crate::net::conn::Trust::built_in(),
    }
}

/// A complete client on the upstream game data, with recording devices, a
/// manual clock and a temporary user folder.
pub struct Harness {
    pub client: Client,
    pub speech: SpeechLog,
    pub audio: AudioLog,
    pub clock: Rc<ManualClock>,
    pub desktop: Rc<RecordingDesktop>,
    /// Every utterance the voice started.
    pub utterance_log: UtteranceLog,
    /// The player's folder; deleted with the harness.
    pub user_dir: tempfile::TempDir,
}

impl Harness {
    /// A client speaking `language`, reading `script` as its keyboard.
    pub fn new(language: &str, script: Script) -> Harness {
        Self::with_config(language, script, Config::default())
    }

    pub fn with_config(language: &str, script: Script, config: Config) -> Harness {
        let user_dir = tempfile::tempdir().expect("temporary folder");
        let paths = Paths {
            root: game_root(),
            user: user_dir.path().to_path_buf(),
        };
        paths.create_user_folders(&Log::silent());
        let log = Log::file_only(&paths.client_log());
        let clock = Rc::new(ManualClock::new());
        let res = open_resources(
            &paths,
            config.parameters.clone(),
            &config.mods,
            &config.soundpacks,
            Some(language.to_owned()),
            &log,
        );
        let (speech_backend, speech) = RecordingSpeech::new();
        let (audio_output, audio) = RecordingAudio::new(clock.clone());
        let desktop = Rc::new(RecordingDesktop::default());
        let utterance_log = UtteranceLog::default();
        let mut source = script.source();
        source.listen(utterance_log.clone());
        let devices = Devices {
            speech: Box::new(speech_backend),
            audio: Box::new(audio_output),
            input: Box::new(source),
            clock: clock.clone(),
            desktop: desktop.clone(),
            random_seed: 1,
            utterances: Some(utterance_log.clone()),
        };
        let mut client = Client::new(paths, config, res, log, devices);
        client.net_config = test_net_config();
        Harness {
            client,
            speech,
            audio,
            clock,
            desktop,
            utterance_log,
            user_dir,
        }
    }

    /// The texts spoken so far, in order.
    pub fn spoken(&self) -> Vec<String> {
        self.speech.spoken()
    }

    /// The utterances started on the voice channel so far, in order: each
    /// one as its texts and the names of its recorded sounds.
    pub fn utterances(&self) -> Vec<Utterance> {
        self.utterance_log.utterances()
    }

    /// Presses a key now, as the window would deliver it (without repeats).
    pub fn press(&self, key: Key, mods: Mods) {
        self.client
            .input
            .post(crate::input::Event::KeyDown(press(key, mods)));
    }

    /// The client log so far (`<user>/tmp/client.log`).
    pub fn log_text(&self) -> String {
        std::fs::read_to_string(self.client.paths.client_log()).unwrap_or_default()
    }

    /// The text of `key` in the active language's `tts.txt`.
    pub fn text(&self, key: &str) -> String {
        self.client
            .sounds
            .text(key)
            .unwrap_or_else(|| panic!("no text for {key}"))
    }

    /// The utterances started since the `from`th, each written as by
    /// [`render`].
    pub fn said_since(&self, from: usize) -> Vec<String> {
        self.utterances().iter().skip(from).map(render).collect()
    }

    /// The multiplayer map `map`.
    pub fn multiplayer_map(&mut self, map: &str) -> Arc<crate::mapfile::Map> {
        self.client
            .res
            .multiplayer_maps()
            .into_iter()
            .find(|m| m.name == map)
            .unwrap_or_else(|| panic!("no multiplayer map {map}"))
    }

    /// Starts a training game on the multiplayer map `map` against
    /// computers of these types, with the world's seed fixed. The objective
    /// is told as the story and the bindings are loaded; no turn has run
    /// yet.
    pub fn start_training(&mut self, map: &str, computers: &[&str], seed: i64) -> TestGame {
        self.client.game.seed = Some(seed);
        let m = self.multiplayer_map(map);
        self.client.res.set_map(Some(&m));
        let players = training_players(&self.client, computers);
        let mut session = Session::training(&self.client, m, players);
        let mut game = session.start(&mut self.client).expect("a valid map");
        self.client.input.clear();
        game.set_turn_due(self.client.clock.now());
        TestGame {
            session: Some(session),
            game,
        }
    }

    /// Connects the client to a [`FakeServer`] as `you`, with `others`
    /// in the lobby.
    pub fn join_fake_server(&mut self, you: &str, others: &[&str]) -> FakeServer {
        let (server, conn) = FakeServer::new(self.clock.clone());
        let content = self.client.res.content_hash();
        let redial = server.redial();
        self.client.net = Some(crate::net::Net::new(
            conn,
            snapshot(you, others),
            content,
            redial,
        ));
        server
    }

    /// Puts the player in `room`, as a server that started its game would
    /// have.
    pub fn enter_fake_room(&mut self, room: &RoomState) {
        let net = self.client.net.as_mut().expect("on a server");
        net.room = Some(crate::net::lobby::RoomModel {
            state: room.clone(),
            map: crate::net::lobby::MapState::Downloading,
        });
    }

    /// Joins, on a [`FakeServer`], the game of `stream` on the multiplayer
    /// map `map` at `speed`: the server sends the seat's opening frames
    /// now. Nothing of the game is handled yet.
    pub fn join_fake_game(
        &mut self,
        server: &FakeServer,
        stream: &FakeStream,
        map: &str,
        speed: Speed,
    ) -> TestGame {
        let m = self.multiplayer_map(map);
        let now = self.clock.now();
        for f in stream.opening(m.info().expect("a map file"), speed) {
            server.frame_at(now, f);
        }
        crate::net::pump(&mut self.client);
        let seated = self
            .client
            .net
            .as_mut()
            .and_then(|n| n.started.take())
            .expect("a seat");
        self.client.res.set_map(Some(&m));
        let game =
            crate::session::join_server_game(&mut self.client, &seated).expect("the game joined");
        self.client.input.clear();
        TestGame {
            session: None,
            game,
        }
    }

    /// Starts the game of `setup`, run here, on the multiplayer map `map`,
    /// as its first player, readied as the game of a server is: the
    /// objective told, no intro. No turn has run yet.
    pub fn start_local(&mut self, map: &str, setup: &GameSetup) -> TestGame {
        let m = self.multiplayer_map(map);
        self.client.res.set_map(Some(&m));
        let host = Host::new(setup).expect("a game");
        let me = host.seats()[0];
        let mut game = GameInterface::local(&mut self.client, host, me);
        crate::session::prepare(&mut self.client, &mut game, Vec::new());
        self.client.input.clear();
        TestGame {
            session: None,
            game,
        }
    }

    /// Starts a game on a real server on 127.0.0.1, which the test owns:
    /// the player creates a room on the multiplayer map `map` at `speed`,
    /// adds these computers, and starts the game, which the server builds
    /// from `seed`. The player has joined the game; nothing of it is
    /// handled yet. Waits in real time for the server, whatever the clock.
    pub fn start_on_server(
        &mut self,
        map: &str,
        computers: &[Computer],
        speed: Speed,
        seed: i64,
    ) -> (soundrts_server::ServerHandle, TestGame) {
        self.start_on_server_with(map, computers, speed, seed, |c| c)
    }

    /// As [`Harness::start_on_server`], with the connection to the server
    /// passed through `wrap` before the client uses it.
    pub fn start_on_server_with(
        &mut self,
        map: &str,
        computers: &[Computer],
        speed: Speed,
        seed: i64,
        wrap: impl FnOnce(Connection) -> Connection,
    ) -> (soundrts_server::ServerHandle, TestGame) {
        let server = test_server(seed, |_| {});
        self.connect_to(&server.local_addr().to_string(), wrap);
        self.open_room(map, computers, speed);
        let game = self.start_room_game(map);
        (server, game)
    }

    /// Connects to the server at `address` as the multiplayer menu does,
    /// with the connection passed through `wrap` before the client uses
    /// it: a connection lost in a game is opened again the same way, and
    /// not passed through `wrap`. Waits in real time for the server.
    pub fn connect_to(&mut self, address: &str, wrap: impl FnOnce(Connection) -> Connection) {
        let content = self.client.res.content_hash();
        let dialer = crate::net::conn::Dialer {
            address: crate::net::conn::ServerAddress::parse(address).expect("an address"),
            trust: self.client.net_config.trust.clone(),
            hello: ClientMsg::Hello {
                protocol: PROTOCOL_VERSION,
                login: Login::new(self.client.config.login.as_str()).expect("a login"),
                content: content.clone(),
            },
            timeout: WAIT_LIMIT,
            liveness: self.client.net_config.liveness,
        };
        let (conn, snapshot) = dialer.open().expect("connected");
        let redial = Box::new(move || dialer.start());
        self.client.net = Some(crate::net::Net::new(wrap(conn), snapshot, content, redial));
    }

    /// Creates a room on the multiplayer map `map` at `speed` and adds
    /// these computers: the room's id. Only those invited join it; anyone
    /// invited may watch. Waits in real time for the server.
    pub fn open_room(&mut self, map: &str, computers: &[Computer], speed: Speed) -> RoomId {
        self.create_room(map, computers, speed, false, true)
    }

    /// As [`Harness::open_room`], for a room anyone may join if `open`,
    /// and watch if `watchers`.
    pub fn create_room(
        &mut self,
        map: &str,
        computers: &[Computer],
        speed: Speed,
        open: bool,
        watchers: bool,
    ) -> RoomId {
        let m = self.multiplayer_map(map);
        let info = m.info().expect("a map file");
        let net = self.client.net.as_mut().expect("on the server");
        net.creating = Some(m);
        net.send(ClientMsg::CreateRoom {
            map: info,
            speed,
            open,
            watchers,
        });
        self.pump_until(|n| n.room.is_some());
        for (i, &computer) in computers.iter().enumerate() {
            crate::net::send(&mut self.client, ClientMsg::AddComputer { computer });
            self.pump_until(|n| n.room.as_ref().is_some_and(|r| r.state.slots.len() > i + 1));
        }
        self.room_id()
    }

    /// The id of the room the player is in.
    pub fn room_id(&self) -> RoomId {
        let net = self.client.net.as_ref().expect("on the server");
        net.room.as_ref().expect("in a room").state.id
    }

    /// Sends `msg` about the room, as its menu does, and waits until the
    /// room is as `done` says. Waits in real time for the server.
    pub fn change_room(
        &mut self,
        msg: ClientMsg,
        done: impl Fn(&soundrts_protocol::RoomState) -> bool,
    ) {
        crate::net::send(&mut self.client, msg);
        self.pump_until(|n| n.room.as_ref().is_some_and(|r| done(&r.state)));
    }

    /// Invites `login` to the room, once they are on the server.
    pub fn invite(&mut self, login: &str) {
        let login = Login::new(login).expect("a login");
        self.pump_until(|n| n.lobby.players.iter().any(|p| p.login == login));
        crate::net::send(&mut self.client, ClientMsg::Invite { login });
    }

    /// Watches the room `room`, or its game: once in it, the room as the
    /// player knows it. Waits in real time for the server.
    pub fn watch_room(&mut self, room: RoomId) {
        crate::net::send(&mut self.client, ClientMsg::Watch { room });
        self.pump_until(|n| n.room.is_some());
    }

    /// Joins the room `room` once invited, and is ready. Waits in real time
    /// for the server.
    pub fn join_room(&mut self, room: RoomId) {
        self.pump_until(|n| n.invitations.iter().any(|i| i.room == room));
        crate::net::send(&mut self.client, ClientMsg::JoinRoom { room });
        self.pump_until(|n| n.room.is_some());
        crate::net::send(&mut self.client, ClientMsg::SetReady { ready: true });
    }

    /// Waits until the room has `slots` slots, every human in it ready.
    pub fn wait_for_ready(&mut self, slots: usize) {
        self.pump_until(|n| {
            n.room
                .as_ref()
                .is_some_and(|r| r.state.slots.len() == slots && r.not_ready().is_empty())
        });
    }

    /// Starts the game of the player's room, as its host, on the
    /// multiplayer map `map`, and joins it: nothing of it is handled yet.
    pub fn start_room_game(&mut self, map: &str) -> TestGame {
        let m = self.multiplayer_map(map);
        let texts = crate::multiplayer::game_texts(&mut self.client, &m).expect("the texts");
        crate::net::send(&mut self.client, ClientMsg::Start { texts });
        self.join_room_game(map)
    }

    /// Joins the game the server started in the player's room, on the
    /// multiplayer map `map`: nothing of it is handled yet.
    pub fn join_room_game(&mut self, map: &str) -> TestGame {
        self.pump_until(|n| {
            n.started.is_some() && n.game.as_ref().is_some_and(|g| g.frames.len() >= 2)
        });
        let seated: Seated = self
            .client
            .net
            .as_mut()
            .and_then(|n| n.started.take())
            .expect("a seat");
        let m = self.multiplayer_map(map);
        self.client.res.set_map(Some(&m));
        let game =
            crate::session::join_server_game(&mut self.client, &seated).expect("the game joined");
        self.client.input.clear();
        TestGame {
            session: None,
            game,
        }
    }

    /// Reads what the server sends until `done` holds, in real time.
    fn pump_until(&mut self, done: impl Fn(&crate::net::Net) -> bool) {
        let start = std::time::Instant::now();
        loop {
            crate::net::pump(&mut self.client);
            let net = self.client.net.as_ref().expect("on a server");
            assert!(net.closed.is_none(), "the connection closed");
            if done(net) {
                return;
            }
            assert!(start.elapsed() < WAIT_LIMIT, "the server did not answer");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// One pass of the game loop (one millisecond of game time).
    pub fn step(&mut self, g: &mut TestGame) {
        g.game
            .step(&mut self.client)
            .expect("the window stays open");
    }

    /// Runs the game loop until the world has played `turn` turns and their
    /// events are handled.
    pub fn run_until_turn(&mut self, g: &mut TestGame, turn: i64) {
        while g.game.view().turn() < turn || g.game.has_pending_server_events() {
            assert!(!g.game.end_loop, "the game ended before turn {turn}");
            self.step(g);
        }
    }

    /// Runs the game loop until the voice has nothing to say (at most 60 s
    /// of game time).
    pub fn run_until_voice_idle(&mut self, g: &mut TestGame) {
        let end = self.clock.now() + 60.0;
        while !self.client.voice.is_idle() && self.clock.now() < end && !g.game.end_loop {
            self.step(g);
        }
    }

    /// Lets the voice speak for `seconds` of the test clock, outside a
    /// game.
    pub fn run_voice_for(&mut self, seconds: f64) {
        let end = self.clock.now() + seconds;
        while self.clock.now() < end && !self.client.voice.is_idle() {
            self.client.voice.update();
            self.clock.sleep(0.01);
        }
    }

    /// Runs the game loop for `seconds` of game time.
    pub fn run_for(&mut self, g: &mut TestGame, seconds: f64) {
        let end = self.clock.now() + seconds;
        while self.clock.now() < end && !g.game.end_loop {
            self.step(g);
        }
    }

    /// Presses a key, lets the game handle it, and waits until the voice is
    /// idle. Returns what was said meanwhile.
    pub fn key(&mut self, g: &mut TestGame, key: Key, mods: Mods) -> Vec<String> {
        let from = self.utterances().len();
        self.press(key, mods);
        self.step(g);
        self.run_until_voice_idle(g);
        self.said_since(from)
    }

    /// Runs the game loop until the game ends, then says the score.
    pub fn finish(&mut self, g: &mut TestGame) {
        while !g.game.end_loop {
            self.step(g);
        }
        match &mut g.session {
            Some(session) => session.finish(&mut self.client, &mut g.game),
            None => {
                crate::session::finish(&mut self.client, &mut g.game);
            }
        }
    }
}

/// A server on 127.0.0.1, on a free port, which the test owns: it plays
/// every game from `seed`, with its settings changed by `adjust`.
pub fn test_server(
    seed: i64,
    adjust: impl FnOnce(&mut soundrts_server::ServerConfig),
) -> soundrts_server::ServerHandle {
    let mut config = soundrts_server::ServerConfig {
        name: soundrts_protocol::ServerName::new("test").expect("a name"),
        ip: std::net::Ipv4Addr::LOCALHOST.into(),
        port: 0,
        seed: Some(seed),
        ..soundrts_server::ServerConfig::default()
    };
    adjust(&mut config);
    soundrts_server::start(config).expect("a server")
}

/// The players of a training game, as its menu makes them: the local
/// player in alliance 1, then computers of these types at the normal
/// level in alliance 2, each with a random faction.
pub fn training_players(client: &Client, computers: &[&str]) -> Vec<Controller> {
    let mut players = vec![Controller::human(client.config.login.as_str(), true)];
    players.extend(computers.iter().map(|c| Controller::computer(c)));
    for (i, p) in players.iter_mut().enumerate() {
        p.alliance = Some(if i == 0 { "1" } else { "2" }.to_owned());
    }
    players
}

/// A game started by [`Harness::start_training`], or on a server by
/// [`Harness::start_on_server`], which has no session here.
pub struct TestGame {
    pub session: Option<Session>,
    pub game: GameInterface,
}

/// An utterance as one line: texts as they are, recorded sounds as
/// `<name>`, separated by spaces.
pub fn render(u: &Utterance) -> String {
    u.iter()
        .map(|p| match p {
            UtterancePart::Text(t) => t.clone(),
            UtterancePart::Sound(s) => format!("<{s}>"),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A network cable between clients and a server, all on 127.0.0.1: a
/// proxy that passes every byte both ways until it is pulled. Pulled, it
/// passes nothing and closes nothing, as a cable pulled out does: each end
/// notices only when it stops hearing the other. A connection made
/// meanwhile is refused at once. Plugged back in, new connections pass;
/// those of before are closed, as an end that gave up answers a late
/// packet with a reset. Dropping it stops it.
pub struct Cable {
    addr: std::net::SocketAddr,
    state: Arc<CableState>,
    thread: Option<std::thread::JoinHandle<()>>,
}

struct CableState {
    /// Each pull starts a new one: connections of an earlier one are cut.
    era: std::sync::atomic::AtomicU64,
    pulled: std::sync::atomic::AtomicBool,
    stop: std::sync::atomic::AtomicBool,
}

impl CableState {
    /// Whether a connection of `era` may pass bytes now; `None` once it is
    /// cut.
    fn passes(&self, era: u64) -> Option<bool> {
        use std::sync::atomic::Ordering::SeqCst;
        if self.stop.load(SeqCst) {
            return None;
        }
        let pulled = self.pulled.load(SeqCst);
        if self.era.load(SeqCst) != era {
            // Made before the last pull: held while pulled, then cut.
            return pulled.then_some(false);
        }
        Some(!pulled)
    }
}

impl Cable {
    /// A cable to `server`, plugged in.
    pub fn new(server: std::net::SocketAddr) -> Cable {
        use std::sync::atomic::Ordering::SeqCst;
        let listener =
            std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).expect("a port");
        listener.set_nonblocking(true).expect("non-blocking");
        let addr = listener.local_addr().expect("an address");
        let state = Arc::new(CableState {
            era: std::sync::atomic::AtomicU64::new(0),
            pulled: std::sync::atomic::AtomicBool::new(false),
            stop: std::sync::atomic::AtomicBool::new(false),
        });
        let shared = Arc::clone(&state);
        let thread = std::thread::spawn(move || {
            while !shared.stop.load(SeqCst) {
                match listener.accept() {
                    Ok((client, _)) if !shared.pulled.load(SeqCst) => {
                        if let Ok(server) = std::net::TcpStream::connect(server) {
                            let era = shared.era.load(SeqCst);
                            pass(&client, &server, era, &shared);
                            pass(&server, &client, era, &shared);
                        }
                    }
                    // Refused: nothing answers.
                    Ok(_) => {}
                    Err(_) => std::thread::sleep(std::time::Duration::from_millis(5)),
                }
            }
        });
        Cable {
            addr,
            state,
            thread: Some(thread),
        }
    }

    /// The address to connect to instead of the server's.
    pub fn addr(&self) -> std::net::SocketAddr {
        self.addr
    }

    /// Pulls the cable out: nothing passes.
    pub fn pull(&self) {
        use std::sync::atomic::Ordering::SeqCst;
        self.state.pulled.store(true, SeqCst);
        self.state.era.fetch_add(1, SeqCst);
    }

    /// Plugs the cable back in: new connections pass, those made before it
    /// was pulled are closed.
    pub fn plug_in(&self) {
        self.state
            .pulled
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

impl Drop for Cable {
    fn drop(&mut self) {
        self.state
            .stop
            .store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Passes the bytes read from `from` to `to`, on a thread, while the
/// cable passes them for a connection of `era`; then closes both.
fn pass(from: &std::net::TcpStream, to: &std::net::TcpStream, era: u64, cable: &Arc<CableState>) {
    use std::io::{Read, Write};
    let (Ok(mut from), Ok(mut to)) = (from.try_clone(), to.try_clone()) else {
        return;
    };
    let cable = Arc::clone(cable);
    let _ = from.set_read_timeout(Some(std::time::Duration::from_millis(10)));
    std::thread::spawn(move || {
        let mut buf = [0u8; 16 * 1024];
        loop {
            match cable.passes(era) {
                None => break,
                Some(false) => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                    continue;
                }
                Some(true) => {}
            }
            match from.read(&mut buf) {
                Ok(0) => break,
                // Bytes read as the cable is pulled are lost.
                Ok(n) if cable.passes(era) == Some(true) => {
                    if to.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
                Ok(_) => {}
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(_) => break,
            }
        }
        let _ = from.shutdown(std::net::Shutdown::Both);
        let _ = to.shutdown(std::net::Shutdown::Both);
    });
}
