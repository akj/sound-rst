//! Replays the scripts of `tests/parity/scenarios` on the client and
//! compares what it says with what the original said for the same keys
//! (`tests/parity/expected`). `tests/parity/README.md` defines the model:
//! the virtual clock, the input points, when a step is delivered, and the
//! transcript grammar.
//!
//! On a difference the test shows the first differing line with its
//! context, and writes the whole transcript of the client to
//! `target/parity/<scenario>.txt`.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use soundrts::audio::{AudioOutput, Channel, DEFAULT_VOLUME, Sound};
use soundrts::client::{Client, Desktop, Devices, open_resources};
use soundrts::clock::Clock;
use soundrts::config::{Config, Login};
use soundrts::input::{
    GameProgress, InputPoint, InputSource, Key, KeyDown, Mods, RawInput, ReadContext,
};
use soundrts::log::Log;
use soundrts::mainmenu::main_menu;
use soundrts::parameters::Parameters;
use soundrts::paths::Paths;
use soundrts::sound::{SoundEvent, SoundLog, StereoVolume};
use soundrts::testkit::{RecordingSpeech, upstream_root};
use soundrts::voice::{UtteranceLog, UtterancePart, VoiceEvent};

/// The virtual epoch: 10^9 s, in microseconds.
const BASE_US: i64 = 1_000_000_000_000_000;

fn parity_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/parity")
}

// ---------------------------------------------------------------------------
// scenarios

struct Scenario {
    name: String,
    settings: Vec<(&'static str, String)>,
    /// (step, the scenario line it comes from)
    steps: Vec<(String, String)>,
}

const SETTINGS: [&str; 6] = ["language", "login", "mods", "soundpacks", "seed", "limit"];
const STEP_KINDS: [&str; 7] = [
    "key",
    "type",
    "wait",
    "wait_turn",
    "wait_turns",
    "wait_game_end",
    "interrupt_after",
];

impl Scenario {
    fn load(name: &str) -> Scenario {
        let path = parity_dir().join("scenarios").join(format!("{name}.txt"));
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
        let mut settings: Vec<(&'static str, String)> = vec![
            ("language", "en".into()),
            ("login", "player".into()),
            ("mods", String::new()),
            ("soundpacks", String::new()),
            ("seed", "0".into()),
            ("limit", "600".into()),
        ];
        let mut steps = Vec::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let words: Vec<&str> = line.split_whitespace().collect();
            if let Some(s) = SETTINGS.iter().find(|s| **s == words[0]) {
                assert!(steps.is_empty(), "{name}: setting after a step: {line}");
                let value = words[1..].join(" ");
                settings.iter_mut().find(|(k, _)| k == s).unwrap().1 = value;
            } else if STEP_KINDS.contains(&words[0]) {
                steps.push(words.join(" "));
            } else {
                panic!("{name}: unknown line: {line}");
            }
        }
        Scenario {
            name: name.to_owned(),
            settings,
            steps: expand_steps(&steps),
        }
    }

    fn setting(&self, k: &str) -> &str {
        &self.settings.iter().find(|(s, _)| *s == k).unwrap().1
    }
}

/// `type bob` becomes one key step per character.
fn expand_steps(steps: &[String]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for s in steps {
        match s.split_once(' ') {
            Some(("type", text)) => {
                for c in text.chars() {
                    let step = if c.is_alphabetic() && c.is_uppercase() {
                        format!("key SHIFT {}", c.to_lowercase())
                    } else if c.is_alphanumeric() {
                        format!("key {c}")
                    } else {
                        match c {
                            ' ' => "key SPACE".to_owned(),
                            '.' => "key PERIOD".to_owned(),
                            _ => panic!("cannot type {c:?}"),
                        }
                    };
                    out.push((step, s.clone()));
                }
            }
            _ => out.push((s.clone(), s.clone())),
        }
    }
    out
}

