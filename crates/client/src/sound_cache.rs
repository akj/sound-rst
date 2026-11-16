//! The text table and the sounds of the active resources: what each sound
//! number of a message turns into.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::arithmetic_side_effects
)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use soundrts_world::msg::{Cue, MsgPart, Pause, Phrase};

use crate::audio::{Sound, SoundBuffer};
use crate::log::Log;
use crate::original_names::names;
use crate::package::Package;
use crate::parameters::Parameters;

pub const TXT_FILE: &str = "ui/tts";
/// The largest `tts.txt` of one layer, in bytes; a larger one is ignored
/// whole. The original's largest is 16 KB.
pub const MAX_TTS_BYTES: usize = 1024 * 1024;
/// A sound of 0.01 s, said as "," when there is no recording.
pub const SHORT_SILENCE: &str = "9998";
/// A sound of 0.2 s, said as "." when there is no recording.
pub const SILENCE: &str = "9999";

/// What a part of a message becomes: words to speak or a sound to play.
#[derive(Clone, Debug)]
pub enum Spoken {
    Text(String),
    Sound(Sound),
}

impl PartialEq for Spoken {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Spoken::Text(a), Spoken::Text(b)) => a == b,
            (Spoken::Sound(a), Spoken::Sound(b)) => std::sync::Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

enum SoundRef {
    Unloaded(Package, String),
    Loaded(Sound),
}

/// Reads the lines `<key> <text>` of `ui/tts.txt` files into `table`.
pub fn update_text_table(table: &mut HashMap<String, String>, txt: &str, log: &Log) {
    if txt.len() > MAX_TTS_BYTES {
        log.warning(format!(
            "'{TXT_FILE}' of {} bytes, more than {MAX_TTS_BYTES}, ignored",
            txt.len()
        ));
        return;
    }
    for line in txt.split('\n') {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match line.split_once(char::is_whitespace) {
            Some((key, value)) => {
                let value = value.trim_start();
                if value.is_empty() {
                    log.warning(format!("in '{TXT_FILE}', line ignored: {line}"));
                } else {
                    table.insert(key.to_owned(), value.to_owned());
                }
            }
            None => log.warning(format!("in '{TXT_FILE}', syntax error: {line}")),
        }
    }
}

/// The texts and sounds of the resource layers, merged: a later layer
/// replaces an earlier one, a translation replaces the original. Sounds are
/// decoded when first used.
pub struct SoundCache {
    txt: HashMap<String, String>,
    sounds: HashMap<String, SoundRef>,
    params: Arc<Parameters>,
    log: Log,
}

/// The sound cache handle. Clones share the same cache.
#[derive(Clone)]
pub struct Sounds(Rc<RefCell<SoundCache>>);

fn is_all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// The key of a phrase of the original in the texts and the sounds: its
/// number, from the table of names. None for a phrase the port added.
fn phrase_key(phrase: Phrase) -> Option<String> {
    let number = names().game_number(phrase.id())?;
    Some(number.to_string())
}

/// The key of a sound the code plays by itself: the original's number for
/// it, from the table of names. A test checks that each cue has one.
fn cue_key(cue: Cue) -> String {
    let number = names().game_number(cue.sound());
    number.map_or_else(|| cue.sound().to_owned(), |n| n.to_string())
}

impl SoundCache {
    /// Builds the cache from the text files and the `.ogg` files found
    /// under `ui` (and its translation) in the layers.
    ///
    /// `texts` are the `ui/tts.txt` contents in priority order; `sound_dirs`
    /// are the (package, folder) pairs in priority order.
    pub fn new(
        texts: &[String],
        sound_dirs: &[(Package, String)],
        params: Arc<Parameters>,
        log: Log,
    ) -> SoundCache {
        let mut txt = HashMap::new();
        for t in texts {
            update_text_table(&mut txt, t, &log);
        }
        let mut sounds = HashMap::new();
        for (package, dir) in sound_dirs.iter().rev() {
            for name in package.relative_paths_of_files_in_subtree(dir) {
                let Some(stem) = name.strip_suffix(".ogg") else {
                    continue;
                };
                let key = stem.rsplit('/').next().unwrap_or(stem).to_owned();
                if txt.contains_key(&key) {
                    log.warning(format!("didn't load {key}.ogg (text exists)"));
                } else {
                    sounds
                        .entry(key)
                        .or_insert_with(|| SoundRef::Unloaded(package.clone(), name.clone()));
                }
            }
        }
        // The silences are used as random noises in style.txt.
        txt.insert(SHORT_SILENCE.to_owned(), ",".to_owned());
        txt.insert(SILENCE.to_owned(), ".".to_owned());
        SoundCache {
            txt,
            sounds,
            params,
            log,
        }
    }

