#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::sync::Arc;

use soundrts_world::{Completeness, Record};

use super::*;
use crate::clock::Clock;
use crate::game::testing::go;
use crate::input::{Key, Mods};
use crate::testkit::{Harness, Script, TestGame, render};

/// Gives the player of `g` a few orders: train a peasant, and send one
/// to a square.
fn orders(g: &mut TestGame) {
    let me = g.game.me;
    let units: Vec<_> = g.game.view().own_units().collect();
    let hall = units.iter().find(|u| &*u.type_name == "townhall").unwrap();
    let peasant = units.iter().find(|u| &*u.type_name == "peasant").unwrap();
    let host = g.game.host_mut();
    host.queue_line(me, &format!("control {}", hall.id.0));
    host.queue_line(me, "order 0 0 train peasant");
    let to = g.game.view().square_at(1, 1).unwrap();
    go(g, me, peasant.id, to);
}

/// A harness whose games play no ambient noise: the noises of units and
/// of the terrain start at random times of the player's own clock, which
/// a listener's clock does not keep.
fn without_ambience() -> Harness {
    let mut h = Harness::new("en", Script::new().keep_open());
    std::sync::Arc::make_mut(&mut h.client.config.parameters).animation_delay = 1e9;
    h
}

/// A training game on jl1 against an aggressive computer, played with a
/// few orders given at turn 2, each turn told until the voice is idle,
/// to turn `turns` at least, without ambient noises: the harness and the
/// game.
fn played(turns: i64) -> (Harness, TestGame) {
    let mut h = without_ambience();
    let mut g = h.start_training("jl1", &["aggressive"], 1);
    // The objective is told while the world waits.
    h.run_until_voice_idle(&mut g);
    // The cursor on the first unit's square, as the first animation puts
    // it, without its name.
    let square = g
        .game
        .view()
        .own_units()
        .find_map(|u| g.game.outer_square(u.place));
    g.game.place = square;
    h.run_until_turn(&mut g, 2);
    orders(&mut g);
    for turn in 3..=turns {
        h.run_until_turn(&mut g, turn);
        h.run_until_voice_idle(&mut g);
    }
    (h, g)
}

/// The record of a training game on jl1 against an aggressive computer,
/// with a few orders given at turn 2, of `turns` turns exactly.
fn record_of(turns: u64) -> Record {
    record_against("aggressive", turns)
}

/// As [`record_of`], against a computer of this type.
fn record_against(computer: &str, turns: u64) -> Record {
    let mut h = Harness::new("en", Script::new().keep_open());
    let mut g = h.start_training("jl1", &[computer], 1);
    h.run_until_turn(&mut g, 2);
    orders(&mut g);
    for _ in 2..turns {
        crate::game::testing::step(&mut g);
    }
    let record = g.game.host().record().clone();
    assert_eq!(record.turns_run(), turns);
    record
}

/// Presses a key and gives the game's answer, the first thing said.
fn answer(h: &mut Harness, g: &mut TestGame, key: Key, mods: Mods) -> String {
    let said = h.key(g, key, mods);
    said.into_iter().next().unwrap_or_default()
}

/// Runs the game for 10 turns from the start of one: how long they took.
fn ten_turns(h: &mut Harness, g: &mut TestGame) -> f64 {
    let turn = g.game.view().turn();
    h.run_until_turn(g, turn + 1);
    let start = h.clock.now();
    h.run_until_turn(g, turn + 11);
    h.clock.now() - start
}

/// A listener to `record` on jl1, following its first player, before
/// the first turn, once "watching" and the objective are said.
fn listening(h: &mut Harness, record: Record) -> TestGame {
    listening_on(h, "jl1", record)
}

/// As [`listening`], to a record of a game on the multiplayer map `map`.
fn listening_on(h: &mut Harness, map: &str, record: Record) -> TestGame {
    let m = h.multiplayer_map(map);
    h.client.res.set_map(Some(&m));
    let replay = Replay::new(Arc::new(record), Completeness::Whole, Keep::AlreadyKept).unwrap();
    let me = replay.first_player().unwrap();
    let mut game = GameInterface::listening(&mut h.client, replay, me).unwrap();
    game.begin(&mut h.client, Vec::new());
    h.client.input.clear();
    h.run_voice_for(30.0);
    game.set_turn_due(h.clock.now());
    TestGame {
        session: None,
        game,
    }
}

