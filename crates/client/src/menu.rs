//! Menus and text entry: every move speaks the item, Enter confirms,
//! Escape picks the last item (always the way back).

use std::rc::Rc;

use soundrts_world::msg::{Msg, MsgPart, msg, nb2msg};
use soundrts_world::msgparts as mp;

use crate::client::Client;
use crate::input::{Event, Key, KeyDown};
use crate::sound::StereoVolume;
use crate::sound_cache::Spoken;

pub const CONFIRM_SOUND: &str = "6116";
pub const SELECT_SOUND: &str = "6115";

/// The program must end: the window was closed, or an action asked to
/// quit. It travels up through every menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Exit;

/// What a menu does after an action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    /// A looping menu runs again.
    Stay,
    /// A looping menu ends.
    CloseMenu,
}

pub type ActionFn = Rc<dyn Fn(&mut Client) -> Result<Flow, Exit>>;

/// What choosing an item does.
#[derive(Clone)]
pub enum Action {
    /// Nothing: a menu run once just ends.
    Nothing,
    /// Ends a looping menu.
    CloseMenu,
    Run(ActionFn),
}

impl Action {
    pub fn run(f: impl Fn(&mut Client) -> Result<Flow, Exit> + 'static) -> Action {
        Action::Run(Rc::new(f))
    }
}

/// A menu item: what it says, what it does, and an explanation said after
/// it.
#[derive(Clone)]
pub struct Choice {
    pub label: Msg,
    pub action: Action,
    pub explanation: Option<Msg>,
}

impl Choice {
    pub fn new(label: Msg, action: Action) -> Choice {
        Choice {
            label,
            action,
            explanation: None,
        }
    }

    pub fn explained(label: Msg, action: Action, explanation: Msg) -> Choice {
        Choice {
            label,
            action,
            explanation: Some(explanation),
        }
    }

    /// Two items are the same if they say the same (actions cannot be
    /// compared).
    fn same(&self, other: &Choice) -> bool {
        self.label == other.label && self.explanation == other.explanation
    }
}

/// A message as Python's `repr()` wrote it, for the remembered choice file.
pub fn py_repr(m: &Msg) -> String {
    let parts: Vec<String> = m
        .iter()
        .map(|p| match p {
            MsgPart::Num(n) => n.to_string(),
            MsgPart::Text(s) => {
                let quote = if s.contains('\'') && !s.contains('"') {
                    '"'
                } else {
                    '\''
                };
                let mut out = String::from(quote);
                for c in s.chars() {
                    match c {
                        '\\' => out.push_str("\\\\"),
                        '\n' => out.push_str("\\n"),
                        '\r' => out.push_str("\\r"),
                        '\t' => out.push_str("\\t"),
                        c if c == quote => {
                            out.push('\\');
                            out.push(c);
                        }
                        c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                            out.push_str(&format!("\\x{:02x}", c as u32))
                        }
                        c => out.push(c),
                    }
                }
                out.push(quote);
                out
            }
        })
        .collect();
    format!("[{}]", parts.join(", "))
}

/// A menu.
pub struct Menu {
    pub title: Msg,
    pub choices: Vec<Choice>,
    choice_index: Option<i64>,
    pub default_choice_index: i64,
    remember: Option<String>,
    remembered_choice: String,
    choice_done: bool,
    end_loop: bool,
}

impl Menu {
    pub fn new(title: Msg, choices: Vec<Choice>) -> Menu {
        Menu {
            title,
            choices,
            choice_index: None,
            default_choice_index: 0,
            remember: None,
            remembered_choice: String::new(),
            choice_done: false,
            end_loop: false,
        }
    }

    /// A menu that remembers its last choice under `name`, and offers it
    /// first next time.
    pub fn remembering(cx: &Client, title: Msg, name: &str) -> Menu {
        let mut m = Menu::new(title, Vec::new());
        m.remembered_choice = std::fs::read_to_string(remember_path(cx, name)).unwrap_or_default();
        m.remember = Some(name.to_owned());
        m
    }

