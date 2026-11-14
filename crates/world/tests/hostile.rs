//! Hostile text. Every rules file, computer player script and map of the
//! original is changed at random many times over, and each changed text
//! must load or be refused without a panic, within [`LOAD_BOUND`]. A map
//! that loads is also populated and played for a few turns, and the whole
//! case must end within [`CASE_BOUND`].
//!
//! The changes come from a fixed seed, so a failure reproduces: the message
//! gives the seed, the file, the case and the changed text. Environment
//! variables change the run: `HOSTILE_CASES` (changed texts per file),
//! `HOSTILE_SEED`, `HOSTILE_ONLY=<file>:<case>` to run one case, and
//! `HOSTILE_VERBOSE=1` to print when each case starts and ends (and, with
//! `HOSTILE_ONLY`, its text first), which is the only report a stack
//! overflow leaves. Each case runs on a stack of 1 MiB, the size of a
//! Windows program's main thread.

mod common;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use soundrts_world::{AiScripts, Controller, Rules, World, multiplayer_default_triggers};

/// The longest a load may take: `Rules::load`, `AiScripts::load` or
/// `World::load_and_build_map`. The original's own texts load in a few
/// milliseconds in a debug build.
const LOAD_BOUND: Duration = Duration::from_secs(1);

/// The longest a whole case may take: loading, then placing the players
/// and playing [`TURNS`] turns. A turn's cost grows with the units a map
/// gives, which `MAX_START_UNITS` bounds; a battle of thousands of units
/// takes about a second a turn. This bound catches a game that hangs.
const CASE_BOUND: Duration = Duration::from_secs(30);

/// Turns played on a map that loads.
const TURNS: usize = 5;

/// Changed texts per rules or scripts file, unless `HOSTILE_CASES` says
/// otherwise. A case takes about 3 ms.
const TEXT_CASES: u64 = 2000;

/// Changed texts per map, unless `HOSTILE_CASES` says otherwise. A case
/// takes about 8 ms, as it plays on the map it changes.
const MAP_CASES: u64 = 150;

const DEFAULT_SEED: u64 = 0x50_756e_6453;

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// SplitMix64: small, and the same on every machine.
struct Rng(u64);

impl Rng {
    fn new(seed: u64, file: u64, case: u64) -> Rng {
        let mut r = Rng(seed ^ file.wrapping_mul(0x9e37_79b9_7f4a_7c15));
        r.0 ^= r.next().wrapping_add(case);
        r
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A number from 0 to `n - 1`, or 0 when `n` is 0.
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }

    fn pick<'a>(&mut self, items: &'a [&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
}

/// A word chosen to break a parser.
fn hostile_word(r: &mut Rng) -> String {
    match r.below(12) {
        0 => "9".repeat(1 + r.below(400)),
        1 => format!("-{}", "9".repeat(1 + r.below(30))),
        2 => r.pick(&["0", "-0", "-1", "00", "+0"]).to_owned(),
        3 => r
            .pick(&[
                "1e308", "-1e308", "inf", "nan", "1e-400", ".", "1.", "_1", "1__0",
            ])
            .to_owned(),
        4 => r
            .pick(&["é", "日本", "\u{202e}", "1é", "a1é", "é1", "\u{0}", "ǅ9"])
            .to_owned(),
        5 => "(".repeat(1 + r.below(3000)),
        6 => ")".repeat(1 + r.below(50)),
        7 => r.pick(&["(", ")", "( (", ") )", "()"]).to_owned(),
        8 => format!("a{}", r.below(200)),
        9 => format!("{}{}", r.pick(&["z", "aa", "a", "b"]), r.below(100_000)),
        10 => r
            .pick(&["\\", ";", ",", "-", "--", "#end_choice"])
            .to_owned(),
        _ => r
            .pick(&["9223372036854775807", "-9223372036854775808", "4294967296"])
            .to_owned(),
    }
}

/// Changes the text one to three times.
fn mutate(text: &str, r: &mut Rng) -> String {
    let mut lines: Vec<String> = text.split('\n').map(str::to_owned).collect();
    for _ in 0..1 + r.below(3) {
        let n = lines.len();
        let i = r.below(n);
        let j = r.below(n);
        match r.below(15) {
            0 if n > 0 => {
                lines.remove(i);
            }
            1 if n > 0 => {
                let l = lines[i].clone();
                lines.insert(j, l);
            }
            2 if n > 0 => lines.swap(i, j),
            3..=5 if n > 0 => {
                let mut words: Vec<String> =
                    lines[i].split_whitespace().map(str::to_owned).collect();
                let k = r.below(words.len());
                let m = r.below(words.len());
                match r.below(5) {
                    0 if !words.is_empty() => {
                        words.remove(k);
                    }
                    1 if !words.is_empty() => {
                        let w = words[k].clone();
                        words.insert(m, w);
                    }
                    2 if !words.is_empty() => words.swap(k, m),
                    3 if !words.is_empty() => words[k] = hostile_word(r),
                    _ => words.insert(k, hostile_word(r)),
                }
                lines[i] = words.join(" ");
            }
            6 if n > 0 => {
                // A keyword of the file, repeated on one line or as many
                // lines.
                let first = lines[i].split_whitespace().next().unwrap_or("def");
                let count = 1 + r.below(300);
                if r.below(2) == 0 {
                    lines.insert(j, vec![first; count].join(" "));
                } else {
                    let l = lines[i].clone();
                    for _ in 0..count {
                        lines.insert(j, l.clone());
                    }
                }
            }
            7 => {
                let depth = 1 + r.below(3000);
                lines.insert(
                    j,
                    format!(
                        "trigger players {}timer 0{} ( victory )",
                        "( ".repeat(depth),
                        " )".repeat(depth)
                    ),
                );
            }
            8 if n > 0 => {
                let l = lines[i].clone();
                lines[i] = format!("{l} \\");
            }
            9 => {
                let owners = (0..1 + r.below(500))
                    .map(|_| r.pick(&["players", "computers", "all", "player1", "computer2"]))
                    .collect::<Vec<_>>()
                    .join(" ");
                lines.insert(j, format!("trigger ( {owners} ) ( timer 0 ) ( victory )"));
            }
            10 if n > 0 => {
                let w = lines[i].split_whitespace().next().unwrap_or("x").to_owned();
                lines.insert(j, format!("{w} {}", hostile_word(r)));
            }
            11 => {
                let w = r.pick(&[
                    "nb_columns",
                    "nb_lines",
                    "square_width",
                    "nb_players_max",
                    "nb_meadows_by_square",
                    "nb_of_resource_types",
                    "starting_squares",
                    "is_a",
                    "def",
                    "class",
                    "clear",
                    "goldmine",
                    "player",
                    "computer_only",
                    "timer_coefficient",
                    "global_food_limit",
                ]);
                lines.insert(j, format!("{w} {}", hostile_word(r)));
            }
            12 => {
                let mut s = lines.join("\n");
                let mut cut = r.below(s.len() + 1);
                while !s.is_char_boundary(cut) {
                    cut -= 1;
                }
                s.truncate(cut);
                lines = s.split('\n').map(str::to_owned).collect();
            }
            13 => {
                // Parents chosen among the classes of the file, which may
                // make cycles.
                let names: Vec<String> = lines
                    .iter()
                    .filter_map(|l| l.strip_prefix("def "))
                    .map(|n| n.trim().to_owned())
                    .collect();
                let parents: Vec<String> = (0..1 + r.below(12))
                    .filter_map(|_| names.get(r.below(names.len())).cloned())
                    .collect();
                lines.insert(j, format!("is_a {}", parents.join(" ")));
            }
            _ if n > 1 => {
                let l = lines.remove(i);
                let k = r.below(lines.len());
                lines[k] = format!("{} {l}", lines[k]);
            }
            _ => {}
        }
    }
    lines.join("\n")
}

/// Runs one case and reports a panic, a load over [`LOAD_BOUND`] or a case
/// over [`CASE_BOUND`] with what reproduces it. `case` returns how long its
/// load took.
fn run_case(id: &str, text: &str, case: impl FnOnce() -> Duration) -> Result<Duration, String> {
    let verbose = std::env::var_os("HOSTILE_VERBOSE").is_some();
    if verbose {
        eprintln!("start {id}");
        if std::env::var_os("HOSTILE_ONLY").is_some() {
            eprintln!("--- changed text ---\n{text}\n--- end ---");
        }
    }
    let start = Instant::now();
    let outcome = catch_unwind(AssertUnwindSafe(case));
    let took = start.elapsed();
    if verbose {
        eprintln!("done {id}");
    }
    let problem = match outcome {
        Err(e) => {
            let m = e
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| e.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                .unwrap_or_default();
            format!("panicked: {m}")
        }
        Ok(load) if load > LOAD_BOUND => format!("loaded in {load:?}, more than {LOAD_BOUND:?}"),
        Ok(_) if took > CASE_BOUND => format!("took {took:?}, more than {CASE_BOUND:?}"),
        Ok(load) => return Ok(load),
    };
    Err(format!(
        "{id} {problem}\n--- changed text ---\n{text}\n--- end ---"
    ))
}

/// Runs `load`, and returns what it gives and how long it took.
fn timed<T>(load: impl FnOnce() -> T) -> (T, Duration) {
    let start = Instant::now();
    let t = load();
    (t, start.elapsed())
}

/// Places a human and, when the map allows, computers, then plays.
fn play(w: &mut World, computers: &[&str]) {
    let mut controllers = vec![Controller::human("me", true)];
    for c in computers {
        controllers.push(Controller::computer(c));
    }
    controllers.truncate(w.map.players_starts.len().max(1));
    let Ok(players) = w.populate_map(&controllers, true, true) else {
        return;
    };
    for _ in 0..TURNS {
        w.update();
        for &p in &players {
            w.drain_events(p);
        }
        w.drain_warnings();
    }
}

/// A text of the original, and the texts it is a layer on.
struct Family {
    base: Vec<String>,
    file: String,
}

fn families(first: &str, layers: &[&str]) -> Vec<Family> {
    let mut out = vec![Family {
        base: Vec::new(),
        file: first.into(),
    }];
    for f in layers {
        out.push(Family {
            base: vec![common::read(first)],
            file: (*f).into(),
        });
    }
    out
}

/// A map, and the index of the rules and scripts it is played with.
struct MapFile {
    path: String,
    layers: usize,
}

fn map_files() -> Vec<MapFile> {
    let mut out = Vec::new();
    for (dir, layers) in [("res/multi", 0), ("single/campaign", 0), ("single/orc", 1)] {
        let mut names: Vec<String> = std::fs::read_dir(common::upstream().join(dir))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| {
                n.ends_with(".txt") && !matches!(n.as_str(), "mods.txt" | "ai.txt" | "rules.txt")
            })
            .collect();
        names.sort();
        for n in names {
            let path = format!("{dir}/{n}");
            if !common::read(&path).starts_with("cut_scene_chapter") {
                out.push(MapFile { path, layers });
            }
        }
    }
    out
}

