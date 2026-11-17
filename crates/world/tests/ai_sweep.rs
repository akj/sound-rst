//! The computer players measured over maps and seeds: computers alone
//! against each other, until one is left or the turn limit. Both runs are
//! ignored, since a run takes minutes; run them in a release build.
//!
//! The sweep fills every seat of the map with a computer at the normal
//! level, and measures timings:
//!
//! ```text
//! cargo test --release -p soundrts-world --test ai_sweep sweep -- --ignored --nocapture
//! ```
//!
//! The ladder plays each skill level against the level above it, one
//! against one. Up to Normal, it checks that the higher level wins 70 to
//! 85 percent of the games. Above Normal, it checks that the higher level
//! wins at least 65 percent of all games and at least 55 percent of the
//! games on each map. A draw counts as a game the higher level did not
//! win:
//!
//! ```text
//! cargo test --release -p soundrts-world --test ai_sweep ladder -- --ignored --nocapture
//! ```
//!
//! The environment changes what is played:
//!
//! - `SWEEP_MAPS`: the maps, by name in `res/multi` or by path in the
//!   original game (`jl1,jl7,js13` by default for the sweep, and
//!   `jl1,js1,can3,js13` for the ladder). The ladder leaves out jl7: its
//!   starts are islands that no ground army can leave, so every game there
//!   ends in a draw.
//! - `SWEEP_SEEDS`: the seeds, a range or a list (`1-20` by default).
//! - `SWEEP_TURNS`: the turn limit of a game of the sweep (4,000 by
//!   default, 20 minutes of game time).
//! - `SWEEP_AI`: the computer types, given to the seats in turn
//!   (`aggressive` by default). The ladder takes one: the personality of
//!   both computers.
//! - `SWEEP_JOBS`: the games played at once (4 by default).
//! - `LADDER_TURNS`: the turn limit of a game of the ladder (12,000 by
//!   default, 60 minutes of game time). A game at the limit is a draw.
//! - `LADDER_PAIRS`: the pairs of levels the ladder plays, the lower level
//!   first (`1-2,2-3,3-4` by default). The ladder checks its band for each
//!   pair it plays.
//! - `LADDER_DETAILS`: `1` to print the sweep's measures of each game of
//!   the ladder too. The ladder runs slower with them.
//!
//! The ladder plays each map and seed twice, once with each level in the
//! first seat, so that a starting place does not decide the result.
//!
//! What is measured for each computer, from the world at the end of each
//! turn, is in [`Seat`]. A time is game time since the start; a turn is
//! 0.3 seconds of it.

mod common;

use std::collections::BTreeSet;

use soundrts_world::entity::{Action, ActionKind, Entity};
use soundrts_world::orders::Keyword;
use soundrts_world::rules::{EntityClass, Role, TypeId};
use soundrts_world::{
    AiScripts, Controller, PlayerId, Rules, Skill, SquareId, Target, VIRTUAL_TIME_INTERVAL, World,
    multiplayer_default_triggers,
};

/// How a computer's game ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum End {
    Won(i64),
    Defeated(i64),
}

/// What is measured of one computer in one game. Times are in
/// milliseconds of game time.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Seat {
    ai: String,
    skill: Skill,
    /// The end of the first turn at which the computer perceives or
    /// remembers a unit or building of an enemy that is a true player (not
    /// a neutral one). An aggressive computer attacks only squares where
    /// it knows of an enemy.
    first_enemy_seen: Option<i64>,
    /// The end of the first turn at which one of its soldiers is attacking
    /// (has the attack action on) a unit or building of an enemy true
    /// player, standing outside its home (see [`home`]), and the soldier is
    /// not its explorer (its first order is not `auto_explore`). This
    /// leaves out defending its base, its workers' fights and its
    /// explorer's fights on the way.
    first_attack: Option<i64>,
    /// The time it was stuck at its population limit: the turns at whose
    /// end its used food was at least its food (no room for any unit), its
    /// food was below the map's food limit (a farm would raise it), no farm
    /// of its faction was being built or ordered built, it had a worker, and no unit or
    /// building of an enemy true player stood on a square where it has a
    /// building or a worker. A base under attack or without workers has its
    /// limit set by the enemy, not by its economy: its defeat says the rest.
    stuck: i64,
    /// The part of `stuck` during which it lacked a resource for a farm of
    /// its faction: it could not raise the limit. The rest of `stuck` it
    /// could pay for a farm and did not build one.
    stuck_short: i64,
    /// Its victory or defeat, and when: the end of the turn at which the
    /// world first says it.
    end: Option<End>,
}

