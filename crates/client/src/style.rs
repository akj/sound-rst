//! `ui/style.txt`: how each kind of object sounds and is named.
//!
//! A map from a server brings its own style, so a text or a style over one
//! of the limits below is cut short with a warning, never followed into a
//! stall.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::arithmetic_side_effects
)]

use std::cell::RefCell;
use std::collections::HashSet;

use indexmap::IndexMap;
use soundrts_world::defs::preprocess;

use crate::log::Log;

/// The largest style text of one layer, in bytes; a larger one is ignored
/// whole. The original's largest, a mod's, is 27 KB.
pub const MAX_STYLE_BYTES: usize = 1024 * 1024;

/// The most objects the style may define, all layers together. The
/// original's style, with every mod, defines 515.
pub const MAX_STYLE_OBJECTS: usize = 5_000;

/// The most parents (`is_a` words) an object may list; the others are
/// ignored. The original's objects list one.
pub const MAX_STYLE_PARENTS: usize = 8;

/// How many generations inheritance reaches whatever order the objects
/// come in: it passes over them at most once more than this, and each pass
/// copies attributes at least one generation down. The original's deepest
/// object has 7 generations of ancestors.
pub const MAX_STYLE_DEPTH: usize = 16;

/// The most bytes all the objects may hold together once they inherit,
/// counted as [`Style::size`] counts them. Inheritance copies values, so
/// this bounds the memory a style takes. The original's style, with every
/// mod, comes to 235 KB.
pub const MAX_STYLE_SIZE: usize = 16 * 1024 * 1024;

type Object = IndexMap<String, StyleValue>;
type Objects = IndexMap<String, Object>;

/// The bytes of an attribute: its name and words, and one for each.
fn attribute_size(attr: &str, value: &StyleValue) -> usize {
    fn words(w: &[String]) -> usize {
        w.iter()
            .fold(0_usize, |n, w| n.saturating_add(w.len()).saturating_add(1))
    }
    fn noise(n: &Noise) -> usize {
        match n {
            Noise::Silent => 0,
            Noise::Loop { sound, .. } => sound.len(),
            Noise::Repeat { sounds, .. } => words(sounds),
            Noise::IfMe(a, b) => noise(a).saturating_add(noise(b)),
            Noise::Invalid(w) => words(w),
        }
    }
    let value = match value {
        StyleValue::Words(w) => words(w),
        StyleValue::IfMe(a, b) => words(a).saturating_add(words(b)),
        StyleValue::Noise(n) => noise(n),
    };
    attr.len().saturating_add(1).saturating_add(value)
}

/// The value of `attr` in `object`. An attribute `x_level_N` falls back to
/// `x`.
fn lookup<'a>(object: &'a Object, attr: &str) -> Option<&'a StyleValue> {
    object.get(attr).or_else(|| {
        let (head, _) = attr.split_at_checked(attr.len().checked_sub(1)?)?;
        object.get(head.strip_suffix("_level_")?)
    })
}

/// A background noise: what an object sounds like while it exists.
#[derive(Clone, Debug, PartialEq)]
pub enum Noise {
    /// Nothing (an empty or unknown noise definition).
    Silent,
    /// A sound played in a loop.
    Loop {
        sound: String,
        volume: f64,
        ambient: bool,
    },
    /// One of `sounds`, about every `interval` seconds.
    Repeat {
        interval: f64,
        sounds: Vec<String>,
        ambient: bool,
    },
    /// One noise for the player's side, another for the others.
    IfMe(Box<Noise>, Box<Noise>),
    /// A definition that could not be read; it is not silent, but plays
    /// nothing.
    Invalid(Vec<String>),
}

/// A style value.
#[derive(Clone, Debug, PartialEq)]
pub enum StyleValue {
    Words(Vec<String>),
    /// `if_me <words> else <words>`: the first words for the player's side.
    IfMe(Vec<String>, Vec<String>),
    /// An attribute whose name starts with `noise`.
    Noise(Noise),
}

