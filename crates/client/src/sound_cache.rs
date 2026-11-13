//! The text table and the sounds of the active resources: what each sound
//! number of a message turns into.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use soundrts_world::msg::{MsgPart, NB_ENCODE_SHIFT};

use crate::audio::{Sound, SoundBuffer};
use crate::log::Log;
use crate::package::Package;
use crate::parameters::Parameters;

pub const TXT_FILE: &str = "ui/tts";
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

/// A part said as written, never looked up as a sound or a phrase: what
/// other players wrote, and their logins.
pub fn literal(s: &str) -> MsgPart {
    MsgPart::Literal(s.to_owned())
}

/// The texts and sounds of the resource layers, merged: a later layer
/// replaces an earlier one, a translation replaces the original. Sounds are
/// decoded when first used.
pub struct SoundCache {
    txt: HashMap<String, String>,
    sounds: HashMap<String, SoundRef>,
    params: Rc<Parameters>,
    log: Log,
}

/// The sound cache handle. Clones share the same cache.
#[derive(Clone)]
pub struct Sounds(Rc<RefCell<SoundCache>>);

fn is_all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
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
        params: Rc<Parameters>,
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

    /// What a message part becomes: its text, else the number it encodes,
    /// else its sound, else its key said as text.
    pub fn translate(&mut self, part: &MsgPart) -> Spoken {
        if let MsgPart::Literal(t) = part {
            return Spoken::Text(t.clone());
        }
        let key = part.key();
        if let Some(t) = self.text(&key) {
            return Spoken::Text(t.to_owned());
        }
        if is_all_digits(&key)
            && let Ok(n) = key.parse::<u64>()
            && n >= NB_ENCODE_SHIFT
        {
            return Spoken::Text((n - NB_ENCODE_SHIFT).to_string());
        }
        if let Some(s) = self.get_sound(&key, false) {
            return Spoken::Sound(s);
        }
        if is_all_digits(&key) {
            self.log
                .warning(format!("this sound may be missing: {key}"));
        }
        Spoken::Text(key)
    }

    /// Applies the parameters' volumes again to the loaded sounds.
    pub fn update_volumes(&mut self, params: Rc<Parameters>) {
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

    pub fn text(&self, key: &str) -> Option<String> {
        self.0.borrow().text(key).map(str::to_owned)
    }

    pub fn translate(&self, part: &MsgPart) -> Spoken {
        self.0.borrow_mut().translate(part)
    }

    pub fn update_volumes(&self, params: Rc<Parameters>) {
        self.0.borrow_mut().update_volumes(params);
    }
}
