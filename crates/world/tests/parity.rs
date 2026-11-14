//! Parity with the original game: the maps built and the games played by
//! the Python code (dumped by `tests/parity/dump_reference.py`) must be
//! reproduced exactly. See `tests/parity/README.md`.

mod common;

use std::collections::BTreeMap;
use std::path::PathBuf;

use soundrts_world::entity::{ActionKind, Entity, Place};
use soundrts_world::{AiScripts, Controller, PlayerId, Rules, World, multiplayer_default_triggers};

fn parity_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/parity")
}

struct Case {
    name: String,
    kind: String,
    map: String,
    rules: Vec<String>,
    ai: Vec<String>,
    seed: i64,
    triggers: String,
    players: String,
    scenario: String,
    ticks: usize,
}

fn cases() -> Vec<Case> {
    let text = std::fs::read_to_string(parity_dir().join("cases.txt")).unwrap();
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let f: BTreeMap<&str, &str> = line
            .split_whitespace()
            .filter_map(|x| x.split_once('='))
            .collect();
        let list = |k: &str, d: &str| {
            f.get(k)
                .copied()
                .unwrap_or(d)
                .split(',')
                .map(str::to_owned)
                .collect()
        };
        out.push(Case {
            name: f["name"].to_owned(),
            kind: f["kind"].to_owned(),
            map: f["map"].to_owned(),
            rules: list("rules", "res/rules.txt"),
            ai: list("ai", "res/ai.txt"),
            seed: f.get("seed").map_or(0, |s| s.parse().unwrap()),
            triggers: f.get("triggers").copied().unwrap_or("none").to_owned(),
            players: f.get("players").copied().unwrap_or("").to_owned(),
            scenario: f.get("scenario").copied().unwrap_or("").to_owned(),
            ticks: f.get("ticks").map_or(0, |s| s.parse().unwrap()),
        });
    }
    out
}

