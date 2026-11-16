//! A computer player's script, as `ai.txt` defines it: one step a line,
//! read once when the scripts load. A `goto` names the line it goes to,
//! a `get` the units it wants, by name; a computer that takes up the
//! script finds the types of those names in its rules ([`Plan`]).
//!
//! A line with a mistake is reported when the scripts load, quoting it,
//! and the computer goes on to the next line when it comes to it.

use std::sync::Arc;

use crate::nofloat::py_int;
use crate::rules::{FactionId, Rules, TypeId};

/// One line of a script.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Step<U> {
    /// `goto`: on to a line, by its index.
    Goto(i64),
    /// `label <name>`: a line to go to. It goes on to the next line.
    Label,
    /// `goto_random`: on to one of the lines, drawn at random.
    GotoRandom(i64, Vec<i64>),
    /// `attack`: attack constantly from now on, then the next line.
    Attack,
    Watchdog(i64),
    ConstantAttacks(i64),
    Research(i64),
    /// `get`: the units to have, each with how many; the next line once
    /// all are there.
    Get(Vec<(i64, U)>),
    /// A line with a mistake: on to the next line.
    Skip,
}

/// A script of `ai.txt`, with each line as written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Script {
    name: String,
    lines: Arc<[String]>,
    steps: Vec<Step<String>>,
}

/// A script as a computer plays it: each unit a type of its rules, as its
/// faction has it.
#[derive(Clone, Debug, Hash)]
pub(crate) struct Plan {
    lines: Arc<[String]>,
    steps: Arc<[Step<Option<TypeId>>]>,
}

/// `re.match("^[0-9]+$", s)`.
fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// `re.match("^[+-][0-9]+$", s)`.
fn is_signed_number(s: &str) -> bool {
    s.strip_prefix(['+', '-']).is_some_and(is_digits)
}

/// A number written in decimal digits, as Python reads it: one too large
/// for the game is the largest it has.
fn count(digits: &str) -> i64 {
    py_int(digits).unwrap_or(i64::MAX)
}

/// `int(s)`: a number, or `None` for a word that is not one. A number too
/// large for the game is the largest of its sign.
fn number(s: &str) -> Option<i64> {
    if let Ok(n) = py_int(s) {
        return Some(n);
    }
    match s.strip_prefix('-') {
        Some(d) if is_digits(d) => Some(i64::MIN),
        _ => is_digits(s.strip_prefix('+').unwrap_or(s)).then_some(i64::MAX),
    }
}

/// `n mod len` of a number written in decimal digits, with a sign or none,
/// however long it is.
fn modulo(signed: &str, len: i64) -> i64 {
    let (negative, digits) = match signed.strip_prefix('-') {
        Some(d) => (true, d),
        None => (false, signed.strip_prefix('+').unwrap_or(signed)),
    };
    let r = digits
        .bytes()
        .fold(0, |r, b| (r * 10 + i64::from(b - b'0')) % len);
    if negative { (len - r) % len } else { r }
}

impl Script {
    /// Reads the lines of the script `name`, with a warning for each line
    /// that has a mistake.
    pub(crate) fn parse(name: &str, lines: Vec<String>, warnings: &mut Vec<String>) -> Script {
        let steps = (0..lines.len())
            .map(|i| {
                let mut mistakes = Vec::new();
                let step = step(&lines, i, &mut mistakes).unwrap_or_else(|mistake| {
                    mistakes.push(mistake);
                    Step::Skip
                });
                let line = lines.get(i).map_or("", |l| l.trim());
                for mistake in mistakes {
                    warnings.push(format!("in ai.txt, {name}, \"{line}\": {mistake}"));
                }
                step
            })
            .collect();
        Script {
            name: name.to_owned(),
            lines: lines.into(),
            steps,
        }
    }

    #[cfg(test)]
    pub(crate) fn lines(&self) -> &[String] {
        &self.lines
    }

    /// The script as a player of `faction` plays it with `rules`, with a
    /// warning for each unit to get that the rules lack. Such a unit is
    /// left out of its line.
    pub(crate) fn plan(
        &self,
        rules: &Rules,
        faction: Option<FactionId>,
        warnings: &mut Vec<String>,
    ) -> Plan {
        // A class the rules have and that is not a type, such as a faction,
        // is a unit the computer never has.
        let unit = |name: &String| -> Option<Option<TypeId>> {
            if !rules.has_class(name) {
                return None;
            }
            let equivalent = match rules.type_id(name) {
                Some(t) => rules.ty(rules.equivalent(faction, t)).name.clone(),
                None => rules.equivalent_name(faction, name),
            };
            Some(rules.type_id(&equivalent))
        };
        let steps = self
            .steps
            .iter()
            .zip(self.lines.iter())
            .map(|(s, line)| match s {
                Step::Get(items) => Step::Get(
                    items
                        .iter()
                        .filter_map(|(n, name)| {
                            let t = unit(name);
                            if t.is_none() {
                                warnings.push(format!(
                                    "in ai.txt, {}, \"{}\": get: unknown unit: '{name}'",
                                    self.name,
                                    line.trim()
                                ));
                            }
                            t.map(|t| (*n, t))
                        })
                        .collect(),
                ),
                Step::Goto(n) => Step::Goto(*n),
                Step::Label => Step::Label,
                Step::GotoRandom(first, rest) => Step::GotoRandom(*first, rest.clone()),
                Step::Attack => Step::Attack,
                Step::Watchdog(v) => Step::Watchdog(*v),
                Step::ConstantAttacks(v) => Step::ConstantAttacks(*v),
                Step::Research(v) => Step::Research(*v),
                Step::Skip => Step::Skip,
            })
            .collect();
        Plan {
            lines: Arc::clone(&self.lines),
            steps,
        }
    }
}