/// A key press as SDL2 and pygame 2 deliver it on a US layout (README,
/// Input).
fn key_event(spec: &str) -> KeyDown {
    let words: Vec<&str> = spec.split_whitespace().collect();
    let (mod_words, name) = words.split_at(words.len() - 1);
    let name = name[0];
    let mut mods = Mods::NONE;
    for m in mod_words {
        match *m {
            "CTRL" => mods.ctrl = true,
            "ALT" => mods.alt = true,
            "SHIFT" => mods.shift = true,
            _ => panic!("unknown modifier {m}"),
        }
    }
    match name {
        "LCTRL" | "RCTRL" => mods.ctrl = true,
        "LALT" | "RALT" => mods.alt = true,
        "LSHIFT" | "RSHIFT" => mods.shift = true,
        _ => {}
    }
    let key = Key::from_pygame_name(name).unwrap_or_else(|| panic!("unknown key {name}"));
    let shifted_digits = ")!@#$%^&*(";
    let punct = [
        ("QUOTE", '\'', '"'),
        ("BACKQUOTE", '`', '~'),
        ("SEMICOLON", ';', ':'),
        ("BACKSLASH", '\\', '|'),
        ("COMMA", ',', '<'),
        ("PERIOD", '.', '>'),
        ("SLASH", '/', '?'),
        ("MINUS", '-', '_'),
        ("EQUALS", '=', '+'),
        ("LEFTBRACKET", '[', '{'),
        ("RIGHTBRACKET", ']', '}'),
    ];
    let unicode = if name.len() == 1 && name.chars().all(|c| c.is_ascii_alphabetic()) {
        let c = name.chars().next().unwrap().to_ascii_lowercase();
        if mods.ctrl {
            char::from(c as u8 - b'a' + 1).to_string()
        } else if mods.shift {
            c.to_ascii_uppercase().to_string()
        } else {
            c.to_string()
        }
    } else if name.len() == 1 && name.chars().all(|c| c.is_ascii_digit()) {
        let d = name.parse::<usize>().unwrap();
        if mods.ctrl {
            String::new()
        } else if mods.shift {
            shifted_digits[d..=d].to_owned()
        } else {
            name.to_owned()
        }
    } else if let Some((_, plain, shifted)) = punct.iter().find(|(n, _, _)| *n == name) {
        if mods.ctrl {
            String::new()
        } else if mods.shift {
            shifted.to_string()
        } else {
            plain.to_string()
        }
    } else {
        match name {
            "SPACE" => " ",
            "RETURN" | "KP_ENTER" => "\r",
            "TAB" => "\t",
            "BACKSPACE" => "\u{8}",
            "ESCAPE" => "\u{1b}",
            "DELETE" => "\u{7f}",
            "KP_PLUS" => "+",
            "KP_MINUS" => "-",
            "KP_MULTIPLY" => "*",
            "KP_DIVIDE" => "/",
            "KP_PERIOD" => ".",
            _ => "",
        }
        .to_owned()
    };
    KeyDown {
        key,
        physical: key,
        mods,
        unicode,
    }
}

// ---------------------------------------------------------------------------
// the replay: steps, input points, transcript

fn fnv64(text: &str) -> String {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for b in text.bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01B3);
    }
    format!("{h:016x}")
}

fn fmt3(x: f64) -> String {
    let s = format!("{x:.3}");
    if s == "-0.000" { "0.000".to_owned() } else { s }
}

fn json_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Microseconds since the virtual epoch of a time read on the clock.
fn us_of(t: f64) -> i64 {
    (t * 1e6).round() as i64 - BASE_US
}

struct Replay {
    us: Rc<Cell<i64>>,
    limit_us: i64,
    steps: Vec<(String, String)>,
    next: usize,
    last_delivery_us: i64,
    turn_at_delivery: Option<i64>,
    turns_at_delivery: u64,
    /// The last progress the client told of (`None`: no game).
    game: Option<GameProgress>,
    /// Keys delivered, not yet read by the client.
    ready: VecDeque<RawInput>,
    lines: Vec<String>,
    sound_lines: Vec<String>,
    step_sounds: Vec<String>,
    voice: UtteranceLog,
    voice_seen: usize,
    sounds: SoundLog,
    sounds_seen: usize,
    log: Log,
    title: Option<String>,
    ended: bool,
    aborted: bool,
    quit_sent: bool,
    reads_at: (i64, u64),
}

