#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::collections::BTreeMap;
use std::ops::ControlFlow;

use super::*;
use crate::command::Cheat;
use crate::player::Controller;
use crate::world::multiplayer_default_triggers;

fn read(relative: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/soundrts-upstream")
        .join(relative);
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("{relative}: {e}: run scripts/fetch-upstream.ps1"));
    String::from_utf8_lossy(&bytes).into_owned()
}

/// A training game on jl1: the local human against an aggressive computer.
fn setup() -> GameSetup {
    let mut me = Controller::human("me", true);
    me.alliance = Some("1".into());
    let mut computer = Controller::computer("aggressive");
    computer.alliance = Some("2".into());
    GameSetup {
        rules: read("res/rules.txt"),
        ai: read("res/ai.txt"),
        map: read("res/multi/jl1.txt"),
        seed: 7,
        controllers: vec![me, computer],
        default_triggers: multiplayer_default_triggers(),
        allow_cheatmode: true,
        random_starts: true,
        equivalents: true,
    }
}

/// What the human does before the step of `turn`: orders as lines of
/// text, and the cheats that act at once among them.
fn give(host: &mut Host, me: PlayerId, turn: u64) {
    let units = |host: &Host| {
        let ids: Vec<String> = host
            .world()
            .player(me)
            .units
            .iter()
            .map(|u| u.0.to_string())
            .collect();
        format!("control {}", ids.join(" "))
    };
    let square = |host: &Host, name: &str| host.world().square_by_name(name).map_or(0, |s| s.0);
    match turn % 100 {
        0 => {
            let control = units(host);
            host.queue_line(me, &control);
            host.queue_line(me, "order 0 0 train peasant");
        }
        5 => {
            let _ = host.queue(me, Command::Cheat(Cheat::Resources));
            let control = units(host);
            host.queue_line(me, &control);
            host.queue_line(me, "order 0 0 train footman");
        }
        20 => {
            let control = units(host);
            let b2 = square(host, "b2");
            host.queue_line(me, &control);
            host.queue_line(me, &format!("order 0 0 go {b2}"));
            let _ = host.queue(me, Command::Cheat(Cheat::AllTechs));
            host.queue_line(me, "order 1 0 go 0");
        }
        41 => {
            let _ = host.queue(me, Command::Cheat(Cheat::InfiniteMana));
            host.queue_line(me, "toggle_cheatmode");
        }
        // Plays as the computer for a while: its controller changes.
        77 => {
            let them = host.seats()[1];
            let _ = host.queue(me, Command::Cheat(Cheat::PlayAs(them)));
        }
        88 => {
            let them = host.seats()[1];
            let _ = host.queue(them, Command::Cheat(Cheat::PlayAs(me)));
        }
        _ => {}
    }
}

/// Plays `turns` turns of the game, the human's commands given before
/// each step; then the commands of the next turn, without its step. The
/// digest of the world after each step, from 0 steps.
fn play(turns: u64) -> (Host, Vec<u64>) {
    let mut host = Host::new(&setup()).unwrap();
    let me = host.seats()[0];
    let mut digests = vec![host.world().digest_u64()];
    for turn in 0..turns {
        give(&mut host, me, turn);
        host.step(me);
        digests.push(host.world().digest_u64());
    }
    give(&mut host, me, turns);
    (host, digests)
}

/// Replays a record, with the digest after each step, from 0 steps.
fn replay(record: &Record) -> Result<(Host, Vec<u64>), ReplayError> {
    let mut digests = Vec::new();
    let host = Host::replay(record, |h| {
        digests.push(h.world().digest_u64());
        ControlFlow::Continue(())
    })?;
    Ok((host, digests))
}

#[test]
fn a_recorded_game_replays_to_the_same_digest_at_every_turn() {
    let (original, digests) = play(400);
    let record = original.record();
    assert_eq!(record.turns_run(), 400);
    assert!(record.turns().len() > 10, "{:?}", record.turns().len());
    let (replayed, replayed_digests) = replay(record).unwrap();
    assert_eq!(replayed_digests, digests[1..]);
    // The commands of turn 400 are queued, cheats taken at once.
    assert_eq!(replayed.world().digest_u64(), original.world().digest_u64());
    assert_eq!(replayed.record(), record);
}

#[test]
fn a_game_loaded_tells_what_the_saved_one_told_and_plays_on_the_same() {
    // Saved after 305 turns: turn 305 has an at-once cheat already
    // taken and orders waiting.
    let (mut original, _) = play(305);
    let me = original.seats()[0];
    let saved = Record::read(&original.record().to_bytes()).unwrap();
    assert_eq!(saved.1, Completeness::Whole);
    let (mut loaded, _) = replay(&saved.0).unwrap();
    assert_eq!(loaded.message(me), original.message(me));
    for turn in 305..420 {
        if turn > 305 {
            give(&mut original, me, turn);
            give(&mut loaded, me, turn);
        }
        let a = original.step(me);
        let b = loaded.step(me);
        assert_eq!(a.message, b.message, "turn {turn}");
        assert_eq!(
            original.world().digest_u64(),
            loaded.world().digest_u64(),
            "turn {turn}"
        );
    }
    assert_eq!(loaded.record(), original.record());
}

