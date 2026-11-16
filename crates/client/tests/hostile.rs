//! Hostile interface files and hostile games on a server.
//!
//! Every style, key bindings and translation file of the original's
//! `res/ui*` folders, its mods and its folder map
//! (`soundrts/tests/res/multi/jl4`) is changed at random many times over,
//! and each changed text must be read without a panic within
//! [`CASE_BOUND`]: as an installed layer, then as the layer of a map that
//! came from a server.
//!
//! The frames a server sends a seat during a real game are changed at
//! random, and put out of order, and the game interface must take each
//! stream without a panic: it plays on, or ends the game, says why, and
//! leaves it.
//!
//! The changes come from a fixed seed, so a failure reproduces: the message
//! gives the file or frame, the case and the start of what changed. The
//! cases run on a stack of 1 MiB, the size of a Windows program's main
//! thread.

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use soundrts::bindings::{BindingError, Bindings, Chord};
use soundrts::clock::Clock;
use soundrts::log::Log;
use soundrts::parameters::Parameters;
use soundrts::sound_cache::{SoundCache, update_text_table};
use soundrts::style::Style;
use soundrts::testkit::{FakeStream, Harness, Script, TestGame, slot, upstream_root};
use soundrts_protocol::game::{RulesHash, SeatFrame, encode_frame};
use soundrts_protocol::{AiLevel, ClockChange, Login, SeatChange, Speed};
use soundrts_world::msg::MsgPart;
use soundrts_world::player::{Objective, StatEvent, StatTarget};
use soundrts_world::{Event, Place};

/// The longest one case may take: reading the changed text as an installed
/// layer and as a downloaded one, then looking up what the game looks up.
/// The original's files take a few milliseconds in a debug build.
const CASE_BOUND: Duration = Duration::from_secs(2);

/// Changed texts per file. With the 39 files, 3,120 cases.
const CASES_PER_FILE: u64 = 80;

const SEED: u64 = 0x50_756e_6453;

/// SplitMix64: small, and the same on every machine.
struct Rng(u64);

impl Rng {
    fn new(file: u64, case: u64) -> Rng {
        let mut r = Rng(SEED ^ file.wrapping_mul(0x9e37_79b9_7f4a_7c15));
        r.0 ^= r.next().wrapping_add(case);
        r
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A number from 0 to `n - 1`, or 0 when `n` is 0.
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }

