//! Keyboard input: keys named as pygame named them, key presses with
//! modifiers and typed text, and the event queue the game polls.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use crate::clock::ClockRef;

macro_rules! keys {
    ($($variant:ident => [$($name:literal),+]),* $(,)?) => {
        /// A key. As a physical key it names a position on the keyboard by
        /// its US label; as a logical key it follows the keyboard layout.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Key {
            $($variant,)*
            /// A layout character with no key of its own above (logical only).
            Char(char),
            Unknown,
        }

        impl Key {
            /// The key for a pygame key name without its `K_`/`KSCAN_`
            /// prefix, as `bindings.txt` writes them ("a", "F2", "KP_ENTER").
            pub fn from_pygame_name(name: &str) -> Option<Key> {
                let upper;
                let name = if name.chars().count() == 1 {
                    upper = name.to_uppercase();
                    upper.as_str()
                } else {
                    name
                };
                match name {
                    $($($name)|+ => Some(Key::$variant),)*
                    _ => None,
                }
            }
        }
    };
}

keys! {
    A => ["A"], B => ["B"], C => ["C"], D => ["D"], E => ["E"], F => ["F"], G => ["G"],
    H => ["H"], I => ["I"], J => ["J"], K => ["K"], L => ["L"], M => ["M"], N => ["N"],
    O => ["O"], P => ["P"], Q => ["Q"], R => ["R"], S => ["S"], T => ["T"], U => ["U"],
    V => ["V"], W => ["W"], X => ["X"], Y => ["Y"], Z => ["Z"],
    Num0 => ["0"], Num1 => ["1"], Num2 => ["2"], Num3 => ["3"], Num4 => ["4"],
    Num5 => ["5"], Num6 => ["6"], Num7 => ["7"], Num8 => ["8"], Num9 => ["9"],
    Return => ["RETURN"], Escape => ["ESCAPE"], Backspace => ["BACKSPACE"], Tab => ["TAB"],
    Space => ["SPACE"], Minus => ["MINUS"], Equals => ["EQUALS"],
    LeftBracket => ["LEFTBRACKET"], RightBracket => ["RIGHTBRACKET"], Backslash => ["BACKSLASH"],
    NonUsHash => ["NONUSHASH"], Semicolon => ["SEMICOLON"], Quote => ["QUOTE", "APOSTROPHE"],
    Backquote => ["BACKQUOTE", "GRAVE"], Comma => ["COMMA"], Period => ["PERIOD"],
    Slash => ["SLASH"], NonUsBackslash => ["NONUSBACKSLASH"], CapsLock => ["CAPSLOCK"],
    F1 => ["F1"], F2 => ["F2"], F3 => ["F3"], F4 => ["F4"], F5 => ["F5"], F6 => ["F6"],
    F7 => ["F7"], F8 => ["F8"], F9 => ["F9"], F10 => ["F10"], F11 => ["F11"], F12 => ["F12"],
    F13 => ["F13"], F14 => ["F14"], F15 => ["F15"], F16 => ["F16"], F17 => ["F17"],
    F18 => ["F18"], F19 => ["F19"], F20 => ["F20"], F21 => ["F21"], F22 => ["F22"],
    F23 => ["F23"], F24 => ["F24"],
    PrintScreen => ["PRINTSCREEN", "PRINT"], ScrollLock => ["SCROLLLOCK"], Pause => ["PAUSE"],
    Insert => ["INSERT"], Home => ["HOME"], PageUp => ["PAGEUP"], Delete => ["DELETE"],
    End => ["END"], PageDown => ["PAGEDOWN"],
    Right => ["RIGHT"], Left => ["LEFT"], Down => ["DOWN"], Up => ["UP"],
    NumLock => ["NUMLOCKCLEAR", "NUMLOCK"], KpDivide => ["KP_DIVIDE"],
    KpMultiply => ["KP_MULTIPLY"], KpMinus => ["KP_MINUS"], KpPlus => ["KP_PLUS"],
    KpEnter => ["KP_ENTER"], KpPeriod => ["KP_PERIOD"], KpEquals => ["KP_EQUALS"],
    Kp0 => ["KP_0", "KP0"], Kp1 => ["KP_1", "KP1"], Kp2 => ["KP_2", "KP2"],
    Kp3 => ["KP_3", "KP3"], Kp4 => ["KP_4", "KP4"], Kp5 => ["KP_5", "KP5"],
    Kp6 => ["KP_6", "KP6"], Kp7 => ["KP_7", "KP7"], Kp8 => ["KP_8", "KP8"],
    Kp9 => ["KP_9", "KP9"],
    Application => ["APPLICATION", "MENU"],
    LCtrl => ["LCTRL"], LShift => ["LSHIFT"], LAlt => ["LALT"], LGui => ["LGUI", "LMETA", "LSUPER"],
    RCtrl => ["RCTRL"], RShift => ["RSHIFT"], RAlt => ["RALT"], RGui => ["RGUI", "RMETA", "RSUPER"],
}