type ReplayRef = Rc<RefCell<Replay>>;

impl Replay {
    fn ms(&self) -> i64 {
        self.us.get() / 1000
    }

    fn recording(&self) -> bool {
        !self.ended && !self.aborted
    }

    fn emit(&mut self, text: String) {
        let ms = self.ms();
        self.lines.push(format!("@{ms} {text}"));
    }

    /// Writes the voice and sound events recorded since the last sync.
    fn sync(&mut self) {
        if !self.recording() {
            return;
        }
        for e in self.voice.events_from(self.voice_seen) {
            self.voice_seen += 1;
            match e {
                VoiceEvent::Started {
                    time,
                    kind,
                    parts,
                    lv,
                    rv,
                } => {
                    let mut line = format!("@{} voice {}", us_of(time) / 1000, kind.name());
                    for p in parts {
                        line.push(' ');
                        match p {
                            UtterancePart::Text(t) => line.push_str(&json_str(&t)),
                            UtterancePart::Sound(s) => line.push_str(&format!("[{s}]")),
                        }
                    }
                    if (lv, rv) != (DEFAULT_VOLUME, DEFAULT_VOLUME) {
                        line.push_str(&format!(" vol={},{}", fmt3(lv), fmt3(rv)));
                    }
                    self.lines.push(line);
                }
                VoiceEvent::Cut { time, queued } => {
                    self.lines
                        .push(format!("@{} voice cut queue={queued}", us_of(time) / 1000));
                }
            }
        }
        for e in self.sounds.events_from(self.sounds_seen) {
            self.sounds_seen += 1;
            let line = sound_line(&e);
            self.sound_lines.push(line.clone());
            self.step_sounds.push(line);
        }
        // Synced before every sleep, so an error is logged at this time.
        for e in self.log.take_errors() {
            self.emit(format!("exception {}", e.replace('\n', " ")));
        }
    }

    fn close_step_sounds(&mut self) {
        if !self.step_sounds.is_empty() {
            let n = self.step_sounds.len();
            let h = fnv64(&self.step_sounds.join("\n"));
            self.step_sounds.clear();
            self.emit(format!("sounds {n} {h}"));
        }
    }

    fn settled(&self, c: &ReadContext) -> bool {
        if c.keydown_waiting || self.ready.iter().any(|e| matches!(e, RawInput::KeyDown(_))) {
            return false;
        }
        if !c.voice_idle {
            return false;
        }
        if c.point == InputPoint::Game {
            let Some(g) = c.game else { return false };
            if g.events_waiting || g.turns_handled <= self.turns_at_delivery {
                return false;
            }
        }
        true
    }

    fn step_ready(&self, step: &str, c: Option<&ReadContext>) -> bool {
        let (kind, arg) = step.split_once(' ').unwrap_or((step, ""));
        let secs = |s: &str| (s.parse::<f64>().unwrap() * 1e6).round() as i64;
        let game = c.and_then(|c| c.game);
        match kind {
            "key" => c.is_some_and(|c| c.point != InputPoint::Voice && self.settled(c)),
            "interrupt_after" => {
                let delay = arg.split(' ').next().unwrap();
                self.us.get() >= self.last_delivery_us + secs(delay)
            }
            "wait" => self.us.get() >= self.last_delivery_us + secs(arg),
            "wait_turn" => game.is_some_and(|g| g.turn >= arg.parse::<i64>().unwrap()),
            "wait_turns" => game.is_some_and(|g| {
                g.turn >= self.turn_at_delivery.unwrap_or(0) + arg.parse::<i64>().unwrap()
            }),
            "wait_game_end" => c.is_some() && game.is_none(),
            _ => panic!("unknown step {step}"),
        }
    }

