//! `ui/style.txt`: how each kind of object sounds and is named.
//!
//! The style gives each type a [`TypeStyle`]: one field for each attribute
//! that the game reads. An attribute is read into its field when the style
//! loads, so a word that names no phrase and no sound, or an attribute that
//! the game does not have, is refused there, with its file and its line.
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

use std::collections::BTreeMap;

use indexmap::IndexMap;
use soundrts_world::defs::{preprocess, source_lines};
use soundrts_world::events::Reason;
use soundrts_world::msg::{Msg, MsgPart, is_name};

use crate::log::Log;
use crate::package::TextFile;

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

/// The most lines of one style text that are reported as refused. The rest
/// are refused without a line in the log.
const MAX_REPORTS: usize = 100;

/// What an attribute says: the phrases and sounds it names, and counts. A
/// `$1` is kept as written: the place of a title's argument.
pub type Said = Vec<MsgPart>;

/// The sounds of an attribute, by name. The game plays one of them.
pub type Sounds = Vec<String>;

/// A value that is the same for everyone, or `if_me <one> else <another>`:
/// one for the player's side and one for the others.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sided<T> {
    mine: T,
    others: Option<T>,
}

impl<T> Sided<T> {
    /// The value for everyone.
    pub fn any(value: T) -> Sided<T> {
        Sided {
            mine: value,
            others: None,
        }
    }

    /// The value for the player's side, and for the others.
    pub fn if_me(mine: T, others: T) -> Sided<T> {
        Sided {
            mine,
            others: Some(others),
        }
    }

    /// The value for the player's side.
    pub fn mine(&self) -> &T {
        &self.mine
    }

    /// The value for a side: the player's and its allies', or the others'.
    pub fn of(&self, allied: bool) -> &T {
        match &self.others {
            Some(others) if !allied => others,
            _ => &self.mine,
        }
    }
}

/// The value of an attribute for the player's side. Empty without one.
pub fn mine<T: Clone + Default>(value: &Option<Sided<T>>) -> T {
    value.as_ref().map(|v| v.mine().clone()).unwrap_or_default()
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

/// Where an attribute is being read, and what its words may name.
struct Reader<'a> {
    file: &'a str,
    line: usize,
    object: &'a str,
    attr: &'a str,
    exists: &'a dyn Fn(&str) -> bool,
    log: &'a Log,
    reports: &'a mut usize,
}

impl Reader<'_> {
    /// Reports a word or an attribute that is refused, with its file and
    /// its line.
    fn refuse(&mut self, what: &str) {
        *self.reports = self.reports.saturating_add(1);
        if *self.reports <= MAX_REPORTS {
            self.log.error(format!(
                "{} line {}: {}.{}: {what}",
                self.file, self.line, self.object, self.attr
            ));
        }
    }

    /// Whether `word` is a name that exists. If not, it is refused.
    fn names(&mut self, word: &str) -> bool {
        if !is_name(word) {
            self.refuse(&format!("{word} is not a name"));
            false
        } else if !(self.exists)(word) {
            self.refuse(&format!("{word} names no phrase and no sound"));
            false
        } else {
            true
        }
    }

    fn said(&mut self, words: &[String]) -> Said {
        let mut said = Vec::with_capacity(words.len());
        for word in words {
            if word.starts_with('$') {
                said.push(MsgPart::literal(word.as_str()));
                continue;
            }
            match MsgPart::named(word) {
                Some(MsgPart::Name(name)) => {
                    if self.names(&name) {
                        said.push(MsgPart::Name(name));
                    }
                }
                Some(part) => said.push(part),
                None => self.refuse(&format!("{word} is not a name or a count")),
            }
        }
        said
    }

    fn sounds(&mut self, words: &[String]) -> Sounds {
        let mut sounds = words.to_vec();
        sounds.retain(|word| self.names(word));
        sounds
    }

    fn noise(&mut self, words: &[String]) -> Noise {
        match parse_noise(words) {
            Ok(Noise::Loop {
                sound,
                volume,
                ambient,
            }) => {
                if self.names(&sound) {
                    Noise::Loop {
                        sound,
                        volume,
                        ambient,
                    }
                } else {
                    Noise::Silent
                }
            }
            Ok(Noise::Repeat {
                interval,
                sounds,
                ambient,
            }) => Noise::Repeat {
                interval,
                sounds: self.sounds(&sounds),
                ambient,
            },
            Ok(noise) => noise,
            Err(e) => {
                self.refuse(&format!("problem with noise: {} ({e})", words.join(" ")));
                Noise::Invalid(words.to_vec())
            }
        }
    }

    /// `if_me <words> else <words>`, or the same words for everyone.
    fn sided<T>(&mut self, words: &[String], read: fn(&mut Self, &[String]) -> T) -> Sided<T> {
        match if_me(words) {
            Some((mine, others)) => Sided::if_me(read(self, mine), read(self, others)),
            None => Sided::any(read(self, words)),
        }
    }

    fn sided_noise(&mut self, words: &[String]) -> Noise {
        match if_me(words) {
            Some((mine, others)) => {
                Noise::IfMe(Box::new(self.noise(mine)), Box::new(self.noise(others)))
            }
            None => self.noise(words),
        }
    }
}