/// The map every changed rules or scripts text is played on.
const PLAYED_MAP: &str = "res/multi/jl1.txt";

/// Runs changed texts of each of `files` (`cases` each, unless
/// `HOSTILE_CASES` says otherwise) on as many threads as the machine has,
/// and fails with every problem found. `check` loads a changed text of the
/// file of that index, and returns how long the load took.
fn run_all(files: &[String], cases: u64, check: impl Fn(usize, &str) -> Duration + Sync) {
    let seed = env_u64("HOSTILE_SEED", DEFAULT_SEED);
    let cases = env_u64("HOSTILE_CASES", cases);
    let only = std::env::var("HOSTILE_ONLY").ok();
    let originals: Vec<String> = files.iter().map(|f| common::read(f)).collect();
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let jobs: Vec<(usize, u64)> = (0..files.len())
        .flat_map(|f| (0..cases).map(move |c| (f, c)))
        .filter(|&(f, c)| {
            let name = format!("{}:{c}", files[f]);
            only.as_deref().is_none_or(|o| o == name)
        })
        .collect();
    let next = AtomicUsize::new(0);
    let problems = Mutex::new(Vec::new());
    let slowest = Mutex::new((Duration::ZERO, String::new()));
    std::thread::scope(|s| {
        for _ in 0..threads {
            // The stack of a Windows program's main thread.
            std::thread::Builder::new()
                .stack_size(1 << 20)
                .spawn_scoped(s, || {
                    while let Some(&(f, c)) = jobs.get(next.fetch_add(1, Ordering::Relaxed)) {
                        let mut r = Rng::new(seed, f as u64, c);
                        let text = mutate(&originals[f], &mut r);
                        let id = format!("HOSTILE_SEED={seed:#x} HOSTILE_ONLY={}:{c}", files[f]);
                        match run_case(&id, &text, || check(f, &text)) {
                            Ok(load) => {
                                let mut slowest = slowest.lock().unwrap();
                                if load > slowest.0 {
                                    *slowest = (load, id);
                                }
                            }
                            Err(e) => problems.lock().unwrap().push(e),
                        }
                    }
                })
                .unwrap();
        }
    });
    let problems = problems.into_inner().unwrap();
    let (load, id) = slowest.into_inner().unwrap();
    eprintln!("{} cases; the slowest load took {load:?}: {id}", jobs.len());
    assert!(
        problems.is_empty(),
        "{} of {} cases failed:\n{}",
        problems.len(),
        jobs.len(),
        problems.join("\n\n")
    );
}