fn make_world(case: &Case) -> World {
    let joined = |paths: &[String]| {
        paths
            .iter()
            .map(|p| common::read(p))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let rules = Rules::load(&[joined(&case.rules)]);
    let ai = AiScripts::load(&[joined(&case.ai)]);
    let triggers = if case.triggers == "multiplayer" {
        multiplayer_default_triggers()
    } else {
        Vec::new()
    };
    let mut w = World::new(rules, ai, triggers, case.seed);
    // The scenarios add units with `cmd add_units`.
    w.allow_cheatmode = true;
    w.load_and_build_map(&common::read(&case.map))
        .unwrap_or_else(|e| panic!("{}: {e}", case.map));
    w
}

fn dump_static(w: &World) -> Vec<String> {
    let m = &w.map;
    let join = |v: &[i64]| v.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
    let mut out = vec![format!(
        "map title={} objective={} players={}-{} starts={} computers={} food={}",
        join(&m.title),
        join(&m.objective),
        m.nb_players_min,
        m.nb_players_max,
        m.players_starts.len(),
        m.computers_starts.len(),
        m.global_food_limit
    )];
    for s in &w.squares {
        out.push(format!(
            "square {} id={} x={} y={} high={} water={} ground={} air={} terrain={} speed={},{} cover={},{}",
            s.name,
            s.id,
            s.x,
            s.y,
            u8::from(s.high_ground),
            u8::from(s.is_water),
            u8::from(s.is_ground),
            u8::from(s.is_air),
            s.type_name,
            s.terrain_speed.0,
            s.terrain_speed.1,
            s.terrain_cover.0,
            s.terrain_cover.1
        ));
        for &e in &s.exits {
            let x = w.entity(e).unwrap();
            let data = x.exit.as_ref().unwrap();
            out.push(format!(
                "  exit {} {} {} {} {} other={} portal={}",
                x.id,
                x.type_name,
                x.x,
                x.y,
                x.o,
                data.other_side.map_or("None".to_owned(), |o| o.to_string()),
                u8::from(data.is_a_portal)
            ));
        }
        for &o in &s.objects {
            let x = w.entity(o).unwrap();
            out.push(format!(
                "  object {} {} {} {} qty={}",
                x.id, x.type_name, x.x, x.y, x.stats.qty
            ));
        }
    }
    let mut saved: Vec<String> = w.save_map().lines().map(str::to_owned).collect();
    saved.sort();
    out.extend(saved.into_iter().map(|l| format!("save {l}")));
    out
}

fn obj_line(w: &World, o: &Entity) -> String {
    let creature = o.is_creature();
    let hp = if creature {
        o.hp.to_string()
    } else {
        "-".into()
    };
    let mana = if creature {
        o.mana.to_string()
    } else {
        "-".into()
    };
    let player = o
        .player
        .map_or("-".to_owned(), |p| w.player(p).obj_id.to_string());
    let orders = o
        .orders
        .iter()
        .map(|x| x.keyword.as_str())
        .collect::<Vec<_>>()
        .join(",");
    let action = match (&o.action, creature) {
        (Some(a), true) => match a.kind {
            ActionKind::Plain => "Action",
            ActionKind::Move => "MoveAction",
            ActionKind::MoveXy => "MoveXYAction",
            ActionKind::Attack => "AttackAction",
        },
        _ => "-",
    };
    format!(
        "{} {} {} {} {} hp={hp} mana={mana} qty={} player={player} orders={orders} action={action}",
        o.id, o.type_name, o.x, o.y, o.o, o.stats.qty
    )
}

fn dump_tick(w: &World) -> Vec<String> {
    let mut lines = vec![format!(
        "time {} random {:016x}",
        w.time,
        w.random.state_fingerprint()
    )];
    for &p in &w.players {
        let pl = w.player(p);
        lines.push(format!(
            "player {} res={} food={} used={} units={} upgrades={} perception={} memory={} observed={} victory={} defeat={}",
            pl.obj_id,
            pl.resources.iter().map(i64::to_string).collect::<Vec<_>>().join(","),
            pl.food,
            pl.used_food,
            pl.units.len(),
            pl.upgrades.join(","),
            pl.perception.len(),
            pl.memory.len(),
            pl.observed_squares.len(),
            u8::from(pl.has_victory),
            u8::from(pl.has_been_defeated)
        ));
    }
    for s in &w.squares {
        for &o in &s.objects {
            let e = w.entity(o).unwrap();
            lines.push(obj_line(w, e));
            if let Some(inside) = &e.inside {
                for &i in inside {
                    lines.push(format!("  in {}", obj_line(w, w.entity(i).unwrap())));
                }
            }
        }
    }
    lines
}

/// The placeholders of the scenarios, resolved as the Python script does.
fn resolve(w: &World, players: &[PlayerId], pi: usize, cmd: &str) -> Option<String> {
    let me = players[pi];
    let other = players
        .get(1 - pi.min(1))
        .copied()
        .filter(|_| players.len() > 1);
    let units = |p: PlayerId, t: &str| -> Vec<soundrts_world::EntityId> {
        w.player(p)
            .units
            .iter()
            .copied()
            .filter(|&u| &*w.entity(u).unwrap().type_name == t)
            .collect()
    };
    let square_id = |place: Option<Place>| match place? {
        Place::Square(s) => Some(w.square(s).id.to_string()),
        Place::Inside(_) => None,
    };
    let mut out = String::new();
    let mut rest = cmd;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let end = rest[start..].find('}')? + start;
        let parts: Vec<&str> = rest[start + 1..end].split(':').collect();
        let idx = |i: usize| parts.get(i).and_then(|s| s.parse::<usize>().ok());
        let value = match parts[0] {
            "u" => units(me, parts[1]).get(idx(2)?)?.to_string(),
            "eu" => units(other?, parts[1]).get(idx(2)?)?.to_string(),
            "here" => square_id(w.entity(*units(me, parts[1]).get(idx(2)?)?)?.place)?,
            "hn" => match w.entity(*units(me, parts[1]).get(idx(2)?)?)?.place? {
                Place::Square(s) => w.square(s).name.clone(),
                Place::Inside(_) => return None,
            },
            "there" => square_id(w.entity(*units(other?, parts[1]).get(idx(2)?)?)?.place)?,
            "at" => {
                let s = w.square_by_name(parts[1])?;
                let objs: Vec<_> = w
                    .square(s)
                    .objects
                    .iter()
                    .copied()
                    .filter(|&o| &*w.entity(o).unwrap().type_name == parts[2])
                    .collect();
                objs.get(idx(3)?)?.to_string()
            }
            "mead" | "loc" => {
                let (t, i) = match parts[0] {
                    "mead" => ("meadow", idx(1)?),
                    _ => (*parts.get(1)?, idx(2)?),
                };
                let first = *w.player(me).units.first()?;
                let Some(Place::Square(s)) = w.entity(first)?.place else {
                    return None;
                };
                let objs: Vec<_> = w
                    .square(s)
                    .objects
                    .iter()
                    .copied()
                    .filter(|&o| &*w.entity(o).unwrap().type_name == t)
                    .collect();
                objs.get(i)?.to_string()
            }
            "ids" => {
                let ids: Vec<i64> = units(me, parts[1]).iter().map(|u| i64::from(u.0)).collect();
                soundrts_world::group::encode(&ids)
            }
            "sq" => w.square(w.square_by_name(parts[1])?).id.to_string(),
            other => panic!("unknown placeholder {other}"),
        };
        out.push_str(&value);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}

/// `login` or `ai_<type>`, then optionally `:<alliance>`.
fn controllers(spec: &str) -> Vec<Controller> {
    spec.split(',')
        .map(|s| {
            let (s, alliance) = match s.split_once(':') {
                Some((s, a)) => (s, Some(a)),
                None => (s, None),
            };
            let mut c = match s.strip_prefix("ai_") {
                Some(ai) => Controller::computer(ai),
                None => Controller::human(s, false),
            };
            if let Some(a) = alliance {
                c.alliance = Some(a.to_owned());
            }
            c
        })
        .collect()
}

fn compare(name: &str, expected: &[String], actual: &[String]) {
    for (i, (e, a)) in expected.iter().zip(actual).enumerate() {
        if e != a {
            let tick = expected[..=i]
                .iter()
                .rev()
                .find(|l| l.starts_with("tick "))
                .cloned();
            let from = i.saturating_sub(3);
            panic!(
                "{name}: line {} differs ({tick:?})\nexpected: {e}\nactual:   {a}\ncontext (expected):\n{}\ncontext (actual):\n{}",
                i + 1,
                expected[from..(i + 3).min(expected.len())].join("\n"),
                actual[from..(i + 3).min(actual.len())].join("\n"),
            );
        }
    }
    assert_eq!(
        expected.len(),
        actual.len(),
        "{name}: not the same number of lines"
    );
}

fn expected(name: &str) -> Vec<String> {
    let path = parity_dir().join("expected").join(format!("{name}.txt"));
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| {
            panic!(
                "{}: {e} (run tests/parity/dump_reference.py)",
                path.display()
            )
        })
        .lines()
        .map(str::to_owned)
        .collect()
}