    /// Adds the texts and sounds of a map that came from a server, as
    /// [`SoundCache::new`] reads them, but only under sound numbers this
    /// cache lacks: the game's own texts and sounds stay, and so do the
    /// numbers and words the game says as they are. Returns how many were
    /// ignored.
    pub fn add_downloaded(&mut self, texts: &[String], sound_dirs: &[(Package, String)]) -> usize {
        let mut ignored: usize = 0;
        let mut txt = HashMap::new();
        for t in texts {
            update_text_table(&mut txt, t, &self.log);
        }
        let taken = |cache: &SoundCache, key: &str| {
            cache.txt.contains_key(key) || cache.sounds.contains_key(key) || !is_all_digits(key)
        };
        for (key, value) in txt {
            if taken(self, &key) {
                ignored = ignored.saturating_add(1);
            } else {
                self.txt.insert(key, value);
            }
        }
        let mut sounds = HashMap::new();
        for (package, dir) in sound_dirs.iter().rev() {
            for name in package.relative_paths_of_files_in_subtree(dir) {
                let Some(stem) = name.strip_suffix(".ogg") else {
                    continue;
                };
                let key = stem.rsplit('/').next().unwrap_or(stem).to_owned();
                sounds
                    .entry(key)
                    .or_insert_with(|| SoundRef::Unloaded(package.clone(), name.clone()));
            }
        }
        for (key, sound) in sounds {
            if taken(self, &key) {
                ignored = ignored.saturating_add(1);
            } else {
                self.sounds.insert(key, sound);
            }
        }
        ignored
    }

    pub fn get_sound(&mut self, name: &str, warn: bool) -> Option<Sound> {
        match self.sounds.get(name) {
            Some(SoundRef::Loaded(s)) => return Some(s.clone()),
            Some(SoundRef::Unloaded(package, path)) => {
                let volume = self.params.sound_volume(name, &package.name);
                let loaded = package
                    .open_binary(path)
                    .map_err(|e| e.to_string())
                    .and_then(|bytes| SoundBuffer::decode(bytes, name, &package.name, volume));
                match loaded {
                    Ok(buffer) => {
                        let sound: Sound = buffer.into();
                        self.sounds
                            .insert(name.to_owned(), SoundRef::Loaded(sound.clone()));
                        return Some(sound);
                    }
                    Err(e) => {
                        self.log
                            .warning(format!("couldn't load {path} from {}: {e}", package.name));
                        self.sounds.remove(name);
                    }
                }
            }
            None => {}
        }
        if warn {
            self.log
                .warning(format!("this sound may be missing: {name}"));
        }
        None
    }

    pub fn text(&self, key: &str) -> Option<&str> {
        self.txt.get(key).map(String::as_str)
    }

    /// What a name of the texts and the sounds becomes: its text, else its
    /// sound.
    fn named(&mut self, key: &str) -> Option<Spoken> {
        if let Some(t) = self.text(key) {
            return Some(Spoken::Text(t.to_owned()));
        }
        self.get_sound(key, false).map(Spoken::Sound)
    }

    /// As [`SoundCache::named`], for a name that should exist: without it,
    /// the name is said as written, and a number is logged as missing.
    fn named_or_said(&mut self, key: String) -> Spoken {
        if let Some(spoken) = self.named(&key) {
            return spoken;
        }
        if is_all_digits(&key) {
            self.log
                .warning(format!("this sound may be missing: {key}"));
        }
        Spoken::Text(key)
    }

