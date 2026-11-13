//! Training games played to the end: an idle human against an aggressive
//! computer. The world must not panic or warn, and the computer must win
//! somewhere, which shows that the AI plays.

mod common;

use soundrts_world::msg::msg;
use soundrts_world::msgparts as mp;
use soundrts_world::{Controller, Event, World, multiplayer_default_triggers};

const MAX_TURNS: usize = 3000;

struct Outcome {
    turns: usize,
    human_defeated: bool,
    defeat_announced: bool,
}

fn training_game(map: &str, seed: i64) -> Outcome {
    let mut w = World::new(
        common::rules(),
        common::ai(),
        multiplayer_default_triggers(),
        seed,
    );
    w.load_and_build_map(&common::read(map))
        .unwrap_or_else(|e| panic!("{map}: {e}"));
    let human = w
        .populate_map(
            &[
                Controller::human("me", true),
                Controller::computer("aggressive"),
            ],
            true,
            true,
        )
        .unwrap()[0];
    let defeat = msg(mp::YOU_HAVE_BEEN_DEFEATED);
    let mut defeat_announced = false;
    let mut turns = 0;
    while turns < MAX_TURNS && !w.true_playing_players().is_empty() {
        w.update();
        turns += 1;
        for e in w.drain_events(human) {
            if let Event::VoiceImportant(m) = e
                && m.starts_with(&defeat)
            {
                defeat_announced = true;
            }
        }
        let warnings = w.drain_warnings();
        assert!(warnings.is_empty(), "{map}, turn {turns}: {warnings:?}");
    }
    Outcome {
        turns,
        human_defeated: w.player(human).has_been_defeated,
        defeat_announced,
    }
}

#[test]
fn an_aggressive_computer_plays_training_games_to_the_end() {
    let maps = [
        ("res/multi/jl1.txt", 1),
        ("res/multi/jl2.txt", 2),
        ("res/multi/js3.txt", 3),
        ("res/multi/kel2.txt", 4),
        ("res/multi/jl5.txt", 5),
    ];
    let mut defeats = Vec::new();
    for (map, seed) in maps {
        let o = training_game(map, seed);
        assert_eq!(
            o.human_defeated, o.defeat_announced,
            "{map}: the defeat state and the defeat event disagree"
        );
        eprintln!(
            "{map}: {} turns, human defeated: {}",
            o.turns, o.human_defeated
        );
        if o.human_defeated {
            defeats.push(map);
        }
    }
    assert!(
        !defeats.is_empty(),
        "the aggressive computer defeated the idle human on no map"
    );
}
