//! What the world stores as a game goes on: the entities it keeps against
//! those on the map, and the copies its players remember. An entity that
//! left the map is freed once nothing names it, and a player who left the
//! game forgets what it knew.
//!
//! The measure of a long game is ignored, since it takes a minute; it
//! prints what is stored and how long a turn takes. Run it in a release
//! build:
//!
//! ```text
//! cargo test --release -p soundrts-world --test storage -- --ignored --nocapture
//! ```
//!
//! `STORAGE_MAP` (`js13` by default), `STORAGE_SEED` (1) and
//! `STORAGE_TURNS` (4,000) change the game: as many aggressive computers as
//! the map has seats, each alone.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use soundrts_world::entity::Kind;
use soundrts_world::test_games::computer;
use soundrts_world::{EntityId, PlayerId, World, multiplayer_default_triggers};

/// What the world holds at the end of a turn.
#[derive(Clone, Copy, Debug, Default)]
struct Held {
    /// Every entity the world stores.
    stored: usize,
    /// The stored entities that have a place: on a square, or inside
    /// another entity.
    on_map: usize,
    /// The remembered copies of the players still playing.
    remembered_playing: usize,
    /// The remembered copies of the defeated players, watching or gone.
    remembered_defeated: usize,
}

fn held(w: &World) -> Held {
    let mut h = Held {
        stored: w.entities().count(),
        on_map: w.entities().filter(|e| e.place.is_some()).count(),
        ..Held::default()
    };
    for (id, p) in w.all_players() {
        if p.has_been_defeated || w.ex_players.contains(&id) {
            h.remembered_defeated += p.memory.len();
        } else {
            h.remembered_playing += p.memory.len();
        }
    }
    h
}

/// A game of computers alone, as many as the map has seats.
fn computers_alone(map: &str, seed: i64) -> World {
    let mut w = World::new(
        common::rules(),
        common::ai(),
        multiplayer_default_triggers(),
        seed,
    );
    w.load_and_build_map(&common::read(&format!("res/multi/{map}.txt")))
        .unwrap();
    let seats = usize::try_from(w.map.nb_players_max).unwrap();
    let controllers: Vec<_> = (0..seats)
        .map(|i| computer("aggressive", &(i + 1).to_string()))
        .collect();
    w.populate_map(&controllers, true, true).unwrap();
    w
}

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_owned())
}

#[test]
#[ignore = "a minute: run by hand in a release build"]
fn measure_what_a_long_game_stores() {
    let map = env_or("STORAGE_MAP", "js13");
    let seed: i64 = env_or("STORAGE_SEED", "1").parse().unwrap();
    let turns: usize = env_or("STORAGE_TURNS", "4000").parse().unwrap();
    let mut w = computers_alone(&map, seed);
    let mut rows: Vec<(Held, Duration)> = Vec::new();
    while rows.len() < turns && !w.true_playing_players().is_empty() {
        let start = Instant::now();
        w.update();
        let took = start.elapsed();
        rows.push((held(&w), took));
    }
    let n = rows.len();
    eprintln!("{map} seed {seed}: {n} turns");
    eprintln!(
        "greatest id stored at the end: {}",
        w.entities().map(|e| e.id.0).max().unwrap_or(0)
    );
    for (name, at) in [("start", 0), ("middle", n / 2), ("end", n - 1)] {
        let (h, _) = rows[at];
        eprintln!(
            "turn {} ({name}): stored {}, on the map {}, remembered by players playing {}, by defeated players {}",
            at + 1,
            h.stored,
            h.on_map,
            h.remembered_playing,
            h.remembered_defeated
        );
    }
    let most = |f: fn(&Held) -> usize| rows.iter().map(|(h, _)| f(h)).max().unwrap_or(0);
    eprintln!(
        "most stored off the map at once: {}",
        most(|h| h.stored - h.on_map)
    );
    eprintln!(
        "most at once: stored {}, on the map {}, remembered by players playing {}, by defeated players {}",
        most(|h| h.stored),
        most(|h| h.on_map),
        most(|h| h.remembered_playing),
        most(|h| h.remembered_defeated)
    );
    let window = 200.min(n);
    for (name, from) in [
        ("first", 0),
        ("middle", n / 2 - window / 2),
        ("last", n - window),
    ] {
        let total: Duration = rows[from..from + window].iter().map(|(_, d)| *d).sum();
        eprintln!(
            "turn time, {name} {window} turns: mean {} microseconds",
            total.as_micros() / window as u128
        );
    }
    let mut off_map: BTreeMap<&str, usize> = BTreeMap::new();
    for e in w.entities().filter(|e| e.place.is_none()) {
        *off_map.entry(&e.type_name).or_default() += 1;
    }
    eprintln!("stored off the map at the end, by type: {off_map:?}");
    let total: Duration = rows.iter().map(|(_, d)| *d).sum();
    eprintln!(
        "turn time, all turns: mean {} microseconds, total {} milliseconds",
        total.as_micros() / n as u128,
        total.as_millis()
    );
}