/// One game, measured.
struct Played {
    map: String,
    seed: i64,
    /// The game time at the end: when no player was left playing, or at
    /// the turn limit.
    time: i64,
    /// The game ended before the turn limit.
    ended: bool,
    seats: Vec<Seat>,
    /// The processor time of the world's turns, without the measures.
    cost: std::time::Duration,
}

/// A computer's farm: its type and its cost.
struct Farm {
    name: String,
    type_id: Option<TypeId>,
    cost: Vec<i64>,
}

fn farm_of(w: &World, p: PlayerId) -> Farm {
    let name = w.rules.role_name(w.player(p).faction, Role::Farm);
    Farm {
        type_id: w.rules.type_id(&name),
        cost: w
            .rules
            .unit_type(&name)
            .map(|t| t.cost.clone())
            .unwrap_or_default(),
        name,
    }
}

fn is_true_enemy(w: &World, p: PlayerId, e: &Entity) -> bool {
    e.player()
        .is_some_and(|q| !w.player(p).allied.contains(&q) && !w.player(q).neutral)
}

fn sees_an_enemy(w: &World, p: PlayerId) -> bool {
    let pl = w.player(p);
    pl.perception
        .iter()
        .filter_map(|&o| w.entity(o))
        .any(|e| is_true_enemy(w, p, e))
        || pl.memory.values().any(|r| is_true_enemy(w, p, &r.snapshot))
}

fn units(w: &World, p: PlayerId) -> impl Iterator<Item = &Entity> {
    w.player(p).units.iter().filter_map(|&u| w.entity(u))
}

/// The squares where `p` has a building, a building site or a worker: its
/// idle soldiers gather where its workers are.
fn home(w: &World, p: PlayerId) -> BTreeSet<SquareId> {
    units(w, p)
        .filter(|e| e.class().is_building_class() || e.class() == EntityClass::Worker)
        .filter_map(Entity::square)
        .collect()
}

/// `p` has a worker, and no enemy stands in its home.
fn at_peace(w: &World, p: PlayerId) -> bool {
    units(w, p).any(|e| e.class() == EntityClass::Worker)
        && !home(w, p).into_iter().any(|s| {
            w.square(s)
                .objects
                .iter()
                .filter_map(|&o| w.entity(o))
                .any(|e| is_true_enemy(w, p, e))
        })
}

