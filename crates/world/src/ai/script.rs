//! A computer player's script, as `ai.txt` defines it: one step a line,
//! read once when the scripts load. A `goto` names the line it goes to,
//! a `get` the units it wants, by name; a computer that takes up the
//! script finds the types of those names in its rules ([`Plan`]).

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
    GotoRandom(Vec<i64>),
    /// `attack`: attack constantly from now on, then the next line.
    Attack,
    Watchdog(i64),
    ConstantAttacks(i64),
    Research(i64),
    /// `get`: the units to have, each with how many; the next line once
    /// all are there.
    Get(Vec<(i64, U)>),
    /// A line read as the original reads it, each time it is played.
    Raw,
}

/// A script of `ai.txt`, with each line as written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Script {
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
pub(super) fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// `re.match("^[+-][0-9]+$", s)`.
pub(super) fn is_signed_number(s: &str) -> bool {
    s.strip_prefix(['+', '-']).is_some_and(is_digits)
}

impl Script {
    /// Reads the lines of a script.
    pub(crate) fn parse(lines: Vec<String>) -> Script {
        let steps = (0..lines.len())
            .map(|i| step(&lines, i).unwrap_or(Step::Raw))
            .collect();
        Script {
            lines: lines.into(),
            steps,
        }
    }

    #[cfg(test)]
    pub(crate) fn lines(&self) -> &[String] {
        &self.lines
    }

    /// The script as a player of `faction` plays it with `rules`.
    pub(crate) fn plan(&self, rules: &Rules, faction: Option<FactionId>) -> Plan {
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
            .map(|s| match s {
                Step::Get(items) => items
                    .iter()
                    .map(|(n, name)| unit(name).map(|t| (*n, t)))
                    .collect::<Option<Vec<_>>>()
                    .map_or(Step::Raw, Step::Get),
                Step::Goto(n) => Step::Goto(*n),
                Step::Label => Step::Label,
                Step::GotoRandom(d) => Step::GotoRandom(d.clone()),
                Step::Attack => Step::Attack,
                Step::Watchdog(v) => Step::Watchdog(*v),
                Step::ConstantAttacks(v) => Step::ConstantAttacks(*v),
                Step::Research(v) => Step::Research(*v),
                Step::Raw => Step::Raw,
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

/// The step of line `i`, or `None` if only the original's reading
/// expresses it.
fn step(lines: &[String], i: usize) -> Option<Step<String>> {
    let words: Vec<&str> = lines.get(i)?.split_whitespace().collect();
    let (&first, args) = words.split_first()?;
    let label = |name: &str| {
        let line = format!("label {name}");
        lines.iter().position(|l| *l == line).map(|i| i as i64)
    };
    let int = |w: Option<&&str>| w.and_then(|w| py_int(w).ok());
    Some(match first {
        "goto" => {
            let dest = *args.first()?;
            if is_signed_number(dest) {
                Step::Goto((i as i64).checked_add(py_int(dest).ok()?)?)
            } else if let Some(l) = label(dest) {
                Step::Goto(l)
            } else if is_digits(dest) {
                Step::Goto(py_int(dest).ok()?)
            } else {
                return None;
            }
        }
        "label" => {
            args.first()?;
            Step::Label
        }
        "goto_random" => {
            if args.is_empty() {
                return None;
            }
            Step::GotoRandom(args.iter().map(|d| label(d)).collect::<Option<_>>()?)
        }
        "attack" => Step::Attack,
        "watchdog" => Step::Watchdog(int(args.first())?),
        "constant_attacks" => Step::ConstantAttacks(int(args.first())?),
        "research" => Step::Research(int(args.first())?),
        "get" => {
            let mut items = Vec::new();
            let mut n = 1;
            for &w in args {
                if is_digits(w) {
                    n = py_int(w).ok()?;
                } else {
                    items.push((n, w.to_owned()));
                    n = 1;
                }
            }
            Step::Get(items)
        }
        _ => return None,
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

    /// The lines of each script of the layers that only the original's
    /// reading expresses, as `script: line`, for every faction of the rules.
    fn raw_lines(rules: &[&str], ai: &[&str]) -> Vec<String> {
        let rules = Rules::load(&rules.iter().map(|f| read(f)).collect::<Vec<_>>());
        let ai = AiScripts::load(&ai.iter().map(|f| read(f)).collect::<Vec<_>>());
        assert_eq!(ai.warnings, Vec::<String>::new());
        let mut factions: Vec<Option<FactionId>> = vec![None];
        factions.extend((0..rules.factions().len()).map(|i| rules.faction_at(i)));
        let mut raw = Vec::new();
        for name in ai.names() {
            let script = ai.get(name).unwrap();
            for &f in &factions {
                let plan = script.plan(&rules, f);
                for (line, step) in plan.lines().iter().zip(plan.steps()) {
                    let l = format!("{name}: {}", line.trim());
                    if *step == Step::Raw && !raw.contains(&l) {
                        raw.push(l);
                    }
                }
            }
        }
        raw
    }

    #[test]
    fn the_original_scripts_and_the_mods_read_into_typed_steps() {
        assert_eq!(
            raw_lines(&["res/rules.txt"], &["res/ai.txt"]),
            Vec::<String>::new()
        );
        for layer in ["mods/orc", "single/orc"] {
            let rules = ["res/rules.txt", &format!("{layer}/rules.txt")];
            let ai = ["res/ai.txt", &format!("{layer}/ai.txt")];
            assert_eq!(raw_lines(&rules, &ai), Vec::<String>::new(), "{layer}");
        }
        let jl4 = "soundrts/tests/res/multi/jl4";
        assert_eq!(
            raw_lines(
                &["res/rules.txt", &format!("{jl4}/rules.txt")],
                &["res/ai.txt", &format!("{jl4}/ai.txt")]
            ),
            ["aggressive: teleportation 1", "timers: retaliate 0"]
        );
    }

    #[test]
    fn a_goto_names_the_line_it_goes_to() {
        let lines = [
            "label a", "goto -1", "goto a", "goto 7", "goto +2", "label 7", "goto 7",
        ];
        let script = Script::parse(lines.iter().map(|l| (*l).to_owned()).collect());
        assert_eq!(
            script.steps,
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
    }
}
