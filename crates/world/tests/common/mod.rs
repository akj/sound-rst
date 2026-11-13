//! Helpers for the tests that read the original game's data.
#![allow(dead_code)]

use std::path::PathBuf;

use soundrts_world::{AiScripts, Controller, EntityId, PlayerId, Rules, World};

/// The pinned original game, fetched by `scripts/fetch-upstream.ps1`.
pub fn upstream() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vendor/soundrts-upstream");
    assert!(
        dir.join("res/rules.txt").is_file(),
        "the original game is missing from {}: run scripts/fetch-upstream.ps1",
        dir.display()
    );
    dir
}

/// A text file of the original, decoded as the original does (UTF-8, bad
/// bytes replaced).
pub fn read(relative: &str) -> String {
    let path = upstream().join(relative);
    let bytes =
        std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    String::from_utf8_lossy(&bytes).into_owned()
}

pub fn rules() -> Rules {
    Rules::load(&[read("res/rules.txt")])
}

pub fn ai() -> AiScripts {
    AiScripts::load(&[read("res/ai.txt")])
}

/// `World()` with the default rules, then `load_and_build_map`.
pub fn world_with_map(relative: &str) -> World {
    let mut w = World::new(rules(), ai(), Vec::new(), 0);
    w.load_and_build_map(&read(relative))
        .unwrap_or_else(|e| panic!("{relative}: {e}"));
    w
}

/// A world built from map text given inline.
pub fn world_with_map_text(text: &str) -> World {
    let mut w = World::new(rules(), ai(), Vec::new(), 0);
    w.load_and_build_map(text).unwrap_or_else(|e| panic!("{e}"));
    w
}

/// The first unit (or the `index`-th) of a type of a player.
pub fn find_player_unit(w: &World, p: PlayerId, type_name: &str, index: usize) -> Option<EntityId> {
    w.player(p)
        .units
        .iter()
        .copied()
        .filter(|&u| &*w.entity(u).unwrap().type_name == type_name)
        .nth(index)
}

/// A computer controller of the given AI type, with an alliance.
pub fn computer(ai_type: &str, alliance: &str) -> Controller {
    let mut c = Controller::computer(ai_type);
    c.alliance = Some(alliance.to_owned());
    c
}

pub fn words(s: &str) -> Vec<String> {
    s.split_whitespace().map(str::to_owned).collect()
}
