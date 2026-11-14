//! Hostile interface files. Every style, key bindings and translation file
//! of the original's `res/ui*` folders, its mods and its folder map
//! (`soundrts/tests/res/multi/jl4`) is changed at random many times over,
//! and each changed text must be read without a panic within
//! [`CASE_BOUND`]: as an installed layer, then as the layer of a map that
//! came from a server.
//!
//! The changes come from a fixed seed, so a failure reproduces: the message
//! gives the file, the case and the start of the changed text. The cases
//! run on a stack of 1 MiB, the size of a Windows program's main thread.

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use soundrts::bindings::{BindingError, Bindings, Chord};
use soundrts::log::Log;
use soundrts::parameters::Parameters;
use soundrts::sound_cache::{SoundCache, update_text_table};
use soundrts::style::Style;
use soundrts::testkit::upstream_root;
use soundrts_world::msg::MsgPart;

/// The longest one case may take: reading the changed text as an installed
/// layer and as a downloaded one, then looking up what the game looks up.
/// The original's files take a few milliseconds in a debug build.
const CASE_BOUND: Duration = Duration::from_secs(2);

/// Changed texts per file. With the 39 files, 3,120 cases.
const CASES_PER_FILE: u64 = 80;

const SEED: u64 = 0x50_756e_6453;

/// SplitMix64: small, and the same on every machine.
struct Rng(u64);

impl Rng {
    fn new(file: u64, case: u64) -> Rng {
        let mut r = Rng(SEED ^ file.wrapping_mul(0x9e37_79b9_7f4a_7c15));
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

/// A word chosen to break a reader of these files.
fn hostile_word(r: &mut Rng) -> String {
    match r.below(11) {
        0 => "9".repeat(1 + r.below(400)),
        1 => r
            .pick(&[
                "-1",
                "0",
                "1e308",
                "nan",
                "inf",
                "-0",
                "99999999999999999999",
            ])
            .to_owned(),
        2 => r
            .pick(&[
                "é",
                "日本",
                "\u{202e}",
                "1é",
                "a_level_é",
                "_level_",
                "x_level_",
                "\u{0}",
            ])
            .to_owned(),
        3 => r
            .pick(&[
                "if_me", "else", "ambient", "loop", "repeat", "is_a", "def", "clear",
            ])
            .to_owned(),
        4 => r
            .pick(&[
                "#define", ":", "::", "CTRL", "ALT", "SHIFT", "WIN", "F13", "KP0",
            ])
            .to_owned(),
        5 => r
            .pick(&["\\", ";", "\r", "\u{feff}", "\t", "  "])
            .to_owned(),
        6 => format!("Z{}", r.below(8)),
        7 => r
            .pick(&["footman", "peasant", "parameters", "unit", "a", "b"])
            .to_owned(),
        8 => "x ".repeat(1 + r.below(3000)),
        9 => format!(
            "{}_level_{}",
            r.pick(&["attack_hit", "title", "noise"]),
            r.below(12)
        ),
        _ => format!("{}", r.below(100_000)),
    }
}

/// Changes the text one to three times.
fn mutate(text: &str, r: &mut Rng) -> String {
    let mut lines: Vec<String> = text.split('\n').map(str::to_owned).collect();
    for _ in 0..1 + r.below(3) {
        let n = lines.len();
        let i = r.below(n);
        let j = r.below(n);
        match r.below(14) {
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
                // A line, or its first word, repeated many times.
                let count = 1 + r.below(2000);
                if r.below(2) == 0 {
                    let first = lines[i].split_whitespace().next().unwrap_or("def");
                    lines.insert(j, vec![first; count].join(" "));
                } else {
                    let l = lines[i].clone();
                    for _ in 0..count {
                        lines.insert(j, l.clone());
                    }
                }
            }
            7 => {
                // Definitions that double: each name twice the last.
                let depth = 1 + r.below(64);
                lines.insert(j, "#define Z0 x x".to_owned());
                for d in 1..depth {
                    lines.insert(j + d, format!("#define Z{d} Z{} Z{}", d - 1, d - 1));
                }
                lines.insert(j + depth, format!("a: z Z{}", depth - 1));
            }
            8 => {
                // A chain or a web of parents.
                let count = 1 + r.below(300);
                let parents = 1 + r.below(12);
                for d in 0..count {
                    let is_a: Vec<String> = (0..parents)
                        .map(|_| format!("o{}", r.below(count + 1)))
                        .collect();
                    lines.insert(j, format!("def o{d}\nis_a {}\nt{d} 1", is_a.join(" ")));
                }
            }
            9 if n > 0 => {
                let l = lines[i].clone();
                lines[i] = format!("{l} \\");
            }
            10 if n > 0 => {
                // Cut a line at any byte, even inside a character.
                let bytes = lines[i].as_bytes();
                let cut = r.below(bytes.len() + 1);
                lines[i] = String::from_utf8_lossy(&bytes[..cut]).into_owned();
            }
            11 => {
                let w = r.pick(&["is_a", "noise", "noise_when_moving", "title", "alert"]);
                lines.insert(j, format!("{w} {}", hostile_word(r)));
            }
            12 => lines.insert(j, format!("{} {}", r.below(100_000), hostile_word(r))),
            _ => lines.insert(j, hostile_word(r)),
        }
    }
    lines.join(if r.below(10) == 0 { "\r\n" } else { "\n" })
}

/// What kind of interface file a file is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Style,
    Bindings,
    Translation,
}