/// Reads a noise definition.
pub fn parse_noise(words: &[String]) -> Result<Noise, String> {
    let Some(first) = words.first() else {
        return Ok(Noise::Silent);
    };
    let (ambient, words) = if first == "ambient" {
        (true, words.get(1..).unwrap_or_default())
    } else {
        (false, words)
    };
    match words.first().map(String::as_str) {
        Some("loop") => {
            let sound = words.get(1).ok_or("loop without a sound")?.clone();
            let volume = match words.get(2) {
                Some(v) => v.parse().map_err(|_| format!("bad volume: {v}"))?,
                None => 1.0,
            };
            Ok(Noise::Loop {
                sound,
                volume,
                ambient,
            })
        }
        Some("repeat") => {
            let interval = words.get(1).ok_or("repeat without an interval")?;
            Ok(Noise::Repeat {
                interval: interval
                    .parse()
                    .map_err(|_| format!("bad interval: {interval}"))?,
                sounds: words.get(2..).unwrap_or_default().to_vec(),
                ambient,
            })
        }
        None if ambient => Err("ambient without a noise".to_owned()),
        _ => Ok(Noise::Silent),
    }
}

/// Turns the words of each attribute into its value: an `if_me` pair, a
/// noise, or words.
fn convert(dict: &mut Objects, log: &Log) {
    for object in dict.values_mut() {
        for (attr, value) in object.iter_mut() {
            let StyleValue::Words(words) = value else {
                continue;
            };
            let words = std::mem::take(words);
            let if_me = words.first().is_some_and(|w| w == "if_me");
            let (then, otherwise) = if if_me {
                let rest = words.get(1..).unwrap_or_default();
                match rest.iter().position(|w| w == "else") {
                    Some(i) => {
                        let (then, otherwise) = rest.split_at(i);
                        (
                            then.to_vec(),
                            otherwise.get(1..).unwrap_or_default().to_vec(),
                        )
                    }
                    None => (rest.to_vec(), Vec::new()),
                }
            } else {
                (words, Vec::new())
            };
            *value = if attr.starts_with("noise") {
                let parse = |w: &[String]| {
                    parse_noise(w).unwrap_or_else(|e| {
                        log.warning(format!("problem with noise: {} ({e})", w.join(" ")));
                        Noise::Invalid(w.to_vec())
                    })
                };
                if if_me {
                    StyleValue::Noise(Noise::IfMe(
                        Box::new(parse(&then)),
                        Box::new(parse(&otherwise)),
                    ))
                } else {
                    StyleValue::Noise(parse(&then))
                }
            } else if if_me {
                StyleValue::IfMe(then, otherwise)
            } else {
                StyleValue::Words(then)
            };
        }
    }
}

/// The style definitions: objects with attributes, inheriting through
/// `is_a`.
#[derive(Default)]
pub struct Style {
    dict: Objects,
    warned: RefCell<HashSet<(String, String)>>,
    log: Log,
}

impl Style {
    pub fn new(log: Log) -> Style {
        Style {
            log,
            ..Style::default()
        }
    }

    /// Reads definitions; each text adds to or overrides the previous ones.
    pub fn load(&mut self, texts: &[&str]) {
        self.dict = self.parse(texts);
        self.apply_inheritance();
    }

    /// Adds the style of a map that came from a server: the attributes
    /// this style lacks, its own, inherited or by level, and nothing else,
    /// even through inheritance. Returns how many attributes were ignored
    /// because the style has them.
    pub fn add_downloaded(&mut self, texts: &[&str]) -> usize {
        let before = self.dict.clone();
        let mut ignored: usize = 0;
        for (name, attributes) in self.parse(texts) {
            if !self.dict.contains_key(&name) && self.dict.len() >= MAX_STYLE_OBJECTS {
                self.log.warning(format!(
                    "style: more than {MAX_STYLE_OBJECTS} objects; {name} ignored"
                ));
                continue;
            }
            let fresh: Vec<(String, StyleValue)> = attributes
                .into_iter()
                .filter(|(attr, _)| {
                    let has = self.value(&name, attr).is_some();
                    if has {
                        ignored = ignored.saturating_add(1);
                    }
                    !has
                })
                .collect();
            self.dict.entry(name).or_default().extend(fresh);
        }
        self.apply_inheritance();
        // An attribute added to a parent reaches the children, where it can
        // stand in front of a value they had by level.
        for (name, object) in &mut self.dict {
            if let Some(old) = before.get(name) {
                object.retain(|attr, _| old.contains_key(attr) || lookup(old, attr).is_none());
            }
        }
        ignored
    }

