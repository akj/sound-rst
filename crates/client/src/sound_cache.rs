//! The phrases and the sounds of the active resources, by name: what each
//! part of a message turns into.
//!
//! A phrase is a line `name = words` of a language file, `text/en.txt` or
//! `text/fr.txt`. A sound is a recording under `sounds/`, named by its path
//! without `.ogg`: `footstep/grass_1`.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::arithmetic_side_effects
)]

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use soundrts_world::msg::{Cue, MsgPart, Pause, Phrase, is_name};

use crate::audio::{Sound, SoundBuffer};
use crate::log::Log;
use crate::package::{Package, TextFile};
use crate::parameters::Parameters;

/// The folder of a package that holds its language files.
pub const TEXT_FOLDER: &str = "text";
/// The folder of a package that holds its recordings.
pub const SOUNDS_FOLDER: &str = "sounds";
/// The largest language file of one layer, in bytes; a larger one is
/// refused whole. The game's largest is 20 KB.
pub const MAX_LANGUAGE_FILE_BYTES: usize = 1024 * 1024;
/// A sound of 0.01 s, said as "," when a message names it.
const SHORT_SILENCE: &str = "silence/short";
/// A sound of 0.2 s, said as "." when a message names it.
const SILENCE: &str = "silence/long";

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

/// The phrases of a language file, in the order of the file: each line is
/// `name = words`, blank, or a comment that starts with `#`. This is the
/// simple subset of Fluent. Any other line refuses the whole file: the
/// error names the line.
pub fn read_language_file(text: &str) -> Result<Vec<(&str, &str)>, String> {
    if text.len() > MAX_LANGUAGE_FILE_BYTES {
        return Err(format!(
            "{} bytes, more than {MAX_LANGUAGE_FILE_BYTES}",
            text.len()
        ));
    }
    let mut phrases = Vec::new();
    for (i, line) in text.split('\n').enumerate() {
        let at = |what: &str| format!("line {}: {what}", i.saturating_add(1));
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let (name, words) = line
            .split_once('=')
            .ok_or_else(|| at("not 'name = words'"))?;
        let (name, words) = (name.trim_end(), words.trim());
        if !is_name(name) || name.contains('/') {
            return Err(at("the name is not lower case letters, digits and '_'"));
        }
        if words.is_empty() {
            return Err(at("no words after '='"));
        }
        phrases.push((name, words));
    }
    Ok(phrases)
}

/// The recordings under `sounds/` of a layer, by name.
fn recordings(package: &Package, dir: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for path in package.relative_paths_of_files_in_subtree(dir) {
        let name = path
            .strip_prefix(dir)
            .and_then(|p| p.strip_prefix('/'))
            .and_then(|p| p.strip_suffix(".ogg"));
        if let Some(name) = name.filter(|n| is_name(n)) {
            found.push((name.to_owned(), path.clone()));
        }
    }
    found
}

/// The phrases and sounds of the resource layers, merged: a later layer
/// replaces an earlier one, a translation replaces the English. Sounds are
/// decoded when first used.
pub struct SoundCache {
    txt: HashMap<String, String>,
    /// The place of each phrase among the phrases, in the order the
    /// language files give them.
    rank: HashMap<String, usize>,
    sounds: HashMap<String, SoundRef>,
    /// The names that named nothing, each reported once.
    missing: HashSet<String>,
    params: Arc<Parameters>,
    log: Log,
}

/// The sound cache handle. Clones share the same cache.
#[derive(Clone)]
pub struct Sounds(Rc<RefCell<SoundCache>>);

impl SoundCache {
    /// Builds the cache from the language files and the recordings of the
    /// layers, both in priority order. A language file with a line that
    /// cannot be read is refused whole, with its path and the line.
    pub fn new(
        texts: &[TextFile],
        sound_dirs: &[(Package, String)],
        params: Arc<Parameters>,
        log: Log,
    ) -> SoundCache {
        let mut cache = SoundCache {
            txt: HashMap::new(),
            rank: HashMap::new(),
            sounds: HashMap::new(),
            missing: HashSet::new(),
            params,
            log,
        };
        for (name, words) in cache.phrases_of(texts) {
            let next = cache.rank.len();
            cache.rank.entry(name.clone()).or_insert(next);
            cache.txt.insert(name, words);
        }
        for (package, dir) in sound_dirs.iter().rev() {
            for (name, path) in recordings(package, dir) {
                if cache.txt.contains_key(&name) {
                    cache
                        .log
                        .warning(format!("didn't load {name}.ogg (text exists)"));
                } else {
                    cache
                        .sounds
                        .entry(name)
                        .or_insert_with(|| SoundRef::Unloaded(package.clone(), path));
                }
            }
        }
        // The silences are used as random noises in style.txt.
        cache.txt.insert(SHORT_SILENCE.to_owned(), ",".to_owned());
        cache.txt.insert(SILENCE.to_owned(), ".".to_owned());
        cache
    }