fn attacks(w: &World, p: PlayerId) -> bool {
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

/// A farm site of `p` stands, or a worker has the order to build one.
fn builds_a_farm(w: &World, p: PlayerId, farm: &Farm) -> bool {
    units(w, p).any(|e| {
        e.site()
            .is_some_and(|s| Some(s.building_type) == farm.type_id)
            || e.orders().iter().any(|o| {
                o.keyword() == Keyword::Build && o.type_name(&w.rules) == Some(farm.name.as_str())
            })
    })
}

/// Updates the end of `p` at the end of a turn.
fn observe_end(w: &World, p: PlayerId, seat: &mut Seat) {
    let pl = w.player(p);
    if seat.end.is_none() {
        if pl.has_victory {
            seat.end = Some(End::Won(w.time));
        } else if pl.has_been_defeated {
            seat.end = Some(End::Defeated(w.time));
        }
    }
}

/// Updates the measures of `p` at the end of a turn, its end aside.
fn observe(w: &World, p: PlayerId, farm: &Farm, seat: &mut Seat) {
    let pl = w.player(p);
    if !pl.is_playing() {
        return;
    }
    if seat.first_enemy_seen.is_none() && sees_an_enemy(w, p) {
        seat.first_enemy_seen = Some(w.time);
    }
    if seat.first_attack.is_none() && attacks(w, p) {
        seat.first_attack = Some(w.time);
    }
    let limit = w.food_limit();
    let food = pl.available_food(limit);
    if food < limit && pl.used_food >= food && !builds_a_farm(w, p, farm) && at_peace(w, p) {
        seat.stuck += VIRTUAL_TIME_INTERVAL;
        let short = farm
            .cost
            .iter()
            .zip(&pl.resources)
            .any(|(&cost, &have)| have < cost);
        if short {
            seat.stuck_short += VIRTUAL_TIME_INTERVAL;
        }
    }
}

/// The original's files a game is built from.
struct Files {
    rules: String,
    ai: String,
}

impl Files {
    fn load() -> Files {
        Files {
            rules: common::read("res/rules.txt"),
            ai: common::read("res/ai.txt"),
        }
    }
}

fn map_path(map: &str) -> String {
    if map.contains('/') {
        map.to_owned()
    } else {
        format!("res/multi/{map}.txt")
    }
}

/// A computer to seat: its type and its level.
#[derive(Clone, Debug)]
struct Who {
    ai: String,
    skill: Skill,
}

/// Plays `map` with `seed`: as many computers as the map has seats, each
/// alone, of the `ais` types in turn at the normal level, for `turns`
/// turns at most.
fn play(files: &Files, map: &str, seed: i64, ais: &[String], turns: usize) -> Played {
    let w = world(files, map, seed);
    let seats = usize::try_from(w.map.nb_players_max).unwrap();
    let who: Vec<Who> = (0..seats)
        .map(|i| Who {
            ai: ais[i % ais.len()].clone(),
            skill: Skill::NORMAL,
        })
        .collect();
    play_seats(w, map, seed, &who, turns, true)
}

/// The world of `map`, built with `seed`.
fn world(files: &Files, map: &str, seed: i64) -> World {
    let mut w = World::new(
        Rules::load(&[files.rules.as_str()]),
        AiScripts::load(&[files.ai.as_str()]),
        multiplayer_default_triggers(),
        seed,
    );
    let text = common::read(&map_path(map));
    w.load_and_build_map(&text)
        .unwrap_or_else(|e| panic!("{map}: {e}"));
    w
}

/// Plays the world of `map` and `seed`: one computer, alone, in each of
/// the first seats, for `turns` turns at most. Without `measure`, only the
/// end of each computer is measured.
fn play_seats(
    mut w: World,
    map: &str,
    seed: i64,
    who: &[Who],
    turns: usize,
    measure: bool,
) -> Played {
    let controllers: Vec<_> = who
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let mut c = Controller::computer_at(&s.ai, s.skill);
            c.alliance = Some((i + 1).to_string());
            c
        })
        .collect();
    let players = w.populate_map(&controllers, true, true).unwrap();
    let farms: Vec<Farm> = players.iter().map(|&p| farm_of(&w, p)).collect();
    let mut measured: Vec<Seat> = who
        .iter()
        .map(|s| Seat {
            ai: s.ai.clone(),
            skill: s.skill,
            ..Seat::default()
        })
        .collect();
    let mut turn = 0;
    let mut cost = std::time::Duration::ZERO;
    while turn < turns && !w.true_playing_players().is_empty() {
        let start = std::time::Instant::now();
        w.update();
        cost += start.elapsed();
        turn += 1;
        for (i, &p) in players.iter().enumerate() {
            observe_end(&w, p, &mut measured[i]);
            if measure {
                observe(&w, p, &farms[i], &mut measured[i]);
            }
        }
    }
    Played {
        map: map.to_owned(),
        seed,
        time: w.time,
        ended: w.true_playing_players().is_empty(),
        seats: measured,
        cost,
    }
}

/// Milliseconds as seconds with one decimal: `95.4 seconds`.
fn seconds(ms: i64) -> String {
    format!("{}.{} seconds", ms / 1000, ms % 1000 / 100)
}