impl Plan {
    pub(crate) fn lines(&self) -> &[String] {
        &self.lines
    }

    pub(crate) fn steps(&self) -> &[Step<Option<TypeId>>] {
        &self.steps
    }
}

/// The step of line `i`, or the mistake that leaves the line nothing to
/// do. A mistake after which the line still has a step is added to
/// `mistakes`.
fn step(lines: &[String], i: usize, mistakes: &mut Vec<String>) -> Result<Step<String>, String> {
    let words: Vec<&str> = lines.get(i).map_or("", |l| l).split_whitespace().collect();
    let Some((&first, args)) = words.split_first() else {
        return Err("an empty line".to_owned());
    };
    // Lines are counted in the script's length, which is under the limit
    // of lines of a text.
    let len = lines.len() as i64;
    let here = i as i64;
    // A label's line reads exactly `label <name>`.
    let label = |name: &str| {
        let line = format!("label {name}");
        lines.iter().position(|l| *l == line).map(|i| i as i64)
    };
    let int = |cmd: &str| {
        args.first()
            .and_then(|w| number(w))
            .ok_or_else(|| format!("{cmd} needs a number"))
    };
    Ok(match first {
        // Python's line numbers are not bounded; a line number is taken
        // modulo the script's length, as the next turn takes it.
        "goto" => {
            let Some(&dest) = args.first() else {
                return Err("goto needs a line or a label".to_owned());
            };
            if is_signed_number(dest) {
                Step::Goto((here + modulo(dest, len)) % len)
            } else if let Some(l) = label(dest) {
                Step::Goto(l)
            } else if is_digits(dest) {
                Step::Goto(modulo(dest, len))
            } else {
                return Err(format!("goto: wrong destination: {dest}"));
            }
        }
        "label" => {
            if args.is_empty() {
                return Err("label needs a name".to_owned());
            }
            Step::Label
        }
        "goto_random" => {
            let Some((first, rest)) = args.split_first() else {
                return Err("goto_random needs labels".to_owned());
            };
            // A label not found is a mistake, and the draw goes on to the
            // next line when it chooses it.
            let mut missing = Vec::new();
            let mut line = |name: &str| {
                label(name).unwrap_or_else(|| {
                    missing.push(name.to_owned());
                    here + 1
                })
            };
            let first = line(first);
            let rest = rest.iter().map(|d| line(d)).collect();
            for name in missing {
                mistakes.push(format!("goto_random: label not found: {name}"));
            }
            Step::GotoRandom(first, rest)
        }
        "attack" => Step::Attack,
        "watchdog" => Step::Watchdog(int(first)?),
        "constant_attacks" => Step::ConstantAttacks(int(first)?),
        "research" => Step::Research(int(first)?),
        "get" => {
            let mut items = Vec::new();
            let mut n = 1;
            for &w in args {
                if is_digits(w) {
                    n = count(w);
                } else {
                    items.push((n, w.to_owned()));
                    n = 1;
                }
            }
            Step::Get(items)
        }
        other => return Err(format!("unknown command: '{other}'")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AiScripts;

    fn read(f: &str) -> String {
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/soundrts-upstream");
        let bytes = std::fs::read(dir.join(f))
            .unwrap_or_else(|e| panic!("{f}: {e}: run scripts/fetch-upstream.ps1"));
        String::from_utf8_lossy(&bytes).replace("\r\n", "\n")
    }

    /// The warnings of loading the scripts of the layers, then of planning
    /// each for every faction of the rules.
    fn warnings(rules: &[&str], ai: &[&str]) -> Vec<String> {
        let rules = Rules::load(&rules.iter().map(|f| read(f)).collect::<Vec<_>>());
        let ai = AiScripts::load(&ai.iter().map(|f| read(f)).collect::<Vec<_>>());
        let mut warnings = ai.warnings.clone();
        let mut factions: Vec<Option<FactionId>> = vec![None];
        factions.extend((0..rules.factions().len()).map(|i| rules.faction_at(i)));
        for name in ai.names() {
            for &f in &factions {
                ai.get(name).unwrap().plan(&rules, f, &mut warnings);
            }
        }
        warnings
    }

    #[test]
    fn the_original_scripts_and_the_mods_have_no_mistake() {
        assert_eq!(
            warnings(&["res/rules.txt"], &["res/ai.txt"]),
            Vec::<String>::new()
        );
        for layer in ["mods/orc", "single/orc"] {
            let rules = ["res/rules.txt", &format!("{layer}/rules.txt")];
            let ai = ["res/ai.txt", &format!("{layer}/ai.txt")];
            assert_eq!(warnings(&rules, &ai), Vec::<String>::new(), "{layer}");
        }
        let jl4 = "soundrts/tests/res/multi/jl4";
        assert_eq!(
            warnings(
                &["res/rules.txt", &format!("{jl4}/rules.txt")],
                &["res/ai.txt", &format!("{jl4}/ai.txt")]
            ),
            [
                "in ai.txt, aggressive, \"teleportation 1\": unknown command: 'teleportation'",
                "in ai.txt, timers, \"retaliate 0\": unknown command: 'retaliate'",
            ]
        );
    }

    fn parse(lines: &[&str]) -> (Vec<Step<String>>, Vec<String>) {
        let mut warnings = Vec::new();
        let lines = lines.iter().map(|l| (*l).to_owned()).collect();
        (Script::parse("s", lines, &mut warnings).steps, warnings)
    }

    #[test]
    fn a_goto_names_the_line_it_goes_to() {
        let lines = [
            "label a", "goto -1", "goto a", "goto 7", "goto +2", "label 7", "goto 7",
        ];
        let (steps, warnings) = parse(&lines);
        assert_eq!(
            steps,
            [
                Step::Label,
                Step::Goto(0),
                Step::Goto(0),
                Step::Goto(5),
                Step::Goto(6),
                Step::Label,
                Step::Goto(5),
            ]
        );
        assert_eq!(warnings, Vec::<String>::new());
    }

    #[test]
    fn a_line_number_is_taken_modulo_the_length_of_the_script() {
        let (steps, _) = parse(&[
            "goto 8",
            "goto -99999999999999999999999",
            "goto +99999999999999999999998",
        ]);
        // 10^23 is 1 modulo 3: from line 1, -(10^23 - 1) is 0 lines on;
        // from line 2, 10^23 - 2 is 2 lines on.
        assert_eq!(steps, [Step::Goto(2), Step::Goto(1), Step::Goto(1)]);
    }

    #[test]
    fn a_number_too_large_is_the_largest() {
        let (steps, warnings) = parse(&[
            "watchdog 99999999999999999999",
            "research -99999999999999999999",
            "get 99999999999999999999 footman",
        ]);
        assert_eq!(
            steps,
            [
                Step::Watchdog(i64::MAX),
                Step::Research(i64::MIN),
                Step::Get(vec![(i64::MAX, "footman".to_owned())]),
            ]
        );
        assert_eq!(warnings, Vec::<String>::new());
    }

    #[test]
    fn a_line_with_a_mistake_is_reported_and_skipped() {
        let (steps, warnings) = parse(&[
            "goto",
            "goto nowhere",
            "label",
            "goto_random",
            "goto_random a nowhere",
            "label a",
            "watchdog soon",
            "research",
            "dance 3",
        ]);
        assert_eq!(
            steps,
            [
                Step::Skip,
                Step::Skip,
                Step::Skip,
                Step::Skip,
                Step::GotoRandom(5, vec![5]),
                Step::Label,
                Step::Skip,
                Step::Skip,
                Step::Skip,
            ]
        );
        assert_eq!(
            warnings,
            [
                "in ai.txt, s, \"goto\": goto needs a line or a label",
                "in ai.txt, s, \"goto nowhere\": goto: wrong destination: nowhere",
                "in ai.txt, s, \"label\": label needs a name",
                "in ai.txt, s, \"goto_random\": goto_random needs labels",
                "in ai.txt, s, \"goto_random a nowhere\": goto_random: label not found: nowhere",
                "in ai.txt, s, \"watchdog soon\": watchdog needs a number",
                "in ai.txt, s, \"research\": research needs a number",
                "in ai.txt, s, \"dance 3\": unknown command: 'dance'",
            ]
        );
    }

    #[test]
    fn a_unit_the_rules_lack_is_left_out_of_its_line_with_a_warning() {
        let rules = Rules::load(&[read("res/rules.txt")]);
        let mut warnings = Vec::new();
        let lines = vec!["get 5 dancer 2 footman".to_owned()];
        let plan = Script::parse("s", lines, &mut warnings).plan(&rules, None, &mut warnings);
        let footman = rules.type_id("footman");
        assert_eq!(plan.steps(), [Step::Get(vec![(2, footman)])]);
        assert_eq!(
            warnings,
            ["in ai.txt, s, \"get 5 dancer 2 footman\": get: unknown unit: 'dancer'"]
        );
    }
}