    fn pick<'a>(&mut self, items: &'a [&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
}

/// A word chosen to break a reader of these files.
fn hostile_word(r: &mut Rng) -> String {
    match r.below(11) {
        0 => "9".repeat(1 + r.below(400)),
        1 => r
            .pick(&[
                "-1",
                "0",
                "1e308",
                "nan",
                "inf",
                "-0",
                "99999999999999999999",
            ])
            .to_owned(),
        2 => r
            .pick(&[
                "é",
                "日本",
                "\u{202e}",
                "1é",
                "a_level_é",
                "_level_",
                "x_level_",
                "\u{0}",
            ])
            .to_owned(),
        3 => r
            .pick(&[
                "if_me", "else", "ambient", "loop", "repeat", "is_a", "def", "clear",
            ])
            .to_owned(),
        4 => r
            .pick(&[
                "#define", ":", "::", "CTRL", "ALT", "SHIFT", "WIN", "F13", "KP0",
            ])
            .to_owned(),
        5 => r
            .pick(&["\\", ";", "\r", "\u{feff}", "\t", "  "])
            .to_owned(),
        6 => format!("Z{}", r.below(8)),
        7 => r
            .pick(&["footman", "peasant", "parameters", "unit", "a", "b"])
            .to_owned(),
        8 => "x ".repeat(1 + r.below(3000)),
        9 => format!(
            "{}_level_{}",
            r.pick(&["attack_hit", "title", "noise"]),
            r.below(12)
        ),
        _ => format!("{}", r.below(100_000)),
    }
}

/// Changes the text one to three times.
fn mutate(text: &str, r: &mut Rng) -> String {
    let mut lines: Vec<String> = text.split('\n').map(str::to_owned).collect();
    for _ in 0..1 + r.below(3) {
        let n = lines.len();
        let i = r.below(n);
        let j = r.below(n);
        match r.below(14) {
            0 if n > 0 => {
                lines.remove(i);
            }
            1 if n > 0 => {
                let l = lines[i].clone();
                lines.insert(j, l);
            }
            2 if n > 0 => lines.swap(i, j),
            3..=5 if n > 0 => {
                let mut words: Vec<String> =
                    lines[i].split_whitespace().map(str::to_owned).collect();
                let k = r.below(words.len());
                let m = r.below(words.len());
                match r.below(5) {
                    0 if !words.is_empty() => {
                        words.remove(k);
                    }
                    1 if !words.is_empty() => {
                        let w = words[k].clone();
                        words.insert(m, w);
                    }
                    2 if !words.is_empty() => words.swap(k, m),
                    3 if !words.is_empty() => words[k] = hostile_word(r),
                    _ => words.insert(k, hostile_word(r)),
                }
                lines[i] = words.join(" ");
            }
            6 if n > 0 => {
                // A line, or its first word, repeated many times.
                let count = 1 + r.below(2000);
                if r.below(2) == 0 {
                    let first = lines[i].split_whitespace().next().unwrap_or("def");
                    lines.insert(j, vec![first; count].join(" "));
                } else {
                    let l = lines[i].clone();
                    for _ in 0..count {
                        lines.insert(j, l.clone());
                    }
                }
            }
            7 => {
                // Definitions that double: each name twice the last.
                let depth = 1 + r.below(64);
                lines.insert(j, "#define Z0 x x".to_owned());
                for d in 1..depth {
                    lines.insert(j + d, format!("#define Z{d} Z{} Z{}", d - 1, d - 1));
                }
                lines.insert(j + depth, format!("a: z Z{}", depth - 1));
            }
            8 => {
                // A chain or a web of parents.
                let count = 1 + r.below(300);
                let parents = 1 + r.below(12);
                for d in 0..count {
                    let is_a: Vec<String> = (0..parents)
                        .map(|_| format!("o{}", r.below(count + 1)))
                        .collect();
                    lines.insert(j, format!("def o{d}\nis_a {}\nt{d} 1", is_a.join(" ")));
                }
            }
            9 if n > 0 => {
                let l = lines[i].clone();
                lines[i] = format!("{l} \\");
            }
            10 if n > 0 => {
                // Cut a line at any byte, even inside a character.
                let bytes = lines[i].as_bytes();
                let cut = r.below(bytes.len() + 1);
                lines[i] = String::from_utf8_lossy(&bytes[..cut]).into_owned();
            }
            11 => {
                let w = r.pick(&["is_a", "noise", "noise_when_moving", "title", "alert"]);
                lines.insert(j, format!("{w} {}", hostile_word(r)));
            }
            12 => lines.insert(j, format!("{} {}", r.below(100_000), hostile_word(r))),
            _ => lines.insert(j, hostile_word(r)),
        }
    }
    lines.join(if r.below(10) == 0 { "\r\n" } else { "\n" })
}

/// What kind of interface file a file is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Style,
    Bindings,
    Translation,
}

/// The files changed, in a fixed order.
fn files() -> Vec<(PathBuf, Kind)> {
    let root = upstream_root();
    let mut found = Vec::new();
    let mut dirs = vec![
        root.join("res"),
        root.join("mods"),
        root.join("soundrts/tests/res/multi/jl4"),
    ];
    while let Some(dir) = dirs.pop() {
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = e.path();
            if path.is_dir() {
                dirs.push(path);
                continue;
            }
            let in_ui = path
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|d| d.to_string_lossy().starts_with("ui"));
            let kind = match path.file_name().and_then(|n| n.to_str()) {
                Some("style.txt") => Kind::Style,
                Some("bindings.txt") => Kind::Bindings,
                Some("tts.txt") => Kind::Translation,
                _ => continue,
            };
            if in_ui {
                found.push((path, kind));
            }
        }
    }
    found.sort();
    found
}

