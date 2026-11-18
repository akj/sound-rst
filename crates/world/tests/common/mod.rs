//! Helpers for the tests that read the game's data, as imported from the
//! original.
#![allow(dead_code)]

use std::collections::BTreeSet;
use std::path::PathBuf;

use soundrts_world::entity::{Action, ActionKind, Entity};
use soundrts_world::orders::Keyword;
use soundrts_world::rules::EntityClass;
use soundrts_world::{AiScripts, Controller, EntityId, PlayerId, Rules, SquareId, Target, World};

/// The pinned original game, fetched and imported by
/// `scripts/fetch-upstream.ps1`.
pub fn upstream() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vendor/game-data");
    assert!(
        dir.join("res/rules.txt").is_file(),
        "the imported game data is missing from {}: run scripts/fetch-upstream.ps1",
        dir.display()
    );
    dir
}

/// A text file of the game data, decoded as the original does (UTF-8, bad
/// bytes replaced).
pub fn read(relative: &str) -> String {
    // The original's own test maps stay in its checkout.
    let path = match relative.strip_prefix("original/") {
        Some(relative) => upstream().join("../soundrts-upstream").join(relative),
        None => upstream().join(relative),
    };
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

/// Whether `e` belongs to an enemy of `p` that is a true player, not a
/// neutral one.
pub fn is_true_enemy(w: &World, p: PlayerId, e: &Entity) -> bool {
    e.player()
        .is_some_and(|q| !w.player(p).allied.contains(&q) && !w.player(q).neutral)
}

/// The units and buildings of `p`.
pub fn units(w: &World, p: PlayerId) -> impl Iterator<Item = &Entity> {
    w.player(p).units.iter().filter_map(|&u| w.entity(u))
}

/// The squares where `p` has a building, a building site or a worker: its
/// idle soldiers gather where its workers are.
pub fn home(w: &World, p: PlayerId) -> BTreeSet<SquareId> {
    units(w, p)
        .filter(|e| e.class().is_building_class() || e.class() == EntityClass::Worker)
        .filter_map(Entity::square)
        .collect()
}

/// Whether a soldier of `p` is attacking a unit or building of an enemy
/// true player that stands outside its home (see [`home`]), and the
/// soldier is not its explorer (its first order is not `auto_explore`).
/// This leaves out defending its base, its workers' fights and its
/// explorer's fights on the way.
pub fn attacks(w: &World, p: PlayerId) -> bool {
    let home = home(w, p);
    units(w, p).any(|e| {
        let Some(Action {
            kind: ActionKind::Attack,
            target: Some(Target::Entity(t)),
        }) = e.creature().and_then(|c| c.action.as_ref())
        else {
            return false;
        };
        e.class() == EntityClass::Soldier
            && e.orders().first().map(|o| o.keyword()) != Some(Keyword::AutoExplore)
            && w.entity(*t).is_some_and(|t| {
                is_true_enemy(w, p, t) && t.square().is_some_and(|s| !home.contains(&s))
            })
    })
}