    fn deliver(&mut self) {
        self.sync();
        let (step, original) = self.steps[self.next].clone();
        self.close_step_sounds();
        self.next += 1;
        let label = if original == step {
            original
        } else {
            format!("{step} ({original})")
        };
        self.emit(format!("step {} {label}", self.next));
        let ms = self.ms();
        self.sound_lines.push(format!("@{ms} step {}", self.next));
        self.last_delivery_us = self.us.get();
        self.turns_at_delivery = self.game.map_or(0, |g| g.turns_handled);
        self.turn_at_delivery = self.game.map(|g| g.turn);
        let key = match step.split_once(' ') {
            Some(("key", k)) => Some(k.to_owned()),
            Some(("interrupt_after", rest)) => Some(rest.splitn(3, ' ').nth(2).unwrap().to_owned()),
            _ => None,
        };
        if let Some(k) = key {
            self.ready.push_back(RawInput::KeyDown(key_event(&k)));
            // The reference neither repeats a key nor posts its release:
            // the queue forgets the key without an event.
            self.ready.push_back(RawInput::FocusLost);
        }
    }

    /// After every sleep: a timed key arrives as soon as it is due.
    fn on_time(&mut self) {
        if self.recording() && self.us.get() > self.limit_us {
            self.sync();
            self.close_step_sounds();
            self.emit("abort virtual time limit exceeded".into());
            self.aborted = true;
        }
        assert!(
            self.us.get() < self.limit_us + 600_000_000,
            "the client goes on long after the end of the scenario"
        );
        if !self.recording() || self.next >= self.steps.len() {
            return;
        }
        let step = self.steps[self.next].0.clone();
        if step.starts_with("interrupt_after") && self.step_ready(&step, None) {
            self.deliver();
        }
    }

    /// At every input point, before the client reads.
    fn on_input(&mut self, c: &ReadContext) {
        self.game = c.game;
        if !self.recording() {
            return;
        }
        let now = self.us.get();
        if self.reads_at.0 == now {
            self.reads_at.1 += 1;
            assert!(
                self.reads_at.1 < 1_000_000,
                "input read a million times without time passing"
            );
        } else {
            self.reads_at = (now, 0);
        }
        while self.next < self.steps.len() {
            let step = self.steps[self.next].0.clone();
            if !self.step_ready(&step, Some(c)) {
                return;
            }
            self.deliver();
            if step.starts_with("key") || step.starts_with("interrupt_after") {
                return;
            }
        }
        if c.point != InputPoint::Voice && self.settled(c) {
            self.sync();
            self.close_step_sounds();
            self.emit("end".into());
            self.ended = true;
        }
    }
}

fn sound_line(e: &SoundEvent) -> String {
    let ms = us_of(e.time()) / 1000;
    match e {
        SoundEvent::Positional {
            sound,
            placement: p,
            looping,
            started,
            ..
        } => {
            let gains = |g: &Option<(f64, f64)>| match g {
                Some((l, r)) => format!(" gains={},{}", fmt3(*l), fmt3(*r)),
                None => String::new(),
            };
            if *looping {
                format!(
                    "@{ms} loop {sound} v={} x={} y={} prio={} {}{}",
                    fmt3(p.volume),
                    fmt3(p.x),
                    fmt3(p.y),
                    p.priority,
                    if started.is_some() {
                        "started"
                    } else {
                        "pending"
                    },
                    gains(started)
                )
            } else {
                format!(
                    "@{ms} pos {sound} v={} x={} y={} prio={} limit={}{} {}{}",
                    fmt3(p.volume),
                    fmt3(p.x),
                    fmt3(p.y),
                    p.priority,
                    fmt3(p.limit),
                    if p.ambient { " ambient" } else { "" },
                    if started.is_some() {
                        "started"
                    } else {
                        "skipped"
                    },
                    gains(started)
                )
            }
        }
        SoundEvent::Stereo {
            sound,
            volume,
            limit,
            started,
            ..
        } => {
            let v = match volume {
                StereoVolume::Both(v) => fmt3(*v),
                StereoVolume::Split(l, r) => format!("{},{}", fmt3(*l), fmt3(*r)),
            };
            format!(
                "@{ms} stereo {sound} vol={v} limit={} {}",
                fmt3(*limit),
                if *started { "started" } else { "skipped" }
            )
        }
    }
}