const CTRL_SHIFT: Mods = Mods {
    ctrl: true,
    alt: false,
    shift: true,
};

/// Presses a key that jumps, lets the replay land, and waits until the
/// voice is idle: what was said meanwhile.
fn jump(h: &mut Harness, g: &mut TestGame, key: Key, mods: Mods) -> Vec<String> {
    let from = h.utterances().len();
    h.press(key, mods);
    h.step(g);
    while g.game.muted() {
        h.step(g);
    }
    h.run_until_voice_idle(g);
    h.said_since(from)
}

/// Every utterance said so far, as lines.
fn said(h: &Harness) -> Vec<String> {
    h.utterances().iter().map(render).collect()
}

/// The names of the sounds played so far, in order.
fn sounds(h: &Harness) -> Vec<String> {
    h.audio.played().into_iter().map(|(_, s)| s).collect()
}

#[test]
fn listening_from_the_start_says_and_plays_what_the_player_heard_live() {
    let turns = 120;
    let (live, game) = played(turns);
    let record = game.game.host().record().clone();
    let (live_said, live_sounds) = (said(&live), sounds(&live));
    let mut h = without_ambience();
    let mut g = listening(&mut h, record.clone());
    h.run_until_turn(&mut g, 2);
    for turn in 3..=turns {
        h.run_until_turn(&mut g, turn);
        h.run_until_voice_idle(&mut g);
    }
    while g.game.view().turn() < record.turns_run() as i64 {
        h.step(&mut g);
    }
    h.run_until_voice_idle(&mut g);
    let heard = said(&h);
    // A listener hears whose side it is, its cursor on that player's
    // first square; then what the player heard, in the same order. (With
    // the ambience on, the player also heard that square named at the
    // first animation, whose cursor was nowhere yet.)
    assert_eq!(live_said[0], "objective: neutralized enemy");
    assert_eq!(heard[0], "watching player");
    assert_eq!(heard[1..], live_said[..]);
    assert!(g.game.place.is_some());
    assert!(
        live_said.len() > 6,
        "too little happened to compare: {live_said:?}"
    );
    assert_eq!(sounds(&h), live_sounds);
    assert!(!live_sounds.is_empty());
    // No more turns: the end is told, and the score.
    while !g.game.end_loop {
        h.step(&mut g);
    }
    assert_eq!(said(&h).last().unwrap(), "end of the recording");
    let from = h.utterances().len();
    h.finish(&mut g);
    let score = h.said_since(from);
    let seconds = record.game_time_ms() / 1000;
    assert_eq!(
        score[0],
        format!(
            "Not over at {} minutes {} seconds",
            seconds / 60,
            seconds % 60
        )
    );
    assert!(score.last().unwrap().starts_with("score "), "{score:?}");
}

#[test]
fn where_the_replay_is_is_the_turn_taken_before_its_end_is_handled() {
    let record = record_of(100);
    let mut h = Harness::new("en", Script::new().keep_open());
    let mut g = listening(&mut h, record);
    h.run_until_turn(&mut g, 9);
    assert_eq!(g.game.view().time(), 2700);
    // The next turn is taken, then the key handled, before the turn's end.
    g.game.set_turn_due(h.clock.now());
    let from = h.utterances().len();
    h.press(Key::F3, Mods::NONE);
    h.step(&mut g);
    assert_eq!(g.game.view().time(), 3000);
    assert!(g.game.has_pending_server_events());
    h.run_until_voice_idle(&mut g);
    let said = h.said_since(from);
    let time = said.iter().find(|s| s.contains(" of 0 minutes 30 seconds"));
    assert_eq!(
        time.map(String::as_str),
        Some("0 minutes 3 seconds of 0 minutes 30 seconds , speed normal")
    );
}