/// One line per fact of a game, each starting with the map, the seed and
/// the computer, so that a search finds every line of a case.
fn game_lines(g: &Played) -> Vec<String> {
    let at = |t: Option<i64>| t.map_or("never".to_owned(), |t| format!("at {}", seconds(t)));
    let mut out = vec![
        format!("{} seed {}: game time {}", g.map, g.seed, seconds(g.time)),
        format!(
            "{} seed {}: {}",
            g.map,
            g.seed,
            if g.ended {
                "ended, nobody left playing"
            } else {
                "stopped at the turn limit"
            }
        ),
    ];
    for (i, s) in g.seats.iter().enumerate() {
        let who = format!("{} seed {} computer {} {}", g.map, g.seed, i + 1, s.ai);
        out.push(format!(
            "{who}: first saw an enemy {}",
            at(s.first_enemy_seen)
        ));
        out.push(format!("{who}: first attack {}", at(s.first_attack)));
        out.push(format!(
            "{who}: stuck at the population limit, {}",
            seconds(s.stuck)
        ));
        out.push(format!(
            "{who}: stuck at the population limit, short of a farm's cost, {}",
            seconds(s.stuck_short)
        ));
        out.push(format!(
            "{who}: {}",
            match s.end {
                Some(End::Won(t)) => format!("won at {}", seconds(t)),
                Some(End::Defeated(t)) => format!("defeated at {}", seconds(t)),
                None => "still playing".to_owned(),
            }
        ));
    }
    out
}

/// The middle value, the lower of the two for an even count.
fn median(sorted: &[i64]) -> i64 {
    sorted[(sorted.len() - 1) / 2]
}

/// The summary of the computers of one type on one map, over the seeds.
fn summary_lines(games: &[&Played], ai: &str) -> Vec<String> {
    let seats: Vec<&Seat> = games
        .iter()
        .flat_map(|g| &g.seats)
        .filter(|s| s.ai == ai)
        .collect();
    let n = seats.len();
    let map = &games[0].map;
    let head = format!("{map}, {ai}");
    let mut out = vec![format!("{head}: {n} computers in {} games", games.len())];
    // A time that may not come: for how many it came, then the median
    // and the worst of those: the latest, or for a defeat the earliest.
    let mut event = |name: &str, earliest_is_worst: bool, f: fn(&Seat) -> Option<i64>| {
        let mut v: Vec<i64> = seats.iter().filter_map(|s| f(s)).collect();
        v.sort_unstable();
        out.push(format!("{head}: {name}, for {} of {n}", v.len()));
        if !v.is_empty() {
            out.push(format!("{head}: {name}, median {}", seconds(median(&v))));
            out.push(if earliest_is_worst {
                format!("{head}: {name}, earliest {}", seconds(v[0]))
            } else {
                format!("{head}: {name}, latest {}", seconds(v[v.len() - 1]))
            });
        }
    };
    event("first saw an enemy", false, |s| s.first_enemy_seen);
    event("first attack", false, |s| s.first_attack);
    event("defeated", true, |s| match s.end {
        Some(End::Defeated(t)) => Some(t),
        _ => None,
    });
    event("won", false, |s| match s.end {
        Some(End::Won(t)) => Some(t),
        _ => None,
    });
    let mut duration = |name: &str, f: fn(&Seat) -> i64| {
        let mut v: Vec<i64> = seats.iter().map(|s| f(s)).collect();
        v.sort_unstable();
        let over = v.iter().filter(|&&t| t > 60_000).count();
        out.push(format!("{head}: {name}, median {}", seconds(median(&v))));
        out.push(format!(
            "{head}: {name}, longest {}",
            seconds(v[v.len() - 1])
        ));
        out.push(format!("{head}: {name}, over a minute for {over} of {n}"));
    };
    duration("stuck at the population limit", |s| s.stuck);
    duration(
        "stuck at the population limit, short of a farm's cost",
        |s| s.stuck_short,
    );
    let mut times: Vec<i64> = games.iter().map(|g| g.time).collect();
    times.sort_unstable();
    let ended = games.iter().filter(|g| g.ended).count();
    out.push(format!(
        "{head}: game time, median {}",
        seconds(median(&times))
    ));
    out.push(format!(
        "{head}: game time, longest {}",
        seconds(times[times.len() - 1])
    ));
    out.push(format!(
        "{head}: games ended before the turn limit, {ended} of {}",
        games.len()
    ));
    out
}

