//! Key help in a game: Control F1 turns it on, and then a key says its
//! name and what it does instead of doing it. Run on the real game data,
//! scripted keys, the recorded speech device and the manual clock: these
//! check what the client sends to the speech device, not what a screen
//! reader says.

use soundrts::clock::Clock;
use soundrts::input::{Key, Mods};
use soundrts::testkit::{Harness, LiveKeys, Script, TestGame};

const ON: &str = "key help on: press a key to hear what it does, Escape to leave";
const OFF: &str = "key help off";

/// A training game on jl1 at its second turn, with nothing left to say,
/// paused unless `running`.
fn game(running: bool) -> (Harness, TestGame, LiveKeys) {
    let live = LiveKeys::default();
    let script = Script::new().keep_open().with_live(live.clone());
    let mut h = Harness::new("en", script);
    let mut g = h.start_training("jl1", &["easy"], 1);
    h.run_until_turn(&mut g, 2);
    if !running {
        g.game.execute_command(&mut h.client, "p");
    }
    h.run_until_voice_idle(&mut g);
    (h, g, live)
}

#[test]
fn a_key_says_its_name_and_command_and_does_not_run_it() {
    let (mut h, mut g, _) = game(false);
    assert_eq!(h.key(&mut g, Key::F1, Mods::CTRL), [ON]);
    assert_eq!(
        h.key(&mut g, Key::Tab, Mods::CTRL),
        ["Control Tab: next useful target"]
    );
    // Tab would have selected a target, and F3 said the time.
    assert_eq!(g.game.target, None);
    assert_eq!(h.key(&mut g, Key::F3, Mods::NONE), ["F3: say the time"]);
    assert_eq!(
        h.key(&mut g, Key::S, Mods::SHIFT),
        ["Shift S: select all workers and soldiers on this square"]
    );
    assert_eq!(
        h.key(&mut g, Key::Num3, Mods::CTRL),
        ["Control 3: control a third of the units of the same types"]
    );
    assert_eq!(
        h.key(&mut g, Key::Z, Mods::NONE),
        ["Z: say the amount of the first resource"]
    );
    assert!(g.game.group.is_empty());
}

#[test]
fn a_key_with_no_command_says_so() {
    let (mut h, mut g, _) = game(false);
    h.key(&mut g, Key::F1, Mods::CTRL);
    assert_eq!(
        h.key(&mut g, Key::Insert, Mods::NONE),
        ["Insert: no command"]
    );
    assert_eq!(
        h.key(&mut g, Key::Escape, Mods::SHIFT),
        ["Shift Escape: no command"]
    );
}

#[test]
fn a_modifier_alone_is_described_when_it_is_released() {
    let (mut h, mut g, live) = game(false);
    h.key(&mut g, Key::F1, Mods::CTRL);
    let from = h.utterances().len();
    let start = h.clock.now();
    live.down_at(start, Key::LCtrl, Mods::CTRL);
    live.up_at(start + 1.0, Key::LCtrl);
    h.run_for(&mut g, 0.9);
    assert_eq!(h.said_since(from), Vec::<String>::new());
    h.run_for(&mut g, 0.2);
    h.run_until_voice_idle(&mut g);
    assert_eq!(
        h.said_since(from),
        ["Left Control: describe the target or the square"]
    );
}

#[test]
fn escape_or_its_own_key_leaves_and_the_game_runs_on_meanwhile() {
    let (mut h, mut g, _) = game(true);
    assert_eq!(h.key(&mut g, Key::F1, Mods::CTRL)[0], ON);
    let turn = g.game.view().turn();
    h.run_for(&mut g, 3.0);
    // Ten turns of 0.3 seconds have passed in key help.
    assert!(g.game.view().turn() >= turn + 9, "{turn}");
    // The news of the running game may follow what the keys say.
    assert_eq!(h.key(&mut g, Key::Escape, Mods::NONE)[0], OFF);
    assert_eq!(h.key(&mut g, Key::F1, Mods::CTRL)[0], ON);
    assert_eq!(h.key(&mut g, Key::F1, Mods::CTRL)[0], OFF);
    // The keys do as before: Z says the gold.
    assert_eq!(h.key(&mut g, Key::Z, Mods::NONE)[0], "10 gold");
}
