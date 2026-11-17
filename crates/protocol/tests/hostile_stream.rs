//! Hostile bytes on the game stream. Messages of a real game, each kind
//! the stream carries, are changed at random many times over, and each
//! changed message must decode, apply and pass the checks, or be refused
//! with an error, and never panic. A message cut short is always refused.
//!
//! The changes come from a fixed seed, so a failure reproduces: the
//! message gives the seed, the kind of message and the case.
//! `HOSTILE_STREAM_CASES` changes the number of changes per message, and
//! `HOSTILE_STREAM_SEED` the seed.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use soundrts_protocol::game::{
    apply_change, decode_command, decode_end, decode_setup, decode_start, decode_turn,
    encode_change, encode_command, encode_end, encode_setup, encode_start, encode_turn,
};
use soundrts_world::orders::Keyword;
use soundrts_world::test_games::{Game, cheat_mode_and_defeat};
use soundrts_world::{
    Command, EntityId, GameStart, KeywordOrder, Order, OrderTarget, Queue, Rules, SeatView,
    SquareId, TurnChange, TurnMessage,
};

fn read(relative: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/game-data")
        .join(relative);
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("{relative}: {e}: run scripts/fetch-upstream.ps1"));
    String::from_utf8_lossy(&bytes).into_owned()
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// SplitMix64: small, and the same on every machine.
struct Rng(u64);