/// The two sides of `if_me <words> else <words>`. Without `else`, the
/// others get no words.
fn if_me(words: &[String]) -> Option<(&[String], &[String])> {
    let rest = words.strip_prefix(&["if_me".to_owned()][..])?;
    Some(match rest.iter().position(|w| w == "else") {
        Some(i) => {
            let (mine, others) = rest.split_at(i);
            (mine, others.get(1..).unwrap_or_default())
        }
        None => (rest, &[]),
    })
}

/// The bytes that a value holds: its words, and one for each.
trait Size {
    fn size(&self) -> usize;
}

fn words_size<S: AsRef<str>>(words: &[S]) -> usize {
    words.iter().fold(0_usize, |n, w| {
        n.saturating_add(w.as_ref().len()).saturating_add(1)
    })
}

impl Size for Vec<String> {
    fn size(&self) -> usize {
        words_size(self)
    }
}

impl Size for Said {
    fn size(&self) -> usize {
        self.iter().fold(0_usize, |n, part| {
            let len = match part {
                MsgPart::Name(s) | MsgPart::Literal(s) => s.len(),
                _ => 8,
            };
            n.saturating_add(len).saturating_add(1)
        })
    }
}

impl<T: Size> Size for Sided<T> {
    fn size(&self) -> usize {
        let others = self.others.as_ref().map_or(0, Size::size);
        self.mine.size().saturating_add(others)
    }
}

impl Size for Noise {
    fn size(&self) -> usize {
        match self {
            Noise::Silent => 0,
            Noise::Loop { sound, .. } => sound.len(),
            Noise::Repeat { sounds, .. } => words_size(sounds),
            Noise::IfMe(a, b) => a.size().saturating_add(b.size()),
            Noise::Invalid(w) => words_size(w),
        }
    }
}

/// What copying the attributes that an object lacks from another did.
#[derive(Default)]
struct Filled {
    /// The bytes added, as [`Style::size`] counts them.
    bytes: usize,
    /// The attributes that the object had already.
    had: usize,
}

fn fill<T: Clone + Size>(to: &mut Option<T>, from: &Option<T>, name: &str, filled: &mut Filled) {
    match (to.is_some(), from) {
        (false, Some(value)) => {
            let size = name.len().saturating_add(1).saturating_add(value.size());
            filled.bytes = filled.bytes.saturating_add(size);
            *to = Some(value.clone());
        }
        (true, Some(_)) => filled.had = filled.had.saturating_add(1),
        (_, None) => {}
    }
}

/// As [`fill`], for the attributes of a family: one for each key.
fn fill_family<T: Clone + Size>(
    to: &mut BTreeMap<String, T>,
    from: &BTreeMap<String, T>,
    name: &str,
    has: impl Fn(&str) -> bool,
    filled: &mut Filled,
) {
    for (key, value) in from {
        if to.contains_key(key) || has(key) {
            filled.had = filled.had.saturating_add(1);
        } else {
            let size = name.len().saturating_add(key.len()).saturating_add(1);
            filled.bytes = filled
                .bytes
                .saturating_add(size.saturating_add(value.size()));
            to.insert(key.clone(), value.clone());
        }
    }
}

fn family_size<T: Size>(family: &BTreeMap<String, T>, name: &str) -> usize {
    family.iter().fold(0_usize, |n, (key, value)| {
        let size = name.len().saturating_add(key.len()).saturating_add(1);
        n.saturating_add(size).saturating_add(value.size())
    })
}