    /// Adds an item (with an empty explanation if none).
    pub fn append(&mut self, label: Msg, action: Action, explanation: Option<Msg>) {
        let remembered = self.remember.is_some() && self.remembered_choice == py_repr(&label);
        self.choices.push(Choice {
            label: label.clone(),
            action: action.clone(),
            explanation: Some(explanation.unwrap_or_default()),
        });
        if remembered {
            self.choices.insert(0, Choice::new(label, action));
        }
    }

    fn index(&self, i: i64) -> usize {
        i.rem_euclid(self.choices.len() as i64) as usize
    }

    fn say_choice(&self, cx: &mut Client) {
        cx.audio.play_stereo(
            cx.sounds.get_sound(SELECT_SOUND),
            StereoVolume::Both(1.0),
            0.0,
        );
        let choice = &self.choices[self.index(self.choice_index.unwrap_or(0))];
        let mut m = choice.label.clone();
        if let Some(explanation) = &choice.explanation {
            m.extend(msg(mp::COMMA));
            m.extend(explanation.iter().cloned());
        }
        cx.voice.item(&m);
    }

    fn choice_exists(&self) -> bool {
        self.choice_index
            .is_some_and(|i| 0 <= i && i < self.choices.len() as i64)
    }

    fn select_next_choice(&mut self, cx: &mut Client, first_letter: Option<&str>, inc: i64) {
        if self.choices.is_empty() {
            return;
        }
        let len = self.choices.len() as i64;
        let mut index = match self.choice_index {
            None => self.default_choice_index - i64::from(inc == -1),
            Some(i) => (i + inc).rem_euclid(len),
        };
        if let Some(letter) = first_letter {
            let letter = letter.to_lowercase();
            let mut found = false;
            for _ in 0..len {
                if first_letter_of(cx, &self.choices[self.index(index)]).as_deref()
                    == Some(letter.as_str())
                {
                    found = true;
                    break;
                }
                index = (index + inc).rem_euclid(len);
            }
            if !found {
                self.choice_index = Some((index - inc).rem_euclid(len));
                return;
            }
        }
        self.choice_index = Some(index);
        self.say_choice(cx);
    }

    fn confirm_choice(&mut self, cx: &mut Client) {
        cx.audio.play_stereo(
            cx.sounds.get_sound(CONFIRM_SOUND),
            StereoVolume::Both(1.0),
            0.0,
        );
        if self.choice_exists() {
            let label = self.choices[self.index(self.choice_index.unwrap_or(0))]
                .label
                .clone();
            cx.voice.confirmation(&label);
            self.choice_done = true;
        }
    }

    fn process_keydown(&mut self, cx: &mut Client, e: &KeyDown) {
        // Building a menu can take time; drop the presses that piled up.
        cx.input.clear_keydowns();
        let (shift, ctrl) = (e.mods.shift, e.mods.ctrl);
        match e.key {
            Key::Escape | Key::Left => {
                self.choice_index = Some(self.choices.len() as i64 - 1);
                self.confirm_choice(cx);
            }
            Key::Tab if shift => self.select_next_choice(cx, None, -1),
            Key::Up => self.select_next_choice(cx, None, -1),
            Key::Tab | Key::Down => self.select_next_choice(cx, None, 1),
            Key::Return | Key::KpEnter | Key::Right => self.confirm_choice(cx),
            Key::F2 if ctrl => cx.toggle_fullscreen(),
            Key::F1 if shift => {
                let m = cx.help_msg("menu", -1);
                cx.voice.item(&m);
            }
            Key::F2 => {
                let m = cx.help_msg("menu", -1);
                cx.voice.item(&m);
            }
            Key::F1 => {
                let m = cx.help_msg("menu", 1);
                cx.voice.item(&m);
            }
            Key::F5 => cx.voice.previous(),
            Key::LAlt | Key::RAlt => cx.voice.say_next(false),
            Key::F6 => cx.voice.say_next(true),
            Key::Home | Key::KpPlus => cx.modify_volume(1),
            Key::End | Key::KpMinus => cx.modify_volume(-1),
            // Chat needs a server; the client has none here.
            Key::F7 => cx.voice.item(&msg(mp::BEEP)),
            _ if !e.unicode.is_empty() && shift => {
                self.select_next_choice(cx, Some(&e.unicode), -1)
            }
            _ if !e.unicode.is_empty() => self.select_next_choice(cx, Some(&e.unicode), 1),
            Key::LShift | Key::RShift => {}
            _ => cx.voice.item(&msg(mp::SELECT_AND_CONFIRM_EXPLANATION)),
        }
    }

