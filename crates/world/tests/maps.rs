//! Every map of the original game loads, and games run on them.

mod common;

use std::path::Path;

use soundrts_world::{AiScripts, Controller, Rules, World, multiplayer_default_triggers};

/// The text maps of a folder of the original, by file name.
fn maps_in(relative: &str) -> Vec<String> {
    let dir = common::upstream().join(relative);
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".txt"))
        .collect();
    names.sort();
    names
}

fn joined(paths: &[&str]) -> String {
    paths
        .iter()
        .map(|p| common::read(p))
        .collect::<Vec<_>>()
        .join("\n")
}

fn build(rules: &str, ai: &str, map: &str, seed: i64) -> Result<World, String> {
    let mut w = World::new(
        Rules::load(&[rules]),
        AiScripts::load(&[ai]),
        multiplayer_default_triggers(),
        seed,
    );
    w.load_and_build_map(map).map_err(|e| e.0)?;
    Ok(w)
}

fn multiplayer_maps() -> Vec<String> {
    maps_in("res/multi")
        .into_iter()
        .map(|n| format!("res/multi/{n}"))
        .collect()
}

#[test]
fn every_multiplayer_map_builds() {
    let rules = joined(&["res/rules.txt"]);
    let ai = joined(&["res/ai.txt"]);
    let maps = multiplayer_maps();
    assert!(maps.len() >= 60, "only {} maps found", maps.len());
    for m in &maps {
        let w = build(&rules, &ai, &common::read(m), 0).unwrap_or_else(|e| panic!("{m}: {e}"));
        assert!(!w.squares.is_empty(), "{m}");
    }
}

/// A campaign folder: its rules and AI layers, and its mission maps.
struct Campaign {
    name: String,
    rules: Vec<String>,
    ai: Vec<String>,
    missions: Vec<String>,
}

/// The campaign folders (cut scene chapters are not maps).
fn campaigns() -> Vec<Campaign> {
    let root = common::upstream().join("single");
    let mut out = Vec::new();
    let mut dirs: Vec<String> = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    dirs.sort();
    for d in dirs {
        let rel = format!("single/{d}");
        let mut rules = vec!["res/rules.txt".to_owned()];
        let mut ai = vec!["res/ai.txt".to_owned()];
        if Path::new(&root.join(&d).join("rules.txt")).is_file() {
            rules.push(format!("{rel}/rules.txt"));
        }
        if Path::new(&root.join(&d).join("ai.txt")).is_file() {
            ai.push(format!("{rel}/ai.txt"));
        }
        let missions: Vec<String> = (0..)
            .map(|n| format!("{rel}/{n}.txt"))
            .take_while(|p| common::upstream().join(p).is_file())
            .filter(|p| !common::read(p).starts_with("cut_scene_chapter\n"))
            .collect();
        out.push(Campaign {
            name: d,
            rules,
            ai,
            missions,
        });
    }
    out
}

#[test]
fn every_campaign_mission_builds() {
    let mut n = 0;
    for c in campaigns() {
        let rules: Vec<&str> = c.rules.iter().map(String::as_str).collect();
        let ai: Vec<&str> = c.ai.iter().map(String::as_str).collect();
        let (rules, ai) = (joined(&rules), joined(&ai));
        for m in c.missions {
            build(&rules, &ai, &common::read(&m), 0)
                .unwrap_or_else(|e| panic!("{} {m}: {e}", c.name));
            n += 1;
        }
    }
    // 12 missions and a cut scene in the main campaign, 1 orc mission.
    assert_eq!(n, 13, "missions built");
}

/// The mods have no maps of their own; their rules must still accept every
/// multiplayer map.
#[test]
fn every_multiplayer_map_builds_with_each_mod() {
    let mods = ["flyingtownhall", "orc", "teleportation"];
    for m in mods {
        let rules = joined(&["res/rules.txt", &format!("mods/{m}/rules.txt")]);
        let ai_path = format!("mods/{m}/ai.txt");
        let ai = if common::upstream().join(&ai_path).is_file() {
            joined(&["res/ai.txt", &ai_path])
        } else {
            joined(&["res/ai.txt"])
        };
        for map in multiplayer_maps() {
            build(&rules, &ai, &common::read(&map), 0)
                .unwrap_or_else(|e| panic!("mod {m}, {map}: {e}"));
        }
    }
}

#[test]
fn two_player_games_run_200_turns_on_every_multiplayer_map() {
    let rules = joined(&["res/rules.txt"]);
    let ai = joined(&["res/ai.txt"]);
    let mut played = 0;
    for (i, m) in multiplayer_maps().iter().enumerate() {
        let mut w = build(&rules, &ai, &common::read(m), i as i64).unwrap();
        if w.map.nb_players_max < 2 {
            continue;
        }
        let players = w
            .populate_map(
                &[Controller::human("me", true), Controller::computer("easy")],
                true,
                true,
            )
            .unwrap_or_else(|e| panic!("{m}: {e}"));
        let me = players[0];
        let townhall = w
            .player(me)
            .units
            .iter()
            .copied()
            .find(|&u| &*w.entity(u).unwrap().type_name == "townhall");
        if let Some(th) = townhall {
            w.queue_command(me, &format!("control {th}"));
            w.queue_command(me, "order 0 0 train peasant");
        }
        w.drain_warnings();
        for _ in 0..200 {
            w.update();
            w.drain_events(me);
        }
        assert_eq!(w.time, 200 * 300, "{m}");
        // A warning during the game would be an error the original catches.
        let warnings = w.drain_warnings();
        assert!(warnings.is_empty(), "{m}: {warnings:?}");
        played += 1;
    }
    assert!(played >= 5, "games played: {played}");
}

/// The error of a small map that asks for too much, made of `lines` and a
/// start on a1.
fn map_error(lines: &str) -> String {
    let text = format!("square_width 12\nnb_columns 3\nstarting_squares a1\n{lines}\n");
    match build(
        &joined(&["res/rules.txt"]),
        &joined(&["res/ai.txt"]),
        &text,
        0,
    ) {
        Ok(_) => panic!("the map built: {text}"),
        Err(e) => e,
    }
}

#[test]
fn a_map_with_too_many_lines_is_refused() {
    assert_eq!(
        map_error("nb_lines 100\nstarting_units farm"),
        "100 lines, more than 99"
    );
}

#[test]
fn a_map_with_too_many_meadows_is_refused() {
    assert_eq!(
        map_error("nb_lines 3\nnb_meadows_by_square 1000000000\nstarting_units farm"),
        "1000000000 meadows by square, more than 50"
    );
}

#[test]
fn a_map_with_too_many_start_units_is_refused() {
    assert_eq!(
        map_error("nb_lines 3\nstarting_units 9000 footman 1001 peasant"),
        "the starts give 10001 units, more than 10000"
    );
}