    /// The objects of `texts`, each text adding to or overriding the
    /// previous ones, with their values read but not yet inherited.
    fn parse(&self, texts: &[&str]) -> Objects {
        let mut dict = Objects::new();
        for s in texts {
            if s.len() > MAX_STYLE_BYTES {
                self.log.warning(format!(
                    "style: a text of {} bytes, more than {MAX_STYLE_BYTES}, ignored",
                    s.len()
                ));
                continue;
            }
            self.read(&mut dict, s);
        }
        convert(&mut dict, &self.log);
        for (name, object) in &mut dict {
            if let Some(StyleValue::Words(parents)) = object.get_mut("is_a")
                && parents.len() > MAX_STYLE_PARENTS
            {
                self.log.warning(format!(
                    "style: {name} has more than {MAX_STYLE_PARENTS} parents; the others are ignored"
                ));
                parents.truncate(MAX_STYLE_PARENTS);
            }
        }
        dict
    }

    fn read(&self, dict: &mut Objects, s: &str) {
        let s = preprocess(s);
        let mut name = "(the name is missing)".to_owned();
        // After an object refused for the limit, its lines are ignored
        // without a warning each.
        let mut refused = false;
        for line in s.split('\n') {
            let words: Vec<String> = line.split_whitespace().map(str::to_owned).collect();
            let Some((first, rest)) = words.split_first() else {
                continue;
            };
            match first.as_str() {
                "clear" => dict.clear(),
                "def" => match rest.first() {
                    Some(n) if dict.contains_key(n) || dict.len() < MAX_STYLE_OBJECTS => {
                        name = n.clone();
                        refused = false;
                        dict.entry(name.clone()).or_default();
                    }
                    Some(n) => {
                        self.log.warning(format!(
                            "style: more than {MAX_STYLE_OBJECTS} objects; {n} ignored"
                        ));
                        refused = true;
                    }
                    None => self
                        .log
                        .warning(format!("error in definition of {name}: {line}")),
                },
                _ if refused => {}
                _ => match dict.get_mut(&name) {
                    Some(object) => {
                        object.insert(first.clone(), StyleValue::Words(rest.to_vec()));
                    }
                    None => self
                        .log
                        .warning(format!("error in definition of {name}: {line}")),
                },
            }
        }
    }

    fn parents(&self, object: &str) -> Vec<String> {
        match self.dict.get(object).and_then(|o| o.get("is_a")) {
            Some(StyleValue::Words(w)) => w.clone(),
            _ => Vec::new(),
        }
    }

    /// Copies each missing attribute from the parents, until nothing
    /// changes, at most [`MAX_STYLE_DEPTH`] times over and until the style
    /// holds [`MAX_STYLE_SIZE`].
    fn apply_inheritance(&mut self) {
        let mut total = self.size();
        for _ in 0..=MAX_STYLE_DEPTH {
            let mut modified = false;
            let names: Vec<String> = self.dict.keys().cloned().collect();
            for ko in names {
                for p in self.parents(&ko) {
                    let (Some(parent), Some(object)) = (self.dict.get(&p), self.dict.get(&ko))
                    else {
                        self.log
                            .warning(format!("error in {ko}.is_a: {p} doesn't exist"));
                        continue;
                    };
                    let missing: Vec<(String, StyleValue)> = parent
                        .iter()
                        .filter(|(k, _)| !object.contains_key(*k))
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect();
                    if missing.is_empty() {
                        continue;
                    }
                    total = missing
                        .iter()
                        .fold(total, |n, (k, v)| n.saturating_add(attribute_size(k, v)));
                    if total > MAX_STYLE_SIZE {
                        self.log.warning(format!(
                            "style: more than {MAX_STYLE_SIZE} bytes once inherited; inheritance stops"
                        ));
                        return;
                    }
                    if let Some(object) = self.dict.get_mut(&ko) {
                        object.extend(missing);
                        modified = true;
                    }
                }
            }
            if !modified {
                return;
            }
        }
        self.log.warning(format!(
            "style: inheritance deeper than {MAX_STYLE_DEPTH} generations; it stops there"
        ));
    }

    /// The value of `object.attr`. An attribute `x_level_N` falls back to
    /// `x`. Inheritance has copied every attribute of the ancestors into
    /// the object, so the object alone is looked at.
    pub fn value(&self, object: &str, attr: &str) -> Option<&StyleValue> {
        lookup(self.dict.get(object)?, attr)
    }

    fn warn_once(&self, object: &str, attr: &str) {
        let key = (object.to_owned(), attr.to_owned());
        if self.warned.borrow_mut().insert(key) {
            self.log.warning(format!(
                "no value found for {object}.{attr} (check style.txt)"
            ));
        }
    }

    /// The value, or a warning (once per attribute) and `None`.
    pub fn get(&self, object: &str, attr: &str) -> Option<&StyleValue> {
        self.get_or_warn(object, attr, true)
    }

