//! CrazyMod, the mod with ten races that comes with the game in
//! `data/mods/crazymod`: its rules and computer players load over the
//! game's own, and each race plays a computer game against the next.

mod common;

use std::path::PathBuf;

use soundrts_world::{AiScripts, Controller, Rules, Skill, World, multiplayer_default_triggers};

/// The races, in the order the author defines them.
const RACES: [&str; 10] = [
    "traditional",
    "technical",
    "robotic",
    "undead",
    "elf",
    "orc",
    "elemental",
    "wild",
    "vermin",
    "dark_elf",
];

/// The turns each game plays: 12.5 minutes of game time. Every computer
/// builds by then, and the first armies meet (undead against elf, elf
/// against orc), but no seat is beaten yet: undead loses to elf near turn
/// 3,500. The vermin computers' scripts are empty, as the author left them;
/// vermin keeps its opening units through these turns.
const TURNS: usize = 2500;

/// A file of the mod, from the checkout.
fn read(file: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../data/mods/crazymod")
        .join(file);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

fn rules_text() -> [String; 2] {
    [common::read("res/rules.txt"), read("rules.txt")]
}

fn ai_text() -> [String; 2] {
    [common::read("res/ai.txt"), read("ai.txt")]
}

#[test]
fn the_rules_and_the_computers_load_without_a_warning() {
    let rules = Rules::load(&rules_text());
    assert_eq!(rules.warnings, Vec::<String>::new());
    assert_eq!(rules.factions(), RACES);
    let ai = AiScripts::load(&ai_text());
    assert_eq!(ai.warnings, Vec::<String>::new());
}

#[test]
fn each_race_plays_a_computer_game_against_the_next() {
    for (i, &race) in RACES.iter().enumerate() {
        let next = RACES[(i + 1) % RACES.len()];
        let mut w = World::new(
            Rules::load(&rules_text()),
            AiScripts::load(&ai_text()),
            multiplayer_default_triggers(),
            i as i64 + 1,
        );
        w.load_and_build_map(&common::read("res/multi/pra1.txt"))
            .unwrap();
        let seats: Vec<Controller> = [race, next]
            .into_iter()
            .enumerate()
            .map(|(k, faction)| {
                let mut c = Controller::computer_at("aggressive", Skill::NORMAL);
                c.alliance = Some((k + 1).to_string());
                c.faction = faction.to_owned();
                c
            })
            .collect();
        let players = w.populate_map(&seats, true, true).unwrap();
        for _ in 0..TURNS {
            w.update();
        }
        // A computer warns of each line of its script it cannot follow.
        assert_eq!(
            w.drain_warnings(),
            Vec::<String>::new(),
            "{race} against {next}"
        );
        for (&p, faction) in players.iter().zip([race, next]) {
            assert!(
                !w.player(p).units.is_empty(),
                "{race} against {next}: {faction} has no units after {TURNS} turns"
            );
        }
    }
}