    /// Replaces the title and items with those of `menu`, keeping the
    /// selected item if it is still there. A new title is said.
    pub fn update_menu(&mut self, cx: &mut Client, menu: Menu) {
        let old_title = std::mem::replace(&mut self.title, menu.title);
        let old_choice = self
            .choice_index
            .filter(|i| (0..self.choices.len() as i64).contains(i) || *i == -1)
            .filter(|_| !self.choices.is_empty())
            .map(|i| self.choices[self.index(i)].clone());
        let old_choices = std::mem::replace(&mut self.choices, menu.choices);
        if !self.title.is_empty() && self.title != old_title {
            let title = self.title.clone();
            cx.voice.menu(&title);
        }
        let same = old_choices.len() == self.choices.len()
            && old_choices
                .iter()
                .zip(&self.choices)
                .all(|(a, b)| a.same(b));
        if !same {
            self.choice_index = old_choice
                .and_then(|old| self.choices.iter().position(|c| c.same(&old)))
                .map(|i| i as i64);
        }
    }

    fn execute_choice(&mut self, cx: &mut Client) -> Result<(), Exit> {
        let index = self.index(self.choice_index.unwrap_or(0));
        let Choice { label, action, .. } = self.choices[index].clone();
        let flow = match &action {
            Action::Nothing => Flow::Stay,
            Action::CloseMenu => Flow::CloseMenu,
            Action::Run(f) => f(cx)?,
        };
        if flow == Flow::CloseMenu {
            self.end_loop = true;
        }
        match &self.remember {
            Some(name) if !matches!(action, Action::Nothing) => {
                let path = remember_path(cx, name);
                if let Err(e) = std::fs::write(&path, py_repr(&label)) {
                    cx.log
                        .warning(format!("cannot write {}: {e}", path.display()));
                }
                self.default_choice_index = self.choice_index.unwrap_or(0);
            }
            _ => self.default_choice_index = 0,
        }
        self.choice_index = None;
        Ok(())
    }

    fn try_to_get_choice(&mut self, cx: &mut Client, e: Option<Event>) -> Result<(), Exit> {
        match e {
            Some(Event::Quit) => return Err(Exit),
            Some(Event::KeyDown(k)) => self.process_keydown(cx, &k),
            _ => {}
        }
        cx.voice.update();
        Ok(())
    }

    fn get_choice_from_static_menu(&mut self, cx: &mut Client) -> Result<(), Exit> {
        self.choice_done = false;
        while !self.choice_done {
            let e = cx.input.poll();
            self.try_to_get_choice(cx, e)?;
            cx.clock.sleep(0.01);
        }
        Ok(())
    }

    /// Handles at most one event; runs the choice if one was confirmed.
    /// For menus that must not block.
    pub fn step(&mut self, cx: &mut Client) -> Result<(), Exit> {
        self.choice_done = false;
        let e = cx.input.poll();
        self.try_to_get_choice(cx, e)?;
        if self.choice_done {
            self.execute_choice(cx)?;
        }
        Ok(())
    }

