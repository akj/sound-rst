//! Key bindings: `ui/bindings.txt` and the player's `bindings.txt`.
//!
//! A line is `#define NAME value`, or `[CTRL] [ALT] [SHIFT] key: command
//! args`, with pygame key names. Bindings match the key's position, as
//! upstream matched pygame scancodes.
//!
//! Definitions are applied to every later line, so a line can grow with
//! each definition it uses. A line past one of the limits below is refused
//! with a warning; past [`MAX_BINDINGS_BYTES`], the rest of the text is.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::arithmetic_side_effects
)]

use std::collections::HashMap;
use std::fmt;
use std::path::Path;

use soundrts_world::defs::preprocess;

use crate::input::{Key, KeyDown, Mods};
use crate::log::Log;
use crate::package::{read_file, universal_newlines};
use crate::resource::Resources;

/// The most bytes of lines the bindings read, every layer and the
/// player's file together, counted once definitions are applied, and the
/// largest player's file read. The original's bindings are 5.8 KB.
pub const MAX_BINDINGS_BYTES: usize = 1024 * 1024;

/// The longest line, before or after its definitions are applied, in
/// bytes. The original's longest is 101 bytes.
pub const MAX_BINDING_LINE_BYTES: usize = 4096;

/// The most names `#define` may give. The original defines one.
pub const MAX_DEFINITIONS: usize = 32;

/// The bindings this client adds to the original's: reading the chat again,
/// chat to the allies, the news log (what did I miss, a category's
/// messages and changing the category, the last alerts' squares), the
/// questions: what is in production, the workers and the fight here
/// beside the original's status keys Z to V, every resource on Shift X,
/// and the other players beside F11; and key help on Control F1, beside
/// help on F1. Each key is one the original leaves free and that neither
/// Windows nor a screen reader takes. Shift Z is left to maps, which bind
/// it to a third resource.
pub const CLIENT_BINDINGS: &str = "CTRL F5: chat_previous\nCTRL F6: chat_next\nSHIFT F7: say_allies\n\
    F4: unheard\nCOMMA: review_previous\nPERIOD: review_next\nSLASH: review_category\n\
    DELETE: jump_to_alert\n\
    b: production_status\nn: workers_status\nm: fight_status\nSHIFT x: all_resources_status\n\
    F12: players_status\nCTRL F1: key_help\n";