// ---------------------------------------------------------------------------
// the devices

/// Integer microseconds on the virtual epoch; every sleep lets the replay
/// deliver a timed key.
struct ParityClock {
    us: Rc<Cell<i64>>,
    replay: RefCell<Option<ReplayRef>>,
}

impl Clock for ParityClock {
    fn now(&self) -> f64 {
        (BASE_US + self.us.get()) as f64 / 1e6
    }

    fn sleep(&self, seconds: f64) {
        assert!(seconds >= 0.0, "negative sleep");
        if let Some(r) = self.replay.borrow().as_ref() {
            r.borrow_mut().sync();
        }
        self.us.set(self.us.get() + (seconds * 1e6).round() as i64);
        if let Some(r) = self.replay.borrow().as_ref() {
            r.borrow_mut().on_time();
        }
    }
}

struct ReplaySource(ReplayRef);

impl InputSource for ReplaySource {
    fn next(&mut self, now: f64) -> Option<(f64, RawInput)> {
        let mut r = self.0.borrow_mut();
        if !r.recording() {
            if r.quit_sent {
                return None;
            }
            r.quit_sent = true;
            return Some((now, RawInput::Quit));
        }
        r.ready.pop_front().map(|e| (now, e))
    }

    fn before_read(&mut self, _now: f64, context: &ReadContext) {
        self.0.borrow_mut().on_input(context);
    }
}

/// A sound keeps its channel busy for exactly its length, in microseconds,
/// times its loops.
struct ParityAudio {
    us: Rc<Cell<i64>>,
    /// channel -> end (`None`: a loop)
    busy: HashMap<usize, Option<i64>>,
    lengths: Rc<RefCell<BTreeMap<String, i64>>>,
}

/// The length as pygame's `Sound.get_length()` gives it with the mixer at
/// 44100 Hz (a single precision division), in microseconds.
fn length_us(sound: &Sound) -> i64 {
    let frames = (sound.samples.len() / 2) as f64;
    let frames_44100 = (frames * 44100.0 / f64::from(sound.sample_rate)).round();
    (f64::from(frames_44100 as f32 / 44100f32) * 1e6).round() as i64
}

impl AudioOutput for ParityAudio {
    fn play(&mut self, channel: Channel, sound: &Sound, looping: bool) {
        let len = length_us(sound);
        {
            let mut lengths = self.lengths.borrow_mut();
            let known = *lengths.entry(sound.key.clone()).or_insert(len);
            if known != len {
                lengths.insert(format!("{}@{}", sound.key, sound.mod_name), len);
            }
        }
        let end = (!looping).then(|| self.us.get() + len);
        self.busy.insert(channel.index(), end);
    }

    fn set_gains(&mut self, _: Channel, _: f32, _: f32) {}

    fn stop(&mut self, channel: Channel) {
        self.busy.remove(&channel.index());
    }

    fn is_busy(&self, channel: Channel) -> bool {
        match self.busy.get(&channel.index()) {
            Some(None) => true,
            Some(Some(end)) => self.us.get() < *end,
            None => false,
        }
    }
}

struct TitleDesk(ReplayRef);

impl Desktop for TitleDesk {
    fn set_title(&self, title: &str) {
        let mut r = self.0.borrow_mut();
        if r.title.as_deref() != Some(title) {
            r.title = Some(title.to_owned());
            if r.recording() {
                r.sync();
                r.emit(format!("title {title}"));
            }
        }
    }

    fn open_folder(&self, _: &Path) {}
}

// ---------------------------------------------------------------------------
// running and comparing