#[test]
fn a_record_reads_back_as_written() {
    let (host, _) = play(150);
    let bytes = host.record().to_bytes();
    let text = String::from_utf8(bytes.clone()).unwrap();
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some(
            format!(
                r#"{{"format":"soundrts-record-1","sim_version":{}}}"#,
                crate::SIM_VERSION
            )
            .as_str()
        )
    );
    assert_eq!(text.lines().last(), Some(r#"{"end":150}"#));
    assert!(
        text.contains(r#"{"turn":5,"commands":[[0,{"Cheat":"Resources"}],[0,{"Control":["#),
        "{text:.2000}"
    );
    let (read, whole) = Record::read(&bytes).unwrap();
    assert_eq!(whole, Completeness::Whole);
    assert_eq!(&read, host.record());
    // Windows line ends read the same.
    let crlf = text.replace('\n', "\r\n");
    assert_eq!(&Record::read(crlf.as_bytes()).unwrap().0, host.record());
}

/// A record cut at every byte of its last lines reads to a whole earlier
/// turn, which replays to the game at that turn, or is refused.
#[test]
fn a_record_cut_short_reads_to_a_whole_earlier_turn() {
    let (host, digests) = play(230);
    let bytes = host.record().to_bytes();
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(
            bytes
                .iter()
                .enumerate()
                .filter(|&(_, &b)| b == b'\n')
                .map(|(i, _)| i + 1),
        )
        .collect();
    // From the start of the fifth line from the end.
    let from = line_starts[line_starts.len() - 6];
    let mut replayed: BTreeMap<(u64, usize), ()> = BTreeMap::new();
    for cut in from..=bytes.len() {
        let (record, whole) = Record::read(&bytes[..cut]).unwrap();
        assert_eq!(
            whole == Completeness::Whole,
            cut == bytes.len(),
            "cut {cut}"
        );
        let original = host.record();
        let r = record.turns_run();
        assert!(r <= original.turns_run());
        // Its turns are the original's first ones.
        assert_eq!(record.turns(), &original.turns()[..record.turns().len()]);
        if replayed.insert((r, record.turns().len()), ()).is_some() {
            continue;
        }
        let (mut h, _) = replay(&record).unwrap();
        if whole == Completeness::Whole {
            assert_eq!(h.world().digest_u64(), host.world().digest_u64());
        } else {
            // The last turn read waits whole for its step.
            h.step(h.seats()[0]);
            assert_eq!(h.world().digest_u64(), digests[r as usize + 1], "cut {cut}");
        }
    }
    assert!(replayed.len() >= 3, "{replayed:?}");
    // The set-up cut short is refused.
    let second = line_starts[2];
    for cut in [0, 1, line_starts[1], second - 1] {
        assert!(Record::read(&bytes[..cut]).is_err(), "cut {cut}");
    }
}

#[test]
fn a_record_of_another_version_is_refused() {
    let (host, _) = play(30);
    let text = String::from_utf8(host.record().to_bytes()).unwrap();
    let other = text.replacen(
        &format!(r#""sim_version":{}"#, crate::SIM_VERSION),
        r#""sim_version":3"#,
        1,
    );
    assert_eq!(
        Record::read(other.as_bytes()),
        Err(ReadError::OtherVersion { found: 3 })
    );
    let other_format = text.replacen("soundrts-record-1", "soundrts-record-2", 1);
    assert_eq!(
        Record::read(other_format.as_bytes()),
        Err(ReadError::NotARecord)
    );
    assert_eq!(Record::read(b"hello\n"), Err(ReadError::NotARecord));
    assert_eq!(Record::read(b""), Err(ReadError::NotARecord));
}

#[test]
fn a_record_out_of_order_or_beyond_its_limits_is_damaged() {
    let (host, _) = play(30);
    let text = String::from_utf8(host.record().to_bytes()).unwrap();
    let damaged = |t: &str| matches!(Record::read(t.as_bytes()), Err(ReadError::Damaged { .. }));
    let mut lines: Vec<&str> = text.lines().collect();
    let end = lines.pop().unwrap();
    let head = lines[..2].join("\n");
    let turns = &lines[2..];
    assert!(turns.len() >= 2);
    // Turns swapped, one twice, one empty, a line after the end, an end
    // before the last turn.
    let swapped = [head.as_str(), turns[1], turns[0], end, ""].join("\n");
    assert!(damaged(&swapped));
    let twice = [head.as_str(), turns[0], turns[0], end, ""].join("\n");
    assert!(damaged(&twice));
    let empty = [head.as_str(), r#"{"turn":3,"commands":[]}"#, end, ""].join("\n");
    assert!(damaged(&empty));
    let after = [head.as_str(), turns[0], end, turns[1], ""].join("\n");
    assert!(damaged(&after));
    let early = [head.as_str(), turns[0], turns[1], r#"{"end":1}"#, ""].join("\n");
    assert!(damaged(&early));
    let beyond = format!("{head}\n{{\"end\":{}}}\n", MAX_RECORD_TURNS + 1);
    assert!(damaged(&beyond));
    let unknown = [head.as_str(), r#"{"turn":3,"commands":[],"x":1}"#, end, ""].join("\n");
    assert!(damaged(&unknown));
    // A text too long.
    let mut s = setup();
    s.map = "x".repeat(MAX_RECORD_TEXT_BYTES + 1);
    let long = format!(
        "{}\n{}\n{{\"end\":0}}\n",
        lines[0],
        serde_json::to_string(&s).unwrap()
    );
    assert!(damaged(&long));
}

#[test]
fn a_record_whose_commands_the_game_refuses_is_not_replayed() {
    let (host, _) = play(30);
    let text = String::from_utf8(host.record().to_bytes()).unwrap();
    // A player the game does not have.
    let changed = text.replacen(r#"[[0,{"Control""#, r#"[[9,{"Control""#, 1);
    assert_ne!(changed, text);
    let (record, _) = Record::read(changed.as_bytes()).unwrap();
    assert!(matches!(
        Host::replay(&record, |_| ControlFlow::Continue(())),
        Err(ReplayError::Refused { turn: 0 })
    ));
    // A cheat in a game that allows none.
    let no_cheats = text.replacen(r#""allow_cheatmode":true"#, r#""allow_cheatmode":false"#, 1);
    let (record, _) = Record::read(no_cheats.as_bytes()).unwrap();
    assert!(matches!(
        Host::replay(&record, |_| ControlFlow::Continue(())),
        Err(ReplayError::Refused { turn: 5 })
    ));
    // A map that cannot be built.
    let mut s = setup();
    s.map = "square_width 12\nnb_columns 0\n".into();
    let host = Host::new(&s);
    assert!(host.is_err());
}

#[test]
fn a_replay_stops_when_asked_and_a_failure_is_an_error() {
    let (host, _) = play(30);
    let mut steps = 0;
    let stopped = Host::replay(host.record(), |_| {
        steps += 1;
        if steps == 10 {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    });
    assert!(matches!(stopped, Err(ReplayError::Stopped)));
    assert_eq!(steps, 10);
    let failed = Host::replay(host.record(), |h| {
        assert!(h.record().turns_run() < 5, "the simulation failed");
        ControlFlow::Continue(())
    });
    assert!(
        matches!(&failed, Err(ReplayError::Failed(why)) if why.contains("the simulation failed")),
        "{:?}",
        failed.err()
    );
}

/// Bytes changed anywhere in a record: it reads, or it is refused; what
/// reads replays, or is refused. Nothing panics.
#[test]
fn a_record_changed_anywhere_is_read_or_refused_without_a_panic() {
    let (host, _) = play(120);
    let bytes = host.record().to_bytes();
    let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let mut read = 0;
    for _ in 0..60 {
        let mut changed = bytes.clone();
        for _ in 0..1 + next() % 3 {
            let at = (next() % changed.len() as u64) as usize;
            changed[at] = match next() % 4 {
                0 => b'0' + (next() % 10) as u8,
                1 => b'"',
                2 => b'\n',
                _ => (next() % 256) as u8,
            };
        }
        if let Ok((record, _)) = Record::read(&changed) {
            read += 1;
            // A change to a number of turns could make the replay long.
            if record.turns_run() <= 2000 {
                let _ = Host::replay(&record, |_| ControlFlow::Continue(()));
            }
        }
    }
    assert!(read > 0);
}

/// How long an hour of a game takes to replay: run with `cargo test
/// --release -p soundrts-world measure_replaying_an_hour -- --ignored
/// --nocapture`.
#[test]
#[ignore = "a measurement"]
fn measure_replaying_an_hour() {
    let mut s = setup();
    s.map = read("res/multi/jl2.txt");
    s.controllers = vec![
        Controller::human("me", true),
        Controller::computer("aggressive"),
        Controller::computer("aggressive"),
        Controller::computer("easy"),
    ];
    for (i, c) in s.controllers.iter_mut().enumerate() {
        c.alliance = Some((i + 1).to_string());
    }
    let turns = 3600 * 1000 / VIRTUAL_TIME_INTERVAL as u64;
    let start = std::time::Instant::now();
    let mut host = Host::new(&s).unwrap();
    let me = host.seats()[0];
    for turn in 0..turns {
        give(&mut host, me, turn);
        host.step(me);
    }
    let played = start.elapsed();
    let bytes = host.record().to_bytes();
    let start = std::time::Instant::now();
    let (record, _) = Record::read(&bytes).unwrap();
    let read_time = start.elapsed();
    let start = std::time::Instant::now();
    let replayed = Host::replay(&record, |_| ControlFlow::Continue(())).unwrap();
    let replay_time = start.elapsed();
    assert_eq!(replayed.world().digest_u64(), host.world().digest_u64());
    println!(
        "{turns} turns, {} turns with commands, {} bytes: played in {played:?}, read in {read_time:?}, replayed in {replay_time:?}; {} units at the end",
        record.turns().len(),
        bytes.len(),
        host.world()
            .all_players()
            .map(|(_, p)| p.units.len())
            .sum::<usize>(),
    );
}