/// `f` of each case, `jobs` cases at once, in the order of the cases.
fn in_parallel<C: Sync, R: Send>(cases: &[C], jobs: usize, f: impl Fn(&C) -> R + Sync) -> Vec<R> {
    let next = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..jobs.max(1))
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(case) = cases.get(i) else {
                            break done;
                        };
                        done.push((i, f(case)));
                    }
                })
            })
            .collect();
        let mut all: Vec<(usize, R)> = workers
            .into_iter()
            .flat_map(|w| w.join().unwrap())
            .collect();
        all.sort_by_key(|(i, _)| *i);
        all.into_iter().map(|(_, r)| r).collect()
    })
}

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_owned())
}

/// `1-20` or `1,5,9`, or both mixed.
fn parse_seeds(s: &str) -> Vec<i64> {
    s.split(',')
        .flat_map(|part| match part.split_once('-') {
            Some((a, b)) => (a.trim().parse().unwrap()..=b.trim().parse().unwrap()).collect(),
            None => vec![part.trim().parse::<i64>().unwrap()],
        })
        .collect()
}

#[test]
#[ignore = "minutes: run by hand in a release build"]
fn sweep_the_computer_players_over_maps_and_seeds() {
    let maps: Vec<String> = env_or("SWEEP_MAPS", "jl1,jl7,js13")
        .split(',')
        .map(|m| m.trim().to_owned())
        .collect();
    let seeds = parse_seeds(&env_or("SWEEP_SEEDS", "1-20"));
    let turns: usize = env_or("SWEEP_TURNS", "4000").parse().unwrap();
    let ais: Vec<String> = env_or("SWEEP_AI", "aggressive")
        .split(',')
        .map(|a| a.trim().to_owned())
        .collect();
    let jobs: usize = env_or("SWEEP_JOBS", "4").parse().unwrap();
    eprintln!(
        "turn limit: {turns} turns, {}",
        seconds(turns as i64 * VIRTUAL_TIME_INTERVAL)
    );
    let files = Files::load();
    let cases: Vec<(&str, i64)> = maps
        .iter()
        .flat_map(|m| seeds.iter().map(move |&s| (m.as_str(), s)))
        .collect();
    let played = in_parallel(&cases, jobs, |&(map, seed)| {
        play(&files, map, seed, &ais, turns)
    });
    for line in played.iter().flat_map(game_lines) {
        eprintln!("{line}");
    }
    eprintln!("summary over seeds {}", env_or("SWEEP_SEEDS", "1-20"));
    for map in &maps {
        let games: Vec<&Played> = played.iter().filter(|g| &g.map == map).collect();
        let present: BTreeSet<&str> = games
            .iter()
            .flat_map(|g| &g.seats)
            .map(|s| s.ai.as_str())
            .collect();
        for ai in present {
            for line in summary_lines(&games, ai) {
                eprintln!("{line}");
            }
        }
    }
}

/// Who won a game of the ladder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Winner {
    Higher,
    Lower,
    /// Nobody won before the turn limit.
    Draw,
}

/// One game of the ladder: a map and a seed, and which level sits first.
struct Rung<'a> {
    map: &'a str,
    seed: i64,
    lower: Skill,
    higher: Skill,
    higher_first: bool,
}

/// A game of the ladder, played.
struct Climbed {
    winner: Winner,
    played: Played,
}

/// Plays a game of the ladder. With `details`, the sweep's measures too.
fn climb(files: &Files, ai: &str, rung: &Rung, turns: usize, details: bool) -> Climbed {
    let lower = Who {
        ai: ai.to_owned(),
        skill: rung.lower,
    };
    let higher = Who {
        ai: ai.to_owned(),
        skill: rung.higher,
    };
    let who = if rung.higher_first {
        [higher, lower]
    } else {
        [lower, higher]
    };
    let w = world(files, rung.map, rung.seed);
    let played = play_seats(w, rung.map, rung.seed, &who, turns, details);
    let won = played
        .seats
        .iter()
        .find(|s| matches!(s.end, Some(End::Won(_))));
    let winner = match won {
        Some(s) if s.skill == rung.higher => Winner::Higher,
        Some(_) => Winner::Lower,
        None => Winner::Draw,
    };
    Climbed { winner, played }
}