    /// Says the title, waits for a choice, and runs it.
    pub fn run(&mut self, cx: &mut Client) -> Result<(), Exit> {
        if self.title.is_empty() {
            cx.voice.menu(&msg(mp::MAKE_A_SELECTION2));
        } else {
            let title = self.title.clone();
            cx.voice.menu(&title);
        }
        self.get_choice_from_static_menu(cx)?;
        self.execute_choice(cx)
    }

    /// Runs the menu again and again until an action closes it.
    pub fn run_loop(&mut self, cx: &mut Client) -> Result<(), Exit> {
        self.end_loop = false;
        while !self.end_loop {
            self.run(cx)?;
        }
        Ok(())
    }

    /// Whether the last event ended a looping menu.
    pub fn ended(&self) -> bool {
        self.end_loop
    }
}

fn remember_path(cx: &Client, name: &str) -> std::path::PathBuf {
    cx.paths.tmp().join(format!("{name}.txt"))
}

/// The lower case first letter of the first part of the item's label that
/// is text.
pub fn first_letter_of(cx: &Client, choice: &Choice) -> Option<String> {
    choice
        .label
        .iter()
        .find_map(|part| match cx.sounds.translate(part) {
            Spoken::Text(t) => t.chars().next().map(|c| c.to_lowercase().collect()),
            Spoken::Sound(_) => None,
        })
}

/// Spells a string: each character apart, digits and dots by name. Without
/// `spell`, the whole string is said as a word.
pub fn string_to_msg(s: &str, spell: bool) -> Msg {
    if !spell {
        return vec![MsgPart::from(s)];
    }
    let mut out = Vec::new();
    for c in s.chars() {
        if c == '.' {
            out.extend(msg(mp::DOT));
        } else if let Some(d) = c.to_digit(10) {
            out.extend(nb2msg(i64::from(d)));
        } else {
            out.push(MsgPart::Text(c.to_string()));
        }
    }
    out
}

/// Asks for a line of text. Each typed character must match `allowed`;
/// Enter accepts, Escape gives `None`, Backspace erases.
pub fn input_string(
    cx: &mut Client,
    prompt: &Msg,
    allowed: &dyn Fn(char) -> bool,
    default: &str,
    spell: bool,
) -> Result<Option<String>, Exit> {
    cx.voice.menu(prompt);
    let mut s = default.to_owned();
    loop {
        match cx.input.poll() {
            Some(Event::Quit) => return Err(Exit),
            Some(Event::KeyDown(e)) => {
                cx.input.clear();
                match e.key {
                    Key::LShift | Key::RShift => continue,
                    Key::Return | Key::KpEnter => {
                        cx.voice.item(&vec![MsgPart::from(s.as_str())]);
                        return Ok(Some(s));
                    }
                    Key::Escape => return Ok(None),
                    Key::Backspace => {
                        s.pop();
                        cx.voice.item(&string_to_msg(&s, spell));
                    }
                    _ => {
                        let mut chars = e.unicode.chars();
                        match (chars.next(), chars.next()) {
                            (Some(c), None) if allowed(c) => {
                                s.push(c);
                                let mut m = string_to_msg(&c.to_string(), true);
                                m.extend(msg(mp::PERIOD));
                                m.extend(string_to_msg(&s, spell));
                                cx.voice.item(&m);
                            }
                            _ => {
                                let mut m = msg(mp::BEEP);
                                m.extend(msg(mp::PERIOD));
                                m.extend(string_to_msg(&s, spell));
                                cx.voice.item(&m);
                            }
                        }
                    }
                }
            }
            Some(_) => {}
            // Nothing to read: wait a little rather than spin.
            None => cx.clock.sleep(0.01),
        }
        cx.voice.update();
    }
}

/// Letters and digits: the characters of a login.
pub fn letters_and_digits(c: char) -> bool {
    c.is_ascii_alphanumeric()
}