    /// The phrases of the language files that can be read, in order.
    fn phrases_of(&self, texts: &[TextFile]) -> Vec<(String, String)> {
        let mut phrases = Vec::new();
        for file in texts {
            match read_language_file(&file.text) {
                Ok(lines) => phrases.extend(
                    lines
                        .into_iter()
                        .map(|(name, words)| (name.to_owned(), words.to_owned())),
                ),
                Err(e) => self.log.error(format!("{} is refused: {e}", file.path)),
            }
        }
        phrases
    }

    /// Adds the phrases and sounds of a layer that only adds names (see
    /// [`crate::layers::Layer::power`]), as [`SoundCache::new`] reads
    /// them, but only under names this cache lacks: the phrases and
    /// sounds below stay. Returns how many were ignored.
    pub fn add_downloaded(
        &mut self,
        texts: &[TextFile],
        sound_dirs: &[(Package, String)],
    ) -> usize {
        let mut ignored: usize = 0;
        let taken = |cache: &SoundCache, name: &str| {
            cache.txt.contains_key(name)
                || cache.sounds.contains_key(name)
                || Phrase::from_id(name).is_some()
        };
        let mut txt = HashMap::new();
        let mut order = Vec::new();
        for (name, words) in self.phrases_of(texts) {
            if txt.insert(name.clone(), words).is_none() {
                order.push(name);
            }
        }
        for name in order {
            if taken(self, &name) {
                ignored = ignored.saturating_add(1);
            } else if let Some(words) = txt.remove(&name) {
                let next = self.rank.len();
                self.rank.entry(name.clone()).or_insert(next);
                self.txt.insert(name, words);
            }
        }
        let mut sounds = HashMap::new();
        for (package, dir) in sound_dirs.iter().rev() {
            for (name, path) in recordings(package, dir) {
                sounds
                    .entry(name)
                    .or_insert_with(|| SoundRef::Unloaded(package.clone(), path));
            }
        }
        for (name, sound) in sounds {
            if taken(self, &name) {
                ignored = ignored.saturating_add(1);
            } else {
                self.sounds.insert(name, sound);
            }
        }
        ignored
    }

    /// Whether a phrase or a sound has this name.
    pub fn has(&self, name: &str) -> bool {
        self.txt.contains_key(name) || self.sounds.contains_key(name)
    }

    /// The place of a phrase among the phrases, in the order the language
    /// files give them. A name that is no phrase comes after them all.
    pub fn rank(&self, name: &str) -> usize {
        self.rank.get(name).copied().unwrap_or(usize::MAX)
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

    /// What a name becomes: its phrase, else its sound. A name that names
    /// nothing says nothing, and is reported once: it is never said as
    /// written, since a map from a server chooses its names.
    fn named(&mut self, name: &str) -> Option<Spoken> {
        if let Some(t) = self.text(name) {
            return Some(Spoken::Text(t.to_owned()));
        }
        let sound = self.get_sound(name, false).map(Spoken::Sound);
        if sound.is_none() && self.missing.len() < 1000 && self.missing.insert(name.to_owned()) {
            self.log
                .warning(format!("{name} names no phrase and no sound"));
        }
        sound
    }

    /// What a message part becomes: words to speak or a sound to play, or
    /// nothing for a name that names nothing. A phrase is the words or
    /// the recording that the resources have for it, in the player's
    /// language, and else its English words.
    pub fn translate(&mut self, part: &MsgPart) -> Option<Spoken> {
        Some(match part {
            MsgPart::Phrase(p) => {
                let known = self.has(p.id()).then(|| self.named(p.id())).flatten();
                known.unwrap_or_else(|| Spoken::Text(p.english().to_owned()))
            }
            MsgPart::Sound(c) => return self.named(c.sound()),
            MsgPart::Count(n) => Spoken::Text(n.to_string()),
            MsgPart::Pause(Pause::Short) => Spoken::Text(",".to_owned()),
            MsgPart::Pause(Pause::Long) => Spoken::Text(".".to_owned()),
            MsgPart::Literal(t) => Spoken::Text(t.clone()),
            MsgPart::Name(name) => return self.named(name),
        })
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
        self.get_sound(cue.sound())
    }

    pub fn text(&self, key: &str) -> Option<String> {
        self.0.borrow().text(key).map(str::to_owned)
    }

    pub fn has(&self, name: &str) -> bool {
        self.0.borrow().has(name)
    }

    pub fn rank(&self, name: &str) -> usize {
        self.0.borrow().rank(name)
    }

    pub fn translate(&self, part: &MsgPart) -> Option<Spoken> {
        self.0.borrow_mut().translate(part)
    }

    pub fn update_volumes(&self, params: Arc<Parameters>) {
        self.0.borrow_mut().update_volumes(params);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::arithmetic_side_effects)]
mod tests {
    use super::*;