/// Declares [`TypeStyle`] from its attributes, by what each holds, as
/// `field = "name in the file"`.
macro_rules! type_style {
    (
        said { $( $(#[doc = $said_doc:literal])* $said:ident = $said_name:literal, )* }
        sounds { $( $(#[doc = $sounds_doc:literal])* $sounds:ident = $sounds_name:literal, )* }
        noises { $( $(#[doc = $noise_doc:literal])* $noise:ident = $noise_name:literal, )* }
        words { $( $(#[doc = $words_doc:literal])* $words:ident = $words_name:literal, )* }
    ) => {
        /// The style of a type: an order, a unit, a building, a terrain, or
        /// one of the objects `messages` and `parameters`. A field is
        /// `None` when no style gives the attribute to the type or to an
        /// ancestor of it.
        #[derive(Clone, Debug, Default, PartialEq)]
        pub struct TypeStyle {
            $( $(#[doc = $said_doc])* pub $said: Option<Sided<Said>>, )*
            $( $(#[doc = $sounds_doc])* pub $sounds: Option<Sided<Sounds>>, )*
            $( $(#[doc = $noise_doc])* pub $noise: Option<Noise>, )*
            $( $(#[doc = $words_doc])* pub $words: Option<Vec<String>>, )*
            /// `resource_<n>_title`: the name of resource `n`.
            pub resource_title: BTreeMap<String, Sided<Said>>,
            /// `not_enough_resource_<n>`: said when resource `n` is short.
            pub not_enough_resource: BTreeMap<String, Sided<Said>>,
            /// `proportion_<n>`: played for `n` tenths of something done.
            pub proportion: BTreeMap<String, Sided<Sounds>>,
            /// `store_resource_<n>`: played when resource `n` is brought in.
            pub store_resource: BTreeMap<String, Sided<Sounds>>,
            /// `attack_hit_level_<n>`: [`TypeStyle::attack_hit`] for an
            /// attacker whose damage is at level `n`.
            pub attack_hit_level: BTreeMap<String, Sided<Sounds>>,
            /// `move_on_<ground>`: the footsteps on a kind of ground. The
            /// kinds are those the `ground` attributes name.
            pub move_on: BTreeMap<String, Sided<Sounds>>,
            /// `noise_when_<activity>`: the noise while the unit is
            /// `building`, or `exploiting_<type of deposit>`.
            pub noise_when: BTreeMap<String, Noise>,
        }

        impl TypeStyle {
            /// The style of a type that no style names.
            const EMPTY: TypeStyle = TypeStyle {
                $( $said: None, )*
                $( $sounds: None, )*
                $( $noise: None, )*
                $( $words: None, )*
                resource_title: BTreeMap::new(),
                not_enough_resource: BTreeMap::new(),
                proportion: BTreeMap::new(),
                store_resource: BTreeMap::new(),
                attack_hit_level: BTreeMap::new(),
                move_on: BTreeMap::new(),
                noise_when: BTreeMap::new(),
            };

            /// Reads the words of one attribute into its field. False if
            /// the game has no attribute of this name.
            fn set(&mut self, attr: &str, words: &[String], r: &mut Reader) -> bool {
                match attr {
                    $( $said_name => self.$said = Some(r.sided(words, Reader::said)), )*
                    $( $sounds_name => self.$sounds = Some(r.sided(words, Reader::sounds)), )*
                    $( $noise_name => self.$noise = Some(r.sided_noise(words)), )*
                    $( $words_name => self.$words = Some(words.to_vec()), )*
                    _ => return self.set_in_family(attr, words, r),
                }
                true
            }

            /// Copies from `from` each attribute that this type lacks.
            /// With `by_level`, a level of `attack_hit` is lacking only if
            /// `attack_hit` is too.
            fn fill_from(&mut self, from: &TypeStyle, by_level: bool) -> Filled {
                let mut filled = Filled::default();
                let f = &mut filled;
                let has_level =
                    by_level && self.attack_hit.is_some();
                $( fill(&mut self.$said, &from.$said, $said_name, f); )*
                $( fill(&mut self.$sounds, &from.$sounds, $sounds_name, f); )*
                $( fill(&mut self.$noise, &from.$noise, $noise_name, f); )*
                $( fill(&mut self.$words, &from.$words, $words_name, f); )*
                let no = |_: &str| false;
                fill_family(&mut self.resource_title, &from.resource_title, RESOURCE_TITLE.0, no, f);
                fill_family(&mut self.not_enough_resource, &from.not_enough_resource, NOT_ENOUGH_RESOURCE, no, f);
                fill_family(&mut self.proportion, &from.proportion, PROPORTION, no, f);
                fill_family(&mut self.store_resource, &from.store_resource, STORE_RESOURCE, no, f);
                let level = |key: &str| has_level && key.chars().count() == 1;
                fill_family(&mut self.attack_hit_level, &from.attack_hit_level, ATTACK_HIT_LEVEL, level, f);
                fill_family(&mut self.move_on, &from.move_on, MOVE_ON, no, f);
                fill_family(&mut self.noise_when, &from.noise_when, NOISE_WHEN, no, f);
                filled
            }

            /// The bytes the attributes hold: for each, its name and its
            /// words, and one for each.
            fn size(&self) -> usize {
                let mut n = 0_usize;
                let mut add = |name: &str, size: Option<usize>| {
                    if let Some(size) = size {
                        n = n.saturating_add(name.len()).saturating_add(1).saturating_add(size);
                    }
                };
                $( add($said_name, self.$said.as_ref().map(Size::size)); )*
                $( add($sounds_name, self.$sounds.as_ref().map(Size::size)); )*
                $( add($noise_name, self.$noise.as_ref().map(Size::size)); )*
                $( add($words_name, self.$words.as_ref().map(Size::size)); )*
                n.saturating_add(family_size(&self.resource_title, RESOURCE_TITLE.0))
                    .saturating_add(family_size(&self.not_enough_resource, NOT_ENOUGH_RESOURCE))
                    .saturating_add(family_size(&self.proportion, PROPORTION))
                    .saturating_add(family_size(&self.store_resource, STORE_RESOURCE))
                    .saturating_add(family_size(&self.attack_hit_level, ATTACK_HIT_LEVEL))
                    .saturating_add(family_size(&self.move_on, MOVE_ON))
                    .saturating_add(family_size(&self.noise_when, NOISE_WHEN))
            }
        }
    };
}

type_style! {
    said {
        /// The name of the type. In the title of an order, `$1` is where
        /// the title of what the order builds, trains or uses goes.
        title = "title",
        /// Said after the title of an order in a menu.
        comment = "comment",
        /// Said when the unit arrives. `$1` is its title.
        added_msg = "added_msg",
        /// Said when the unit or the building is complete. `$1` is its
        /// title.
        complete_msg = "complete_msg",
        /// Said when the building has done a research.
        research_complete_msg = "research_complete_msg",
        /// Said for an exit on the way. A type with it is an exit.
        when_moving_through = "when_moving_through",
        /// Of `messages`: why an order was refused.
        the_queue_is_full = "the_queue_is_full",
        not_enough_food = "not_enough_food",
        population_limit_reached = "population_limit_reached",
        production_deferred = "production_deferred",
        cannot_build_here = "cannot_build_here",
        not_enough_mana = "not_enough_mana",
        not_enough_space = "not_enough_space",
        count_limit_reached = "count_limit_reached",
        /// Of `parameters`: words of the sentences the game makes.
        no_path_in_this_direction = "no_path_in_this_direction",
        requires = "requires",
        and = "and",
        food_title = "food_title",
    }
    sounds {
        /// Played far away for something the player should know of.
        alert = "alert",
        attack = "attack",
        /// Played on the target that this type hits.
        attack_hit = "attack_hit",
        launch_attack = "launch_attack",
        /// The footsteps, where no ground has its own.
        footsteps = "move",
        death = "death",
        complete = "complete",
        exhausted = "exhausted",
        added = "added",
        production = "production",
        flee = "flee",
        missed = "missed",
        healed = "healed",
        win_fight = "win_fight",
        lose_fight = "lose_fight",
        exit = "exit",
        enter = "enter",
        blocked = "blocked",
        order_ok = "order_ok",
        order_impossible = "order_impossible",
    }
    noises {
        noise = "noise",
        noise_if_damaged = "noise_if_damaged",
        noise_if_very_damaged = "noise_if_very_damaged",
    }
    words {
        /// The types that this one takes its missing attributes from.
        is_a = "is_a",
        /// The letter that picks the order or the type in a menu.
        shortcut = "shortcut",
        /// The place of the order in the menu. An order without one is
        /// not in the menu.
        index = "index",
        /// The name of the key that selects units of this type.
        keyboard = "keyboard",
        /// The kind of ground of a terrain or of an object, then how far
        /// it reaches compared to others.
        ground = "ground",
        /// Of `parameters`: the maps listed first.
        recommended_maps = "recommended_maps",
    }
}

/// The attributes that the original's styles give and the game does not
/// read. They are accepted, and nothing is done with them.
const UNREAD: [&str; 3] = ["color", "shape", "can_upgrade_to"];

/// The start and the end of the name of `resource_<n>_title`.
const RESOURCE_TITLE: (&str, &str) = ("resource_", "_title");
const NOT_ENOUGH_RESOURCE: &str = "not_enough_resource_";
const PROPORTION: &str = "proportion_";
const STORE_RESOURCE: &str = "store_resource_";
const ATTACK_HIT_LEVEL: &str = "attack_hit_level_";
const MOVE_ON: &str = "move_on_";
const NOISE_WHEN: &str = "noise_when_";

impl TypeStyle {
    /// Reads an attribute of a family: its name holds a number, a kind of
    /// ground or an activity.
    fn set_in_family(&mut self, attr: &str, words: &[String], r: &mut Reader) -> bool {
        let key = |start: &str| attr.strip_prefix(start).filter(|k| !k.is_empty());
        let title = key(RESOURCE_TITLE.0).and_then(|k| k.strip_suffix(RESOURCE_TITLE.1));
        if let Some(k) = title.filter(|k| !k.is_empty()) {
            let value = r.sided(words, Reader::said);
            self.resource_title.insert(k.to_owned(), value);
        } else if let Some(k) = key(NOT_ENOUGH_RESOURCE) {
            let value = r.sided(words, Reader::said);
            self.not_enough_resource.insert(k.to_owned(), value);
        } else if let Some(k) = key(PROPORTION) {
            let value = r.sided(words, Reader::sounds);
            self.proportion.insert(k.to_owned(), value);
        } else if let Some(k) = key(STORE_RESOURCE) {
            let value = r.sided(words, Reader::sounds);
            self.store_resource.insert(k.to_owned(), value);
        } else if let Some(k) = key(ATTACK_HIT_LEVEL) {
            let value = r.sided(words, Reader::sounds);
            self.attack_hit_level.insert(k.to_owned(), value);
        } else if let Some(k) = key(MOVE_ON) {
            let value = r.sided(words, Reader::sounds);
            self.move_on.insert(k.to_owned(), value);
        } else if let Some(k) = key(NOISE_WHEN) {
            let value = r.sided_noise(words);
            self.noise_when.insert(k.to_owned(), value);
        } else {
            return UNREAD.contains(&attr);
        }
        true
    }

    /// The sounds played on a target that this type hits with damage at
    /// `level`: those of the level, else those of `attack_hit`. A level of
    /// more than one character has only its own, as in the original.
    pub fn attack_hit_at(&self, level: i64) -> Option<&Sided<Sounds>> {
        let level = level.to_string();
        let base = || self.attack_hit.as_ref().filter(|_| level.len() == 1);
        self.attack_hit_level.get(&level).or_else(base)
    }

    /// What `messages` says for an order refused for `reason`.
    pub fn refusal(&self, reason: &Reason) -> Option<&Sided<Said>> {
        match reason {
            Reason::TheQueueIsFull => self.the_queue_is_full.as_ref(),
            Reason::NotEnoughFood => self.not_enough_food.as_ref(),
            Reason::PopulationLimitReached => self.population_limit_reached.as_ref(),
            Reason::NotEnoughResource(i) => self.not_enough_resource.get(&i.to_string()),
            Reason::NotEnoughSpace => self.not_enough_space.as_ref(),
            Reason::NotEnoughMana => self.not_enough_mana.as_ref(),
            Reason::CannotBuildHere => self.cannot_build_here.as_ref(),
            Reason::CountLimitReached => self.count_limit_reached.as_ref(),
        }
    }

    /// What `messages` says for `reason`, for the player's side.
    pub fn refusal_msg(&self, reason: &Reason) -> Msg {
        let said = self.refusal(reason);
        said.map(|s| s.mine().clone()).unwrap_or_default()
    }

    /// The name of resource `n`, for the player's side. Empty without one.
    pub fn resource_title_msg(&self, n: impl std::fmt::Display) -> Msg {
        let said = self.resource_title.get(&n.to_string());
        said.map(|s| s.mine().clone()).unwrap_or_default()
    }

    /// The title of the type for the player's side. Empty without one.
    pub fn title_msg(&self) -> Msg {
        mine(&self.title)
    }

    /// The title without its `$1`: its names and its counts.
    pub fn title_names(&self) -> Msg {
        let mut title = self.title_msg();
        title.retain(|part| matches!(part, MsgPart::Name(_) | MsgPart::Count(_)));
        title
    }

    fn parents(&self) -> &[String] {
        self.is_a.as_deref().unwrap_or_default()
    }
}

type Objects = IndexMap<String, TypeStyle>;

/// The style definitions: objects with attributes, inheriting through
/// `is_a`.
#[derive(Default)]
pub struct Style {
    dict: Objects,
    log: Log,
}

impl Style {
    pub fn new(log: Log) -> Style {
        Style {
            log,
            ..Style::default()
        }
    }

    /// Reads definitions; each file adds to or overrides the previous
    /// ones. `exists` says whether a phrase or a sound has a name: a word
    /// that names none is refused, with its file and its line.
    pub fn load(&mut self, files: &[TextFile], exists: &dyn Fn(&str) -> bool) {
        self.dict = self.parse(files, exists);
        self.apply_inheritance();
    }

    /// Adds the style of a map that came from a server: the attributes
    /// this style lacks, its own, inherited or by level, and nothing else,
    /// even through inheritance. Returns how many attributes were ignored
    /// because the style has them.
    pub fn add_downloaded(&mut self, files: &[TextFile], exists: &dyn Fn(&str) -> bool) -> usize {
        let before = self.dict.clone();
        let mut ignored: usize = 0;
        for (name, attributes) in self.parse(files, exists) {
            if !self.dict.contains_key(&name) && self.dict.len() >= MAX_STYLE_OBJECTS {
                self.log.warning(format!(
                    "style: more than {MAX_STYLE_OBJECTS} objects; {name} ignored"
                ));
                continue;
            }
            let object = self.dict.entry(name).or_default();
            ignored = ignored.saturating_add(object.fill_from(&attributes, true).had);
        }
        self.apply_inheritance();
        // A level added to a parent reaches the children, where it can
        // stand in front of the `attack_hit` they had.
        for (name, object) in &mut self.dict {
            if let Some(old) = before.get(name).filter(|old| old.attack_hit.is_some()) {
                object.attack_hit_level.retain(|level, _| {
                    old.attack_hit_level.contains_key(level) || level.chars().count() != 1
                });
            }
        }
        ignored
    }

    /// The objects of `files`, each file adding to or overriding the
    /// previous ones, with their values read but not yet inherited.
    fn parse(&self, files: &[TextFile], exists: &dyn Fn(&str) -> bool) -> Objects {
        let mut dict = Objects::new();
        for file in files {
            if file.text.len() > MAX_STYLE_BYTES {
                self.log.warning(format!(
                    "{}: {} bytes, more than {MAX_STYLE_BYTES}; ignored",
                    file.path,
                    file.text.len()
                ));
                continue;
            }
            self.read(&mut dict, file, exists);
        }
        for (name, object) in &mut dict {
            if let Some(parents) = object.is_a.as_mut()
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

    fn read(&self, dict: &mut Objects, file: &TextFile, exists: &dyn Fn(&str) -> bool) {
        let numbers = source_lines(&file.text);
        let text = preprocess(&file.text);
        let mut name = "(the name is missing)".to_owned();
        // After an object refused for the limit, its lines are ignored
        // without a warning each.
        let mut refused = false;
        let mut reports = 0_usize;
        for (i, line) in text.split('\n').enumerate() {
            let words: Vec<String> = line.split_whitespace().map(str::to_owned).collect();
            let Some((first, rest)) = words.split_first() else {
                continue;
            };
            let mut r = Reader {
                file: &file.path,
                line: numbers.get(i).copied().unwrap_or(0),
                object: &name,
                attr: first,
                exists,
                log: &self.log,
                reports: &mut reports,
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
                    None => r.refuse("a definition without a name"),
                },
                _ if refused => {}
                _ => match dict.get_mut(&name) {
                    Some(object) => {
                        if !object.set(first, rest, &mut r) {
                            r.refuse("the game has no attribute of this name");
                        }
                    }
                    None => r.refuse("an attribute before the first definition"),
                },
            }
        }
        if reports > MAX_REPORTS {
            self.log.error(format!(
                "{}: {} more lines are refused",
                file.path,
                reports.saturating_sub(MAX_REPORTS)
            ));
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
                let parents: Vec<String> = self
                    .dict
                    .get(&ko)
                    .map(|object| object.parents().to_vec())
                    .unwrap_or_default();
                for p in &parents {
                    let (Some(parent), Some(object)) = (self.dict.get(p), self.dict.get(&ko))
                    else {
                        self.log
                            .warning(format!("error in {ko}.is_a: {p} doesn't exist"));
                        continue;
                    };
                    let mut child = object.clone();
                    let filled = child.fill_from(parent, false);
                    if filled.bytes == 0 {
                        continue;
                    }
                    total = total.saturating_add(filled.bytes);
                    if total > MAX_STYLE_SIZE {
                        self.log.warning(format!(
                            "style: more than {MAX_STYLE_SIZE} bytes once inherited; inheritance stops"
                        ));
                        return;
                    }
                    if let Some(object) = self.dict.get_mut(&ko) {
                        *object = child;
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

    /// The style of a type, with what it inherits. A type that no style
    /// names has no attribute.
    pub fn get(&self, object: &str) -> &TypeStyle {
        static EMPTY: TypeStyle = TypeStyle::EMPTY;
        self.dict.get(object).unwrap_or(&EMPTY)
    }

    /// Whether a style names the type.
    pub fn has(&self, object: &str) -> bool {
        self.dict.contains_key(object)
    }

    /// The defined objects, in definition order.
    pub fn objects(&self) -> impl Iterator<Item = (&str, &TypeStyle)> {
        self.dict
            .iter()
            .map(|(name, object)| (name.as_str(), object))
    }

    /// How many bytes the objects hold together, inherited attributes
    /// included: each attribute's name and words, and one for each.
    pub fn size(&self) -> usize {
        self.dict
            .values()
            .fold(0_usize, |n, object| n.saturating_add(object.size()))
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

    fn file(text: &str) -> TextFile {
        TextFile {
            path: "orc/ui/style.txt".to_owned(),
            text: text.to_owned(),
        }
    }

    /// Every name exists but those that start with `no_`.
    fn exists(name: &str) -> bool {
        !name.rsplit('/').next().unwrap().starts_with("no_")
    }

    fn loaded(texts: &[&str]) -> Style {
        let mut style = Style::new(Log::silent());
        let files: Vec<TextFile> = texts.iter().map(|t| file(t)).collect();
        style.load(&files, &exists);
        style
    }

    fn name(s: &str) -> MsgPart {
        MsgPart::Name(s.to_owned())
    }

    #[test]
    fn noises_are_parsed_as_upstream() {
        assert_eq!(parse_noise(&[]), Ok(Noise::Silent));
        assert_eq!(
            parse_noise(&words("loop ambience/wood")),
            Ok(Noise::Loop {
                sound: "ambience/wood".into(),
                volume: 1.0,
                ambient: false
            })
        );
        assert_eq!(
            parse_noise(&words("ambient loop ambience/wood 0.5")),
            Ok(Noise::Loop {
                sound: "ambience/wood".into(),
                volume: 0.5,
                ambient: true
            })
        );
        assert_eq!(
            parse_noise(&words("repeat 5 a/b c/d")),
            Ok(Noise::Repeat {
                interval: 5.0,
                sounds: words("a/b c/d"),
                ambient: false
            })
        );
        assert_eq!(parse_noise(&words("other 5")), Ok(Noise::Silent));
        assert!(parse_noise(&words("repeat x")).is_err());
    }

    #[test]
    fn each_attribute_is_read_into_its_field() {
        let style = loaded(&[
            "def unit\ntitle footman 2 $1\nnoise loop ambience/camp\n\n\
             def footman\nis_a unit\nalert if_me alert/horn else alert/bell\n\
             noise_when_building if_me repeat 2 work/build_1 else loop work/build_2\n\
             shortcut f\nindex 3\nground grass 2\nmove_on_grass step/a step/b\n\
             proportion_10 ui/done\nresource_0_title gold\n",
            "def footman\nattack_hit combat/sword\nattack_hit_level_2 combat/sword_upgraded\n",
        ]);
        let footman = style.get("footman");
        assert_eq!(
            footman.title_msg(),
            [name("footman"), MsgPart::Count(2), MsgPart::literal("$1")]
        );
        assert_eq!(footman.title_names(), [name("footman"), MsgPart::Count(2)]);
        assert_eq!(
            footman.alert,
            Some(Sided::if_me(words("alert/horn"), words("alert/bell")))
        );
        let alert = footman.alert.as_ref().unwrap();
        assert_eq!(alert.of(true), &words("alert/horn"));
        assert_eq!(alert.of(false), &words("alert/bell"));
        assert_eq!(mine(&footman.alert), words("alert/horn"));
        assert_eq!(
            footman.noise,
            Some(Noise::Loop {
                sound: "ambience/camp".into(),
                volume: 1.0,
                ambient: false
            })
        );
        assert_eq!(
            footman.noise_when.get("building"),
            Some(&Noise::IfMe(
                Box::new(Noise::Repeat {
                    interval: 2.0,
                    sounds: words("work/build_1"),
                    ambient: false
                }),
                Box::new(Noise::Loop {
                    sound: "work/build_2".into(),
                    volume: 1.0,
                    ambient: false
                })
            ))
        );
        assert_eq!(footman.shortcut, Some(words("f")));
        assert_eq!(footman.index, Some(words("3")));
        assert_eq!(footman.ground, Some(words("grass 2")));
        assert_eq!(footman.is_a, Some(words("unit")));
        assert_eq!(
            footman.move_on.get("grass"),
            Some(&Sided::any(words("step/a step/b")))
        );
        assert_eq!(
            footman.proportion.get("10"),
            Some(&Sided::any(words("ui/done")))
        );
        assert_eq!(
            footman.resource_title.get("0"),
            Some(&Sided::any(vec![name("gold")]))
        );
        // A level has its own sounds, and else those of `attack_hit`.
        let hit = |level| footman.attack_hit_at(level).map(|s| s.mine().clone());
        assert_eq!(hit(2), Some(words("combat/sword_upgraded")));
        assert_eq!(hit(1), Some(words("combat/sword")));
        assert_eq!(hit(10), None);
        // A type that no style names, and an attribute that none gives.
        assert!(!style.has("knight"));
        assert_eq!(style.get("knight"), &TypeStyle::default());
        assert_eq!(footman.comment, None);
        assert_eq!(mine(&footman.comment), Vec::new());
    }

    #[test]
    fn a_word_that_names_nothing_and_an_attribute_the_game_lacks_are_refused_with_their_line() {
        let log = Log::silent();
        log.keep_errors();
        let mut style = Style::new(log.clone());
        let text = "; a mod\ndef footman\ntitle footman no_such Footman\n\n\
                    alert alert/horn no_sound \\\n alert/bell\ntitel footman\n\
                    noise loop ambience/no_camp\nnoise_if_damaged repeat x\n\
                    color red\nmove_on_ noise/a\nshortcut\n";
        style.load(&[file(text)], &exists);
        let footman = style.get("footman");
        assert_eq!(footman.title_msg(), [name("footman")]);
        assert_eq!(mine(&footman.alert), words("alert/horn alert/bell"));
        assert_eq!(footman.noise, Some(Noise::Silent));
        assert_eq!(
            footman.noise_if_damaged,
            Some(Noise::Invalid(words("repeat x")))
        );
        assert_eq!(footman.shortcut, Some(Vec::new()));
        assert_eq!(
            log.take_errors(),
            [
                "orc/ui/style.txt line 3: footman.title: no_such names no phrase and no sound",
                "orc/ui/style.txt line 3: footman.title: Footman is not a name or a count",
                "orc/ui/style.txt line 5: footman.alert: no_sound names no phrase and no sound",
                "orc/ui/style.txt line 7: footman.titel: the game has no attribute of this name",
                "orc/ui/style.txt line 8: footman.noise: ambience/no_camp names no phrase and no sound",
                "orc/ui/style.txt line 9: footman.noise_if_damaged: problem with noise: repeat x (bad interval: x)",
                "orc/ui/style.txt line 11: footman.move_on_: the game has no attribute of this name",
            ]
        );
    }

    #[test]
    fn a_style_with_many_bad_lines_reports_the_first_ones() {
        let log = Log::silent();
        log.keep_errors();
        let mut style = Style::new(log.clone());
        let text = format!("def a\n{}", "titel x\n".repeat(MAX_REPORTS + 50));
        style.load(&[file(&text)], &exists);
        let warnings = log.take_errors();
        assert_eq!(warnings.len(), MAX_REPORTS + 1);
        assert_eq!(
            warnings.last().unwrap(),
            "orc/ui/style.txt: 50 more lines are refused"
        );
    }

    #[test]
    fn a_type_that_is_its_own_ancestor_has_no_missing_attribute() {
        let style = loaded(&["def a\ntitle one\nis_a a b b\n\ndef b\nis_a a a\n"]);
        assert_eq!(style.get("a").title_msg(), [name("one")]);
        assert_eq!(style.get("b").title_msg(), [name("one")]);
        assert_eq!(style.get("a").comment, None);
        assert_eq!(style.get("b").attack_hit_at(2), None);
    }

    #[test]
    fn a_style_text_over_the_limit_is_ignored_whole() {
        let big = format!("def a\ntitle two\n{}", " ".repeat(MAX_STYLE_BYTES));
        let style = loaded(&["def a\ntitle one\n", &big]);
        assert_eq!(style.get("a").title_msg(), [name("one")]);
    }

    #[test]
    fn objects_past_the_limit_are_ignored() {
        let text: String = (0..=MAX_STYLE_OBJECTS)
            .map(|i| format!("def o{i}\ntitle t{i}\n"))
            .collect();
        let style = loaded(&[&text]);
        assert_eq!(style.objects().count(), MAX_STYLE_OBJECTS);
        assert_eq!(style.get("o4999").title_msg(), [name("t4999")]);
        assert!(!style.has("o5000"));
    }

    #[test]
    fn parents_past_the_limit_are_ignored() {
        let style = loaded(
            &["def p1\ntitle a\ndef p8\ncomment b\ndef p9\nadded_msg c\n\
             def o\nis_a p1 p2 p3 p4 p5 p6 p7 p8 p9\n"],
        );
        let o = style.get("o");
        assert!(o.title.is_some());
        assert!(o.comment.is_some());
        assert!(o.added_msg.is_none());
    }

    #[test]
    fn inheritance_stops_after_the_limit_of_generations() {
        // Each object is defined before its parent, so each pass over the
        // objects copies the title one generation down.
        let n = 40;
        let mut text: String = (0..n)
            .map(|i| format!("def a{i}\nis_a a{}\n", i + 1))
            .collect();
        text.push_str(&format!("def a{n}\ntitle one\n"));
        let style = loaded(&[&text]);
        let has = |i: usize| style.get(&format!("a{i}")).title.is_some();
        assert!(has(n - 1));
        assert!(has(n - MAX_STYLE_DEPTH - 1));
        assert!(!has(n - MAX_STYLE_DEPTH - 2));
        assert!(!has(0));
    }

    #[test]
    fn inheritance_stops_at_the_limit_of_bytes() {
        // One attribute of 64 KB, which 5,000 children would copy.
        let mut text = format!("def parent\ntitle {}\n", "x ".repeat(32 * 1024));
        for i in 0..4999 {
            text.push_str(&format!("def child{i}\nis_a parent\n"));
        }
        let style = loaded(&[&text]);
        assert!(style.size() <= MAX_STYLE_SIZE);
        assert!(style.get("child0").title.is_some());
        assert!(style.get("child4998").title.is_none());
    }

    #[test]
    fn a_downloaded_style_adds_and_never_replaces() {
        let mut style =
            loaded(&["def unit\ntitle unit\nattack_hit combat/sword\n\ndef footman\nis_a unit\n"]);
        let ignored = style.add_downloaded(
            &[file(
                "clear\ndef footman\ntitle evil\nattack_hit_level_2 combat/evil\n\
                 noise loop ambience/golem\n\
                 def golem\nis_a footman\ntitle golem\n\
                 def unit\nmove step/golem\n",
            )],
            &exists,
        );
        assert_eq!(ignored, 2);
        // What the game has stays: its own, inherited and by level.
        let footman = style.get("footman");
        assert_eq!(footman.title_msg(), [name("unit")]);
        let hit = footman.attack_hit_at(2).unwrap();
        assert_eq!(hit.mine(), &words("combat/sword"));
        // What it lacks is added, and inherited.
        assert_eq!(
            footman.noise,
            Some(Noise::Loop {
                sound: "ambience/golem".into(),
                volume: 1.0,
                ambient: false
            })
        );
        assert_eq!(style.get("golem").title_msg(), [name("golem")]);
        assert_eq!(mine(&style.get("golem").attack_hit), words("combat/sword"));
        assert_eq!(mine(&footman.footsteps), words("step/golem"));
    }

    #[test]
    fn a_level_a_downloaded_style_adds_to_a_parent_does_not_replace_a_childs_attack_hit() {
        let mut style =
            loaded(&["def unit\ntitle unit\n\ndef footman\nis_a unit\nattack_hit combat/sword\n"]);
        let text = "def unit\nattack_hit_level_2 combat/evil\n";
        let ignored = style.add_downloaded(&[file(text)], &exists);
        assert_eq!(ignored, 0);
        let hit = |object: &str| style.get(object).attack_hit_at(2).unwrap().mine().clone();
        assert_eq!(hit("unit"), words("combat/evil"));
        assert_eq!(hit("footman"), words("combat/sword"));
    }
}