/// Two levels, `2-3`, the lower first.
fn pair(text: &str) -> (Skill, Skill) {
    let level = |n: &str| n.trim().parse().ok().and_then(Skill::from_number);
    let pair = text
        .split_once('-')
        .and_then(|(a, b)| Some((level(a)?, level(b)?)));
    match pair {
        Some((lower, higher)) if lower < higher => (lower, higher),
        _ => panic!("LADDER_PAIRS: {text} is not two levels from 1 to 4, the lower first"),
    }
}

/// The lines of the ladder's report for one pair of levels, with how the
/// pair missed its bar, if it did. Up to Normal, the higher level must win
/// 70 to 85 percent of the games. Above Normal, it must win at least 65
/// percent of all games and at least 55 percent on each map. A draw is a
/// game it did not win.
fn pair_lines(maps: &[String], games: &[(&Rung, &Climbed)]) -> (Vec<String>, Vec<String>) {
    let (lower, higher) = (games[0].0.lower.number(), games[0].0.higher.number());
    let head = format!("level {lower} against level {higher}");
    let count = |games: &[&(&Rung, &Climbed)], w: Winner| {
        games.iter().filter(|(_, c)| c.winner == w).count()
    };
    let tally = |games: &[&(&Rung, &Climbed)]| {
        format!(
            "{} games, level {higher} won {}, level {lower} won {}, draws {}",
            games.len(),
            count(games, Winner::Higher),
            count(games, Winner::Lower),
            count(games, Winner::Draw),
        )
    };
    let above_normal = games[0].0.higher > Skill::NORMAL;
    let mut out = Vec::new();
    let mut missed = Vec::new();
    for map in maps {
        let on: Vec<_> = games.iter().filter(|(r, _)| r.map == map).collect();
        out.push(format!("{head} on {map}: {}", tally(&on)));
        let won = count(&on, Winner::Higher);
        if above_normal && won * 100 < 55 * on.len() {
            missed.push(format!(
                "on {map}, level {higher} won {won} of {} games against level {lower}, less than 55 percent",
                on.len()
            ));
        }
    }
    let all: Vec<_> = games.iter().collect();
    let n = all.len();
    let wins = count(&all, Winner::Higher);
    out.push(format!(
        "{head}: {}, win rate {} percent",
        tally(&all),
        wins * 100 / n
    ));
    let mut times: Vec<i64> = games.iter().map(|(_, c)| c.played.time).collect();
    times.sort_unstable();
    out.push(format!(
        "{head}: game time, median {}, longest {}",
        seconds(median(&times)),
        seconds(times[n - 1])
    ));
    let mut costs: Vec<u128> = games
        .iter()
        .map(|(_, c)| c.played.cost.as_millis())
        .collect();
    costs.sort_unstable();
    let turns: i64 = games
        .iter()
        .map(|(_, c)| c.played.time / VIRTUAL_TIME_INTERVAL)
        .sum();
    let total: u128 = costs.iter().sum();
    out.push(format!(
        "{head}: processor time per game, median {} ms, longest {} ms, per 1,000 turns {} ms",
        costs[(n - 1) / 2],
        costs[n - 1],
        total * 1000 / u128::try_from(turns.max(1)).unwrap()
    ));
    if above_normal {
        if wins * 100 < 65 * n {
            missed.push(format!(
                "level {higher} won {wins} of {n} games against level {lower}, less than 65 percent"
            ));
        }
    } else if !(70 * n..=85 * n).contains(&(wins * 100)) {
        missed.push(format!(
            "level {higher} won {wins} of {n} games against level {lower}, outside 70 to 85 percent"
        ));
    }
    (out, missed)
}