#[test]
fn a_paused_record_runs_no_turn_and_the_keys_still_answer() {
    let record = record_of(100);
    let mut h = Harness::new("en", Script::new().keep_open());
    let mut g = listening(&mut h, record);
    h.run_until_turn(&mut g, 10);
    assert_eq!(answer(&mut h, &mut g, Key::Space, Mods::SHIFT), "paused");
    let turn = g.game.view().turn();
    h.run_for(&mut g, 5.0);
    assert_eq!(g.game.view().turn(), turn);
    assert_eq!(
        answer(&mut h, &mut g, Key::F3, Mods::NONE),
        format!(
            "0 minutes {} seconds of 0 minutes 30 seconds , speed normal , paused",
            turn * 3 / 10
        )
    );
    assert_eq!(answer(&mut h, &mut g, Key::Z, Mods::NONE), "6 gold");
    assert_eq!(g.game.view().turn(), turn);
    assert_eq!(
        answer(&mut h, &mut g, Key::Space, Mods::SHIFT),
        "speed normal"
    );
    // At normal speed, 10 turns take 3 seconds.
    let took = ten_turns(&mut h, &mut g);
    assert!((took - 3.0).abs() < 0.1, "{took}");
    assert_eq!(
        answer(&mut h, &mut g, Key::Equals, Mods::NONE),
        "speed fast 2"
    );
    let took = ten_turns(&mut h, &mut g);
    assert!((took - 1.5).abs() < 0.1, "{took}");
    assert_eq!(
        answer(&mut h, &mut g, Key::Minus, Mods::NONE),
        "speed normal"
    );
    assert_eq!(answer(&mut h, &mut g, Key::Minus, Mods::NONE), "speed slow");
    assert_eq!(answer(&mut h, &mut g, Key::Minus, Mods::NONE), "speed slow");
}

#[test]
fn a_jump_lands_on_its_turn_and_says_nothing_of_the_stretch_skipped() {
    let record = record_of(700);
    let mut h = Harness::new("en", Script::new().keep_open());
    let mut g = listening(&mut h, record);
    h.run_until_turn(&mut g, 5);
    // Paused, the replay stays where each jump lands.
    assert_eq!(answer(&mut h, &mut g, Key::Space, Mods::SHIFT), "paused");
    h.run_until_voice_idle(&mut g);
    let before = g.game.view().turn();
    let log_before = h.client.voice.log().count();
    // A minute forward: 200 turns.
    let seconds = (before + 200) * 3 / 10;
    assert_eq!(
        jump(&mut h, &mut g, Key::RightBracket, Mods::NONE),
        [format!(
            "{} minutes {} seconds of 3 minutes 30 seconds",
            seconds / 60,
            seconds % 60
        )]
    );
    assert_eq!(g.game.view().turn(), before + 200);
    assert_eq!(
        h.client.voice.log().count(),
        log_before,
        "nothing of the stretch skipped is news"
    );
    // Five minutes forward stops at the end.
    let said = jump(&mut h, &mut g, Key::RightBracket, Mods::SHIFT);
    assert_eq!(said, ["3 minutes 30 seconds of 3 minutes 30 seconds"]);
    assert_eq!(g.game.view().turn(), 700);
    // A minute back replays from the start.
    let said = jump(&mut h, &mut g, Key::LeftBracket, Mods::NONE);
    assert_eq!(said, ["2 minutes 30 seconds of 3 minutes 30 seconds"]);
    assert_eq!(g.game.view().turn(), 500);
    let said = jump(&mut h, &mut g, Key::LeftBracket, Mods::SHIFT);
    assert_eq!(said, ["0 minutes 0 seconds of 3 minutes 30 seconds"]);
    assert_eq!(g.game.view().turn(), 0);
    // It plays on from there.
    assert_eq!(
        answer(&mut h, &mut g, Key::Space, Mods::SHIFT),
        "speed normal"
    );
    h.run_until_turn(&mut g, 3);
}

