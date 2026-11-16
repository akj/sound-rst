//! The size and cost of the game stream, measured on whole games: what
//! each seat would be sent, in each encoding compared. Ignored, since a
//! run takes minutes; run it in a release build:
//!
//! ```text
//! cargo test --release -p soundrts-protocol --test stream_measure -- --ignored --nocapture
//! ```
//!
//! Each game is played twice, since the machine may be busy with other
//! work: the bytes must be the same both times, and the times show how
//! much the machine moved them. `MEASURE_TURNS` changes the number of
//! turns (6,000 by default) and `MEASURE_GAMES` picks games by name
//! (`jl1,jl7,js13`).
//!
//! What is measured, for every seat on every turn (a whole message, on
//! every tenth turn only):
//!
//! - the whole message, as a seat that joins or returns is sent it;
//! - the change since the seat's last message, as it is sent every turn;
//! - the turn's events alone, a part of the change;
//! - the change for a seat 10 and 50 turns behind, with the events it
//!   missed that are not about an entity;
//! - the time to make and encode a change, and to decode and apply it.
//!
//! Compressed encodings stand for the WebSocket's per-message compression
//! (RFC 7692) with its default context takeover: raw deflate, one stream
//! per seat, flushed at the end of each message, without the 4 bytes the
//! RFC drops. A whole message or a catch-up starts a fresh stream, as on a
//! new connection.

use std::collections::VecDeque;
use std::time::Instant;

use std::io::Write;

use flate2::Compression;
use flate2::write::{DeflateDecoder, DeflateEncoder};
use soundrts_protocol::game;
use soundrts_world::test_games::{Case, Game, every_one_alone};
use soundrts_world::{Event, TurnChange, TurnMessage};

fn read(relative: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/game-data")
        .join(relative);
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("{relative}: {e}: run scripts/fetch-upstream.ps1"));
    String::from_utf8_lossy(&bytes).into_owned()
}

/// An encoding compared.
#[derive(Clone, Copy, PartialEq)]
enum Encoding {
    Json,
    JsonDeflate,
    Postcard,
    PostcardDeflate,
}

const ENCODINGS: [Encoding; 4] = [
    Encoding::Json,
    Encoding::JsonDeflate,
    Encoding::Postcard,
    Encoding::PostcardDeflate,
];

impl Encoding {
    fn name(self) -> &'static str {
        match self {
            Encoding::Json => "JSON",
            Encoding::JsonDeflate => "JSON, deflate",
            Encoding::Postcard => "postcard",
            Encoding::PostcardDeflate => "postcard, deflate",
        }
    }

    fn deflated(self) -> bool {
        matches!(self, Encoding::JsonDeflate | Encoding::PostcardDeflate)
    }

    fn encode<T: serde::Serialize>(self, value: &T) -> Vec<u8> {
        match self {
            Encoding::Json | Encoding::JsonDeflate => serde_json::to_vec(value).unwrap(),
            Encoding::Postcard | Encoding::PostcardDeflate => postcard::to_allocvec(value).unwrap(),
        }
    }

    fn decode<T: serde::de::DeserializeOwned>(self, bytes: &[u8]) -> T {
        match self {
            Encoding::Json | Encoding::JsonDeflate => serde_json::from_slice(bytes).unwrap(),
            Encoding::Postcard | Encoding::PostcardDeflate => postcard::from_bytes(bytes).unwrap(),
        }
    }
}

/// Both ends of one seat's per-message-deflate stream.
struct Deflate {
    out: DeflateEncoder<Vec<u8>>,
    back: DeflateDecoder<Vec<u8>>,
}

impl Deflate {
    fn new() -> Deflate {
        Deflate {
            out: DeflateEncoder::new(Vec::new(), Compression::default()),
            back: DeflateDecoder::new(Vec::new()),
        }
    }

    /// The message compressed, as the stream sends it.
    fn compress(&mut self, message: &[u8]) -> Vec<u8> {
        self.out.write_all(message).unwrap();
        self.out.flush().unwrap();
        let mut out = std::mem::take(self.out.get_mut());
        assert!(out.ends_with(&[0, 0, 0xff, 0xff]));
        out.truncate(out.len() - 4);
        out
    }

    /// The message back, as the other end reads it.
    fn decompress(&mut self, sent: &[u8]) -> Vec<u8> {
        self.back.write_all(sent).unwrap();
        self.back.write_all(&[0, 0, 0xff, 0xff]).unwrap();
        self.back.flush().unwrap();
        std::mem::take(self.back.get_mut())
    }
}

