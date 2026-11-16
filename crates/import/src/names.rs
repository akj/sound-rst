//! The name of each number of the original's files: `original_names.txt`.
//!
//! The original names everything it says or plays with a number: a line of
//! `ui/tts.txt` (a phrase) or a recording under `ui/` (a sound). The table
//! gives each number a name, for each package: the game (`res`), a map
//! with files of its own, a mod, a campaign. `scripts/name-numbers.ps1`
//! keeps the table, and a person corrects the names in it.
//!
//! A name with a slash is a sound: its path under `sounds/`. A name without
//! one is a phrase. A package lists only the numbers the game lacks: a
//! number that the game has keeps the game's name there.
//!
//! The importer is the only reader of the table. The game knows no number.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::arithmetic_side_effects
)]

use std::collections::HashMap;
use std::sync::OnceLock;

use soundrts_world::msg::is_name;

/// The text of the table.
pub const TEXT: &str = include_str!("original_names.txt");

/// The package of the game itself.
pub const GAME: &str = "res";

/// One number of a package and its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub number: u32,
    pub name: &'static str,
    /// No style, map, help file or code refers to the number.
    pub unused: bool,
}

impl Row {
    /// Whether the name is a sound's: a path, not a phrase's bare word.
    pub fn is_sound(&self) -> bool {
        self.name.contains('/')
    }
}

/// The table: the rows of each package, in the order of the file.
#[derive(Debug, Default)]
pub struct Names {
    packages: Vec<(&'static str, Vec<Row>)>,
    game_numbers: HashMap<&'static str, u32>,
    game_names: HashMap<u32, &'static str>,
}

impl Names {
    /// The packages, with their rows.
    pub fn packages(&self) -> impl Iterator<Item = (&'static str, &[Row])> {
        self.packages.iter().map(|(p, rows)| (*p, rows.as_slice()))
    }

    /// The rows of a package. Empty for a package the table lacks.
    pub fn rows(&self, package: &str) -> &[Row] {
        self.packages
            .iter()
            .find(|(p, _)| *p == package)
            .map_or(&[], |(_, rows)| rows.as_slice())
    }

    /// The number the game gives a name.
    pub fn game_number(&self, name: &str) -> Option<u32> {
        self.game_numbers.get(name).copied()
    }

    /// The name of a number of the game.
    pub fn game_name(&self, number: u32) -> Option<&'static str> {
        self.game_names.get(&number).copied()
    }
}

/// Reads the table. The error names the line.
fn parse(text: &'static str) -> Result<Names, String> {
    let mut names = Names::default();
    for (i, line) in text.lines().enumerate() {
        let at = |what: &str| format!("original_names.txt line {}: {what}", i.saturating_add(1));
        let mut words = line.split(';').next().unwrap_or("").split_whitespace();
        let Some(first) = words.next() else {
            continue;
        };
        let second = words.next().ok_or_else(|| at("one word only"))?;
        if first == "package" {
            if names.packages.iter().any(|(p, _)| *p == second) {
                return Err(at("this package has rows above"));
            }
            names.packages.push((second, Vec::new()));
            continue;
        }
        let number = first.parse().map_err(|_| at("not a number"))?;
        if !is_name(second) {
            return Err(at("not a name"));
        }
        let unused = match words.next() {
            None => false,
            Some("unused") => true,
            Some(_) => return Err(at("a word that is not 'unused' after the name")),
        };
        let Some((_, rows)) = names.packages.last_mut() else {
            return Err(at("a row before the first package"));
        };
        if rows
            .iter()
            .any(|r: &Row| r.number == number || r.name == second)
        {
            return Err(at("this package has this number or this name above"));
        }
        rows.push(Row {
            number,
            name: second,
            unused,
        });
    }
    names.game_numbers = names
        .rows(GAME)
        .iter()
        .map(|r| (r.name, r.number))
        .collect();
    names.game_names = names
        .rows(GAME)
        .iter()
        .map(|r| (r.number, r.name))
        .collect();
    for (package, rows) in names.packages() {
        if let Some(r) = rows
            .iter()
            .find(|r| package != GAME && names.game_numbers.contains_key(r.name))
        {
            return Err(format!(
                "original_names.txt: '{}' of {package} is a name of the game",
                r.name
            ));
        }
    }
    Ok(names)
}

/// The table. A test checks that it reads without an error.
pub fn names() -> &'static Names {
    static NAMES: OnceLock<Names> = OnceLock::new();
    NAMES.get_or_init(|| parse(TEXT).unwrap_or_default())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn the_table_reads_without_an_error() {
        let names = parse(TEXT).unwrap();
        assert_eq!(names.game_number("cannot_build_there"), Some(4204));
        assert_eq!(names.game_name(4204), Some("cannot_build_there"));
        assert_eq!(names.game_number("footstep/grass_1"), Some(1053));
        assert_eq!(names.game_number("pig_farm"), None);
        let orc = names.rows("mods/orc");
        assert!(orc.iter().any(|r| r.number == 7000 && r.name == "pig_farm"));
    }

    #[test]
    fn a_bad_line_is_refused_with_its_number() {
        let bad = |text: &'static str| parse(text).unwrap_err();
        assert!(bad("1 a\n").contains("line 1: a row before the first package"));
        assert!(bad("package res\n1 A\n").contains("line 2: not a name"));
        assert!(bad("package res\n1 a/\n").contains("line 2: not a name"));
        assert!(bad("package res\nx a\n").contains("line 2: not a number"));
        assert!(bad("package res\n1 a\n1 b\n").contains("line 3"));
        assert!(bad("package res\n1 a\n2 a\n").contains("line 3"));
        assert!(bad("package res\n1 a used\n").contains("line 2"));
        assert!(bad("package res\n1 a\npackage mods/m\n7 a\n").contains("'a' of mods/m"));
        let ok = parse("; c\npackage res\n1 a/b unused ; c\n").unwrap();
        assert_eq!(
            ok.rows(GAME),
            [Row {
                number: 1,
                name: "a/b",
                unused: true
            }]
        );
    }
}
