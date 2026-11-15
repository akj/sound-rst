//! A ratchet on the places where the simulation can stop part-way or
//! panic. For each source file of the crate, outside its tests, it counts:
//!
//! - the stops raised, by kind (see `src/abort.rs`): a map's trigger that
//!   cannot be read, a broken invariant, and a stop of the computer player
//!   not yet given a kind;
//! - `.unwrap()`, `.expect(` and indexing with `[]`, each of which panics
//!   when it is wrong, and a panic ends a single-player game.
//!
//! The test fails when a count differs from the number committed below.
//! A count may only fall: lowering it is part of the change that earns
//! it, and the message gives the table to paste.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The counts of one file, in the order of [`COLUMNS`].
type Counts = [usize; 6];

const COLUMNS: [&str; 6] = [
    "trigger",
    "invariant",
    "unclassified",
    "unwrap",
    "expect",
    "index",
];

/// The committed counts. A file not listed has none.
#[rustfmt::skip]
const COMMITTED: &[(&str, Counts)] = &[
    // file                   trig inv uncl unwrap expect index
    ("ai.rs",                 [  0,  0, 23,  0, 14, 24]),
    ("ai/order_sites.rs",     [  0,  0,  0, 26,  0, 17]),
    ("map.rs",                [  0,  0,  0,  0,  7,  2]),
    ("nofloat.rs",            [  0,  0,  0,  0,  0,  3]),
    ("orders.rs",             [  0, 21,  0,  0, 14,  3]),
    ("pathfinding.rs",        [  0,  1,  0,  0,  0,  3]),
    ("player.rs",             [  0,  0,  0,  0,  0,  1]),
    ("playerlogic.rs",        [  0,  3,  0,  0,  2,  4]),
    ("pyrandom.rs",           [  0,  0,  0,  0,  0, 25]),
    ("questions.rs",          [  0,  0,  0,  0,  0,  2]),
    ("rules.rs",              [  0,  0,  0,  0,  0,  2]),
    ("seat_view.rs",          [  0,  0,  0,  0,  1,  0]),
    ("square.rs",             [  0,  0,  0,  0,  0,  1]),
    ("squarelogic.rs",        [  0,  0,  0,  0,  1,  1]),
    ("test_games.rs",         [  0,  0,  0,  3,  1,  2]),
    ("triggers.rs",           [  9,  0,  0,  0,  0,  4]),
    ("unit.rs",               [  0, 17,  0,  0,  3,  3]),
    ("world.rs",              [  0,  0,  0,  0,  2,  8]),
];

fn src_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// The source files of the crate that are not tests, relative to `src`,
/// with `/` between the parts.
fn source_files() -> Vec<(String, PathBuf)> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let path = e.unwrap().path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n != "tests") {
                    walk(&path, out);
                }
            } else if path.extension().is_some_and(|x| x == "rs") {
                out.push(path);
            }
        }
    }
    let mut paths = Vec::new();
    walk(&src_dir(), &mut paths);
    let mut files: Vec<(String, PathBuf)> = paths
        .into_iter()
        .map(|p| {
            let rel = p.strip_prefix(src_dir()).unwrap();
            let name = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            (name, p)
        })
        .filter(|(name, _)| {
            !matches!(
                name.rsplit('/').next(),
                Some("tests.rs" | "upstream_tests.rs")
            )
        })
        .collect();
    files.sort();
    files
}

/// The code of a file without its comments, the contents of its string
/// and character literals, and the items marked `#[cfg(test)]`.
fn code(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < b.len() {
        if text[i..].starts_with("//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if text[i..].starts_with("/*") {
            i = text[i..].find("*/").map_or(b.len(), |j| i + j + 2);
        } else if text[i..].starts_with("r#\"") {
            i = text[i + 3..].find("\"#").map_or(b.len(), |j| i + 3 + j + 2);
            out.push_str("\"\"");
        } else if b[i] == b'"' {
            i += 1;
            while i < b.len() && b[i] != b'"' {
                i += if b[i] == b'\\' { 2 } else { 1 };
            }
            i += 1;
            out.push_str("\"\"");
        } else if b[i] == b'\'' && is_char_literal(&text[i..]) {
            i += 1;
            while i < b.len() && b[i] != b'\'' {
                i += if b[i] == b'\\' { 2 } else { 1 };
            }
            i += 1;
            out.push_str("' '");
        } else if text[i..].starts_with("#[cfg(test)]") {
            i = end_of_item(b, i + "#[cfg(test)]".len());
        } else {
            let c = text[i..].chars().next().unwrap();
            out.push(c);
            i += c.len_utf8();
        }
    }
    out
}

