//! Key bindings: `ui/bindings.txt` and the player's `bindings.txt`.
//!
//! A line is `#define NAME value`, or `[CTRL] [ALT] [SHIFT] key: command
//! args`, with pygame key names. Bindings match the key's position, as
//! upstream matched pygame scancodes.

use std::collections::HashMap;
use std::fmt;
use std::path::Path;

use soundrts_world::defs::preprocess;

use crate::input::{Key, KeyDown, Mods};
use crate::log::Log;
use crate::package::universal_newlines;
use crate::resource::Resources;

/// The text of the game's bindings: every `ui/bindings.txt` of the
/// resource layers (translations included), then the player's own
/// `bindings.txt` if there is one, so the player's lines win.
pub fn game_bindings_text(res: &Resources, custom: &Path) -> String {
    let mut text = res.text_appended("ui/bindings", true);
    if let Ok(bytes) = std::fs::read(custom) {
        text.push('\n');
        text.push_str(&universal_newlines(&String::from_utf8_lossy(&bytes)));
    }
    text
}

/// A key with the modifiers that must be held.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Chord {
    pub mods: Mods,
    pub key: Key,
}

impl Chord {
    /// Reads "CTRL SHIFT F2"-style text.
    pub fn parse(s: &str) -> Result<Chord, BindingError> {
        let words: Vec<&str> = s.split_whitespace().collect();
        let Some((key, mods)) = words.split_last() else {
            return Err(BindingError("the key is missing".into()));
        };
        let mut m = Mods::NONE;
        for w in mods {
            match *w {
                "CTRL" => m.ctrl = true,
                "ALT" => m.alt = true,
                "SHIFT" => m.shift = true,
                other => {
                    return Err(BindingError(format!(
                        "'{other}' is not an allowed key modifier"
                    )));
                }
            }
        }
        let key = Key::from_pygame_name(key)
            .ok_or_else(|| BindingError(format!("'{key}' is not a key")))?;
        Ok(Chord { mods: m, key })
    }

    /// The chord a key press triggers. A modifier key pressed alone is
    /// never modified by other modifiers.
    pub fn of(e: &KeyDown) -> Chord {
        let mods = if e.key.is_modifier() {
            Mods::NONE
        } else {
            e.mods
        };
        Chord {
            mods,
            key: e.physical,
        }
    }
}

/// Why a line of `bindings.txt` was ignored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindingError(pub String);

impl fmt::Display for BindingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Replaces each whole-word `name` in `line` with `value`.
fn replace_word(line: &str, name: &str, value: &str) -> String {
    if name.is_empty() {
        return line.to_owned();
    }
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    let mut previous: Option<char> = None;
    while let Some(i) = rest.find(name) {
        let before = rest[..i].chars().next_back().or(previous);
        let after = rest[i + name.len()..].chars().next();
        out.push_str(&rest[..i]);
        if !before.is_some_and(is_word) && !after.is_some_and(is_word) {
            out.push_str(value);
        } else {
            out.push_str(name);
        }
        previous = name.chars().next_back();
        rest = &rest[i + name.len()..];
    }
    out.push_str(rest);
    out
}

/// The bindings, from chord to command. A command is parsed once, when its
/// line is read; `C` is the parsed form.
#[derive(Clone, Debug)]
pub struct Bindings<C> {
    bindings: HashMap<Chord, C>,
    definitions: Vec<(String, String)>,
}

impl<C> Default for Bindings<C> {
    fn default() -> Self {
        Bindings {
            bindings: HashMap::new(),
            definitions: Vec::new(),
        }
    }
}

/// Reads a command name and its arguments into a command, or says why the
/// name is not a command.
pub type CommandParser<'a, C> = &'a dyn Fn(&str, Vec<String>) -> Result<C, BindingError>;

impl<C> Bindings<C> {
    pub fn new() -> Bindings<C> {
        Bindings::default()
    }

    fn apply_definitions(&self, line: &str) -> String {
        let mut line = line.to_owned();
        for (name, value) in &self.definitions {
            line = replace_word(&line, name, value);
        }
        line
    }

    fn add_definition(&mut self, line: &str) -> Result<(), BindingError> {
        let mut parts = line.trim().splitn(3, ' ');
        let (_, name, value) = (parts.next(), parts.next(), parts.next());
        let (Some(name), Some(value)) = (name, value) else {
            return Err(BindingError("the defined value is missing".into()));
        };
        match self.definitions.iter_mut().find(|(n, _)| n == name) {
            Some(d) => d.1 = value.to_owned(),
            None => self.definitions.push((name.to_owned(), value.to_owned())),
        }
        Ok(())
    }

    fn add_binding(&mut self, line: &str, parse: CommandParser<C>) -> Result<(), BindingError> {
        let (key, command) = line.trim().split_once(':').expect("checked by the caller");
        let chord = Chord::parse(key)?;
        let mut words = command.split_whitespace();
        let name = words.next().unwrap_or("").to_owned();
        let args = words.map(str::to_owned).collect();
        let command = parse(&name, args)?;
        self.bindings.insert(chord, command);
        Ok(())
    }