/// Values per seat per turn, for the median, the 99th percentile and the
/// largest.
#[derive(Default)]
struct Spread(Vec<u64>);

impl Spread {
    fn add(&mut self, v: u64) {
        self.0.push(v);
    }

    fn summary(&self) -> [u64; 3] {
        let mut v = self.0.clone();
        if v.is_empty() {
            return [0; 3];
        }
        v.sort_unstable();
        let at = |q: f64| v[((q * v.len() as f64).ceil() as usize).clamp(1, v.len()) - 1];
        [at(0.5), at(0.99), *v.last().unwrap()]
    }
}

#[derive(Default)]
struct Figures {
    whole: Spread,
    change: Spread,
    events: Spread,
    behind_10: Spread,
    behind_50: Spread,
    /// Microseconds, as nanoseconds.
    make_and_encode_ns: Spread,
    decode_and_apply_ns: Spread,
}

fn sent(enc: Encoding, deflate: &mut Deflate, bytes: &[u8]) -> Vec<u8> {
    if enc.deflated() {
        deflate.compress(bytes)
    } else {
        bytes.to_vec()
    }
}

/// Plays the case once and measures every encoding. Returns the figures,
/// by encoding, and the number of turns played.
fn measure(case: &Case) -> (Vec<Figures>, usize) {
    let mut game = Game::new(case, read, true);
    let start = game.start();
    // The same game without events for computers: the messages but for
    // events must stay the same, which shows that telling every seat its
    // events changes nothing else.
    let mut quiet = Game::new(case, read, false);
    let seats = game.players().len();
    let mut figures: Vec<Figures> = ENCODINGS.iter().map(|_| Figures::default()).collect();
    // Each seat's stream, both ends, by encoding.
    let mut streams: Vec<Vec<Deflate>> = ENCODINGS
        .iter()
        .map(|_| (0..seats).map(|_| Deflate::new()).collect())
        .collect();
    let mut last: Vec<TurnMessage> = game.messages();
    // Each seat's last 50 messages, as postcard, with the events of their
    // turns.
    let mut history: Vec<VecDeque<(Vec<u8>, Vec<Event>)>> =
        (0..seats).map(|_| VecDeque::new()).collect();
    let mut turn = 0;
    while let Some(now) = game.step() {
        let quiet_now = quiet.step().expect("the same game");
        turn += 1;
        if turn % 500 == 0 {
            for (a, b) in now.iter().zip(&quiet_now) {
                let without_events = TurnMessage {
                    events: Vec::new(),
                    ..a.clone()
                };
                assert_eq!(
                    &without_events, b,
                    "{}: telling events changed the game",
                    case.name
                );
            }
        }
        for (s, m) in now.iter().enumerate() {
            let t = Instant::now();
            let change = TurnChange::between(&last[s], m);
            let make = t.elapsed().as_nanos() as u64;
            let h = &history[s];
            let behind = |back: usize| {
                (h.len() >= back).then(|| {
                    let (old, _) = &h[h.len() - back];
                    let old: TurnMessage = postcard::from_bytes(old).unwrap();
                    let missed = h.iter().skip(h.len() - back + 1).flat_map(|(_, e)| e);
                    TurnChange::catching_up(&old, missed, m)
                })
            };
            let behind = [behind(10), behind(50)];
            for (k, &enc) in ENCODINGS.iter().enumerate() {
                let f = &mut figures[k];
                let stream = &mut streams[k][s];
                let t = Instant::now();
                let bytes = enc.encode(&change);
                let wire = sent(enc, stream, &bytes);
                f.make_and_encode_ns
                    .add(make + t.elapsed().as_nanos() as u64);
                f.change.add(wire.len() as u64);
                let t = Instant::now();
                let plain = if enc.deflated() {
                    stream.decompress(&wire)
                } else {
                    wire
                };
                let applied = match enc {
                    // The protocol's own function, which also checks what
                    // it applied.
                    Encoding::Postcard | Encoding::PostcardDeflate => {
                        game::apply_change(&plain, &last[s], &start).unwrap()
                    }
                    Encoding::Json | Encoding::JsonDeflate => {
                        let back: TurnChange = enc.decode(&plain);
                        let applied = back.apply(&last[s]).unwrap();
                        start.check(&applied).unwrap();
                        applied
                    }
                };
                f.decode_and_apply_ns.add(t.elapsed().as_nanos() as u64);
                assert_eq!(&applied, m);
                if enc == Encoding::Postcard {
                    assert_eq!(bytes, game::encode_change(&change).unwrap());
                }
                let events = enc.encode(&change.events);
                f.events
                    .add(sent(enc, &mut Deflate::new(), &events).len() as u64);
                if turn % 10 == 0 {
                    let whole = enc.encode(m);
                    f.whole
                        .add(sent(enc, &mut Deflate::new(), &whole).len() as u64);
                }
                for (c, spread) in behind.iter().zip([&mut f.behind_10, &mut f.behind_50]) {
                    if let Some(c) = c {
                        let bytes = enc.encode(c);
                        spread.add(sent(enc, &mut Deflate::new(), &bytes).len() as u64);
                    }
                }
            }
            let h = &mut history[s];
            h.push_back((postcard::to_allocvec(m).unwrap(), m.events.clone()));
            if h.len() > 50 {
                h.pop_front();
            }
        }
        last = now;
    }
    (figures, turn)
}