/// The text of the game's bindings: every `ui/bindings.txt` of the
/// resource layers (translations included) but a map that came from a
/// server, this client's own additions,
/// then the player's own `bindings.txt` if there is one, so the player's
/// lines win.
pub fn game_bindings_text(res: &Resources, custom: &Path) -> String {
    let mut text = res.text_appended("ui/bindings", true);
    text.push('\n');
    text.push_str(CLIENT_BINDINGS);
    let limit = u64::try_from(MAX_BINDINGS_BYTES).unwrap_or(u64::MAX);
    if let Ok(bytes) = read_file(custom, limit) {
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

    /// The chord as a player names it: "Control Shift Tab".
    pub fn spoken(self) -> String {
        let mut words: Vec<&str> = Vec::new();
        for (held, word) in [
            (self.mods.ctrl, "Control"),
            (self.mods.alt, "Alt"),
            (self.mods.shift, "Shift"),
        ] {
            if held {
                words.push(word);
            }
        }
        let key = key_name(self.key);
        words.push(&key);
        words.join(" ")
    }
}

/// A key as a player names it, in English.
fn key_name(key: Key) -> String {
    use Key as K;
    let name = match key {
        K::Char(c) => return c.to_uppercase().collect(),
        K::Unknown => "unknown key",
        K::Return => "Enter",
        K::Escape => "Escape",
        K::Backspace => "Backspace",
        K::Tab => "Tab",
        K::Space => "Space",
        K::Minus => "Minus",
        K::Equals => "Equals",
        K::LeftBracket => "Left Bracket",
        K::RightBracket => "Right Bracket",
        K::Backslash => "Backslash",
        K::NonUsHash => "Hash",
        K::Semicolon => "Semicolon",
        K::Quote => "Apostrophe",
        K::Backquote => "Grave",
        K::Comma => "Comma",
        K::Period => "Period",
        K::Slash => "Slash",
        K::NonUsBackslash => "Less Than",
        K::CapsLock => "Caps Lock",
        K::PrintScreen => "Print Screen",
        K::ScrollLock => "Scroll Lock",
        K::Pause => "Pause",
        K::Insert => "Insert",
        K::Home => "Home",
        K::PageUp => "Page Up",
        K::Delete => "Delete",
        K::End => "End",
        K::PageDown => "Page Down",
        K::Right => "Right Arrow",
        K::Left => "Left Arrow",
        K::Down => "Down Arrow",
        K::Up => "Up Arrow",
        K::NumLock => "Num Lock",
        K::KpDivide => "Keypad Slash",
        K::KpMultiply => "Keypad Star",
        K::KpMinus => "Keypad Minus",
        K::KpPlus => "Keypad Plus",
        K::KpEnter => "Keypad Enter",
        K::KpPeriod => "Keypad Period",
        K::KpEquals => "Keypad Equals",
        K::Kp0 => "Keypad 0",
        K::Kp1 => "Keypad 1",
        K::Kp2 => "Keypad 2",
        K::Kp3 => "Keypad 3",
        K::Kp4 => "Keypad 4",
        K::Kp5 => "Keypad 5",
        K::Kp6 => "Keypad 6",
        K::Kp7 => "Keypad 7",
        K::Kp8 => "Keypad 8",
        K::Kp9 => "Keypad 9",
        K::Application => "Applications",
        K::LCtrl => "Left Control",
        K::LShift => "Left Shift",
        K::LAlt => "Left Alt",
        K::LGui => "Left Windows",
        K::RCtrl => "Right Control",
        K::RShift => "Right Shift",
        K::RAlt => "Right Alt",
        K::RGui => "Right Windows",
        // Letters, digits and function keys are named as written.
        other => return other.pygame_name().unwrap_or("unknown key").to_owned(),
    };
    name.to_owned()
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

/// Replaces each whole-word `name` in `line` with `value`; `None` if the
/// result would be longer than [`MAX_BINDING_LINE_BYTES`].
fn replace_word(line: &str, name: &str, value: &str) -> Option<String> {
    if name.is_empty() {
        return Some(line.to_owned());
    }
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    let mut previous: Option<char> = None;
    while let Some(i) = rest.find(name) {
        let (head, found) = rest.split_at_checked(i)?;
        let tail = found.get(name.len()..)?;
        let before = head.chars().next_back().or(previous);
        let after = tail.chars().next();
        out.push_str(head);
        if !before.is_some_and(is_word) && !after.is_some_and(is_word) {
            out.push_str(value);
        } else {
            out.push_str(name);
        }
        if out.len() > MAX_BINDING_LINE_BYTES {
            return None;
        }
        previous = name.chars().next_back();
        rest = tail;
    }
    out.push_str(rest);
    (out.len() <= MAX_BINDING_LINE_BYTES).then_some(out)
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

    /// The line with the definitions applied, and the most bytes it held
    /// on the way, or `None` if it held more than
    /// [`MAX_BINDING_LINE_BYTES`].
    fn apply_definitions(&self, line: &str) -> Option<(String, usize)> {
        if line.len() > MAX_BINDING_LINE_BYTES {
            return None;
        }
        let mut peak = line.len();
        let mut line = line.to_owned();
        for (name, value) in &self.definitions {
            line = replace_word(&line, name, value)?;
            peak = peak.max(line.len());
        }
        Some((line, peak))
    }

    fn add_definition(&mut self, line: &str) -> Result<(), BindingError> {
        let mut parts = line.trim().splitn(3, ' ');
        let (_, name, value) = (parts.next(), parts.next(), parts.next());
        let (Some(name), Some(value)) = (name, value) else {
            return Err(BindingError("the defined value is missing".into()));
        };
        let count = self.definitions.len();
        match self.definitions.iter_mut().find(|(n, _)| n == name) {
            Some(d) => d.1 = value.to_owned(),
            None if count < MAX_DEFINITIONS => {
                self.definitions.push((name.to_owned(), value.to_owned()))
            }
            None => {
                return Err(BindingError(format!(
                    "more than {MAX_DEFINITIONS} definitions"
                )));
            }
        }
        Ok(())
    }

    fn add_binding(&mut self, line: &str, parse: CommandParser<C>) -> Result<(), BindingError> {
        let Some((key, command)) = line.trim().split_once(':') else {
            return Err(BindingError("the colon is missing".into()));
        };
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
        // Each line costs the most bytes it held, a refused line the most
        // it could, so the work of applying the definitions stays bounded.
        let mut left = MAX_BINDINGS_BYTES;
        for raw in preprocess(text).split('\n') {
            let expanded = self.apply_definitions(raw);
            let cost = expanded
                .as_ref()
                .map_or(MAX_BINDING_LINE_BYTES, |(_, peak)| *peak)
                .max(1);
            let Some(l) = left.checked_sub(cost) else {
                log.warning(format!(
                    "bindings.txt: more than {MAX_BINDINGS_BYTES} bytes; the rest is ignored"
                ));
                return;
            };
            left = l;
            let Some((line, _)) = expanded else {
                let start: String = raw.chars().take(80).collect();
                log.warning(format!(
                    "error in bindings.txt (line ignored):\n{start}\n(longer than {MAX_BINDING_LINE_BYTES} bytes once its definitions are applied)"
                ));
                continue;
            };
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
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
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
    fn the_clients_keys_are_free_in_the_original_and_name_its_commands() {
        let text = std::fs::read_to_string(upstream_root().join("res/ui/bindings.txt")).unwrap();
        let log = Log::silent();
        let mut upstream = Bindings::new();
        upstream.load(&text, &any, &log);
        let mut client = Bindings::new();
        client.load(
            CLIENT_BINDINGS,
            &crate::game::command::GameCommand::parse,
            &log,
        );
        assert_eq!(client.len(), CLIENT_BINDINGS.lines().count());
        for (c, command) in client.iter() {
            assert_eq!(upstream.get(c), None, "{c:?} is the original's");
            assert!(
                !matches!(command, crate::game::command::GameCommand::Broken { .. }),
                "{command:?}"
            );
        }
        // Maps bind Shift Z to a third resource.
        assert_eq!(client.get(&chord("SHIFT z")), None);
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

    /// Definitions that double at each step: `Z1` is `Z0 Z0`, `Z2` is
    /// `Z1 Z1`, and so on up to `Z<n>`.
    fn doubling(n: usize) -> String {
        let mut text = "#define Z0 x\n".to_owned();
        for i in 1..=n {
            text.push_str(&format!("#define Z{i} Z{} Z{}\n", i - 1, i - 1));
        }
        text
    }

    #[test]
    fn a_line_that_grows_too_long_through_definitions_is_refused() {
        // Z10 is 1,024 words, 2,047 bytes: within the limit. Twice that is
        // not.
        let text = format!("{}a: one Z10\nb: two Z10 Z10\nc: three\n", doubling(10));
        let mut b = Bindings::new();
        b.load(&text, &any, &Log::silent());
        assert_eq!(b.get(&chord("a")).map(|c| c.1.len()), Some(1024));
        assert_eq!(b.get(&chord("b")), None);
        assert_eq!(b.get(&chord("c")), Some(&command("three", "")));
    }

    #[test]
    fn definitions_that_double_without_end_stop_at_the_limit() {
        // The definition of Z11, 4,095 bytes, is refused, and so is every
        // one past the limit of names, so Z40 means nothing. Without the
        // limits, Z40 would be 4 TB.
        let text = format!("{}a: one Z40\n", doubling(40));
        let mut b = Bindings::new();
        b.load(&text, &any, &Log::silent());
        assert_eq!(b.get(&chord("a")), Some(&command("one", "Z40")));
    }

    #[test]
    fn at_most_the_limit_of_names_can_be_defined() {
        let mut text: String = (0..=MAX_DEFINITIONS)
            .map(|i| format!("#define N{i} x{i}\n"))
            .collect();
        text.push_str("a: one N0 N31 N32\n");
        let mut b = Bindings::new();
        b.load(&text, &any, &Log::silent());
        assert_eq!(b.get(&chord("a")), Some(&command("one", "x0 x31 N32")));
    }

    #[test]
    fn lines_past_the_limit_of_bytes_are_ignored() {
        let long = format!("a: one {}\n", "x ".repeat(2000));
        let count = MAX_BINDINGS_BYTES / long.len() + 1;
        let text = format!("{}b: two\n", long.repeat(count));
        let mut b = Bindings::new();
        b.load(&text, &any, &Log::silent());
        assert!(b.get(&chord("a")).is_some());
        assert_eq!(b.get(&chord("b")), None);
    }
}