/// The record of a training game on jl1 against a quiet computer, whose
/// two farms stand on the player's square from turn 2, of 300 turns.
fn record_with_enemies_in_sight() -> Record {
    let mut h = Harness::new("en", Script::new().keep_open());
    let mut g = h.start_training("jl1", &["quiet"], 1);
    h.run_until_turn(&mut g, 2);
    let (me, them) = (g.game.me, crate::game::testing::enemy(&g));
    let hall = crate::game::testing::unit(&g, me, "townhall");
    let square = g.game.host().world().entity(hall).unwrap().square();
    let home = g.game.view().square(square.unwrap()).name().to_owned();
    crate::game::testing::cheat(&mut g, them, &format!("add_units {home} 2 farm"));
    for _ in 2..300 {
        crate::game::testing::step(&mut g);
    }
    g.game.host().record().clone()
}

#[test]
fn a_jump_says_nothing_of_the_enemies_in_sight_where_it_lands() {
    let mut h = Harness::new("en", Script::new().keep_open());
    let mut g = listening(&mut h, record_with_enemies_in_sight());
    h.run_until_turn(&mut g, 5);
    assert_eq!(answer(&mut h, &mut g, Key::Space, Mods::SHIFT), "paused");
    h.run_until_voice_idle(&mut g);
    assert!(said(&h).iter().any(|s| s.contains("2 farm enemy at b 1")));
    let log_before = h.client.voice.log().count();
    jump(&mut h, &mut g, Key::RightBracket, Mods::NONE);
    assert_eq!(
        h.client.voice.log().count(),
        log_before,
        "nothing of the stretch skipped is news"
    );
    // Nor when the replay goes on from there.
    let from = h.utterances().len();
    assert_eq!(
        answer(&mut h, &mut g, Key::Space, Mods::SHIFT),
        "speed normal"
    );
    let turn = g.game.view().turn() + 5;
    h.run_until_turn(&mut g, turn);
    h.run_until_voice_idle(&mut g);
    let said = h.said_since(from);
    assert!(
        !said.iter().any(|s| s.contains("enemy")),
        "the enemies in sight where the jump landed are not news: {said:?}"
    );
}

#[test]
fn at_the_fastest_pace_only_how_far_it_has_got_is_said() {
    let record = record_of(400);
    let mut h = Harness::new("en", Script::new().keep_open());
    let mut g = listening(&mut h, record);
    for _ in 0..3 {
        h.key(&mut g, Key::Equals, Mods::NONE);
    }
    let from = h.utterances().len();
    h.press(Key::Equals, Mods::NONE);
    h.step(&mut g);
    let played = h.audio.played().len();
    while !g.game.end_loop {
        h.step(&mut g);
    }
    h.finish(&mut g);
    let said: Vec<String> = h
        .said_since(from)
        .into_iter()
        .take_while(|s| !s.starts_with("Not over"))
        .collect();
    assert_eq!(said.first().unwrap(), "speed fastest, silent");
    assert_eq!(said.last().unwrap(), "end of the recording");
    assert!(
        said[1..said.len() - 1]
            .iter()
            .all(|s| s.ends_with(" percent")),
        "{said:?}"
    );
    assert_eq!(h.audio.played().len(), played, "nothing played");
    assert_eq!(g.game.view().turn(), 400);
}

#[test]
fn changing_player_gives_the_other_sides_whole_message() {
    let record = record_of(100);
    let mut h = Harness::new("en", Script::new().keep_open());
    let mut g = listening(&mut h, record.clone());
    let first = g.game.me;
    h.run_until_turn(&mut g, 10);
    assert_eq!(
        answer(&mut h, &mut g, Key::F12, Mods::CTRL),
        "watching aggressive computer , level 3"
    );
    let computer = g.game.me;
    assert_ne!(computer, first);
    let turn = g.game.view().turn() + 3;
    h.run_until_turn(&mut g, turn);
    // What the computer is told at that turn of the record.
    let mut replayer = soundrts_world::Replayer::new(&record).unwrap();
    for _ in 0..turn {
        replayer.step_seats(&[]).unwrap();
    }
    assert_eq!(
        g.game.view().message(),
        &replayer.message(computer).unwrap()
    );
    assert_eq!(
        answer(&mut h, &mut g, Key::F12, CTRL_SHIFT),
        "watching player"
    );
    assert_eq!(g.game.me, first);
}

