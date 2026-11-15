//! A long game against an aggressive computer while a scripted human
//! presses every bound key, then a fixed mix of them: nothing panics, and
//! the client logs nothing but the warnings listed here. This runs the
//! interface and the world headlessly; it says nothing about speech output.

use soundrts::bindings::{Bindings, Chord};
use soundrts::game::command::GameCommand;
use soundrts::input::{Key, Mods};
use soundrts::rng::ClientRng;
use soundrts::testkit::{Harness, Script};

/// What the client may log during such a game: upstream's own style
/// warnings for objects without some sounds. A warning of the world is a
/// stop of a unit's or a player's update, which an ordinary game must not
/// reach.
const EXPECTED_WARNINGS: &[&str] = &["no value found for "];

const TURNS: i64 = 2000;

fn soak(map: &str, seed: i64) {
    let mut h = Harness::new("en", Script::new().keep_open());
    let mut g = h.start_training(map, &["aggressive"], seed);
    // A garrison at the town hall, so that the human holds 2000 turns: the
    // point is a long fight, not a quick defeat.
    let me = g.game.me;
    let square = g
        .game
        .host()
        .world()
        .player(me)
        .units
        .iter()
        .filter_map(|&u| g.game.host().world().entity(u))
        .find(|e| &*e.type_name == "townhall")
        .and_then(|e| e.square())
        .map(|s| g.game.host().world().square(s).name.clone())
        .expect("a town hall");
    g.game
        .execute_command(&mut h.client, &format!("add_units {square} 30 footman"));
    let mut chords: Vec<(Chord, GameCommand)> = {
        let mut b: Bindings<GameCommand> = Bindings::new();
        let text = soundrts::bindings::game_bindings_text(
            &h.client.res,
            &h.client.paths.custom_bindings(),
        );
        b.load(&text, &GameCommand::parse, &h.client.log);
        b.iter().map(|(c, g)| (*c, g.clone())).collect()
    };
    chords.sort_by_key(|(c, _)| format!("{c:?}"));
    let rng = ClientRng::new(seed as u64);
    let mut presses = 0;
    let mut i = 0;
    while g.game.host().world().turn() < TURNS && !g.game.end_loop {
        let (chord, command) = if i < chords.len() {
            chords[i].clone()
        } else {
            chords[rng.below(chords.len())].clone()
        };
        i += 1;
        h.press(chord.key, chord.mods);
        presses += 1;
        // Playing the other side would leave this one unattended: switch
        // back at once.
        if command == GameCommand::ChangePlayer {
            h.step(&mut g);
            h.run_for(&mut g, 1.0);
            h.press(chord.key, chord.mods);
            presses += 1;
        }
        h.step(&mut g);
        // The game menu and the console take the keys until they close:
        // leave them with Escape.
        if matches!(command, GameCommand::GameMenu | GameCommand::Console) {
            h.press(Key::Escape, Mods::NONE);
            h.step(&mut g);
        }
        let pause = 0.2 + rng.random() * 2.0;
        h.run_for(&mut g, pause);
    }
    if !g.game.end_loop {
        h.run_until_turn(&mut g, TURNS);
    }
    if g.game.end_loop {
        let all = h.said_since(0);
        println!("{map} END {:#?}", &all[all.len().saturating_sub(40)..]);
    }
    assert!(
        g.game.host().world().turn() >= TURNS,
        "{map}: the game ended at turn {}",
        g.game.host().world().turn()
    );
    assert!(presses >= 300, "{map}: {presses} key presses");
    let unexpected: Vec<String> = h
        .log_text()
        .lines()
        .filter(|l| !EXPECTED_WARNINGS.iter().any(|w| l.contains(w)))
        .map(str::to_owned)
        .collect();
    println!(
        "{map}: {presses} presses, turn {}",
        g.game.host().world().turn()
    );
    println!("{map} log:\n{}", h.log_text());
    assert!(unexpected.is_empty(), "{map}: {unexpected:#?}");
}

#[test]
fn a_human_pressing_every_key_against_an_aggressive_computer_on_jl5() {
    soak("jl5", 21);
}

#[test]
fn a_human_pressing_every_key_against_an_aggressive_computer_on_jl2() {
    soak("jl2", 12);
}