fn kb(bytes: u64) -> String {
    format!("{:.2}", bytes as f64 / 1000.0)
}

fn report(name: &str, run: usize, turns: usize, figures: &[Figures]) {
    eprintln!("\n{name}, run {run}: {turns} turns");
    eprintln!("per seat per turn: median / 99th percentile / largest");
    for (k, &enc) in ENCODINGS.iter().enumerate() {
        let f = &figures[k];
        let b = |s: &Spread| {
            let [a, b, c] = s.summary();
            format!("{a} / {b} / {c}")
        };
        let per_s = |s: &Spread| {
            let [a, b, c] = s.summary();
            format!(
                "{} / {} / {}",
                kb(a * 10 / 3),
                kb(b * 10 / 3),
                kb(c * 10 / 3)
            )
        };
        let us = |s: &Spread| {
            let [a, b, c] = s.summary();
            format!(
                "{:.1} / {:.1} / {:.1}",
                a as f64 / 1e3,
                b as f64 / 1e3,
                c as f64 / 1e3
            )
        };
        eprintln!("  {}:", enc.name());
        eprintln!("    whole message, bytes: {}", b(&f.whole));
        eprintln!("    change, bytes: {}", b(&f.change));
        eprintln!("    events, bytes: {}", b(&f.events));
        eprintln!(
            "    change, kB a second at 300 ms a turn: {}",
            per_s(&f.change)
        );
        eprintln!("    10 turns behind, bytes: {}", b(&f.behind_10));
        eprintln!("    50 turns behind, bytes: {}", b(&f.behind_50));
        eprintln!(
            "    make and encode a change, us: {}",
            us(&f.make_and_encode_ns)
        );
        eprintln!(
            "    decode and apply a change, us: {}",
            us(&f.decode_and_apply_ns)
        );
    }
}

#[test]
#[ignore = "minutes: run by hand in a release build"]
fn measure_the_game_stream() {
    let turns: usize = std::env::var("MEASURE_TURNS")
        .ok()
        .and_then(|t| t.parse().ok())
        .unwrap_or(6000);
    let only = std::env::var("MEASURE_GAMES").unwrap_or_else(|_| "jl1,jl7,js13".into());
    let cases = [
        (
            "jl1",
            every_one_alone("jl1, 2 computers", "res/multi/jl1.txt", 2, turns),
        ),
        (
            "jl7",
            every_one_alone("jl7, 8 computers", "res/multi/jl7.txt", 8, turns),
        ),
        (
            "js13",
            every_one_alone("js13, 16 computers", "res/multi/js13.txt", 16, turns),
        ),
    ];
    for (key, case) in &cases {
        if !only.split(',').any(|o| o == *key) {
            continue;
        }
        let mut bytes: Option<Vec<[u64; 3]>> = None;
        for run in 1..=2 {
            let (figures, played) = measure(case);
            report(case.name, run, played, &figures);
            let sizes: Vec<[u64; 3]> = figures
                .iter()
                .flat_map(|f| [&f.whole, &f.change, &f.events, &f.behind_10, &f.behind_50])
                .map(Spread::summary)
                .collect();
            if let Some(first) = &bytes {
                assert_eq!(
                    first, &sizes,
                    "{}: the bytes differ between runs",
                    case.name
                );
            }
            bytes = Some(sizes);
        }
    }
}