/// Reads any name as a command.
fn any(name: &str, args: Vec<String>) -> Result<(String, Vec<String>), BindingError> {
    Ok((name.to_owned(), args))
}

/// Reads `text` as the game would, as an installed layer over `base`
/// (the game's own text of that kind), then as a downloaded map's.
fn read(kind: Kind, base: &str, text: &str) {
    let log = Log::silent();
    match kind {
        Kind::Style => {
            let mut style = Style::new(log.clone());
            style.load(&[base, text]);
            look_up(&style);
            let mut style = Style::new(log);
            style.load(&[base]);
            style.add_downloaded(&[text]);
            look_up(&style);
        }
        Kind::Bindings => {
            let mut b = Bindings::new();
            b.load(&format!("{base}\n{text}"), &any, &log);
            for chord in ["a", "SHIFT z", "CTRL SHIFT F2", "ALT F4"] {
                if let Ok(c) = Chord::parse(chord) {
                    let _ = b.get(&c);
                }
            }
            assert!(b.len() < 100_000);
        }
        Kind::Translation => {
            let mut table = HashMap::new();
            update_text_table(&mut table, text, &log);
            let params = Arc::new(Parameters::default());
            let mut cache = SoundCache::new(
                &[base.to_owned(), text.to_owned()],
                &[],
                params.clone(),
                log.clone(),
            );
            translate(&mut cache);
            let mut cache = SoundCache::new(&[base.to_owned()], &[], params, log);
            cache.add_downloaded(&[text.to_owned()], &[]);
            translate(&mut cache);
        }
    }
}

fn look_up(style: &Style) {
    let objects: Vec<String> = style.objects().map(str::to_owned).collect();
    for o in objects.iter().take(300) {
        for attr in [
            "title",
            "noise",
            "is_a",
            "attack_hit_level_2",
            "x_level_",
            "é_level_1",
        ] {
            let _ = style.get_or_warn(o, attr, false);
        }
        let _ = style.noise(o, "noise_when_moving");
        let _ = style.words(o, "alert");
    }
}

fn translate(cache: &mut SoundCache) {
    for key in [
        "0",
        "5",
        "9000",
        "1001",
        "999999999999",
        "18446744073709551616",
        "é",
    ] {
        let _ = cache.translate(&MsgPart::from(key));
    }
}