/// The land that the entities on the map cover: a building's meadow, or
/// the meadow under a deposit, put back when they go.
fn covered_land(w: &World) -> BTreeSet<EntityId> {
    w.entities()
        .filter(|e| e.place.is_some())
        .filter_map(|e| match &e.kind {
            Kind::Deposit(d) => d.building_land,
            Kind::Creature(c) => c.building_land,
            Kind::Meadow | Kind::Exit(_) | Kind::Corpse(_) => None,
        })
        .collect()
}

/// The most entities a game may store off the map, beside the land that
/// the entities on the map cover: those that left during the turn, and
/// those something still names, such as what a computer player still
/// tracks.
const OFF_THE_MAP: usize = 60;

/// Computers fight on kel6 until one is left: the world stores what is on
/// the map, the land it covers, and at most [`OFF_THE_MAP`] entities more.
/// A world that kept every entity would store hundreds more by the end.
/// Ids are never given again, and a player who left the game keeps no
/// knowledge past the turn it left.
#[test]
fn a_long_game_stores_little_beyond_what_is_on_the_map() {
    let mut w = computers_alone("kel6", 1);
    let mut ever_stored: BTreeSet<EntityId> = w.entities().map(|e| e.id).collect();
    let mut freed: BTreeSet<EntityId> = BTreeSet::new();
    let mut gone_before: Vec<PlayerId> = Vec::new();
    let mut turn = 0;
    while turn < 3000 && !w.true_playing_players().is_empty() {
        w.update();
        turn += 1;
        let last = ever_stored.last().copied();
        let new: Vec<EntityId> = w
            .entities()
            .map(|e| e.id)
            .filter(|id| !ever_stored.contains(id))
            .collect();
        assert!(
            new.iter().all(|&id| Some(id) > last),
            "turn {turn}: an id given again in {new:?}"
        );
        ever_stored.extend(new);
        let stored: BTreeSet<EntityId> = w.entities().map(|e| e.id).collect();
        assert!(
            freed.is_disjoint(&stored),
            "turn {turn}: a freed id names an entity again"
        );
        freed.extend(ever_stored.difference(&stored));
        let land = covered_land(&w);
        let off_the_map = w
            .entities()
            .filter(|e| e.place.is_none() && !land.contains(&e.id))
            .count();
        assert!(
            off_the_map <= OFF_THE_MAP,
            "turn {turn}: {off_the_map} entities stored off the map"
        );
        for &p in &gone_before {
            let pl = w.player(p);
            assert!(
                pl.memory.is_empty() && pl.perception.is_empty(),
                "turn {turn}: player {} left and still knows",
                p.0
            );
        }
        gone_before.clone_from(&w.ex_players);
    }
    assert!(!gone_before.is_empty(), "nobody left the game");
    let on_map = w.entities().filter(|e| e.place.is_some()).count();
    let land = covered_land(&w).len();
    assert!(
        ever_stored.len() - on_map - land > 3 * OFF_THE_MAP,
        "too few entities left the map for the bound to tell: {} stored at some time, {on_map} on the map",
        ever_stored.len()
    );
    assert!(freed.len() > 3 * OFF_THE_MAP, "only {} freed", freed.len());
}
