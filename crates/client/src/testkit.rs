//! Test support: fake devices that record what the client would say and
//! play, a scripted keyboard, a manual clock, and a harness that builds a
//! complete client on the real game data with them.
//!
//! A test runs the real menus or game interface against [`Harness`] and
//! checks [`Harness::spoken`].

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use crate::audio::{AudioOutput, Channel, Sound};
use crate::client::{Client, Desktop, Devices, open_resources};
use crate::clock::{Clock, ManualClock};
use crate::config::Config;
use crate::game::GameInterface;
use crate::input::{InputSource, Key, KeyDown, KeyUp, Mods, RawInput};
use crate::log::Log;
use crate::parameters::Parameters;
use crate::paths::Paths;
use crate::session::Session;
use crate::tts::SpeechBackend;
use crate::voice::{Utterance, UtteranceLog, UtterancePart};

/// The upstream checkout the tests read the game data from. Panics with
/// instructions if it is missing.
pub fn upstream_root() -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/soundrts-upstream");
    assert!(
        root.join("res/rules.txt").is_file(),
        "The upstream game data is missing at {}. Run scripts/fetch-upstream.ps1 first.",
        root.display()
    );
    root
}

/// The upstream test data folder.
pub fn upstream_tests_dir() -> PathBuf {
    upstream_root().join("soundrts/tests")
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
pub struct SpeechLog(Rc<RefCell<Vec<SpeechEvent>>>);

impl SpeechLog {
    pub fn events(&self) -> Vec<SpeechEvent> {
        self.0.borrow().clone()
    }

    /// The texts spoken, in order.
    pub fn spoken(&self) -> Vec<String> {
        self.0
            .borrow()
            .iter()
            .filter_map(|e| match e {
                SpeechEvent::Speak(t) => Some(t.clone()),
                SpeechEvent::Stop => None,
            })
            .collect()
    }

    pub fn clear(&self) {
        self.0.borrow_mut().clear();
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
            .0
            .borrow_mut()
            .push(SpeechEvent::Speak(text.to_owned()));
        Ok(())
    }

    fn stop(&mut self) -> Result<(), String> {
        self.log.0.borrow_mut().push(SpeechEvent::Stop);
        Ok(())
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

    fn set_gains(&mut self, channel: Channel, left: f32, right: f32) {
        let mut r = self.log.0.borrow_mut();
        r.gains.insert(channel.index(), (left, right));
        r.events.push(AudioEvent::Gains {
            channel: channel.index(),
            left,
            right,
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
        self.insert(time, RawInput::KeyDown(press(key, mods)));
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

/// A timed list of key presses and releases.
#[derive(Clone, Default)]
pub struct Script {
    events: VecDeque<(f64, RawInput)>,
    time: f64,
    keep_open: bool,
    live: Option<LiveKeys>,
    deadline: Option<f64>,
}

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

    /// Also delivers the keys scheduled on `live`, in time order with the
    /// script's.
    pub fn with_live(mut self, live: LiveKeys) -> Script {
        self.live = Some(live);
        self
    }

    /// The following events happen at `time` seconds (on the test clock).
    pub fn at(mut self, time: f64) -> Script {
        self.time = time;
        self
    }

    pub fn key_down(mut self, key: Key) -> Script {
        self.events
            .push_back((self.time, RawInput::KeyDown(down(key))));
        self
    }

    pub fn key_up(mut self, key: Key) -> Script {
        self.events
            .push_back((self.time, RawInput::KeyUp(KeyUp { key, physical: key })));
        self
    }

    /// A press and release.
    pub fn tap(self, key: Key) -> Script {
        self.tap_with(key, Mods::NONE)
    }

    pub fn tap_with(mut self, key: Key, mods: Mods) -> Script {
        self.events
            .push_back((self.time, RawInput::KeyDown(press(key, mods))));
        self.key_up(key)
    }

    /// A press and release of a fully described key press (another layout,
    /// other typed text).
    pub fn tap_event(mut self, e: KeyDown) -> Script {
        let up = KeyUp {
            key: e.key,
            physical: e.physical,
        };
        self.events.push_back((self.time, RawInput::KeyDown(e)));
        self.events.push_back((self.time, RawInput::KeyUp(up)));
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
            live: self.live,
            deadline: self.deadline,
            last_time: 0.0,
            quit_sent: self.keep_open,
        }
    }
}

/// Plays a [`Script`]. Once it has run out, and time has moved past its
/// last event, the window "closes" so that every blocking loop ends.
pub struct ScriptedInput {
    events: VecDeque<(f64, RawInput)>,
    live: Option<LiveKeys>,
    deadline: Option<f64>,
    last_time: f64,
    quit_sent: bool,
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
            return Some((now, RawInput::Quit));
        }
        if let Some(live) = &self.live {
            let mut q = live.0.borrow_mut();
            let live_first = q.front().map(|(t, _)| *t);
            let script_first = self.events.front().map(|(t, _)| *t);
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
        match self.events.front() {
            Some((t, _)) if *t <= now => {
                self.last_time = *t;
                self.events.pop_front()
            }
            Some(_) => None,
            None if !self.quit_sent && now > self.last_time => {
                self.quit_sent = true;
                Some((now, RawInput::Quit))
            }
            None => None,
        }
    }
}

/// A desktop that records titles, opened folders and the clipboard. The
/// clipboard record can be shared with another thread.
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
            root: upstream_root(),
            user: user_dir.path().to_path_buf(),
        };
        paths.create_user_folders(&Log::silent());
        let log = Log::file_only(&paths.client_log());
        let params =
            Rc::new(Parameters::load(&paths.parameters_file(), &log).expect("cfg/parameters.toml"));
        let clock = Rc::new(ManualClock::new());
        let res = open_resources(
            &paths,
            params,
            &config.mods,
            &config.soundpacks,
            Some(language.to_owned()),
            &log,
        );
        let (speech_backend, speech) = RecordingSpeech::new();
        let (audio_output, audio) = RecordingAudio::new(clock.clone());
        let desktop = Rc::new(RecordingDesktop::default());
        let utterance_log = UtteranceLog::default();
        let devices = Devices {
            speech: Box::new(speech_backend),
            audio: Box::new(audio_output),
            input: Box::new(script.source()),
            clock: clock.clone(),
            desktop: desktop.clone(),
            random_seed: 1,
            utterances: Some(utterance_log.clone()),
        };
        let client = Client::new(paths, config, res, log, devices);
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
    pub fn text(&self, key: u64) -> String {
        self.client
            .sounds
            .text(&key.to_string())
            .unwrap_or_else(|| panic!("no text for {key}"))
    }

    /// The utterances started since the `from`th, each written as by
    /// [`render`].
    pub fn said_since(&self, from: usize) -> Vec<String> {
        self.utterances().iter().skip(from).map(render).collect()
    }

    /// Starts a training game on the multiplayer map `map` against
    /// computers of these types, with the world's seed fixed. The objective
    /// is said and the bindings are loaded; no turn has run yet.
    pub fn start_training(&mut self, map: &str, computers: &[&str], seed: i64) -> TestGame {
        self.client.game.seed = Some(seed);
        let m = self
            .client
            .res
            .multiplayer_maps()
            .into_iter()
            .find(|m| m.name == map)
            .unwrap_or_else(|| panic!("no multiplayer map {map}"));
        self.client.res.set_map(Some(&m));
        let mut players = vec![self.client.config.login.as_str().to_owned()];
        players.extend(computers.iter().map(|c| (*c).to_owned()));
        let factions = vec!["random_faction".to_owned(); players.len()];
        let mut alliances = vec!["1".to_owned()];
        alliances.extend(std::iter::repeat_n("2".to_owned(), players.len() - 1));
        let mut session = Session::training(&self.client, m, &players, &factions, &alliances);
        let mut game = session.start(&mut self.client).expect("a valid map");
        self.client.input.clear();
        game.next_update = self.client.clock.now();
        TestGame { session, game }
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
        while g.game.world.turn() < turn || g.game.has_pending_server_events() {
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
        g.session.finish(&mut self.client, &mut g.game);
    }
}

/// A game started by [`Harness::start_training`].
pub struct TestGame {
    pub session: Session,
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
