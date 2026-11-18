//! CrazyMod, the mod with ten races that comes with the game in
//! `data/mods/crazymod`: its rules and computer players load over the
//! game's own, and each race plays a computer game against the next.

mod common;

use std::path::PathBuf;

use soundrts_world::rules::EntityClass;
use soundrts_world::{
    AiScripts, Controller, PlayerId, Rules, Skill, World, multiplayer_default_triggers,
};

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

/// The turns each game plays: 9 minutes of game time, short of the first
/// defeat: vermin falls to dark elf near turn 2,100, and wild to vermin
/// near turn 2,500.
const TURNS: usize = 1800;

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

/// A game of the mod on a map of the game, between two computers, each
/// alone: `seats` gives the faction and the computer type of each.
fn game(map: &str, seed: i64, seats: [(&str, &str); 2]) -> (World, Vec<PlayerId>) {
    let mut w = World::new(
        Rules::load(&rules_text()),
        AiScripts::load(&ai_text()),
        multiplayer_default_triggers(),
        seed,
    );
    w.load_and_build_map(&common::read(map)).unwrap();
    let seats: Vec<Controller> = seats
        .into_iter()
        .enumerate()
        .map(|(k, (faction, ai))| {
            let mut c = Controller::computer_at(ai, Skill::NORMAL);
            c.alliance = Some((k + 1).to_string());
            c.faction = faction.to_owned();
            c
        })
        .collect();
    let players = w.populate_map(&seats, true, true).unwrap();
    (w, players)
}

/// How many fighters `p` has: soldiers that do damage. A larva is a
/// soldier that does none.
fn fighters(w: &World, p: PlayerId) -> usize {
    common::units(w, p)
        .filter(|e| e.class() == EntityClass::Soldier && e.stats.damage > 0)
        .count()
}

#[test]
fn each_race_plays_a_computer_game_against_the_next() {
    for (i, &race) in RACES.iter().enumerate() {
        let next = RACES[(i + 1) % RACES.len()];
        let (mut w, players) = game(
            "res/multi/pra1.txt",
            i as i64 + 1,
            [(race, "aggressive"), (next, "aggressive")],
        );
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

/// The author left the vermin computers without a script, and a vermin
/// computer only gathered. The port's scripts raise larvae in batches that
/// the hatchery spawns at once, and grow them into fighters. Here vermin
/// attacks at turn 1,177 with 13 fighters.
#[test]
fn the_aggressive_vermin_computer_trains_an_army_and_attacks() {
    let (mut w, players) = game(
        "res/multi/pra1.txt",
        4,
        [("vermin", "aggressive"), ("robotic", "aggressive")],
    );
    let vermin = players[0];
    let mut most = 0;
    let attack = (1..=1500).find(|_| {
        w.update();
        most = most.max(fighters(&w, vermin));
        common::attacks(&w, vermin)
    });
    assert!(
        attack.is_some(),
        "vermin did not attack in 1,500 turns; it had {most} fighters at most"
    );
    assert!(most >= 9, "vermin attacked with {most} fighters at most");
}

/// The quiet computer gets no scripted attack. Here it has 9 fighters at
/// turn 1,200.
#[test]
fn the_quiet_vermin_computer_builds_an_army() {
    let (mut w, players) = game(
        "res/multi/pra1.txt",
        1,
        [("vermin", "easy"), ("orc", "easy")],
    );
    for _ in 0..1200 {
        w.update();
    }
    let n = fighters(&w, players[0]);
    assert!(
        n >= 9,
        "the quiet vermin computer has {n} fighters after 1,200 turns"
    );
}

/// On jl1 each player starts with one worker. A vermin worker comes from a
/// larva, which the hatchery spawns, or from the termite mound's supply
/// drop. The computer asks for the maker it can make now, larvae, and not
/// the first one the rules list, the mound, whose stump would use the
/// hatchery's mana and expire before the mana returns.
#[test]
fn the_vermin_computer_grows_its_workers_from_a_one_worker_start() {
    let (mut w, players) = game(
        "res/multi/jl1.txt",
        1,
        [("vermin", "aggressive"), ("orc", "aggressive")],
    );
    let workers = |w: &World| {
        common::units(w, players[0])
            .filter(|e| e.class() == EntityClass::Worker)
            .count()
    };
    assert_eq!(workers(&w), 1);
    for _ in 0..1000 {
        w.update();
    }
    let n = workers(&w);
    assert!(n >= 6, "vermin has {n} workers after 1,000 turns on jl1");
}