impl Key {
    /// Ctrl, Alt and Shift, left or right.
    pub fn is_modifier(self) -> bool {
        matches!(
            self,
            Key::LCtrl | Key::RCtrl | Key::LAlt | Key::RAlt | Key::LShift | Key::RShift
        )
    }

    /// The key that types `c` on a US layout, or [`Key::Char`].
    pub fn from_char(c: char) -> Key {
        let c = c.to_ascii_lowercase();
        let letters = [
            Key::A,
            Key::B,
            Key::C,
            Key::D,
            Key::E,
            Key::F,
            Key::G,
            Key::H,
            Key::I,
            Key::J,
            Key::K,
            Key::L,
            Key::M,
            Key::N,
            Key::O,
            Key::P,
            Key::Q,
            Key::R,
            Key::S,
            Key::T,
            Key::U,
            Key::V,
            Key::W,
            Key::X,
            Key::Y,
            Key::Z,
        ];
        let digits = [
            Key::Num0,
            Key::Num1,
            Key::Num2,
            Key::Num3,
            Key::Num4,
            Key::Num5,
            Key::Num6,
            Key::Num7,
            Key::Num8,
            Key::Num9,
        ];
        match c {
            'a'..='z' => letters[(c as u8 - b'a') as usize],
            '0'..='9' => digits[(c as u8 - b'0') as usize],
            ' ' => Key::Space,
            '-' => Key::Minus,
            '=' => Key::Equals,
            '[' => Key::LeftBracket,
            ']' => Key::RightBracket,
            '\\' => Key::Backslash,
            ';' => Key::Semicolon,
            '\'' => Key::Quote,
            '`' => Key::Backquote,
            ',' => Key::Comma,
            '.' => Key::Period,
            '/' => Key::Slash,
            c => Key::Char(c),
        }
    }
}

/// The modifiers held with a key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Mods {
    pub const NONE: Mods = Mods {
        ctrl: false,
        alt: false,
        shift: false,
    };
    pub const SHIFT: Mods = Mods {
        ctrl: false,
        alt: false,
        shift: true,
    };
    pub const CTRL: Mods = Mods {
        ctrl: true,
        alt: false,
        shift: false,
    };
    pub const ALT: Mods = Mods {
        ctrl: false,
        alt: true,
        shift: false,
    };
}

/// A key press, as pygame's KEYDOWN event carried it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyDown {
    /// The key as the layout names it (pygame's `key`).
    pub key: Key,
    /// The key's position (pygame's `scancode`).
    pub physical: Key,
    /// The modifiers held, including this key if it is one.
    pub mods: Mods,
    /// The text the press typed, if any (pygame's `unicode`).
    pub unicode: String,
}

/// A key release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyUp {
    pub key: Key,
    pub physical: Key,
}

/// What the game polls.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    KeyDown(KeyDown),
    KeyUp(KeyUp),
    /// The window was closed.
    Quit,
}

/// What an input source delivers. Repeats are not delivered: the queue
/// makes its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RawInput {
    KeyDown(KeyDown),
    KeyUp(KeyUp),
    Quit,
    /// The window lost the keyboard: held keys stop repeating.
    FocusLost,
}

/// Where key presses come from.
pub trait InputSource {
    /// The next input that has happened by `now`, with the time it
    /// happened, if any. Never blocks.
    fn next(&mut self, now: f64) -> Option<(f64, RawInput)>;
}

/// The first repeat comes this long after a key goes down...
pub const REPEAT_DELAY: f64 = 0.5;
/// ...then one every this long while it stays down.
pub const REPEAT_INTERVAL: f64 = 0.1;

struct Repeat {
    event: KeyDown,
    next: f64,
}

/// The event queue, with pygame's behaviour: the last key pressed repeats
/// after 500 ms every 100 ms until released.
pub struct EventQueue {
    source: Box<dyn InputSource>,
    clock: ClockRef,
    pending: VecDeque<Event>,
    repeat: Option<Repeat>,
}