    pub fn get_or_warn(
        &self,
        object: &str,
        attr: &str,
        warn_if_not_found: bool,
    ) -> Option<&StyleValue> {
        let v = self.value(object, attr);
        if v.is_none() && warn_if_not_found {
            self.warn_once(object, attr);
        }
        v
    }

    /// The words of `object.attr`, or none (with a warning once). For an
    /// `if_me` value, the words for the player's side.
    pub fn words(&self, object: &str, attr: &str) -> Vec<String> {
        match self.get(object, attr) {
            Some(StyleValue::Words(w)) | Some(StyleValue::IfMe(w, _)) => w.clone(),
            _ => Vec::new(),
        }
    }

    /// The noise of `object.attr`, silent if none.
    pub fn noise(&self, object: &str, attr: &str) -> Noise {
        match self.get(object, attr) {
            Some(StyleValue::Noise(n)) => n.clone(),
            _ => Noise::Silent,
        }
    }

    pub fn has(&self, object: &str, attr: &str) -> bool {
        self.value(object, attr).is_some()
    }

    /// The names of the defined objects, in definition order.
    pub fn objects(&self) -> impl Iterator<Item = &str> {
        self.dict.keys().map(String::as_str)
    }

    /// How many bytes the objects hold together, inherited attributes
    /// included: each attribute's name and words, and one for each.
    pub fn size(&self) -> usize {
        self.dict
            .values()
            .flat_map(|o| o.iter())
            .fold(0_usize, |n, (k, v)| n.saturating_add(attribute_size(k, v)))
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

    fn words(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn noises_are_parsed_as_upstream() {
        assert_eq!(parse_noise(&[]), Ok(Noise::Silent));
        assert_eq!(
            parse_noise(&words("loop 1001")),
            Ok(Noise::Loop {
                sound: "1001".into(),
                volume: 1.0,
                ambient: false
            })
        );
        assert_eq!(
            parse_noise(&words("ambient loop 1001 0.5")),
            Ok(Noise::Loop {
                sound: "1001".into(),
                volume: 0.5,
                ambient: true
            })
        );
        assert_eq!(
            parse_noise(&words("repeat 5 1 2")),
            Ok(Noise::Repeat {
                interval: 5.0,
                sounds: words("1 2"),
                ambient: false
            })
        );
        assert_eq!(parse_noise(&words("other 5")), Ok(Noise::Silent));
        assert!(parse_noise(&words("repeat x")).is_err());
    }

    #[test]
    fn attributes_inherit_and_if_me_splits() {
        let mut style = Style::new(Log::silent());
        style.load(&[
            "def unit\ntitle 1\nnoise loop 5\n\ndef footman\nis_a unit\nalert if_me 7 else 8\nnoise_when_moving if_me repeat 2 3 else loop 4\n",
            "def footman\nattack_hit 9\n",
        ]);
        assert_eq!(style.words("footman", "title"), ["1"]);
        assert_eq!(style.words("footman", "attack_hit_level_2"), ["9"]);
        assert_eq!(
            style.get("footman", "alert"),
            Some(&StyleValue::IfMe(words("7"), words("8")))
        );
        assert_eq!(
            style.noise("footman", "noise"),
            Noise::Loop {
                sound: "5".into(),
                volume: 1.0,
                ambient: false
            }
        );
        assert_eq!(
            style.noise("footman", "noise_when_moving"),
            Noise::IfMe(
                Box::new(Noise::Repeat {
                    interval: 2.0,
                    sounds: words("3"),
                    ambient: false
                }),
                Box::new(Noise::Loop {
                    sound: "4".into(),
                    volume: 1.0,
                    ambient: false
                })
            )
        );
        assert!(style.has("footman", "title"));
        assert!(!style.has("footman", "nothing"));
        assert_eq!(style.words("footman", "nothing"), Vec::<String>::new());
    }

    #[test]
    fn a_type_that_is_its_own_ancestor_has_no_missing_attribute() {
        let mut style = Style::new(Log::silent());
        style.load(&["def a\ntitle 1\nis_a a b b\n\ndef b\nis_a a a\n"]);
        assert_eq!(style.words("a", "title"), ["1"]);
        assert_eq!(style.words("b", "title"), ["1"]);
        assert_eq!(style.value("a", "nothing"), None);
        assert_eq!(style.value("b", "nothing_level_2"), None);
    }

    #[test]
    fn a_style_text_over_the_limit_is_ignored_whole() {
        let mut style = Style::new(Log::silent());
        let big = format!("def a\ntitle 2\n{}", " ".repeat(MAX_STYLE_BYTES));
        style.load(&["def a\ntitle 1\n", &big]);
        assert_eq!(style.words("a", "title"), ["1"]);
    }

    #[test]
    fn objects_past_the_limit_are_ignored() {
        let text: String = (0..=MAX_STYLE_OBJECTS)
            .map(|i| format!("def o{i}\ntitle {i}\n"))
            .collect();
        let mut style = Style::new(Log::silent());
        style.load(&[&text]);
        assert_eq!(style.objects().count(), MAX_STYLE_OBJECTS);
        assert_eq!(style.words("o4999", "title"), ["4999"]);
        assert!(!style.has("o5000", "title"));
    }

    #[test]
    fn parents_past_the_limit_are_ignored() {
        let mut style = Style::new(Log::silent());
        style.load(&[
            "def p1\na 1\ndef p8\nb 8\ndef p9\nc 9\ndef o\nis_a p1 p2 p3 p4 p5 p6 p7 p8 p9\n",
        ]);
        assert!(style.has("o", "a"));
        assert!(style.has("o", "b"));
        assert!(!style.has("o", "c"));
    }

    #[test]
    fn inheritance_stops_after_the_limit_of_generations() {
        // Each object is defined before its parent, so each pass over the
        // objects copies the title one generation down.
        let n = 40;
        let mut text: String = (0..n)
            .map(|i| format!("def a{i}\nis_a a{}\n", i + 1))
            .collect();
        text.push_str(&format!("def a{n}\ntitle 1\n"));
        let mut style = Style::new(Log::silent());
        style.load(&[&text]);
        assert!(style.has(&format!("a{}", n - 1), "title"));
        assert!(style.has(&format!("a{}", n - MAX_STYLE_DEPTH - 1), "title"));
        assert!(!style.has(&format!("a{}", n - MAX_STYLE_DEPTH - 2), "title"));
        assert!(!style.has("a0", "title"));
    }

    #[test]
    fn inheritance_stops_at_the_limit_of_bytes() {
        // One attribute of 64 KB, which 5,000 children would copy.
        let mut text = format!("def parent\nbig {}\n", "x ".repeat(32 * 1024));
        for i in 0..4999 {
            text.push_str(&format!("def child{i}\nis_a parent\n"));
        }
        let mut style = Style::new(Log::silent());
        style.load(&[&text]);
        assert!(style.size() <= MAX_STYLE_SIZE);
        assert!(style.has("child0", "big"));
        assert!(!style.has("child4998", "big"));
    }

    #[test]
    fn a_downloaded_style_adds_and_never_replaces() {
        let mut style = Style::new(Log::silent());
        style.load(&["def unit\ntitle 1\nattack_hit 5\n\ndef footman\nis_a unit\n"]);
        let ignored = style.add_downloaded(&[
            "clear\ndef footman\ntitle 666\nattack_hit_level_2 666\nnoise loop 7\n\
             def golem\nis_a footman\ntitle 9000\n\
             def unit\nmove 8\n",
        ]);
        assert_eq!(ignored, 2);
        // What the game has stays: its own, inherited and by level.
        assert_eq!(style.words("footman", "title"), ["1"]);
        assert_eq!(style.words("footman", "attack_hit_level_2"), ["5"]);
        // What it lacks is added, and inherited.
        assert_eq!(
            style.noise("footman", "noise"),
            Noise::Loop {
                sound: "7".into(),
                volume: 1.0,
                ambient: false
            }
        );
        assert_eq!(style.words("golem", "title"), ["9000"]);
        assert_eq!(style.words("golem", "attack_hit"), ["5"]);
        assert_eq!(style.words("footman", "move"), ["8"]);
    }

    #[test]
    fn an_attribute_a_downloaded_style_adds_to_a_parent_does_not_replace_a_childs_level() {
        let mut style = Style::new(Log::silent());
        style.load(&["def unit\ntitle 1\n\ndef footman\nis_a unit\nattack_hit 5\n"]);
        let ignored = style.add_downloaded(&["def unit\nattack_hit_level_2 666\n"]);
        assert_eq!(ignored, 0);
        assert_eq!(style.words("unit", "attack_hit_level_2"), ["666"]);
        assert_eq!(style.words("footman", "attack_hit_level_2"), ["5"]);
    }
}