#[test]
#[ignore = "minutes: run by hand in a release build"]
fn ladder_each_level_beats_the_level_below() {
    let maps: Vec<String> = env_or("SWEEP_MAPS", "jl1,js1,can3,js13")
        .split(',')
        .map(|m| m.trim().to_owned())
        .collect();
    let seeds = parse_seeds(&env_or("SWEEP_SEEDS", "1-20"));
    let ai = env_or("SWEEP_AI", "aggressive");
    assert!(!ai.contains(','), "SWEEP_AI: the ladder takes one type");
    let turns: usize = env_or("LADDER_TURNS", "12000").parse().unwrap();
    let pairs: Vec<(Skill, Skill)> = env_or("LADDER_PAIRS", "1-2,2-3,3-4")
        .split(',')
        .map(pair)
        .collect();
    let jobs: usize = env_or("SWEEP_JOBS", "4").parse().unwrap();
    let details = env_or("LADDER_DETAILS", "") == "1";
    eprintln!(
        "ladder: {ai} computers, one against one, maps {}, seeds {}, each with both seat orders",
        maps.join(","),
        env_or("SWEEP_SEEDS", "1-20")
    );
    eprintln!(
        "ladder: turn limit {turns} turns, {}",
        seconds(turns as i64 * VIRTUAL_TIME_INTERVAL)
    );
    let files = Files::load();
    let mut rungs = Vec::new();
    for &(lower, higher) in &pairs {
        for map in &maps {
            for &seed in &seeds {
                for higher_first in [true, false] {
                    rungs.push(Rung {
                        map,
                        seed,
                        lower,
                        higher,
                        higher_first,
                    });
                }
            }
        }
    }
    let climbed = in_parallel(&rungs, jobs, |r| climb(&files, &ai, r, turns, details));
    for (r, c) in rungs.iter().zip(&climbed) {
        let first = if r.higher_first { r.higher } else { r.lower };
        let won = c.played.seats.iter().find_map(|s| match s.end {
            Some(End::Won(t)) => Some((s.skill, t)),
            _ => None,
        });
        let result = match won {
            Some((level, t)) => format!("level {} won at {}", level.number(), seconds(t)),
            None => "a draw at the turn limit".to_owned(),
        };
        eprintln!(
            "{} seed {} level {} against level {}, level {} first: {result}",
            r.map,
            r.seed,
            r.lower.number(),
            r.higher.number(),
            first.number()
        );
        if details {
            for line in game_lines(&c.played) {
                eprintln!("{line}");
            }
        }
    }
    let mut missed = Vec::new();
    for &(lower, higher) in &pairs {
        let games: Vec<(&Rung, &Climbed)> = rungs
            .iter()
            .zip(&climbed)
            .filter(|(r, _)| (r.lower, r.higher) == (lower, higher))
            .collect();
        let (lines, pair_missed) = pair_lines(&maps, &games);
        for line in lines {
            eprintln!("{line}");
        }
        missed.extend(pair_missed);
    }
    assert!(
        missed.is_empty(),
        "a pair of levels missed its bar: {}",
        missed.join("; ")
    );
}

#[test]
fn the_measures_of_a_game_on_jl1() {
    let g = play(&Files::load(), "jl1", 1, &["aggressive".to_owned()], 4000);
    let expected = [
        "jl1 seed 1: game time 607.5 seconds",
        "jl1 seed 1: ended, nobody left playing",
        "jl1 seed 1 computer 1 aggressive: first saw an enemy at 16.8 seconds",
        "jl1 seed 1 computer 1 aggressive: first attack at 555.9 seconds",
        "jl1 seed 1 computer 1 aggressive: stuck at the population limit, 19.2 seconds",
        "jl1 seed 1 computer 1 aggressive: stuck at the population limit, short of a farm's cost, 17.7 seconds",
        "jl1 seed 1 computer 1 aggressive: defeated at 607.5 seconds",
        "jl1 seed 1 computer 2 aggressive: first saw an enemy at 16.8 seconds",
        "jl1 seed 1 computer 2 aggressive: first attack at 427.8 seconds",
        "jl1 seed 1 computer 2 aggressive: stuck at the population limit, 50.7 seconds",
        "jl1 seed 1 computer 2 aggressive: stuck at the population limit, short of a farm's cost, 26.4 seconds",
        "jl1 seed 1 computer 2 aggressive: won at 607.5 seconds",
    ];
    assert_eq!(game_lines(&g), expected);
}