#[test]
fn an_order_key_changes_nothing_and_the_listener_hears_so_once() {
    let record = record_of(30);
    let mut h = Harness::new("en", Script::new().keep_open());
    let mut g = listening(&mut h, record.clone());
    h.run_until_turn(&mut g, 5);
    h.run_until_voice_idle(&mut g);
    let from = h.utterances().len();
    for (key, mods) in [
        (Key::E, Mods::NONE),
        (Key::Backspace, Mods::NONE),
        (Key::Num1, Mods::CTRL),
        (Key::Backspace, Mods::NONE),
    ] {
        h.key(&mut g, key, mods);
        assert!(g.game.pending_commands().is_empty());
    }
    let said = h.said_since(from);
    let told = said
        .iter()
        .filter(|s| *s == "watching: you give no orders")
        .count();
    assert_eq!(told, 1, "{said:?}");
    // The console and the cheats are not the listener's.
    assert_eq!(
        answer(&mut h, &mut g, Key::Backquote, Mods::NONE),
        "<ui/beep>"
    );
    while !g.game.end_loop {
        h.step(&mut g);
    }
    let r = g.game.source.replay().expect("a replay");
    let mine = r.replayer.host().record();
    assert_eq!(mine, &record);
}

#[test]
fn a_record_that_breaks_part_way_stops_there_and_the_keys_still_answer() {
    let record = record_of(30);
    // A cheat in a game that allows none: the first turn with commands
    // is refused.
    let first = record.turns()[0].number();
    let text = String::from_utf8(record.to_bytes()).unwrap();
    let mut h = Harness::new("en", Script::new().keep_open());
    let line = format!(r#"{{"turn":{first},"commands":["#);
    let cheat = format!(r#"{line}[0,{{"Cheat":"Resources"}}],"#);
    let changed = text.replacen(&line, &cheat, 1).replacen(
        r#""allow_cheatmode":true"#,
        r#""allow_cheatmode":false"#,
        1,
    );
    let (record, _) = Record::read(changed.as_bytes()).unwrap();
    let mut g = listening(&mut h, record);
    let from = h.utterances().len();
    h.run_for(&mut g, 5.0);
    assert!(
        h.said_since(from)
            .contains(&"the recording is damaged: it stops here".to_owned())
    );
    assert_eq!(g.game.view().turn(), first as i64);
    assert!(!g.game.end_loop);
    assert_eq!(
        answer(&mut h, &mut g, Key::F3, Mods::NONE),
        format!(
            "0 minutes {} seconds of 0 minutes 9 seconds , speed normal",
            first * 3 / 10
        )
    );
    assert_eq!(
        answer(&mut h, &mut g, Key::RightBracket, Mods::NONE),
        "the recording is damaged: it stops here"
    );
}

#[test]
fn the_listeners_keys_in_a_game_played_say_they_are_for_listening() {
    let (mut h, mut g) = crate::game::testing::training();
    for key in [Key::Equals, Key::Minus, Key::RightBracket, Key::LeftBracket] {
        assert_eq!(
            answer(&mut h, &mut g, key, Mods::NONE),
            "only while you listen to a recorded game"
        );
    }
    assert_eq!(
        answer(&mut h, &mut g, Key::Space, Mods::SHIFT),
        "only while you listen to a recorded game"
    );
}

/// The record of a training game whose player quits at turn 10, of
/// `turns` turns.
fn quit_at_10(turns: u64) -> Record {
    let mut h = Harness::new("en", Script::new().keep_open());
    let mut g = h.start_training("jl1", &["aggressive"], 1);
    h.run_until_turn(&mut g, 2);
    let me = g.game.me;
    for turn in 2..turns {
        if turn == 10 {
            let _told = g.game.host_mut().queue(me, soundrts_world::Command::Quit);
        }
        crate::game::testing::step(&mut g);
    }
    g.game.host().record().clone()
}

#[test]
fn when_the_player_followed_leaves_and_no_person_is_left_the_listening_ends() {
    let record = quit_at_10(40);
    let mut h = Harness::new("en", Script::new().keep_open());
    let mut g = listening(&mut h, record);
    while !g.game.end_loop {
        h.step(&mut g);
    }
    // The record goes on to turn 40; the listening ends when the player
    // followed leaves, with nobody left to follow.
    assert!(g.game.view().turn() < 15, "{}", g.game.view().turn());
    let from = h.utterances().len();
    h.finish(&mut g);
    assert_eq!(h.said_since(from)[0], "Defeat at 0 minutes 3 seconds");
}

/// How long a jump over an hour of a game takes, in the time of the
/// machine: run with `cargo test -p soundrts --lib
/// measure_jumping_an_hour -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement"]
fn measure_jumping_an_hour() {
    let record = record_against("quiet", 12_000);
    let mut h = Harness::new("en", Script::new().keep_open());
    let mut g = listening(&mut h, record);
    assert_eq!(answer(&mut h, &mut g, Key::Space, Mods::SHIFT), "paused");
    let start = std::time::Instant::now();
    for _ in 0..12 {
        jump(&mut h, &mut g, Key::RightBracket, Mods::SHIFT);
    }
    let forward = start.elapsed();
    assert_eq!(g.game.view().turn(), 12_000);
    let start = std::time::Instant::now();
    jump(&mut h, &mut g, Key::LeftBracket, Mods::SHIFT);
    let back = start.elapsed();
    println!(
        "an hour forward, 5 minutes at a time: {forward:?}; 5 minutes back from the end: {back:?}"
    );
}

#[test]
fn a_recording_that_ends_with_its_player_leaving_says_so_then_the_result() {
    let mut h = Harness::new("en", Script::new().keep_open());
    let mut g = listening(&mut h, quit_at_10(11));
    let from = h.utterances().len();
    while !g.game.end_loop {
        h.step(&mut g);
    }
    h.finish(&mut g);
    let said = h.said_since(from);
    let end = said
        .iter()
        .position(|s| s == "end of the recording")
        .expect("the end said");
    assert_eq!(said[end + 1], "Defeat at 0 minutes 3 seconds");
}

#[test]
fn a_jump_past_the_players_departure_ends_with_its_result() {
    let mut h = Harness::new("en", Script::new().keep_open());
    let mut g = listening(&mut h, quit_at_10(40));
    h.press(Key::RightBracket, Mods::SHIFT);
    while !g.game.end_loop {
        h.step(&mut g);
    }
    let from = h.utterances().len();
    h.finish(&mut g);
    assert_eq!(h.said_since(from)[0], "Defeat at 0 minutes 3 seconds");
}

#[test]
fn going_fastest_hears_nothing_more_of_the_turn_waiting() {
    let record = record_of(1000);
    let mut h = Harness::new("en", Script::new().keep_open());
    let mut g = listening(&mut h, record);
    h.run_until_turn(&mut g, 20);
    // News of the turn taken, not yet handled.
    let news = vec![soundrts_world::msg::MsgPart::Literal(
        "news of the turn".into(),
    )];
    g.game
        .srv_queue
        .push_back(crate::game::SrvEvent::VoiceImportant(news));
    let played = h.audio.played().len();
    let log = h.client.voice.log().count();
    let r = g.game.source.replay_mut().expect("a replay");
    r.pace = Pace::Fast8;
    h.press(Key::Equals, Mods::NONE);
    h.step(&mut g);
    assert!(!g.game.has_pending_server_events());
    assert_eq!(h.audio.played().len(), played);
    assert_eq!(h.client.voice.log().count(), log);
}

#[test]
fn going_fastest_on_the_players_last_turn_hears_nothing_of_it() {
    let mut h = Harness::new("en", Script::new().keep_open());
    let mut g = listening(&mut h, quit_at_10(40));
    let me = g.game.me;
    // The turn the player leaves on is taken, and what it told waits, its
    // leaving last: each turn is taken once the last is handled.
    while g.game.view().players().contains(&me) {
        let r = g.game.source.replay_mut().expect("a replay");
        if r.waiting_for_world_update || !g.game.srv_queue.is_empty() {
            h.step(&mut g);
        } else {
            r.next_update = 0.0;
            g.game.replay_update(&mut h.client);
        }
    }
    assert!(!g.game.end_loop);
    assert!(!g.game.srv_queue.is_empty());
    let news = vec![soundrts_world::msg::MsgPart::Literal(
        "news of the turn".into(),
    )];
    g.game
        .srv_queue
        .push_front(crate::game::SrvEvent::VoiceImportant(news));
    let r = g.game.source.replay_mut().expect("a replay");
    r.pace = Pace::Fast8;
    let log = h.client.voice.log().count();
    h.press(Key::Equals, Mods::NONE);
    h.step(&mut g);
    assert!(g.game.end_loop);
    assert_eq!(h.client.voice.log().count(), log);
}

/// The record of a game on jl2 of `turns` turns, between a person and
/// two computers, each alone, where the person quits at turn 10 and the
/// computers play on.
fn person_quits_at_10_of_three(turns: u64) -> Record {
    let mut h = Harness::new("en", Script::new().keep_open());
    let m = h.multiplayer_map("jl2");
    let texts = crate::multiplayer::game_texts(&mut h.client, &m).unwrap();
    let alone = |mut c: soundrts_world::Controller, alliance: &str| {
        c.alliance = Some(alliance.into());
        c
    };
    let controllers = vec![
        alone(soundrts_world::Controller::human("me", true), "1"),
        alone(soundrts_world::Controller::computer("aggressive"), "2"),
        alone(soundrts_world::Controller::computer("aggressive"), "3"),
    ];
    let setup = soundrts_world::GameSetup::multiplayer(
        texts.rules.into_string(),
        texts.ai.into_string(),
        texts.map.into_string(),
        1,
        controllers,
    );
    let mut host = soundrts_world::Host::new(&setup).unwrap();
    let me = host.seats()[0];
    for turn in 0..turns {
        if turn == 10 {
            let _told = host.queue(me, soundrts_world::Command::Quit);
        }
        host.step_seats(&[]);
    }
    host.record().clone()
}

/// A listener who follows a computer jumps past the person's departure,
/// and asks for the next player while the jump is under way and once it
/// has landed: the person who left is never followed. Every pass of a jump brings the view to where the replay is
/// before a key is taken, so the next player is chosen from those still
/// in the game.
#[test]
fn a_player_who_left_during_a_jump_is_not_followed() {
    let record = person_quits_at_10_of_three(400);
    let mut h = Harness::new("en", Script::new().keep_open());
    let mut g = listening_on(&mut h, "jl2", record);
    let person = g.game.me;
    h.run_until_turn(&mut g, 3);
    assert_eq!(answer(&mut h, &mut g, Key::Space, Mods::SHIFT), "paused");
    h.run_until_voice_idle(&mut g);
    h.key(&mut g, Key::F12, Mods::CTRL);
    let computer = g.game.me;
    assert_ne!(computer, person);
    h.run_until_voice_idle(&mut g);
    // The first pass of a jump of a minute: 50 turns, past turn 10.
    h.press(Key::RightBracket, Mods::NONE);
    while g.game.view().turn() < 10 {
        h.step(&mut g);
    }
    assert!(g.game.muted(), "the jump is under way");
    assert_eq!(g.game.view().turn(), 53);
    assert!(!g.game.view().players().contains(&person));
    let from = h.utterances().len();
    h.press(Key::F12, Mods::CTRL);
    while g.game.muted() {
        h.step(&mut g);
    }
    h.run_until_voice_idle(&mut g);
    // The other computer, the next player still in the game.
    let said = h.said_since(from);
    assert!(
        said.contains(&"watching aggressive computer , level 3".to_owned()),
        "{said:?}"
    );
    let other = g.game.me;
    assert!(other != person && other != computer, "{other:?}");
    // Once landed, the person is not among the players to follow either.
    for expected in [computer, other, computer] {
        h.key(&mut g, Key::F12, Mods::CTRL);
        assert_eq!(g.game.me, expected);
    }
}