/// Whether a `'` starts a character literal rather than a lifetime.
fn is_char_literal(s: &str) -> bool {
    let mut it = s.chars().skip(1);
    matches!(
        (it.next(), it.next()),
        (Some('\\'), _) | (Some(_), Some('\''))
    )
}

/// The end of the item that starts at `i`: after its `;`, or after the
/// brace that closes its body.
fn end_of_item(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && b[i] != b'{' && b[i] != b';' {
        i += 1;
    }
    if i < b.len() && b[i] == b';' {
        return i + 1;
    }
    let mut depth = 0;
    while i < b.len() {
        match b[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    i
}

fn count(code: &str, needle: &str) -> usize {
    code.matches(needle).count()
}

/// Indexing: a `[` right after a name, a `)` or a `]`.
fn count_indexing(code: &str) -> usize {
    let b = code.as_bytes();
    (1..b.len())
        .filter(|&i| {
            b[i] == b'['
                && (b[i - 1].is_ascii_alphanumeric() || matches!(b[i - 1], b'_' | b')' | b']'))
        })
        .count()
}

fn counts_of(name: &str, text: &str) -> Counts {
    let code = code(text);
    // The kinds are defined, and written out, in `abort.rs`.
    let stops = |needle: &str| {
        if name == "abort.rs" {
            0
        } else {
            count(&code, needle)
        }
    };
    [
        stops("TriggerError::"),
        stops("Invariant::"),
        stops("Abort::new("),
        count(&code, ".unwrap()"),
        count(&code, ".expect("),
        count_indexing(&code),
    ]
}

fn table(counts: &BTreeMap<String, Counts>) -> String {
    let mut s = String::new();
    for (name, c) in counts {
        if c.iter().any(|&n| n != 0) {
            let cells: Vec<String> = c.iter().map(|n| format!("{n:>3}")).collect();
            let _ = writeln!(
                s,
                "    ({:<24} [{}]),",
                format!("{name:?},"),
                cells.join(",")
            );
        }
    }
    s
}

#[test]
fn the_places_that_stop_or_panic_only_become_fewer() {
    let actual: BTreeMap<String, Counts> = source_files()
        .into_iter()
        .map(|(name, path)| {
            let text = std::fs::read_to_string(path).unwrap();
            let c = counts_of(&name, &text);
            (name, c)
        })
        .collect();
    let committed: BTreeMap<String, Counts> = COMMITTED
        .iter()
        .map(|(n, c)| ((*n).to_owned(), *c))
        .collect();
    let mut problems = Vec::new();
    let names: std::collections::BTreeSet<&String> =
        actual.keys().chain(committed.keys()).collect();
    for name in names {
        let a = actual.get(name).copied().unwrap_or_default();
        let c = committed.get(name).copied().unwrap_or_default();
        for (k, column) in COLUMNS.iter().enumerate() {
            if a[k] > c[k] {
                problems.push(format!("{name}: {column} rose from {} to {}", c[k], a[k]));
            } else if a[k] < c[k] {
                problems.push(format!(
                    "{name}: {column} fell from {} to {}: lower the number",
                    c[k], a[k]
                ));
            }
        }
    }
    let total = |m: &BTreeMap<String, Counts>| {
        let mut t = [0; 6];
        for c in m.values() {
            for k in 0..6 {
                t[k] += c[k];
            }
        }
        t
    };
    println!("totals {COLUMNS:?}: {:?}", total(&actual));
    assert!(
        problems.is_empty(),
        "{}\n\nThe counts now, columns {COLUMNS:?}:\n{}",
        problems.join("\n"),
        table(&actual)
    );
}

#[test]
fn the_counting_reads_code_only() {
    let text = r#"
fn f(v: &[i64]) -> i64 {
    // v[0].unwrap() in a comment
    let s = "a[1].expect(";
    let c = '[';
    v[0] + g()[1] + v.first().copied().unwrap()
}

#[cfg(test)]
mod tests {
    fn t() { x[0].unwrap(); }
}
"#;
    assert_eq!(counts_of("x.rs", text), [0, 0, 0, 1, 0, 2]);
}