    /// What a message part becomes: words to speak or a sound to play. A
    /// phrase of the original is the text or the recording that the
    /// resources have for it, in the player's language, and else its
    /// English words.
    pub fn translate(&mut self, part: &MsgPart) -> Spoken {
        match part {
            MsgPart::Phrase(p) => phrase_key(*p)
                .and_then(|key| self.named(&key))
                .unwrap_or_else(|| Spoken::Text(p.english().to_owned())),
            MsgPart::Sound(c) => self.named_or_said(cue_key(*c)),
            MsgPart::Count(n) => Spoken::Text(n.to_string()),
            MsgPart::Pause(Pause::Short) => Spoken::Text(",".to_owned()),
            MsgPart::Pause(Pause::Long) => Spoken::Text(".".to_owned()),
            MsgPart::Literal(t) => Spoken::Text(t.clone()),
            MsgPart::Original(name) => self.named_or_said(name.clone()),
        }
    }

    /// Applies the parameters' volumes again to the loaded sounds.
    pub fn update_volumes(&mut self, params: Arc<Parameters>) {
        for s in self.sounds.values() {
            if let SoundRef::Loaded(s) = s {
                s.set_volume(params.sound_volume(&s.key, &s.mod_name));
            }
        }
        self.params = params;
    }
}

impl Sounds {
    pub fn new(cache: SoundCache) -> Sounds {
        Sounds(Rc::new(RefCell::new(cache)))
    }

    /// Replaces the cache, for every holder of this handle.
    pub fn replace(&self, cache: SoundCache) {
        *self.0.borrow_mut() = cache;
    }

    pub fn get_sound(&self, name: &str) -> Option<Sound> {
        self.0.borrow_mut().get_sound(name, true)
    }

    /// The sound of a cue.
    pub fn cue(&self, cue: Cue) -> Option<Sound> {
        self.get_sound(&cue_key(cue))
    }

    pub fn text(&self, key: &str) -> Option<String> {
        self.0.borrow().text(key).map(str::to_owned)
    }

    pub fn translate(&self, part: &MsgPart) -> Spoken {
        self.0.borrow_mut().translate(part)
    }

    pub fn update_volumes(&self, params: Arc<Parameters>) {
        self.0.borrow_mut().update_volumes(params);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_downloaded_text_cannot_stand_for_a_number_or_a_word() {
        let log = Log::silent();
        let params = Arc::new(Parameters::default());
        let mut cache =
            SoundCache::new(&["5 What are your orders?\n".to_owned()], &[], params, log);
        let added = "5 Press Alt F4.\n9000 golem\n1000005 six\nhello bye\n".to_owned();
        assert_eq!(cache.add_downloaded(&[added], &[]), 2);
        let said = |cache: &mut SoundCache, part: MsgPart| match cache.translate(&part) {
            Spoken::Text(t) => t,
            Spoken::Sound(_) => "a sound".to_owned(),
        };
        let original = MsgPart::original;
        assert_eq!(said(&mut cache, original("5")), "What are your orders?");
        assert_eq!(said(&mut cache, original("9000")), "golem");
        // A count is never looked up, in the code or in a file.
        assert_eq!(said(&mut cache, MsgPart::Count(5)), "5");
        assert_eq!(said(&mut cache, original("1000005")), "5");
        assert_eq!(said(&mut cache, original("hello")), "hello");
    }

    #[test]
    fn a_phrase_is_its_text_in_the_resources_and_else_its_english() {
        let log = Log::silent();
        let params = Arc::new(Parameters::default());
        let french = "4202 en attente de vos ordres\n".to_owned();
        let mut cache = SoundCache::new(&[french], &[], params, log);
        let said = |cache: &mut SoundCache, part: MsgPart| match cache.translate(&part) {
            Spoken::Text(t) => t,
            Spoken::Sound(_) => "a sound".to_owned(),
        };
        assert_eq!(
            said(&mut cache, Phrase::AwaitingYourOrders.into()),
            "en attente de vos ordres"
        );
        // The resources lack this line of the original.
        assert_eq!(said(&mut cache, Phrase::Welcome.into()), "welcome");
        // The port's own phrase has no number, so no text can replace it.
        assert_eq!(said(&mut cache, Phrase::AreWelcome.into()), "welcome");
        assert_eq!(said(&mut cache, Pause::Short.into()), ",");
        assert_eq!(said(&mut cache, Pause::Long.into()), ".");
        assert_eq!(said(&mut cache, MsgPart::literal("4202")), "4202");
    }
}