impl Rng {
    fn new(seed: u64, sample: u64, case: u64) -> Rng {
        Rng(seed
            ^ sample.wrapping_mul(0x9e37_79b9_7f4a_7c15)
            ^ case.wrapping_mul(0xbf58_476d_1ce4_e5b9))
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// A random change to the bytes. Returns whether it cut them short.
fn mutate(rng: &mut Rng, bytes: &mut Vec<u8>) -> bool {
    let len = bytes.len();
    match rng.below(7) {
        0 => {
            bytes.truncate(rng.below(len));
            return true;
        }
        1 => {
            let at = rng.below(len);
            bytes[at] = rng.next() as u8;
        }
        2 => {
            // A large number where a small one was: a length, an id, a
            // variant.
            let at = rng.below(len);
            let big = [0xff, 0xff, 0xff, 0xff, 0x0f];
            bytes.splice(at..at, big);
        }
        3 => {
            let at = rng.below(len + 1);
            let n = 1 + rng.below(8);
            let noise: Vec<u8> = (0..n).map(|_| rng.next() as u8).collect();
            bytes.splice(at..at, noise);
        }
        4 => {
            let at = rng.below(len);
            let n = 1 + rng.below((len - at).min(16));
            bytes.drain(at..at + n);
        }
        5 => {
            let at = rng.below(len);
            let n = 1 + rng.below((len - at).min(64));
            let copy = bytes[at..at + n].to_vec();
            let to = rng.below(len + 1);
            bytes.splice(to..to, copy);
        }
        _ => {
            for _ in 0..1 + rng.below(4) {
                let at = rng.below(len);
                bytes[at] ^= 1 << rng.below(8);
            }
        }
    }
    false
}

/// What passes the checks, a seat's view answers from: asks it the
/// questions that read the message's places and numbers.
fn ask(start: &Arc<GameStart>, m: TurnMessage) {
    let v = SeatView::new(Arc::clone(start), m);
    for e in v.message().perceived.iter().chain(&v.message().remembered) {
        let _ = (
            v.known(e.id),
            e.place.and_then(|p| p.square()).map(|q| v.square(q)),
        );
    }
    for u in v.controlled_units() {
        for &k in Keyword::ALL {
            let _ = (v.order_menu(u.id, k, false), v.order_info(u.id, k, None));
        }
        let keep = v.game_start().rules.type_id("keep");
        let _ = v.order_info(u.id, Keyword::UpgradeTo, keep);
        let _ = v.default_order(u.id, OrderTarget::Entity(u.id));
    }
}

/// What a sample is, and how a receiver reads it.
enum Kind {
    Start,
    Whole,
    /// A change, with the message it applies to.
    Change(Box<TurnMessage>),
    End,
    Command,
    Setup,
}

#[test]
fn hostile_game_messages_are_refused_without_a_panic() {
    let seed = env_u64("HOSTILE_STREAM_SEED", 0x5eed_0f57_ea00);
    let cases = env_u64("HOSTILE_STREAM_CASES", 2000);

    let case = cheat_mode_and_defeat(120);
    let setup = case.setup(read);
    let mut game = Game::new(&case, read, true);
    let host_start = game.start();
    let start = Arc::new(GameStart::from_message(
        host_start.message(),
        Rules::load(&[setup.rules.as_str()]),
    ));
    let mut samples: Vec<(String, Kind, Vec<u8>)> = vec![
        (
            "start".into(),
            Kind::Start,
            encode_start(&host_start.message()).unwrap(),
        ),
        ("setup".into(), Kind::Setup, encode_setup(&setup).unwrap()),
    ];
    for c in [
        Command::Control(vec![EntityId(3), EntityId(9)]),
        Command::Order {
            order: Order::Keyword(KeywordOrder::typed(Keyword::Build, "farm").unwrap().at(
                OrderTarget::Point {
                    square: SquareId(2),
                    x: 4000,
                    y: 8000,
                },
            )),
            queue: Queue::Append,
            imperative: true,
        },
        Command::Speed(2.0),
    ] {
        samples.push((format!("{c:?}"), Kind::Command, encode_command(&c).unwrap()));
    }
    let mut last = game.messages();
    let mut turn = 0;
    while let Some(now) = game.step() {
        turn += 1;
        if turn % 20 == 0 {
            for (s, m) in now.iter().enumerate() {
                let label = format!("turn {turn} seat {s}");
                samples.push((
                    format!("{label}, whole"),
                    Kind::Whole,
                    encode_turn(m).unwrap(),
                ));
                let change = encode_change(&TurnChange::between(&last[s], m)).unwrap();
                samples.push((
                    format!("{label}, change"),
                    Kind::Change(Box::new(last[s].clone())),
                    change,
                ));
            }
        }
        last = now;
    }
    for &p in game.players() {
        samples.push((
            format!("end of {}", p.0),
            Kind::End,
            encode_end(&game.game_end(p)).unwrap(),
        ));
    }

    let (mut refused, mut accepted) = (0u64, 0u64);
    for (i, (label, kind, bytes)) in samples.iter().enumerate() {
        for case in 0..cases {
            let mut rng = Rng::new(seed, i as u64, case);
            let mut b = bytes.clone();
            let cut = mutate(&mut rng, &mut b);
            let read = catch_unwind(AssertUnwindSafe(|| match kind {
                Kind::Start => decode_start(&b).map(drop),
                Kind::Setup => decode_setup(&b).map(drop),
                Kind::Command => decode_command(&b).map(drop),
                Kind::End => decode_end(&b).map(drop),
                Kind::Whole => decode_turn(&b, &start).map(|m| ask(&start, m)),
                Kind::Change(last) => apply_change(&b, last, &start).map(|m| ask(&start, m)),
            }));
            let Ok(result) = read else {
                panic!("a panic: seed {seed:#x}, {label}, case {case}");
            };
            if cut {
                assert!(
                    result.is_err(),
                    "a cut message decoded: seed {seed:#x}, {label}, case {case}"
                );
            }
            match result {
                Ok(()) => accepted += 1,
                Err(_) => refused += 1,
            }
        }
    }
    eprintln!(
        "{} messages, {cases} changes each: {refused} refused, {accepted} read",
        samples.len()
    );
    assert!(refused > 0);
}

/// A set-up that names a level of the computers that does not exist is
/// refused; one at level 4 decodes.
#[test]
fn a_setup_with_a_level_that_does_not_exist_is_refused() {
    let mut setup = cheat_mode_and_defeat(120).setup(read);
    setup.computer_skill = soundrts_world::Skill::NORMAL;
    let normal = encode_setup(&setup).unwrap();
    setup.computer_skill = soundrts_world::Skill::Level4;
    let four = encode_setup(&setup).unwrap();
    assert_eq!(
        decode_setup(&four).unwrap().computer_skill,
        soundrts_world::Skill::Level4
    );
    // The level is the one byte that differs.
    let at: Vec<usize> = (0..four.len()).filter(|&i| four[i] != normal[i]).collect();
    assert_eq!(at.len(), 1);
    for bad in [4, 5, 0x7f] {
        let mut changed = four.clone();
        changed[at[0]] = bad;
        assert!(decode_setup(&changed).is_err(), "{bad}");
    }
}