fn run(s: &Scenario) -> String {
    let user_dir = tempfile::tempdir().expect("temporary folder");
    let paths = Paths {
        root: upstream_root(),
        user: user_dir.path().to_path_buf(),
    };
    paths.create_user_folders(&Log::silent());
    let log = Log::file_only(&paths.client_log());
    log.keep_errors();

    let us = Rc::new(Cell::new(0));
    let voice = UtteranceLog::default();
    let sounds = SoundLog::default();
    let replay = Rc::new(RefCell::new(Replay {
        us: us.clone(),
        limit_us: (s.setting("limit").parse::<f64>().unwrap() * 1e6) as i64,
        steps: s.steps.clone(),
        next: 0,
        last_delivery_us: 0,
        turn_at_delivery: None,
        turns_at_delivery: 0,
        game: None,
        ready: VecDeque::new(),
        lines: Vec::new(),
        sound_lines: Vec::new(),
        step_sounds: Vec::new(),
        voice: voice.clone(),
        voice_seen: 0,
        sounds: sounds.clone(),
        sounds_seen: 0,
        log: log.clone(),
        title: None,
        ended: false,
        aborted: false,
        quit_sent: false,
        reads_at: (-1, 0),
    }));
    let clock = Rc::new(ParityClock {
        us: us.clone(),
        replay: RefCell::new(None),
    });
    let lengths = Rc::new(RefCell::new(BTreeMap::new()));

    let config = Config {
        login: Login::parse(s.setting("login")).expect("a valid login"),
        mods: s.setting("mods").to_owned(),
        soundpacks: s.setting("soundpacks").to_owned(),
        ..Config::default()
    };
    config.save(&paths.config_file(), &log);
    let params =
        Rc::new(Parameters::load(&paths.parameters_file(), &log).expect("cfg/parameters.toml"));
    let res = open_resources(
        &paths,
        params,
        &config.mods,
        &config.soundpacks,
        Some(s.setting("language").to_owned()),
        &log,
    );
    let seed: i64 = s.setting("seed").parse().unwrap();
    let (speech, _) = RecordingSpeech::new();
    let devices = Devices {
        speech: Box::new(speech),
        audio: Box::new(ParityAudio {
            us: us.clone(),
            busy: HashMap::new(),
            lengths: lengths.clone(),
        }),
        input: Box::new(ReplaySource(replay.clone())),
        clock: clock.clone(),
        desktop: Rc::new(TitleDesk(replay.clone())),
        random_seed: seed as u64,
        utterances: Some(voice),
    };
    let mut client = Client::new(paths, config, res, log, devices);
    client.audio.record_to(sounds);
    client.game.seed = Some(seed);
    *clock.replay.borrow_mut() = Some(replay.clone());

    let result = main_menu(&mut client);
    {
        let mut r = replay.borrow_mut();
        if r.recording() {
            r.sync();
            r.close_step_sounds();
            let unfinished = r.next < r.steps.len();
            let line = match (result, unfinished) {
                (Ok(()), true) => "abort the client quit before the end of the script",
                (Err(_), true) => "abort the client exited before the end of the script",
                (Ok(()), false) => "end",
                (Err(_), false) => "exit",
            };
            r.emit(line.into());
        }
    }
    client.close();
    *clock.replay.borrow_mut() = None;

    let r = replay.borrow();
    let mut out = vec![
        "soundrts-ui-transcript 1".to_owned(),
        format!("scenario {}", s.name),
        format!(
            "settings {}",
            s.settings
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(" ")
        ),
    ];
    let lengths = lengths.borrow();
    let mut names: Vec<(&String, &i64)> = lengths.iter().collect();
    names.sort_by(|a, b| (a.0.len(), a.0).cmp(&(b.0.len(), b.0)));
    for (name, len) in names {
        out.push(format!("length {name} {len}"));
    }
    out.extend(r.lines.iter().cloned());
    let mut text = out.join("\n");
    text.push('\n');
    let mut sounds_text = r.sound_lines.join("\n");
    sounds_text.push('\n');
    drop(r);
    text + "\u{0}" + &sounds_text
}

/// The first difference between two transcripts, with context, if any.
fn first_difference(expected: &str, actual: &str) -> Option<String> {
    let e: Vec<&str> = expected.lines().collect();
    let a: Vec<&str> = actual.lines().collect();
    let i = (0..e.len().max(a.len())).find(|&i| e.get(i) != a.get(i))?;
    let from = i.saturating_sub(6);
    let show = |lines: &[&str]| {
        lines
            .iter()
            .enumerate()
            .skip(from)
            .take(i - from + 6)
            .map(|(n, l)| format!("{}{:5} {l}", if n == i { ">" } else { " " }, n + 1))
            .collect::<Vec<_>>()
            .join("\n")
    };
    Some(format!(
        "first difference at line {}\n--- expected (original)\n{}\n--- actual (Rust)\n{}",
        i + 1,
        show(&e),
        show(&a)
    ))
}