#[test]
fn changed_rules_load_and_play_or_are_refused() {
    let families = families(
        "res/rules.txt",
        &[
            "mods/flyingtownhall/rules.txt",
            "mods/orc/rules.txt",
            "mods/teleportation/rules.txt",
            "single/orc/rules.txt",
        ],
    );
    let files: Vec<String> = families.iter().map(|f| f.file.clone()).collect();
    let map = common::read(PLAYED_MAP);
    let ai = common::ai();
    run_all(&files, TEXT_CASES, |f, text| {
        let mut layers = families[f].base.clone();
        layers.push(text.to_owned());
        let (rules, load) = timed(|| Rules::load(&layers));
        let mut w = World::new(rules, ai.clone(), multiplayer_default_triggers(), 0);
        if w.load_and_build_map(&map).is_ok() {
            play(&mut w, &["easy"]);
        }
        load
    });
}

#[test]
fn changed_scripts_load_and_play_or_are_refused() {
    let families = families("res/ai.txt", &["mods/orc/ai.txt", "single/orc/ai.txt"]);
    let files: Vec<String> = families.iter().map(|f| f.file.clone()).collect();
    let map = common::read(PLAYED_MAP);
    let rules = common::rules();
    run_all(&files, TEXT_CASES, |f, text| {
        let mut layers = families[f].base.clone();
        layers.push(text.to_owned());
        let (ai, load) = timed(|| AiScripts::load(&layers));
        let mut w = World::new(rules.clone(), ai, multiplayer_default_triggers(), 0);
        if w.load_and_build_map(&map).is_ok() {
            play(&mut w, &["easy", "aggressive"]);
        }
        load
    });
}

#[test]
fn changed_maps_load_and_play_or_are_refused() {
    let maps = map_files();
    assert!(maps.len() >= 90, "only {} maps found", maps.len());
    let files: Vec<String> = maps.iter().map(|m| m.path.clone()).collect();
    let read = |paths: &[&str]| paths.iter().map(|p| common::read(p)).collect::<Vec<_>>();
    let layers = [
        (
            Rules::load(&read(&["res/rules.txt"])),
            AiScripts::load(&read(&["res/ai.txt"])),
        ),
        (
            Rules::load(&read(&["res/rules.txt", "single/orc/rules.txt"])),
            AiScripts::load(&read(&["res/ai.txt", "single/orc/ai.txt"])),
        ),
    ];
    run_all(&files, MAP_CASES, |f, text| {
        let (rules, ai) = &layers[maps[f].layers];
        let mut w = World::new(rules.clone(), ai.clone(), multiplayer_default_triggers(), 0);
        let (built, load) = timed(|| w.load_and_build_map(text));
        if built.is_ok() {
            play(&mut w, &["easy"]);
        }
        load
    });
}

/// Plays 30 turns of a human and an `easy` computer on [`PLAYED_MAP`] with
/// the original's rules and scripts and a layer of each on top, on the stack
/// of a Windows program's main thread, and returns the warnings.
fn warnings_of_a_game(rules: &str, ai: &str) -> Vec<String> {
    let (rules, ai) = (rules.to_owned(), ai.to_owned());
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(move || {
            let mut w = World::new(
                Rules::load(&[common::read("res/rules.txt"), rules]),
                AiScripts::load(&[common::read("res/ai.txt"), ai]),
                multiplayer_default_triggers(),
                0,
            );
            w.load_and_build_map(&common::read(PLAYED_MAP)).unwrap();
            w.populate_map(
                &[Controller::human("me", true), Controller::computer("easy")],
                true,
                true,
            )
            .unwrap();
            let mut warnings = Vec::new();
            for _ in 0..30 {
                w.update();
                warnings.extend(w.drain_warnings());
            }
            assert_eq!(w.time, 30 * 300);
            warnings
        })
        .unwrap()
        .join()
        .unwrap()
}

/// A farm that costs food sends the computer to build a farm for the food,
/// which costs food, and so on: this overflowed the stack. Now the
/// computer's update ends with a warning each turn it plays, as the
/// original's recursion error ends it, and the game goes on.
#[test]
fn a_farm_that_costs_food_ends_the_computers_update_with_a_warning() {
    assert_eq!(
        warnings_of_a_game("def farm\nfood_cost 100\n", ""),
        ["maximum recursion depth exceeded"; 3]
    );
}

/// The same recursion within the script's plan is caught there, as in the
/// original: the plan goes on with its next line.
#[test]
fn a_plan_line_that_recurses_is_skipped_with_a_warning() {
    assert_eq!(
        warnings_of_a_game(
            "def farm\nfood_cost 100\ndef peasant\nfood_cost 0\n",
            "def easy\nget 2 farm\n"
        ),
        ["recursion error with easy; current ai.txt line is: get 2 farm"; 3]
    );
}
