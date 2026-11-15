//! What the world stores as a game goes on: the entities it keeps against
//! those on the map, and the copies its players remember.
//!
//! The measure of a long game is ignored, since it takes a minute; run it
//! in a release build:
//!
//! ```text
//! cargo test --release -p soundrts-world --test storage -- --ignored --nocapture
//! ```
//!
//! `STORAGE_MAP` (`js13` by default), `STORAGE_SEED` (1) and
//! `STORAGE_TURNS` (4,000) change the game: as many aggressive computers as
//! the map has seats, each alone.

mod common;

use std::time::{Duration, Instant};

use soundrts_world::test_games::computer;
use soundrts_world::{World, multiplayer_default_triggers};

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
    let total: Duration = rows.iter().map(|(_, d)| *d).sum();
    eprintln!(
        "turn time, all turns: mean {} microseconds, total {} milliseconds",
        total.as_micros() / n as u128,
        total.as_millis()
    );
}
