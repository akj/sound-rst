//! A message of the voice: its parts, its speaker volumes, and whether and
//! until when it is worth saying.

use soundrts_world::msg::Msg;

use crate::audio::DEFAULT_VOLUME;
use crate::sound_cache::{Sounds, Spoken};

/// The optional settings of a queued message.
#[derive(Clone, Debug, PartialEq)]
pub struct InfoOptions {
    pub lv: f64,
    pub rv: f64,
    /// After this many seconds unsaid, the message is dropped.
    pub expiration_delay: f64,
    /// A newer message of the same type makes this one obsolete.
    pub update_type: Option<String>,
    /// Jumps ahead of the unsaid ordinary messages, and is never dropped
    /// as a duplicate or by a newer message of its type (chat).
    pub priority: bool,
}

impl Default for InfoOptions {
    fn default() -> Self {
        InfoOptions {
            lv: DEFAULT_VOLUME,
            rv: DEFAULT_VOLUME,
            expiration_delay: 45.0,
            update_type: None,
            priority: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Message {
    pub parts: Msg,
    pub lv: f64,
    pub rv: f64,
    pub said: bool,
    pub expiration_time: f64,
    pub update_type: Option<String>,
    pub priority: bool,
}

impl Message {
    pub fn new(parts: Msg, options: InfoOptions, said: bool, now: f64) -> Message {
        Message {
            parts,
            lv: options.lv,
            rv: options.rv,
            said,
            expiration_time: now + options.expiration_delay,
            update_type: options.update_type,
            priority: options.priority,
        }
    }

    pub fn has_expired(&self, now: f64) -> bool {
        self.expiration_time < now
    }

    /// The message as texts and sounds, with neighbouring texts joined by a
    /// space. Without sounds if `remove_sounds` (texts that a sound
    /// separated stay separate).
    pub fn translate_and_collapse(&self, sounds: &Sounds, remove_sounds: bool) -> Vec<Spoken> {
        let mut result: Vec<Spoken> = Vec::new();
        let mut after_text = false;
        for part in &self.parts {
            match sounds.translate(part) {
                Spoken::Sound(s) => {
                    after_text = false;
                    if !remove_sounds {
                        result.push(Spoken::Sound(s));
                    }
                }
                Spoken::Text(t) => {
                    match result.last_mut() {
                        Some(Spoken::Text(previous)) if after_text => {
                            previous.push(' ');
                            previous.push_str(&t);
                        }
                        _ => result.push(Spoken::Text(t)),
                    }
                    after_text = true;
                }
            }
        }
        result
    }
}