/// The files changed, in a fixed order.
fn files() -> Vec<(PathBuf, Kind)> {
    let root = upstream_root();
    let mut found = Vec::new();
    let mut dirs = vec![
        root.join("res"),
        root.join("mods"),
        root.join("soundrts/tests/res/multi/jl4"),
    ];
    while let Some(dir) = dirs.pop() {
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = e.path();
            if path.is_dir() {
                dirs.push(path);
                continue;
            }
            let in_ui = path
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|d| d.to_string_lossy().starts_with("ui"));
            let kind = match path.file_name().and_then(|n| n.to_str()) {
                Some("style.txt") => Kind::Style,
                Some("bindings.txt") => Kind::Bindings,
                Some("tts.txt") => Kind::Translation,
                _ => continue,
            };
            if in_ui {
                found.push((path, kind));
            }
        }
    }
    found.sort();
    found
}

/// Reads any name as a command.
fn any(name: &str, args: Vec<String>) -> Result<(String, Vec<String>), BindingError> {
    Ok((name.to_owned(), args))
}

/// Reads `text` as the game would, as an installed layer over `base`
/// (the game's own text of that kind), then as a downloaded map's.
fn read(kind: Kind, base: &str, text: &str) {
    let log = Log::silent();
    match kind {
        Kind::Style => {
            let mut style = Style::new(log.clone());
            style.load(&[base, text]);
            look_up(&style);
            let mut style = Style::new(log);
            style.load(&[base]);
            style.add_downloaded(&[text]);
            look_up(&style);
        }
        Kind::Bindings => {
            let mut b = Bindings::new();
            b.load(&format!("{base}\n{text}"), &any, &log);
            for chord in ["a", "SHIFT z", "CTRL SHIFT F2", "ALT F4"] {
                if let Ok(c) = Chord::parse(chord) {
                    let _ = b.get(&c);
                }
            }
            assert!(b.len() < 100_000);
        }
        Kind::Translation => {
            let mut table = HashMap::new();
            update_text_table(&mut table, text, &log);
            let params = Rc::new(Parameters::default());
            let mut cache = SoundCache::new(
                &[base.to_owned(), text.to_owned()],
                &[],
                params.clone(),
                log.clone(),
            );
            translate(&mut cache);
            let mut cache = SoundCache::new(&[base.to_owned()], &[], params, log);
            cache.add_downloaded(&[text.to_owned()], &[]);
            translate(&mut cache);
        }
    }
}

fn look_up(style: &Style) {
    let objects: Vec<String> = style.objects().map(str::to_owned).collect();
    for o in objects.iter().take(300) {
        for attr in [
            "title",
            "noise",
            "is_a",
            "attack_hit_level_2",
            "x_level_",
            "é_level_1",
        ] {
            let _ = style.get_or_warn(o, attr, false);
        }
        let _ = style.noise(o, "noise_when_moving");
        let _ = style.words(o, "alert");
    }
}

fn translate(cache: &mut SoundCache) {
    for key in [
        "0",
        "5",
        "9000",
        "1001",
        "999999999999",
        "18446744073709551616",
        "é",
    ] {
        let _ = cache.translate(&MsgPart::from(key));
    }
}

#[test]
fn hostile_interface_files_are_read_without_a_panic() {
    let files = files();
    assert_eq!(files.len(), 39, "{files:#?}");
    let root = upstream_root();
    let base = |name: &str| std::fs::read_to_string(root.join("res/ui").join(name)).unwrap();
    let bases = [base("style.txt"), base("bindings.txt"), base("tts.txt")];
    let worker = std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(move || {
            let mut failures = Vec::new();
            let mut slowest = Duration::ZERO;
            for (f, (path, kind)) in files.iter().enumerate() {
                let original = std::fs::read_to_string(path).unwrap();
                let base = match kind {
                    Kind::Style => &bases[0],
                    Kind::Bindings => &bases[1],
                    Kind::Translation => &bases[2],
                };
                for case in 0..CASES_PER_FILE {
                    let mut r = Rng::new(f as u64, case);
                    let text = mutate(&original, &mut r);
                    let start = Instant::now();
                    let result = catch_unwind(AssertUnwindSafe(|| read(*kind, base, &text)));
                    let took = start.elapsed();
                    slowest = slowest.max(took);
                    if result.is_err() || took > CASE_BOUND {
                        let head: String = text.chars().take(300).collect();
                        failures.push(format!(
                            "{} case {case}: {} after {took:?}\n{head}",
                            path.display(),
                            if result.is_err() { "panic" } else { "too slow" }
                        ));
                    }
                }
            }
            (failures, slowest)
        })
        .unwrap();
    let (failures, slowest) = worker.join().unwrap();
    eprintln!("slowest case: {slowest:?}");
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