fn fnv64(text: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Runs a scripted game: one summary line per update (turn, hash of the
/// full dump, first line of the dump), and the full dump.
fn run_trace(case: &Case) -> (Vec<String>, Vec<String>) {
    let mut w = make_world(case);
    let players = w
        .populate_map(&controllers(&case.players), true, true)
        .unwrap();
    let scenario = std::fs::read_to_string(parity_dir().join(&case.scenario)).unwrap();
    let mut by_tick: BTreeMap<usize, Vec<(usize, String)>> = BTreeMap::new();
    for line in scenario.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut it = line.splitn(3, ' ');
        let tick = it.next().unwrap().parse().unwrap();
        let pi = it.next().unwrap().parse().unwrap();
        by_tick
            .entry(tick)
            .or_default()
            .push((pi, it.next().unwrap().to_owned()));
    }
    let mut summary = Vec::new();
    let mut full = Vec::new();
    for t in 0..case.ticks {
        for (pi, cmd) in by_tick.get(&t).cloned().unwrap_or_default() {
            if let Some(r) = resolve(&w, &players, pi, &cmd)
                && w.players.contains(&players[pi])
            {
                w.queue_command(players[pi], &r);
            }
        }
        w.update();
        let lines = dump_tick(&w);
        summary.push(format!(
            "tick {t} {} {}",
            fnv64(&lines.join("\n")),
            lines[0]
        ));
        full.push(format!("tick {t}"));
        full.extend(lines);
    }
    (summary, full)
}

/// Where a failed trace writes its full dump.
fn actual_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/parity-actual")
}

