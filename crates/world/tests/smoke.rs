mod common;

use soundrts_world::{Controller, World, multiplayer_default_triggers};

#[test]
fn a_two_player_game_runs_on_jl1() {
    let mut w = World::new(
        common::rules(),
        common::ai(),
        multiplayer_default_triggers(),
        7,
    );
    w.load_and_build_map(&common::read("res/multi/jl1.txt"))
        .unwrap();
    let players = w
        .populate_map(
            &[Controller::human("me", true), Controller::computer("easy")],
            true,
            true,
        )
        .unwrap();
    for _ in 0..200 {
        w.update();
    }
    assert!(!w.player(players[0]).units.is_empty());
    let warnings = w.drain_warnings();
    assert!(warnings.is_empty(), "{warnings:?}");
}
