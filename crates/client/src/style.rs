//! `ui/style.txt`: how each kind of object sounds and is named.

use std::cell::RefCell;
use std::collections::HashSet;

use indexmap::IndexMap;
use soundrts_world::defs::preprocess;

use crate::log::Log;

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

impl StyleValue {
    /// Whether the value says anything (upstream's truth value).
    pub fn is_set(&self) -> bool {
        match self {
            StyleValue::Words(w) => !w.is_empty(),
            StyleValue::IfMe(..) => true,
            StyleValue::Noise(n) => *n != Noise::Silent,
        }
    }
}

/// Reads a noise definition.
pub fn parse_noise(words: &[String]) -> Result<Noise, String> {
    let Some(first) = words.first() else {
        return Ok(Noise::Silent);
    };
    let (ambient, words) = if first == "ambient" {
        (true, &words[1..])
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
                sounds: words[2..].to_vec(),
                ambient,
            })
        }
        None if ambient => Err("ambient without a noise".to_owned()),
        _ => Ok(Noise::Silent),
    }
}

/// The style definitions: objects with attributes, inheriting through
/// `is_a`.
#[derive(Default)]
pub struct Style {
    dict: IndexMap<String, IndexMap<String, StyleValue>>,
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
        self.dict.clear();
        for s in texts {
            self.read(s);
        }
        for object in self.dict.values_mut() {
            for (attr, value) in object.iter_mut() {
                let StyleValue::Words(words) = value else {
                    continue;
                };
                let words = std::mem::take(words);
                let if_me = words.first().is_some_and(|w| w == "if_me");
                let (then, otherwise) = if if_me {
                    match words.iter().position(|w| w == "else") {
                        Some(i) => (words[1..i].to_vec(), words[i + 1..].to_vec()),
                        None => (words[1..].to_vec(), Vec::new()),
                    }
                } else {
                    (words, Vec::new())
                };
                *value = if attr.starts_with("noise") {
                    let parse = |w: &[String]| {
                        parse_noise(w).unwrap_or_else(|e| {
                            self.log
                                .warning(format!("problem with noise: {} ({e})", w.join(" ")));
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
        self.apply_inheritance();
    }

    fn read(&mut self, s: &str) {
        let s = preprocess(s);
        let mut name = "(the name is missing)".to_owned();
        for line in s.split('\n') {
            let words: Vec<String> = line.split_whitespace().map(str::to_owned).collect();
            let Some(first) = words.first() else {
                continue;
            };
            match first.as_str() {
                "clear" => self.dict.clear(),
                "def" => match words.get(1) {
                    Some(n) => {
                        name = n.clone();
                        self.dict.entry(name.clone()).or_default();
                    }
                    None => self
                        .log
                        .warning(format!("error in definition of {name}: {line}")),
                },
                _ => match self.dict.get_mut(&name) {
                    Some(object) => {
                        object.insert(first.clone(), StyleValue::Words(words[1..].to_vec()));
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

    /// Copies each missing attribute from the parents, until nothing changes.
    fn apply_inheritance(&mut self) {
        let mut modified = true;
        while modified {
            modified = false;
            let names: Vec<String> = self.dict.keys().cloned().collect();
            for ko in names {
                for p in self.parents(&ko) {
                    let Some(parent) = self.dict.get(&p) else {
                        self.log
                            .warning(format!("error in {ko}.is_a: {p} doesn't exist"));
                        continue;
                    };
                    let parent = parent.clone();
                    let object = self.dict.get_mut(&ko).expect("listed object");
                    for (k, v) in parent {
                        if !object.contains_key(&k) {
                            object.insert(k, v);
                            modified = true;
                        }
                    }
                }
            }
        }
    }

    fn val(&self, object: &str, attr: &str) -> Option<&StyleValue> {
        let o = self.dict.get(object)?;
        if let Some(v) = o.get(attr) {
            return Some(v);
        }
        self.parents(object)
            .iter()
            .filter(|p| self.dict.contains_key(p.as_str()))
            .find_map(|p| self.val(p, attr))
    }

    /// The value of `object.attr`. An attribute `x_level_N` falls back to `x`.
    pub fn value(&self, object: &str, attr: &str) -> Option<&StyleValue> {
        let v = self.val(object, attr);
        if v.is_none() {
            let n = attr.len();
            if n >= 8
                && attr.is_char_boundary(n - 8)
                && attr.is_char_boundary(n - 1)
                && &attr[n - 8..n - 1] == "_level_"
            {
                return self.val(object, &attr[..n - 8]);
            }
        }
        v
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
}

#[cfg(test)]
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
}