fn check(name: &str) {
    let s = Scenario::load(name);
    let both = run(&s);
    let (transcript, sounds) = both.split_once('\u{0}').unwrap();
    let out_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/parity");
    std::fs::create_dir_all(&out_dir).expect("target/parity");
    std::fs::write(out_dir.join(format!("{name}.txt")), transcript).expect("write transcript");
    std::fs::write(out_dir.join(format!("{name}.sounds.txt")), sounds).expect("write sounds");
    let expected_dir = parity_dir().join("expected");
    let expected = std::fs::read_to_string(expected_dir.join(format!("{name}.txt")))
        .expect("expected transcript");
    let expected = without_exception_types(&expected);
    // The voice, the titles and the steps first: a difference there says
    // more than the sound summaries it shifts.
    let voice_only = |t: &str| {
        t.lines()
            .filter(|l| !l.starts_with("length ") && !is_sound_summary(l))
            .collect::<Vec<_>>()
            .join("\n")
    };
    if let Some(d) = first_difference(&voice_only(&expected), &voice_only(transcript)) {
        panic!(
            "{name}: the voice differs from the original's\n{d}\n(full transcript: target/parity/{name}.txt)"
        );
    }
    if let Some(d) = first_difference(&expected, transcript) {
        panic!(
            "{name}: the sound lengths or the sounds of a step differ from the original's\n{d}\n\
             (full transcript: target/parity/{name}.txt, sounds: target/parity/{name}.sounds.txt)"
        );
    }
    if let Ok(expected_sounds) =
        std::fs::read_to_string(expected_dir.join(format!("{name}.sounds.txt")))
        && let Some(d) = first_difference(&expected_sounds, sounds)
    {
        panic!(
            "{name}: the positional sounds differ from the original's\n{d}\n(full stream: target/parity/{name}.sounds.txt)"
        );
    }
}

/// The original's exception lines without the Python exception class:
/// `exception <message> (<Class>: <text>)` becomes `exception <message>:
/// <text>`, as the client logs its errors.
fn without_exception_types(transcript: &str) -> String {
    let mut out = String::new();
    for line in transcript.lines() {
        let rewritten = line
            .split_once(" exception ")
            .filter(|(t, _)| t.starts_with('@'))
            .and_then(|(t, rest)| {
                let body = rest.strip_suffix(')')?;
                let (message, inner) = body.rsplit_once(" (")?;
                let (class, text) = inner.split_once(": ")?;
                class
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
                    .then(|| format!("{t} exception {message}: {text}"))
            });
        out.push_str(rewritten.as_deref().unwrap_or(line));
        out.push('\n');
    }
    out
}

/// `@<ms> sounds <count> <fnv64>`: the summary of a step's sounds.
fn is_sound_summary(line: &str) -> bool {
    let mut w = line.split(' ');
    matches!(
        (w.next(), w.next(), w.next(), w.next(), w.next()),
        (Some(t), Some("sounds"), Some(n), Some(h), None)
            if t.starts_with('@')
                && n.bytes().all(|b| b.is_ascii_digit())
                && h.len() == 16
                && h.bytes().all(|b| b.is_ascii_hexdigit())
    )
}

#[test]
fn menus_main() {
    check("menus_main");
}

#[test]
fn menus_options() {
    check("menus_options");
}

#[test]
fn menus_single() {
    check("menus_single");
}

#[test]
fn training_jl1() {
    check("training_jl1");
}

#[test]
fn training_long() {
    check("training_long");
}

#[test]
fn campaign() {
    check("campaign");
}

#[test]
fn french() {
    check("french");
}

#[test]
fn square_orders() {
    check("square_orders");
}

#[test]
fn events() {
    check("events");
}

#[test]
fn transport() {
    check("transport");
}