#[test]
fn hostile_interface_files_are_read_without_a_panic() {
    let files = files();
    assert_eq!(files.len(), 39, "{files:#?}");
    let root = upstream_root();
    let base = |name: &str| std::fs::read_to_string(root.join("res/ui").join(name)).unwrap();
    let bases = [base("style.txt"), base("bindings.txt"), base("tts.txt")];
    let worker = std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(move || {
            let mut failures = Vec::new();
            let mut slowest = Duration::ZERO;
            for (f, (path, kind)) in files.iter().enumerate() {
                let original = std::fs::read_to_string(path).unwrap();
                let base = match kind {
                    Kind::Style => &bases[0],
                    Kind::Bindings => &bases[1],
                    Kind::Translation => &bases[2],
                };
                for case in 0..CASES_PER_FILE {
                    let mut r = Rng::new(f as u64, case);
                    let text = mutate(&original, &mut r);
                    let start = Instant::now();
                    let result = catch_unwind(AssertUnwindSafe(|| read(*kind, base, &text)));
                    let took = start.elapsed();
                    slowest = slowest.max(took);
                    if result.is_err() || took > CASE_BOUND {
                        let head: String = text.chars().take(300).collect();
                        failures.push(format!(
                            "{} case {case}: {} after {took:?}\n{head}",
                            path.display(),
                            if result.is_err() { "panic" } else { "too slow" }
                        ));
                    }
                }
            }
            (failures, slowest)
        })
        .unwrap();
    let (failures, slowest) = worker.join().unwrap();
    eprintln!("slowest case: {slowest:?}");
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

// -- Hostile games on a server --------------------------------------------

/// Turns of the real game whose frames are changed.
const STREAM_TURNS: usize = 30;

/// Streams with a frame changed at random.
const FRAME_CASES: u64 = 300;

/// The frames a server sends Andrew in a real game against a computer on
/// jl1, encoded: the seat, the start, the whole message, a change a turn,
/// and the end.
fn real_stream(h: &mut Harness) -> Vec<Vec<u8>> {
    let slots = [
        slot(
            soundrts_protocol::SlotPlayer::Human(Login::new("andrew").unwrap()),
            1,
        ),
        slot(soundrts_protocol::SlotPlayer::Computer(AiLevel::Easy), 2),
    ];
    let mut stream = FakeStream::new(h, "jl1", &slots, 0, 1);
    let map = h.multiplayer_map("jl1").info().unwrap();
    let mut frames = stream.opening(map, Speed::Normal);
    for _ in 0..STREAM_TURNS {
        frames.extend(stream.turn(Vec::new()));
    }
    frames.push(stream.end());
    frames.iter().map(|f| encode_frame(f).unwrap()).collect()
}

/// Changes one to four bytes of `bytes`, cuts it, or grows it.
fn mutate_bytes(r: &mut Rng, bytes: &mut Vec<u8>) {
    for _ in 0..1 + r.below(4) {
        let i = r.below(bytes.len());
        match r.below(6) {
            0 if !bytes.is_empty() => bytes[i] ^= 1 << r.below(8),
            1 if !bytes.is_empty() => bytes[i] = r.next() as u8,
            2 if !bytes.is_empty() => bytes[i] = 0xff,
            3 => bytes.truncate(i),
            4 => bytes.insert(i, r.next() as u8),
            _ if !bytes.is_empty() => {
                bytes.remove(i);
            }
            _ => bytes.push(0),
        }
    }
}

/// Plays a stream of frames as Andrew's client takes them from a server,
/// a turn every 0.3 s, to its end. Whether the game ended on a frame that
/// cannot be right.
fn play_stream(h: &mut Harness, frames: &[Vec<u8>]) -> bool {
    let server = h.join_fake_server("andrew", &[]);
    let t0 = h.clock.now();
    for (i, f) in frames.iter().enumerate() {
        server.bytes_at(t0 + 0.3 * i.saturating_sub(2) as f64, f);
    }
    let said = h.utterances().len();
    let refused = |h: &Harness| {
        h.said_since(said)
            .iter()
            .any(|s| s == "game ended: the server sent what cannot be right")
    };
    soundrts::net::pump(&mut h.client);
    let Some(seated) = h.client.net.as_mut().and_then(|n| n.started.take()) else {
        return false;
    };
    let map = h.multiplayer_map("jl1");
    h.client.res.set_map(Some(&map));
    let Some(game) = soundrts::session::join_server_game(&mut h.client, &seated) else {
        h.client.res.set_map(None);
        h.run_voice_for(5.0);
        return refused(h);
    };
    let mut g = TestGame {
        session: None,
        game,
    };
    h.run_for(&mut g, 0.3 * frames.len() as f64 + 5.0);
    assert!(g.game.end_loop, "the stream ended, and the game did not");
    h.finish(&mut g);
    h.client.res.set_map(None);
    refused(h)
}

/// The frame at `i` of `frames`, decoded.
fn frame(frames: &[Vec<u8>], i: usize) -> SeatFrame {
    soundrts_protocol::game::decode_frame(&frames[i]).unwrap()
}

fn encoded(f: SeatFrame) -> Vec<u8> {
    encode_frame(&f).unwrap()
}

/// Streams in the wrong order, or with frames no game sends. Each ends the
/// game as soon as it is read, except those [`MAY_PLAY_ON`] names.
fn out_of_order(frames: &[Vec<u8>]) -> Vec<(&'static str, Vec<Vec<u8>>)> {
    let n = frames.len();
    let with = |at: usize, f: Vec<u8>| {
        let mut s = frames.to_vec();
        s.insert(at, f);
        s
    };
    let mut cases = vec![
        ("a change before the start", {
            let mut s = frames.to_vec();
            s.swap(1, 3);
            s
        }),
        ("the start twice", with(2, frames[1].clone())),
        ("the whole message twice", with(3, frames[2].clone())),
        (
            "a whole message during the game",
            with(10, frames[2].clone()),
        ),
        ("a turn played again", with(10, frames[5].clone())),
        ("a turn missed", {
            let mut s = frames.to_vec();
            s.remove(6);
            s
        }),
        ("the end twice", with(n - 1, frames[n - 1].clone())),
        ("a turn after the end", with(n, frames[n - 2].clone())),
        ("a seat during the game", with(8, frames[0].clone())),
        (
            "news of a slot that is not in the room",
            with(
                8,
                encoded(SeatFrame::Seat {
                    slot: usize::MAX,
                    change: SeatChange::Returned,
                }),
            ),
        ),
        (
            "a refused start during the game",
            with(
                8,
                encoded(SeatFrame::NotStarted(soundrts_protocol::StartRefusal::Busy)),
            ),
        ),
        ("bytes that are no frame", with(8, vec![0xff; 40])),
        ("food out of range", {
            let mut s = frames.to_vec();
            let SeatFrame::Whole(mut m) = frame(frames, 2) else {
                panic!("a whole message");
            };
            m.state.available_food = i64::MAX;
            m.state.used_food = -1;
            s[2] = encoded(SeatFrame::Whole(m));
            s
        }),
        ("an entity inside itself", {
            let mut s = frames.to_vec();
            let SeatFrame::Whole(mut m) = frame(frames, 2) else {
                panic!("a whole message");
            };
            let e = &mut m.perceived[0];
            e.place = Some(Place::Inside(e.id));
            s[2] = encoded(SeatFrame::Whole(m));
            s
        }),
        ("too many objectives", {
            let mut s = frames.to_vec();
            let SeatFrame::Whole(mut m) = frame(frames, 2) else {
                panic!("a whole message");
            };
            m.state.objectives = (0..=soundrts_world::MAX_OBJECTIVES)
                .map(|i| Objective {
                    number: i.to_string(),
                    description: vec![1],
                })
                .collect();
            s[2] = encoded(SeatFrame::Whole(m));
            s
        }),
        ("statistics out of range", {
            let mut s = frames.to_vec();
            let SeatFrame::End(mut end) = frame(frames, n - 1) else {
                panic!("an end");
            };
            end.stats
                .add(StatEvent::Gathered, StatTarget::Resource(9), i64::MAX);
            s[n - 1] = encoded(SeatFrame::End(end));
            s
        }),
        ("more events than a game can tell", {
            let mut s = frames.to_vec();
            let SeatFrame::Change(mut c) = frame(frames, 3) else {
                panic!("a change");
            };
            c.events
                .extend(std::iter::repeat_n(Event::Speed(1.0), 70_000));
            s[3] = encoded(SeatFrame::Change(c));
            s
        }),
    ];
    cases.push((
        "a clock change",
        with(
            8,
            encoded(SeatFrame::Clock {
                by: Login::new("nobody").unwrap(),
                change: ClockChange::Paused,
            }),
        ),
    ));
    cases
}

/// The streams of [`out_of_order`] that may play on: a turn missed may
/// look like a seat catching up, the game is over at the first end, and
/// clock news names its player by login, who need not be in the room.
const MAY_PLAY_ON: [&str; 4] = [
    "a turn missed",
    "the end twice",
    "a turn after the end",
    "a clock change",
];

#[test]
fn hostile_frames_of_a_game_on_a_server_are_taken_without_a_panic() {
    let worker = std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(|| {
            let mut h = Harness::new("en", Script::new().keep_open());
            let frames = real_stream(&mut h);
            let mut failures = Vec::new();
            // The real stream plays to its end.
            assert!(!play_stream(&mut h, &frames), "the real stream was refused");
            for (what, stream) in out_of_order(&frames) {
                let refused = catch_unwind(AssertUnwindSafe(|| play_stream(&mut h, &stream)));
                match refused {
                    Ok(true) => {}
                    Ok(false) if MAY_PLAY_ON.contains(&what) => {}
                    Ok(false) => failures.push(format!("{what}: not refused")),
                    Err(_) => failures.push(format!("{what}: panic")),
                }
            }
            let mut refused = 0;
            for case in 0..FRAME_CASES {
                let mut r = Rng::new(0xF2A3, case);
                let mut stream = frames.clone();
                let i = r.below(stream.len());
                mutate_bytes(&mut r, &mut stream[i]);
                let start = Instant::now();
                let result = catch_unwind(AssertUnwindSafe(|| play_stream(&mut h, &stream)));
                let took = start.elapsed();
                refused += usize::from(matches!(result, Ok(true)));
                if result.is_err() {
                    let kind = format!("{:?}", frame(&frames, i));
                    let kind: String = kind.chars().take(60).collect();
                    failures.push(format!(
                        "case {case}: frame {i} ({kind}...): panic after {took:?}"
                    ));
                }
            }
            (failures, refused)
        })
        .unwrap();
    let (failures, refused) = worker.join().unwrap();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    // Most changed frames cannot be right; some still are.
    assert!(
        refused > FRAME_CASES as usize / 4,
        "{refused} changed streams refused"
    );
}

/// Andrew's game against a computer on jl1 for a fake server, encoded: what
/// is sent before the connection is lost (the seat, the start, the whole
/// message, 5 turns); after 3 turns he missed, what is sent when he
/// returns (the seat, the start, the whole message); and 3 turns more,
/// then the end.
fn return_stream(h: &mut Harness) -> [Vec<Vec<u8>>; 3] {
    let mut stream = FakeStream::new(h, "jl1", &slots(), 0, 1);
    let map = h.multiplayer_map("jl1").info().unwrap();
    let mut before = stream.opening(map.clone(), Speed::Normal);
    for _ in 0..5 {
        before.extend(stream.turn(Vec::new()));
    }
    for _ in 0..3 {
        stream.turn(Vec::new());
    }
    let back = stream.opening(map, Speed::Normal);
    let mut later = Vec::new();
    for _ in 0..3 {
        later.extend(stream.turn(Vec::new()));
    }
    later.push(stream.end());
    let encode = |frames: Vec<SeatFrame>| frames.into_iter().map(encoded).collect();
    [encode(before), encode(back), encode(later)]
}

fn slots() -> [soundrts_protocol::Slot; 2] {
    [
        slot(
            soundrts_protocol::SlotPlayer::Human(Login::new("andrew").unwrap()),
            1,
        ),
        slot(soundrts_protocol::SlotPlayer::Computer(AiLevel::Easy), 2),
    ]
}

/// Plays `before` as Andrew's client takes it from a server, a turn every
/// 0.3 s; loses the connection; then returns to a server that sends
/// `back`, as it comes. What was said meanwhile.
fn play_return(h: &mut Harness, before: &[Vec<u8>], back: &[Vec<u8>]) -> Vec<String> {
    let server = h.join_fake_server("andrew", &[]);
    let map = h.multiplayer_map("jl1").info().unwrap();
    let room = soundrts::testkit::fake_room(map, slots().to_vec());
    h.enter_fake_room(&room);
    let t0 = h.clock.now();
    for (i, f) in before.iter().enumerate() {
        server.bytes_at(t0 + 0.3 * i.saturating_sub(2) as f64, f);
    }
    let lost = t0 + 0.3 * before.len() as f64;
    server.close_at(lost);
    let returned = server.dial_to(soundrts::testkit::welcome_back("andrew", &room, &[]));
    for (i, f) in back.iter().enumerate() {
        returned.bytes_at(lost + 0.5 + 0.3 * i.saturating_sub(2) as f64, f);
    }
    let said = h.utterances().len();
    soundrts::net::pump(&mut h.client);
    let seated = h
        .client
        .net
        .as_mut()
        .and_then(|n| n.started.take())
        .unwrap();
    let map = h.multiplayer_map("jl1");
    h.client.res.set_map(Some(&map));
    let game = soundrts::session::join_server_game(&mut h.client, &seated).unwrap();
    let mut g = TestGame {
        session: None,
        game,
    };
    h.run_for(&mut g, lost - t0 + 0.3 * back.len() as f64 + 5.0);
    let ended = g.game.end_loop;
    h.finish(&mut g);
    h.client.res.set_map(None);
    h.run_voice_for(5.0);
    let said = h.said_since(said);
    assert!(
        ended || !refused(&said),
        "the game said it ended, and went on"
    );
    said
}

/// Whether the game ended on what cannot be right.
fn refused(said: &[String]) -> bool {
    said.iter()
        .any(|s| s == "game ended: the server sent what cannot be right")
}

#[test]
fn hostile_frames_of_a_return_to_a_game_are_taken_without_a_panic() {
    let worker = std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(|| {
            let mut h = Harness::new("en", Script::new().keep_open());
            let [before, back, later] = return_stream(&mut h);
            let returned = |back: Vec<Vec<u8>>| [back, later.clone()].concat();
            // The real return plays on to the end.
            let said = play_return(&mut h, &before, &returned(back.clone()));
            assert!(!refused(&said), "the real return was refused: {said:#?}");
            assert!(said.iter().any(|s| s == "back in the game"), "{said:#?}");
            let with = |i: usize, f: Vec<u8>| {
                let mut s = back.clone();
                s[i] = f;
                returned(s)
            };
            // The opening of another seat of the game, or of another game.
            let mut other = |map: &str, you: usize| {
                let mut stream = FakeStream::new(&mut h, map, &slots(), you, 1);
                for _ in 0..8 {
                    stream.turn(Vec::new());
                }
                let map = h.multiplayer_map(map).info().unwrap();
                let opening = stream.opening(map, Speed::Normal);
                opening.into_iter().map(encoded).collect::<Vec<_>>()
            };
            let another_game = other("jl6", 0);
            let another_player = other("jl1", 1);
            let SeatFrame::Seated {
                map,
                speed,
                paused,
                hold,
                token,
                ..
            } = frame(&back, 0)
            else {
                panic!("a seat");
            };
            let SeatFrame::Start { start, .. } = frame(&back, 1) else {
                panic!("a start");
            };
            let cases = [
                (
                    "the seat of another slot",
                    with(
                        0,
                        encoded(SeatFrame::Seated {
                            you: soundrts_protocol::game::Role::Player(1),
                            map,
                            speed,
                            paused,
                            hold,
                            token,
                        }),
                    ),
                ),
                (
                    "the start of another game",
                    with(1, another_game[1].clone()),
                ),
                (
                    "the start of the game with other rules",
                    with(
                        1,
                        encoded(SeatFrame::Start {
                            start,
                            rules: RulesHash::of("other rules"),
                        }),
                    ),
                ),
                (
                    "the message of another player",
                    with(2, another_player[2].clone()),
                ),
                ("a message back in time", with(2, before[2].clone())),
                (
                    "a change before the whole message",
                    with(2, later[0].clone()),
                ),
                ("the whole message before the start", {
                    let mut s = back.clone();
                    s.swap(1, 2);
                    returned(s)
                }),
                ("the start before the seat", {
                    let mut s = back.clone();
                    s.swap(0, 1);
                    returned(s)
                }),
                ("bytes that are no frame", with(1, vec![0xff; 40])),
            ];
            let mut failures = Vec::new();
            for (what, stream) in cases {
                match catch_unwind(AssertUnwindSafe(|| play_return(&mut h, &before, &stream))) {
                    Ok(said) if refused(&said) => {}
                    Ok(_) => failures.push(format!("{what}: not refused")),
                    Err(_) => failures.push(format!("{what}: panic")),
                }
            }
            for case in 0..RETURN_CASES {
                let mut r = Rng::new(0x8E7, case);
                let mut stream = back.clone();
                let i = r.below(stream.len());
                mutate_bytes(&mut r, &mut stream[i]);
                let result = catch_unwind(AssertUnwindSafe(|| {
                    play_return(&mut h, &before, &returned(stream))
                }));
                if result.is_err() {
                    failures.push(format!("case {case}: frame {i} of the return: panic"));
                }
            }
            failures
        })
        .unwrap();
    let failures = worker.join().unwrap();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Returns with a frame changed at random.
const RETURN_CASES: u64 = 60;

/// The frames a server sends Carol, who watches Andrew's game against a
/// computer on jl1: the seat, the start, Andrew's whole message, then a
/// change a turn, and Andrew's whole message again every other turn, as
/// when she asks to follow the next player and he is the only one; then
/// the end.
fn watcher_stream(h: &mut Harness) -> Vec<Vec<u8>> {
    let mut stream = FakeStream::new(h, "jl1", &slots(), 0, 1).watching();
    let map = h.multiplayer_map("jl1").info().unwrap();
    let mut frames = stream.opening(map, Speed::Normal);
    for turn in 0..STREAM_TURNS {
        frames.extend(stream.turn(Vec::new()));
        if turn % 2 == 1 {
            frames.push(stream.follow(0));
        }
    }
    frames.push(stream.end());
    frames.into_iter().map(encoded).collect()
}

#[test]
fn hostile_frames_of_a_watched_game_are_taken_without_a_panic() {
    let worker = std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(|| {
            let mut h = Harness::new("en", Script::new().keep_open());
            let frames = watcher_stream(&mut h);
            let mut failures = Vec::new();
            // The real stream plays to its end: a whole message during the
            // game is a watcher's own.
            assert!(!play_stream(&mut h, &frames), "the real stream was refused");
            let whole = |i: usize| {
                let SeatFrame::Whole(m) = frame(&frames, i) else {
                    panic!("a whole message at {i}");
                };
                m
            };
            let later = frames
                .iter()
                .rposition(|f| {
                    matches!(
                        soundrts_protocol::game::decode_frame(f),
                        Ok(SeatFrame::Whole(_))
                    )
                })
                .unwrap();
            let with = |at: usize, f: Vec<u8>| {
                let mut s = frames.clone();
                s.insert(at, f);
                s
            };
            let cases = [
                // The first whole message, from the start of the game.
                (
                    "a whole message back in time",
                    with(later + 1, frames[2].clone()),
                ),
                ("a whole message of a player no longer in the game", {
                    let mut m = whole(later);
                    m.state.players.retain(|&p| p != m.seat);
                    with(later + 1, encoded(SeatFrame::Whole(m)))
                }),
                ("a whole message of a player that is none", {
                    let mut m = whole(later);
                    m.seat = soundrts_world::PlayerId(200);
                    with(later + 1, encoded(SeatFrame::Whole(m)))
                }),
            ];
            for (what, stream) in cases {
                match catch_unwind(AssertUnwindSafe(|| play_stream(&mut h, &stream))) {
                    Ok(true) => {}
                    Ok(false) => failures.push(format!("{what}: not refused")),
                    Err(_) => failures.push(format!("{what}: panic")),
                }
            }
            for case in 0..FRAME_CASES / 3 {
                let mut r = Rng::new(0x5A7C, case);
                let mut stream = frames.clone();
                let i = r.below(stream.len());
                mutate_bytes(&mut r, &mut stream[i]);
                if catch_unwind(AssertUnwindSafe(|| play_stream(&mut h, &stream))).is_err() {
                    failures.push(format!("case {case}: frame {i}: panic"));
                }
            }
            failures
        })
        .unwrap();
    let failures = worker.join().unwrap();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