    /// Adds the bindings of `text`, each command read by `parse`. A line
    /// with an error is reported and ignored.
    pub fn load(&mut self, text: &str, parse: CommandParser<C>, log: &Log) {
        for line in preprocess(text).split('\n') {
            let line = self.apply_definitions(line);
            let result = if line.starts_with("#define ") {
                self.add_definition(&line)
            } else if line.contains(':') {
                self.add_binding(&line, parse)
            } else if !line.is_empty() {
                Err(BindingError(
                    "the line must be a binding or a definition".into(),
                ))
            } else {
                Ok(())
            };
            if let Err(e) = result {
                log.warning(format!(
                    "error in bindings.txt (line ignored):\n{line}\n({e})"
                ));
            }
        }
    }

    pub fn get(&self, chord: &Chord) -> Option<&C> {
        self.bindings.get(chord)
    }

    /// The command a key press triggers, if any.
    pub fn command_for(&self, e: &KeyDown) -> Option<&C> {
        self.get(&Chord::of(e))
    }

    /// Every binding, in no particular order.
    pub fn iter(&self) -> impl Iterator<Item = (&Chord, &C)> {
        self.bindings.iter()
    }

    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::upstream_root;

    /// A command kept as its name and arguments.
    type Command = (String, Vec<String>);

    fn command(name: &str, args: &str) -> Command {
        (
            name.into(),
            args.split_whitespace().map(str::to_owned).collect(),
        )
    }

    /// Reads any name as a command.
    fn any(name: &str, args: Vec<String>) -> Result<Command, BindingError> {
        Ok((name.to_owned(), args))
    }

    fn chord(s: &str) -> Chord {
        Chord::parse(s).unwrap()
    }

    #[test]
    fn the_upstream_bindings_parse_completely() {
        let text = std::fs::read_to_string(upstream_root().join("res/ui/bindings.txt")).unwrap();
        let log = Log::silent();
        let mut b = Bindings::new();
        b.load(&text, &any, &log);
        // Every line with a colon is a binding; each distinct chord is bound.
        let keys: Vec<String> = preprocess(&text)
            .split('\n')
            .filter(|l| l.contains(':') && !l.starts_with("#define"))
            .map(|l| l.split(':').next().unwrap().to_owned())
            .collect();
        assert_eq!(keys.len(), 172);
        let distinct: std::collections::HashSet<Chord> = keys.iter().map(|k| chord(k)).collect();
        assert_eq!(b.len(), distinct.len());

        assert_eq!(
            b.get(&chord("CTRL SHIFT F2")),
            Some(&command("toggle_cheatmode", ""))
        );
        assert_eq!(
            b.get(&chord("SHIFT s")),
            Some(&command(
                "select_units",
                "local worker soldier1 soldier2 soldier3 soldier4 soldier5 soldier6 soldier7"
            ))
        );
        assert_eq!(b.get(&chord("ALT F4")), Some(&command("gamemenu", "")));
        assert_eq!(
            b.get(&chord("QUOTE")),
            Some(&command("select_unit", "1 worker idle"))
        );
        assert_eq!(b.get(&chord("ALT a")), Some(&command("order_shortcut", "")));
        assert_eq!(b.get(&chord("LALT")), Some(&command("history_stop", "")));
        // The later binding of a chord wins.
        assert_eq!(
            b.get(&chord("d")),
            Some(&command("select_units", "local worker"))
        );
        assert_eq!(
            b.get(&chord("CTRL SHIFT F4")),
            Some(&command("change_player", ""))
        );
    }

    #[test]
    fn a_key_press_finds_its_binding_by_position_and_modifiers() {
        let mut b = Bindings::new();
        b.load(
            "SHIFT a: select_order -1\nLSHIFT: history_stop\n",
            &any,
            &Log::silent(),
        );
        let press = |key, physical, mods| KeyDown {
            key,
            physical,
            mods,
            unicode: String::new(),
        };
        // On an AZERTY layout the key labelled Q sits where QWERTY has A.
        assert_eq!(
            b.command_for(&press(Key::Q, Key::A, Mods::SHIFT)),
            Some(&command("select_order", "-1"))
        );
        assert_eq!(b.command_for(&press(Key::A, Key::A, Mods::NONE)), None);
        // A modifier pressed alone ignores the modifiers it sets itself.
        assert_eq!(
            b.command_for(&press(Key::LShift, Key::LShift, Mods::SHIFT)),
            Some(&command("history_stop", ""))
        );
    }

    #[test]
    fn bad_lines_are_ignored_with_a_warning() {
        let mut b = Bindings::new();
        b.load(
            "WIN a: x\nb: unknown\nnonsense\nc: known\n",
            &|name, args| match name {
                "known" => any(name, args),
                _ => Err(BindingError(format!("'{name}' is not a command"))),
            },
            &Log::silent(),
        );
        assert_eq!(b.len(), 1);
        assert_eq!(b.get(&chord("c")), Some(&command("known", "")));
    }
}