impl EventQueue {
    pub fn new(source: Box<dyn InputSource>, clock: ClockRef) -> EventQueue {
        EventQueue {
            source,
            clock,
            pending: VecDeque::new(),
            repeat: None,
        }
    }

    /// Queues the repeats due by `time`.
    fn repeat_until(&mut self, time: f64) {
        if let Some(r) = self.repeat.as_mut() {
            while time >= r.next {
                self.pending.push_back(Event::KeyDown(r.event.clone()));
                r.next += REPEAT_INTERVAL;
            }
        }
    }

    fn pump(&mut self) {
        let now = self.clock.now();
        while let Some((time, raw)) = self.source.next(now) {
            self.repeat_until(time);
            match raw {
                RawInput::KeyDown(e) => {
                    self.repeat = Some(Repeat {
                        event: e.clone(),
                        next: time + REPEAT_DELAY,
                    });
                    self.pending.push_back(Event::KeyDown(e));
                }
                RawInput::KeyUp(e) => {
                    if self
                        .repeat
                        .as_ref()
                        .is_some_and(|r| r.event.physical == e.physical)
                    {
                        self.repeat = None;
                    }
                    self.pending.push_back(Event::KeyUp(e));
                }
                RawInput::Quit => self.pending.push_back(Event::Quit),
                RawInput::FocusLost => self.repeat = None,
            }
        }
        self.repeat_until(now);
    }
}

/// The handle to the event queue. Clones share the same queue.
#[derive(Clone)]
pub struct Input(Rc<RefCell<EventQueue>>);

impl Input {
    pub fn new(source: Box<dyn InputSource>, clock: ClockRef) -> Input {
        Input(Rc::new(RefCell::new(EventQueue::new(source, clock))))
    }

    /// The next event, if one is waiting.
    pub fn poll(&self) -> Option<Event> {
        let mut q = self.0.borrow_mut();
        q.pump();
        q.pending.pop_front()
    }

    /// Every waiting event.
    pub fn get_all(&self) -> Vec<Event> {
        let mut q = self.0.borrow_mut();
        q.pump();
        q.pending.drain(..).collect()
    }

    /// Takes the waiting key presses out of the queue, leaving the rest.
    pub fn get_keydowns(&self) -> Vec<KeyDown> {
        let mut q = self.0.borrow_mut();
        q.pump();
        let mut keys = Vec::new();
        q.pending.retain(|e| match e {
            Event::KeyDown(k) => {
                keys.push(k.clone());
                false
            }
            _ => true,
        });
        keys
    }

    /// Puts an event at the end of the queue.
    pub fn post(&self, event: Event) {
        self.0.borrow_mut().pending.push_back(event);
    }

    /// Drops every waiting event.
    pub fn clear(&self) {
        let mut q = self.0.borrow_mut();
        q.pump();
        q.pending.clear();
    }

    /// Drops the waiting key presses.
    pub fn clear_keydowns(&self) {
        self.get_keydowns();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::ManualClock;
    use crate::testkit::{Script, down};

    #[test]
    fn a_held_key_repeats_after_500_ms_every_100_ms() {
        let clock = Rc::new(ManualClock::new());
        let script = Script::new().key_down(Key::Down).at(0.95).key_up(Key::Down);
        let input = Input::new(Box::new(script.source()), clock.clone());
        assert_eq!(input.get_keydowns(), vec![down(Key::Down)]);
        clock.advance(0.49);
        assert!(input.get_keydowns().is_empty());
        clock.advance(0.01);
        assert_eq!(input.get_keydowns().len(), 1);
        clock.advance(0.25);
        // Repeats at 0.6 and 0.7.
        assert_eq!(input.get_keydowns().len(), 2);
        clock.advance(0.5);
        // Repeats at 0.8 and 0.9, then the release at 0.95 ends them.
        assert_eq!(input.get_keydowns().len(), 2);
        assert!(matches!(input.poll(), Some(Event::KeyUp(_))));
        clock.advance(1.0);
        assert!(input.get_keydowns().is_empty());
    }

    #[test]
    fn pygame_key_names_are_case_insensitive_for_single_characters() {
        assert_eq!(Key::from_pygame_name("a"), Some(Key::A));
        assert_eq!(Key::from_pygame_name("KP_ENTER"), Some(Key::KpEnter));
        assert_eq!(Key::from_pygame_name("QUOTE"), Some(Key::Quote));
        assert_eq!(Key::from_pygame_name("kp_enter"), None);
    }
}