    fn file(text: &str) -> TextFile {
        TextFile {
            path: "a map/text/en.txt".to_owned(),
            text: text.to_owned(),
        }
    }

    fn said(cache: &mut SoundCache, part: MsgPart) -> Option<String> {
        cache.translate(&part).map(|spoken| match spoken {
            Spoken::Text(t) => t,
            Spoken::Sound(_) => "a sound".to_owned(),
        })
    }

    fn name(s: &str) -> MsgPart {
        MsgPart::Name(s.to_owned())
    }

    #[test]
    fn a_language_file_is_lines_of_name_and_words() {
        let text =
            "# The orders.\n\nwhat_are_your_orders = What are your orders?\r\nat  =  a = b \n";
        assert_eq!(
            read_language_file(text),
            Ok(vec![
                ("what_are_your_orders", "What are your orders?"),
                ("at", "a = b")
            ])
        );
        for (bad, line) in [
            ("a = b\nhello\n", "line 2: not 'name = words'"),
            ("5 = five\n", "line 1: the name"),
            ("Hello = hi\n", "line 1: the name"),
            ("voice/own = hi\n", "line 1: the name"),
            ("a =\n", "line 1: no words"),
            ("a = b\n  continued\n", "line 2: not 'name = words'"),
            (" # indented\n", "line 1: not 'name = words'"),
        ] {
            let error = read_language_file(bad).unwrap_err();
            assert!(error.starts_with(line), "{bad:?}: {error}");
        }
        let big = "a = b\n".repeat(MAX_LANGUAGE_FILE_BYTES / 6 + 1);
        assert!(read_language_file(&big).unwrap_err().contains("bytes"));
    }

    #[test]
    fn a_language_file_with_a_bad_line_is_refused_whole() {
        let log = Log::silent();
        log.keep_errors();
        let params = Arc::new(Parameters::default());
        let good = file("footman = footman\n");
        let bad = file("knight = knight\nPress Alt F4 now\n");
        let mut cache = SoundCache::new(&[good, bad], &[], params, log.clone());
        assert_eq!(
            said(&mut cache, name("footman")).as_deref(),
            Some("footman")
        );
        assert_eq!(said(&mut cache, name("knight")), None);
        assert_eq!(
            log.take_errors(),
            ["a map/text/en.txt is refused: line 2: not 'name = words'"]
        );
    }

    #[test]
    fn a_downloaded_phrase_cannot_take_a_name_of_the_game() {
        let log = Log::silent();
        let params = Arc::new(Parameters::default());
        let game = file("what_are_your_orders = What are your orders?\n");
        let mut cache = SoundCache::new(&[game], &[], params, log);
        let added =
            file("what_are_your_orders = Press Alt F4.\ngolem = golem\nare_welcome = go away\n");
        assert_eq!(cache.add_downloaded(&[added], &[]), 2);
        assert_eq!(
            said(&mut cache, name("what_are_your_orders")).as_deref(),
            Some("What are your orders?")
        );
        assert_eq!(said(&mut cache, name("golem")).as_deref(), Some("golem"));
        // A phrase of the port, which no language file has yet.
        assert_eq!(
            said(&mut cache, Phrase::AreWelcome.into()).as_deref(),
            Some("welcome")
        );
        assert_eq!(said(&mut cache, MsgPart::Count(5)).as_deref(), Some("5"));
    }

    #[test]
    fn a_name_that_names_nothing_says_nothing() {
        let params = Arc::new(Parameters::default());
        let mut cache = SoundCache::new(&[], &[], params, Log::silent());
        assert_eq!(said(&mut cache, name("press_alt_f4_now")), None);
        assert_eq!(said(&mut cache, name("press_alt_f4_now")), None);
        assert_eq!(said(&mut cache, MsgPart::Sound(Cue::Beep)), None);
        assert_eq!(cache.missing.len(), 2);
    }

    #[test]
    fn a_phrase_is_its_words_in_the_resources_and_else_its_english() {
        let log = Log::silent();
        let params = Arc::new(Parameters::default());
        let french = file("awaiting_your_orders = en attente de vos ordres\n");
        let mut cache = SoundCache::new(&[french], &[], params, log);
        let said = |cache: &mut SoundCache, part: MsgPart| said(cache, part).unwrap();
        assert_eq!(
            said(&mut cache, Phrase::AwaitingYourOrders.into()),
            "en attente de vos ordres"
        );
        // The resources lack this phrase.
        assert_eq!(said(&mut cache, Phrase::Welcome.into()), "welcome");
        assert_eq!(said(&mut cache, Pause::Short.into()), ",");
        assert_eq!(said(&mut cache, Pause::Long.into()), ".");
        assert_eq!(
            said(&mut cache, MsgPart::literal("awaiting_your_orders")),
            "awaiting_your_orders"
        );
        assert_eq!(cache.rank("awaiting_your_orders"), 0);
        assert_eq!(cache.rank("welcome"), usize::MAX);
    }
}
