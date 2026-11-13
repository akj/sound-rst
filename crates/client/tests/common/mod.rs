//! A scripted player for the game tests: it presses keys when the voice is
//! idle, waits for turns or for something to be said, and notes the game's
//! state, all from the game loop's hook.

#![allow(dead_code)]

use std::cell::RefCell;
use std::rc::Rc;

use soundrts::client::Client;
use soundrts::game::GameInterface;
use soundrts::input::{Key, Mods};
use soundrts::testkit::{LiveKeys, render};
use soundrts::voice::UtteranceLog;

/// One thing the scripted player does.
pub enum Step {
    /// Presses a key once the voice is idle.
    Key(&'static str, Key, Mods),
    /// Presses a key, again and again (at most `usize` times), until what it
    /// says contains a text.
    KeyUntil(&'static str, Key, Mods, &'static str, usize),
    /// Waits until the world has played this many turns.
    Turn(i64),
    /// Waits (at most until a turn) until an utterance contains a text.
    Said(&'static str, i64),
    /// Notes something about the game.
    Probe(&'static str, fn(&GameInterface) -> String),
    /// Schedules keys at times after now: (delay, key, mods).
    Keys(&'static str, Vec<(f64, Key, Mods)>),
    /// Opens the console, types a command a second per character after a
    /// delay, then presses Return.
    Console(&'static str, f64, &'static str),
}

/// What happened at a step: its label, the utterances said from that step
/// until the next one, or the probe's note.
#[derive(Debug, Clone)]
pub struct Mark {
    pub label: String,
    pub from: usize,
    pub note: String,
}

pub struct ScriptedPlayer {
    steps: Vec<Step>,
    next: usize,
    live: LiveKeys,
    log: UtteranceLog,
    pub marks: Vec<Mark>,
    pub done: bool,
    presses: usize,
}

pub type PlayerRef = Rc<RefCell<ScriptedPlayer>>;

impl ScriptedPlayer {
    pub fn new(steps: Vec<Step>, live: LiveKeys, log: UtteranceLog) -> PlayerRef {
        Rc::new(RefCell::new(ScriptedPlayer {
            steps,
            next: 0,
            live,
            log,
            marks: Vec::new(),
            done: false,
            presses: 0,
        }))
    }

    fn mark(&mut self, label: &str, note: String) {
        let from = self.log.utterances().len();
        self.marks.push(Mark {
            label: label.to_owned(),
            from,
            note,
        });
    }

    /// One pass of the hook.
    pub fn act(&mut self, g: &mut GameInterface, cx: &mut Client) {
        let now = cx.clock.now();
        // Keys scheduled far ahead (after the game) do not hold the script.
        let keys_due = self.live.next_time().is_some_and(|t| t <= now + 60.0);
        if keys_due || !cx.voice.is_idle() {
            return;
        }
        let Some(step) = self.steps.get(self.next) else {
            self.done = true;
            return;
        };
        match step {
            Step::Key(label, key, mods) => {
                let (label, key, mods) = (*label, *key, *mods);
                self.mark(label, String::new());
                self.live.tap_at(now, key, mods);
            }
            Step::KeyUntil(label, key, mods, text, max) => {
                let (label, key, mods, text, max) = (*label, *key, *mods, *text, *max);
                if self.presses > 0 {
                    let last = self.log.utterances().last().map(render).unwrap_or_default();
                    if last.contains(text) || self.presses >= max {
                        self.presses = 0;
                        self.next += 1;
                        return;
                    }
                } else {
                    self.mark(label, String::new());
                }
                self.presses += 1;
                self.live.tap_at(now, key, mods);
                return;
            }
            Step::Turn(t) => {
                if g.world.turn() < *t {
                    return;
                }
            }
            Step::Said(text, max_turn) => {
                let (text, max_turn) = (*text, *max_turn);
                let found = self
                    .log
                    .utterances()
                    .iter()
                    .skip(self.marks.last().map_or(0, |m| m.from))
                    .any(|u| render(u).contains(text));
                if !found && g.world.turn() < max_turn {
                    return;
                }
                self.mark(&format!("said {text}"), format!("found: {found}"));
            }
            Step::Probe(label, f) => {
                let note = f(g);
                let label = *label;
                self.mark(label, note);
            }
            Step::Keys(label, keys) => {
                let label = *label;
                for (delay, key, mods) in keys.clone() {
                    self.live.tap_at(now + delay, key, mods);
                }
                self.mark(label, String::new());
            }
            Step::Console(label, delay, text) => {
                let (label, delay, text) = (*label, *delay, *text);
                self.live.tap_at(now, Key::Backquote, Mods::NONE);
                self.live.type_at(now + delay, 1.0, text);
                let end = now + delay + text.len() as f64 + 1.0;
                self.live.tap_at(end, Key::Return, Mods::NONE);
                self.mark(label, String::new());
            }
        }
        self.next += 1;
    }

    /// The utterances of each mark, rendered, with its note.
    pub fn transcript(&self) -> Vec<(String, String, Vec<String>)> {
        let all: Vec<String> = self.log.utterances().iter().map(render).collect();
        let mut out = Vec::new();
        for (i, m) in self.marks.iter().enumerate() {
            let end = self.marks.get(i + 1).map_or(all.len(), |n| n.from);
            out.push((m.label.clone(), m.note.clone(), all[m.from..end].to_vec()));
        }
        out
    }

    /// The utterances said after the step labelled `label`, until the next
    /// step.
    pub fn said_at(&self, label: &str) -> Vec<String> {
        self.transcript()
            .into_iter()
            .find(|(l, _, _)| l == label)
            .unwrap_or_else(|| panic!("no step {label}"))
            .2
    }

    pub fn note(&self, label: &str) -> String {
        self.marks
            .iter()
            .find(|m| m.label == label)
            .unwrap_or_else(|| panic!("no step {label}"))
            .note
            .clone()
    }
}

/// Installs the scripted player as the game hook of `cx`.
pub fn install(cx: &mut Client, player: &PlayerRef) {
    let p = player.clone();
    cx.game.hook = Some(Rc::new(move |g: &mut GameInterface, cx: &mut Client| {
        p.borrow_mut().act(g, cx);
    }));
}

pub const N: Mods = Mods::NONE;