/// The hash of each built map's dump, by case.
fn static_hashes() -> BTreeMap<String, String> {
    let path = parity_dir().join("expected/static_hashes.txt");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| {
            panic!(
                "{}: {e} (run tests/parity/dump_reference.py)",
                path.display()
            )
        })
        .lines()
        .filter_map(|l| l.split_once(' '))
        .map(|(n, h)| (n.to_owned(), h.to_owned()))
        .collect()
}

fn check_static(case: &Case, hashes: &BTreeMap<String, String>) {
    let dump = dump_static(&make_world(case));
    let actual = fnv64(&dump.join("\n"));
    let expected = hashes
        .get(&case.name)
        .unwrap_or_else(|| panic!("{}: no hash in static_hashes.txt", case.name));
    if actual != *expected {
        let dir = actual_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{}.txt", case.name));
        std::fs::write(&path, dump.join("\n") + "\n").unwrap();
        panic!(
            "{}: the built map differs from the original's (hash {actual}, expected {expected}).\n\
             The dump is in {}. To get the original's, run from vendor/soundrts-upstream:\n    \
             ..\\.venv-ref\\Scripts\\python.exe ..\\..\\crates\\world\\tests\\parity\\dump_reference.py \
             --only {} --full ..\\..\\target\\parity-reference\n\
             then compare it with target/parity-reference/{}.txt",
            case.name,
            path.display(),
            case.name,
            case.name
        );
    }
}

fn check(prefix: &str) {
    let mut n = 0;
    let hashes = static_hashes();
    for case in cases().into_iter().filter(|c| c.name.starts_with(prefix)) {
        match case.kind.as_str() {
            "static" => check_static(&case, &hashes),
            _ => {
                let (summary, full) = run_trace(&case);
                let expected = expected(&case.name);
                if expected != summary {
                    let dir = actual_dir();
                    std::fs::create_dir_all(&dir).unwrap();
                    let path = dir.join(format!("{}.txt", case.name));
                    std::fs::write(&path, full.join("\n") + "\n").unwrap();
                    eprintln!(
                        "full trace written to {}; get the original's with \
                         dump_reference.py --only {} --full ..\\..\\target\\parity-reference \
                         (see tests/parity/README.md)",
                        path.display(),
                        case.name
                    );
                }
                compare(&case.name, &expected, &summary);
            }
        }
        n += 1;
    }
    assert!(n > 0, "no case starting with {prefix}");
}

/// The comparison is not vacuous: another seed gives another game.
#[test]
fn a_different_seed_gives_a_different_trace() {
    let mut case = cases().into_iter().find(|c| c.name == "trace_jl1").unwrap();
    let expected = expected(&case.name);
    case.seed += 1;
    let (actual, _) = run_trace(&case);
    assert_eq!(expected.len(), case.ticks, "one line per update");
    let first_difference = expected.iter().zip(&actual).position(|(e, a)| e != a);
    assert!(
        first_difference.is_some(),
        "a different seed gave the same trace"
    );
}

#[test]
fn built_maps_match_the_original() {
    check("static_");
}

#[test]
fn game_on_jl1_matches_the_original_tick_by_tick() {
    check("trace_jl1");
}

#[test]
fn fight_on_jl7_matches_the_original_tick_by_tick() {
    check("trace_jl7");
}

#[test]
fn campaign_mission_3_matches_the_original_tick_by_tick() {
    check("trace_campaign3");
}

#[test]
fn portals_on_the_cyclic_map_match_the_original_tick_by_tick() {
    check("trace_cyclic");
}

#[test]
fn boats_walls_spells_and_computer_orders_on_jl5_match_the_original_tick_by_tick() {
    check("trace_jl5");
}

#[test]
fn campaign_mission_2_triggers_match_the_original_tick_by_tick() {
    check("trace_campaign2");
}

/// `PARITY_ONLY=<prefix>` narrows the AI games to the cases starting with
/// it, to debug one of them.
#[test]
fn games_with_computer_players_match_the_original_tick_by_tick() {
    let only = std::env::var("PARITY_ONLY").unwrap_or_default();
    check(&format!(
        "trace_ai_{}",
        only.trim_start_matches("trace_ai_")
    ));
}
