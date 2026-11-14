//! The rules and the computer player scripts (`definitions.py`, without
//! `Style`, which belongs to the client).
//!
//! `rules.txt` defines classes with `def <name>` followed by properties.
//! Classes inherit missing properties through `is_a`. A class whose `class`
//! is one of the base classes (worker, soldier, building, effect, deposit,
//! upgrade, ability) becomes a [`UnitType`]; the others (factions,
//! `parameters`) stay as raw definitions, read with [`Rules::get`].
//!
//! The rules of a map come from a server, so a text over one of the limits
//! below is refused whole, with a warning, as if it were empty.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::arithmetic_side_effects
)]

use std::collections::{BTreeMap, BTreeSet};

use crate::defs::{bounded, check_size, preprocess_within_limits};
use crate::nofloat::{PRECISION, py_int, to_int};

/// Game time between two world updates, in milliseconds.
pub const VIRTUAL_TIME_INTERVAL: i64 = 300;
/// The largest number of resource types a map can use.
pub const MAX_NB_OF_RESOURCE_TYPES: usize = 10;

/// The most classes the rules may define, or scripts a scripts text. The
/// original's rules with a mod define 81 classes; its scripts are 3.
pub const MAX_DEFINITIONS: usize = 500;

/// The most parents (`is_a` words) a class may list. The original's
/// classes list one.
pub const MAX_PARENTS: usize = 8;

/// The most ancestors (parents, their parents, and so on) a class may have,
/// which bounds how deep inheritance goes. The original's classes have 2
/// at most.
pub const MAX_ANCESTORS: usize = 32;

/// The most properties a class may have once it inherits, counted as its
/// own and each ancestor's added together. The original's classes have 20
/// of their own and at most 25 counted this way.
pub const MAX_PROPERTIES: usize = 200;

/// The most properties all the classes may have together once they
/// inherit, counted as for [`MAX_PROPERTIES`]. Inheritance copies
/// properties to every class below, and each unknown one is a warning. The
/// original's rules come to 675 at most.
pub const MAX_INHERITED_PROPERTIES: usize = 10_000;

/// A property value as written in the definition file, after conversion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RawValue {
    /// A string property (`airground_type`).
    Str(String),
    /// An integer property, or a decimal one already multiplied by
    /// [`PRECISION`].
    Int(i64),
    /// A list of integers (`storable_resource_types`), or of decimals
    /// multiplied by [`PRECISION`] (`cost`, `storage_bonus`).
    IntList(Vec<i64>),
    /// Any other property: the words after the property name.
    Words(Vec<String>),
    /// `damage_vs` or `armor_vs` once interpreted: type name to value.
    Versus(Vec<(String, i64)>),
}

impl RawValue {
    /// The words of a list property, if this is one.
    pub fn words(&self) -> Option<&[String]> {
        match self {
            RawValue::Words(w) => Some(w),
            _ => None,
        }
    }

    /// Python truthiness: zero, empty text and empty lists are false.
    pub fn is_truthy(&self) -> bool {
        match self {
            RawValue::Str(s) => !s.is_empty(),
            RawValue::Int(n) => *n != 0,
            RawValue::IntList(l) => !l.is_empty(),
            RawValue::Words(w) => !w.is_empty(),
            RawValue::Versus(v) => !v.is_empty(),
        }
    }
}

type Def = Vec<(String, RawValue)>;

fn def_get<'a>(d: &'a Def, k: &str) -> Option<&'a RawValue> {
    d.iter().find(|(name, _)| name == k).map(|(_, v)| v)
}

fn def_get_mut<'a>(d: &'a mut Def, k: &str) -> Option<&'a mut RawValue> {
    d.iter_mut().find(|(name, _)| name == k).map(|(_, v)| v)
}

fn def_set(d: &mut Def, k: &str, v: RawValue) {
    match def_get_mut(d, k) {
        Some(old) => *old = v,
        None => d.push((k.to_owned(), v)),
    }
}

fn def_remove(d: &mut Def, k: &str) {
    d.retain(|(name, _)| name != k);
}

/// The definitions of one file family, in definition order.
#[derive(Clone, Debug, Default)]
struct Definitions {
    defs: Vec<(String, Def)>,
}

impl Definitions {
    fn index(&self, name: &str) -> Option<usize> {
        self.defs.iter().position(|(n, _)| n == name)
    }

    fn def(&self, name: &str) -> Option<&Def> {
        self.defs.iter().find(|(n, _)| n == name).map(|(_, d)| d)
    }

    /// `_Definitions._val`: the value of a property, looked up through the
    /// parents when the class does not define it. Each class is searched
    /// once, and at most 64 parents deep, so that a class that is its own
    /// parent, or a parent listed twice, cannot make the search grow.
    fn val<'a>(
        &'a self,
        obj: &'a str,
        attr: &str,
        depth: usize,
        visited: &mut Vec<&'a str>,
    ) -> Option<RawValue> {
        let o = self.def(obj)?;
        if let Some(v) = def_get(o, attr) {
            return Some(v.clone());
        }
        if depth > 64 {
            return None;
        }
        visited.push(obj);
        if let Some(RawValue::Words(parents)) = def_get(o, "is_a") {
            for p in parents {
                if self.def(p).is_some()
                    && !visited.contains(&p.as_str())
                    && let Some(v) = self.val(p, attr, depth.saturating_add(1), visited)
                {
                    return Some(v);
                }
            }
        }
        None
    }

    fn get(&self, obj: &str, attr: &str) -> Option<RawValue> {
        let mut v = self.val(obj, attr, 0, &mut Vec::new());
        if v.is_none()
            && let Some(base) = without_level(attr)
        {
            v = self.val(obj, base, 0, &mut Vec::new());
        }
        v
    }

    /// Refuses a class with more than [`MAX_PARENTS`] parents, more than
    /// [`MAX_ANCESTORS`] ancestors, or more than [`MAX_PROPERTIES`]
    /// properties once it inherits, and classes with more than
    /// [`MAX_INHERITED_PROPERTIES`] together, before inheritance is applied.
    fn check_inheritance(&self) -> Result<(), String> {
        let mut total: usize = 0;
        let index: BTreeMap<&str, usize> = self
            .defs
            .iter()
            .enumerate()
            .map(|(i, (n, _))| (n.as_str(), i))
            .collect();
        let parents = |d: &'_ Def| -> Vec<String> {
            match def_get(d, "is_a") {
                Some(RawValue::Words(w)) => w.clone(),
                _ => Vec::new(),
            }
        };
        for (name, d) in &self.defs {
            let mut queue = parents(d);
            if queue.len() > MAX_PARENTS {
                return Err(format!(
                    "{name} has {} parents, more than {MAX_PARENTS}",
                    queue.len()
                ));
            }
            let mut ancestors = BTreeSet::new();
            let mut properties = d.len();
            let mut next = 0;
            while let Some(p) = queue.get(next) {
                next = next.saturating_add(1);
                let Some(&pi) = index.get(p.as_str()) else {
                    continue;
                };
                if !ancestors.insert(pi) {
                    continue;
                }
                if ancestors.len() > MAX_ANCESTORS {
                    return Err(format!("{name} has more than {MAX_ANCESTORS} ancestors"));
                }
                if let Some((_, pd)) = self.defs.get(pi) {
                    properties = properties.saturating_add(pd.len());
                    queue.extend(parents(pd));
                }
            }
            if properties > MAX_PROPERTIES {
                return Err(format!(
                    "{name} has {properties} properties with those it inherits, more than {MAX_PROPERTIES}"
                ));
            }
            total = total.saturating_add(properties);
            if total > MAX_INHERITED_PROPERTIES {
                return Err(format!(
                    "the classes have more than {MAX_INHERITED_PROPERTIES} properties with those they inherit"
                ));
            }
        }
        Ok(())
    }

    /// Copies to each class the properties of its parents it does not have,
    /// pass after pass until nothing changes. With `expanded_is_a`, also
    /// lists each class's ancestors in `expanded_is_a`, nearest first.
    fn apply_inheritance(&mut self, expanded_is_a: bool, warnings: &mut Vec<String>) {
        for _ in 0..1000 {
            let mut modified = false;
            for oi in 0..self.defs.len() {
                let Some((child_name, child)) = self.defs.get_mut(oi) else {
                    continue;
                };
                let Some(RawValue::Words(is_a)) = def_get(child, "is_a").cloned() else {
                    continue;
                };
                if expanded_is_a && def_get(child, "expanded_is_a").is_none() {
                    child.push(("expanded_is_a".into(), RawValue::Words(is_a.clone())));
                    modified = true;
                }
                let child_name = child_name.clone();
                let mut keys: BTreeSet<String> = child.iter().map(|(k, _)| k.clone()).collect();
                for p in &is_a {
                    let Some(pi) = self.index(p) else {
                        warnings.push(format!("error in {child_name}.is_a: {p} doesn't exist"));
                        continue;
                    };
                    // A class that is its own parent has nothing to copy.
                    let Ok([(_, child), (_, parent)]) = self.defs.get_disjoint_mut([oi, pi]) else {
                        continue;
                    };
                    for (k, v) in parent.iter() {
                        if expanded_is_a && k == "expanded_is_a" {
                            let RawValue::Words(parent_list) = v else {
                                continue;
                            };
                            if let Some(RawValue::Words(own)) = def_get_mut(child, "expanded_is_a")
                            {
                                for item in parent_list {
                                    if !own.contains(item) {
                                        own.push(item.clone());
                                        modified = true;
                                    }
                                }
                            }
                        } else if !keys.contains(k) {
                            keys.insert(k.clone());
                            child.push((k.clone(), v.clone()));
                            modified = true;
                        }
                    }
                }
            }
            if !modified {
                break;
            }
        }
    }
}

/// `attr` without its `_level_<character>` ending, if it has one.
fn without_level(attr: &str) -> Option<&str> {
    let n = attr.len().checked_sub(8)?;
    let (base, end) = (attr.get(..n)?, attr.get(n..)?);
    (end.get(..7)? == "_level_").then_some(base)
}

const STRING_PROPERTIES: &[&str] = &["airground_type"];

const INT_PROPERTIES: &[&str] = &[
    "nb_of_resource_types",
    "collision",
    "corpse",
    "food_cost",
    "food_provided",
    "harm_level",
    "heal_level",
    "resource_type",
    "is_repairable",
    "is_healable",
    "is_vulnerable",
    "is_undead",
    "is_a_building_land",
    "is_buildable_anywhere",
    "bonus_height",
    "transport_capacity",
    "transport_volume",
    "is_invisible",
    "is_cloakable",
    "is_a_detector",
    "is_a_cloaker",
    "universal_notification",
    "presence",
    "provides_survival",
    "is_ballistic",
    "is_teleportable",
    "is_a_gate",
    "is_buildable_on_exits_only",
    "is_buildable_near_water_only",
    "count_limit",
];

const PRECISION_PROPERTIES: &[&str] = &[
    "armor",
    "damage",
    "minimal_damage",
    "damage_radius",
    "range",
    "minimal_range",
    "decay",
    "qty",
    "extraction_qty",
    "hp_max",
    "mana_cost",
    "mana_max",
    "mana_start",
    "extraction_time",
    "time_cost",
    "cooldown",
    "hp_regen",
    "mana_regen",
    "speed",
    "effect_range",
    "effect_radius",
    "sight_range",
    "cloaking_range",
    "detection_range",
];

fn is_precision_property(k: &str) -> bool {
    PRECISION_PROPERTIES.contains(&k)
        || k.strip_suffix("_bonus")
            .is_some_and(|base| PRECISION_PROPERTIES.contains(&base))
}

/// Reads definition text into `defs` (`_Definitions.read` with the
/// property types of `Rules`). A line that cannot be read is skipped with a
/// warning, like the original. A text over a limit is an error.
fn read_rules_text(s: &str, d: &mut Definitions, warnings: &mut Vec<String>) -> Result<(), String> {
    let s = preprocess_within_limits(s)?;
    let mut name = String::from("(the name is missing)");
    for line in s.split('\n') {
        let words: Vec<String> = line.split_whitespace().map(str::to_owned).collect();
        let Some((w0, args)) = words.split_first() else {
            continue;
        };
        if w0 == "clear" {
            d.defs.clear();
            continue;
        }
        if w0 == "def" {
            match args.first() {
                Some(n) => name = n.clone(),
                None => warnings.push(format!("error in definition of {name}: {line}")),
            }
            if !args.is_empty() && d.index(&name).is_none() {
                if d.defs.len() >= MAX_DEFINITIONS {
                    return Err(format!("more than {MAX_DEFINITIONS} classes"));
                }
                d.defs.push((name.clone(), Vec::new()));
            }
            continue;
        }
        let Some(value) = property_value(w0, args) else {
            warnings.push(format!("error in definition of {name}: {line}"));
            continue;
        };
        let Some((_, def)) = d.defs.iter_mut().find(|(n, _)| *n == name) else {
            warnings.push(format!("error in definition of {name}: {line}"));
            continue;
        };
        if def.len() >= MAX_PROPERTIES && def_get(def, w0).is_none() {
            return Err(format!("{name} has more than {MAX_PROPERTIES} properties"));
        }
        def_set(def, w0, value);
    }
    Ok(())
}

/// The value of property `w0` written with the words `args`, or `None` when
/// it cannot be read.
fn property_value(w0: &str, args: &[String]) -> Option<RawValue> {
    let first = || args.first();
    Some(if STRING_PROPERTIES.contains(&w0) {
        RawValue::Str(first()?.clone())
    } else if INT_PROPERTIES.contains(&w0) {
        RawValue::Int(int(first()?)?)
    } else if is_precision_property(w0) {
        let v = match first()?.as_str() {
            "square" if w0 == "effect_range" => "6",
            "nearby" if w0 == "effect_range" => "12",
            "anywhere" | "inf" if w0 == "effect_range" => "2147483",
            "inf" => "2147483",
            other => other,
        };
        RawValue::Int(decimal(v)?)
    } else if w0 == "storable_resource_types" {
        RawValue::IntList(args.iter().map(|x| int(x)).collect::<Option<_>>()?)
    } else if w0 == "cost" || w0 == "storage_bonus" {
        RawValue::IntList(args.iter().map(|x| decimal(x)).collect::<Option<_>>()?)
    } else {
        let mut words = args.to_vec();
        if w0 == "effect" && first()? == "bonus" {
            if is_precision_property(args.get(1)?) {
                for w in words.iter_mut().skip(2).take(2) {
                    *w = decimal(w)?.to_string();
                }
                args.get(2)?;
            } else {
                // The simulation reads these as integers and adds them.
                for w in words.iter().skip(2).take(2) {
                    if py_int(w).is_ok() {
                        int(w)?;
                    }
                }
            }
        }
        RawValue::Words(words)
    })
}

/// An integer of the rules, within [`crate::defs::MAX_NUMBER`].
fn int(s: &str) -> Option<i64> {
    py_int(s).ok().and_then(bounded)
}

/// A decimal of the rules, multiplied by [`PRECISION`], within
/// [`crate::defs::MAX_NUMBER`].
fn decimal(s: &str) -> Option<i64> {
    to_int(s).ok().and_then(bounded)
}

/// Replaces `class race` (at the start of a line) with `class faction`.
fn replace_class_race(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for (i, line) in s.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let trimmed = line.trim_start_matches([' ', '\t']);
        let replaced = trimmed.strip_prefix("class").and_then(|rest| {
            let after_spaces = rest.trim_start_matches(' ');
            if after_spaces.len() == rest.len() {
                return None;
            }
            let after = after_spaces.strip_prefix("race")?;
            let boundary = after
                .chars()
                .next()
                .is_none_or(|c| !(c.is_alphanumeric() || c == '_'));
            boundary.then_some(after)
        });
        match replaced {
            Some(after) => {
                out.push_str("class faction");
                out.push_str(after);
            }
            None => out.push_str(line),
        }
    }
    out
}

/// The Python class a rules class derives from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BaseClass {
    Worker,
    Soldier,
    Building,
    Effect,
    Deposit,
    Upgrade,
    Ability,
}

impl BaseClass {
    fn from_name(s: &str) -> Option<BaseClass> {
        Some(match s {
            "worker" => BaseClass::Worker,
            "soldier" => BaseClass::Soldier,
            "building" => BaseClass::Building,
            "effect" => BaseClass::Effect,
            "deposit" => BaseClass::Deposit,
            "upgrade" => BaseClass::Upgrade,
            "ability" => BaseClass::Ability,
            _ => return None,
        })
    }

    /// Whether the Python base class has an attribute called `k` that is
    /// not a method. A rules property that is not one of them is dropped
    /// with a warning.
    fn has_attribute(self, k: &str) -> bool {
        let lists: &[&[&str]] = match self {
            BaseClass::Worker => &[CREATURE_ATTRS, UNIT_ATTRS, WORKER_ATTRS],
            BaseClass::Soldier | BaseClass::Effect => &[CREATURE_ATTRS, UNIT_ATTRS],
            BaseClass::Building => &[CREATURE_ATTRS, BUILDING_ATTRS],
            BaseClass::Deposit => &[DEPOSIT_ATTRS],
            BaseClass::Upgrade => &[UPGRADE_ATTRS],
            BaseClass::Ability => &[ABILITY_ATTRS],
        };
        lists.iter().any(|l| l.contains(&k))
    }

    fn is_creature(self) -> bool {
        matches!(
            self,
            BaseClass::Worker | BaseClass::Soldier | BaseClass::Building | BaseClass::Effect
        )
    }
}

/// Attributes shared by every creature class (worldunit.Creature and
/// worldentity.Entity), as listed by `dir()` without the methods.
const CREATURE_ATTRS: &[&str] = &[
    "_previous_square",
    "_rotations",
    "_smooth_rotations",
    "action_target",
    "activity",
    "ai_mode",
    "airground_type",
    "any_land",
    "armor",
    "armor_vs",
    "basic_abilities",
    "blocked_exit",
    "bonus_height",
    "building_land",
    "can_build",
    "can_research",
    "can_switch_ai_mode",
    "can_train",
    "can_upgrade_to",
    "can_use",
    "cloaking_range",
    "collision",
    "cooldown",
    "corpse",
    "cost",
    "count_limit",
    "damage",
    "damage_level",
    "damage_radius",
    "damage_vs",
    "decay",
    "detection_range",
    "distance_to_goal",
    "expanded_is_a",
    "food_cost",
    "food_provided",
    "group",
    "harm_level",
    "harm_target_type",
    "heal_level",
    "height",
    "hp_delta",
    "hp_max",
    "hp_regen",
    "id",
    "inside",
    "is_a",
    "is_a_building",
    "is_a_building_land",
    "is_a_cloaker",
    "is_a_detector",
    "is_a_gate",
    "is_a_unit",
    "is_ballistic",
    "is_buildable_anywhere",
    "is_cloakable",
    "is_cloaked",
    "is_fully_repaired",
    "is_healable",
    "is_idle",
    "is_inside",
    "is_invisible",
    "is_melee",
    "is_memory",
    "is_moving",
    "is_near_water",
    "is_repairable",
    "is_teleportable",
    "is_undead",
    "is_vulnerable",
    "mana_max",
    "mana_regen",
    "mana_start",
    "menace",
    "minimal_range",
    "next_attack_time",
    "number",
    "o",
    "place",
    "player",
    "presence",
    "provides_survival",
    "qty",
    "radius",
    "rallying_point",
    "range",
    "repair_cost",
    "requirements",
    "sight_range",
    "speed",
    "speed_on_terrain",
    "splash",
    "stat_type",
    "storable_resource_types",
    "storage_bonus",
    "target_types",
    "time_cost",
    "time_limit",
    "transport_capacity",
    "transport_volume",
    "type_name",
    "upgrades",
    "walked",
    "x",
    "y",
];
/// worldunit.Unit adds these.
const UNIT_ATTRS: &[&str] = &["_basic_abilities", "_destination"];
/// worldunit.Worker adds these.
const WORKER_ATTRS: &[&str] = &["auto_gather", "auto_repair", "cargo"];
/// worldunit.Building adds these.
const BUILDING_ATTRS: &[&str] = &["is_buildable_near_water_only", "is_buildable_on_exits_only"];
const DEPOSIT_ATTRS: &[&str] = &[
    "_previous_square",
    "activity",
    "airground_type",
    "any_land",
    "blocked_exit",
    "bonus_height",
    "building_land",
    "cloaking_range",
    "collision",
    "detection_range",
    "extraction_qty",
    "extraction_time",
    "harm_level",
    "id",
    "is_a_building_land",
    "is_a_cloaker",
    "is_a_detector",
    "is_cloakable",
    "is_cloaked",
    "is_healable",
    "is_inside",
    "is_invisible",
    "is_memory",
    "is_moving",
    "is_near_water",
    "is_repairable",
    "is_teleportable",
    "is_undead",
    "is_vulnerable",
    "menace",
    "o",
    "place",
    "player",
    "qty",
    "radius",
    "resource_regen",
    "resource_type",
    "sight_range",
    "speed",
    "speed_on_terrain",
    "time_limit",
    "transport_capacity",
    "transport_volume",
    "x",
    "y",
];
const UPGRADE_ATTRS: &[&str] = &[
    "cost",
    "count_limit",
    "effect",
    "food_cost",
    "requirements",
    "time_cost",
];
const ABILITY_ATTRS: &[&str] = &[
    "cost",
    "count_limit",
    "effect",
    "effect_radius",
    "effect_range",
    "effect_target",
    "food_cost",
    "mana_cost",
    "requirements",
    "time_cost",
    "universal_notification",
];

macro_rules! stats {
    ($($name:ident),* $(,)?) => {
        /// The numeric attributes of an entity or a unit type, by their
        /// Python names. Booleans are 0 or 1, as Python treats them as
        /// integers. Decimal values are multiplied by [`PRECISION`].
        #[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
        pub struct Stats {
            $(pub $name: i64,)*
        }

        impl Stats {
            /// Every attribute name.
            pub const NAMES: &'static [&'static str] = &[$(stringify!($name)),*];

            /// The value of the attribute called `name`.
            pub fn get(&self, name: &str) -> Option<i64> {
                match name {
                    $(stringify!($name) => Some(self.$name),)*
                    _ => None,
                }
            }

            /// The attribute called `name`, to change it.
            pub fn get_mut(&mut self, name: &str) -> Option<&mut i64> {
                match name {
                    $(stringify!($name) => Some(&mut self.$name),)*
                    _ => None,
                }
            }
        }
    };
}

stats! {
    collision, bonus_height, harm_level, heal_level, qty, is_vulnerable, is_repairable,
    is_healable, is_undead, is_teleportable, is_a_building_land, transport_capacity,
    transport_volume, is_invisible, is_cloakable, sight_range, is_a_detector, detection_range,
    is_a_cloaker, cloaking_range, speed, hp_max, hp_regen, mana_max, mana_start, mana_regen,
    time_cost, food_cost, food_provided, armor, damage, damage_radius, range, is_ballistic,
    minimal_range, cooldown, decay, corpse, presence, count_limit, is_a_gate, provides_survival,
    is_buildable_anywhere, is_buildable_on_exits_only, is_buildable_near_water_only, is_a_unit,
    is_a_building, mana_cost, effect_range, effect_radius, universal_notification,
    extraction_time, extraction_qty,
}

/// The kind of Python class an entity is an instance of.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EntityClass {
    Deposit,
    Meadow,
    Corpse,
    Exit,
    Worker,
    Soldier,
    Effect,
    Building,
    BuildingSite,
}

impl EntityClass {
    /// `isinstance(o, Creature)`.
    pub fn is_creature(self) -> bool {
        matches!(
            self,
            EntityClass::Worker
                | EntityClass::Soldier
                | EntityClass::Effect
                | EntityClass::Building
                | EntityClass::BuildingSite
        )
    }

    /// `isinstance(o, Unit)`.
    pub fn is_unit(self) -> bool {
        matches!(
            self,
            EntityClass::Worker | EntityClass::Soldier | EntityClass::Effect
        )
    }

    /// `isinstance(o, _Building)`.
    pub fn is_building_class(self) -> bool {
        matches!(self, EntityClass::Building | EntityClass::BuildingSite)
    }

    /// The class attribute defaults of the Python class.
    pub fn default_stats(self) -> Stats {
        let mut s = Stats {
            collision: 1,
            transport_volume: 99,
            sight_range: 85 * PRECISION / 10,
            detection_range: 85 * PRECISION / 10,
            cloaking_range: 6 * PRECISION,
            ..Stats::default()
        };
        match self {
            EntityClass::Deposit => {}
            EntityClass::Meadow => {
                s.is_a_building_land = 1;
                s.collision = 0;
            }
            EntityClass::Corpse | EntityClass::Exit => s.collision = 0,
            _ => {
                // Creature
                s.is_buildable_anywhere = 1;
                s.transport_volume = 1;
                s.is_vulnerable = 1;
                s.is_healable = 1;
                s.corpse = 1;
                s.presence = 1;
                if self.is_unit() {
                    s.food_cost = 1;
                    s.is_cloakable = 1;
                    s.is_a_gate = 1;
                    s.is_a_unit = 1;
                }
                match self {
                    EntityClass::Worker | EntityClass::Soldier => s.is_teleportable = 1,
                    EntityClass::Effect => {
                        s.collision = 0;
                        s.corpse = 0;
                        s.food_cost = 0;
                        s.is_vulnerable = 0;
                        s.presence = 0;
                    }
                    EntityClass::Building | EntityClass::BuildingSite => {
                        s.is_repairable = 1;
                        s.is_healable = 0;
                        s.is_a_building = 1;
                        s.transport_volume = 99;
                        s.corpse = 0;
                        if self == EntityClass::Building {
                            s.is_buildable_anywhere = 0;
                            s.provides_survival = 1;
                        }
                    }
                    _ => {}
                }
            }
        }
        s
    }
}

/// `"ground"`, `"air"` or `"water"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum AirGround {
    #[default]
    Ground,
    Air,
    Water,
}

impl AirGround {
    pub fn parse(s: &str) -> Option<AirGround> {
        Some(match s {
            "ground" => AirGround::Ground,
            "air" => AirGround::Air,
            "water" => AirGround::Water,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            AirGround::Ground => "ground",
            AirGround::Air => "air",
            AirGround::Water => "water",
        }
    }

    /// `ground_or_air`: water units count as ground units.
    pub fn ground_or_air(self) -> &'static str {
        match self {
            AirGround::Air => "air",
            _ => "ground",
        }
    }
}

/// A class of `rules.txt` built on one of the base classes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitType {
    pub name: String,
    pub base: BaseClass,
    pub stats: Stats,
    pub airground_type: AirGround,
    /// The cost in each resource, with [`PRECISION`]. Deposits have none.
    pub cost: Vec<i64>,
    pub requirements: Vec<String>,
    pub is_a: Vec<String>,
    /// The parents, grandparents and so on, nearest first.
    pub expanded_is_a: Vec<String>,
    pub can_build: Vec<String>,
    pub can_train: Vec<String>,
    pub can_use: Vec<String>,
    pub can_research: Vec<String>,
    pub can_upgrade_to: Vec<String>,
    pub target_types: Vec<String>,
    pub harm_target_type: Vec<String>,
    pub speed_on_terrain: Vec<String>,
    pub storable_resource_types: Vec<i64>,
    pub storage_bonus: Vec<i64>,
    pub damage_vs: Vec<(String, i64)>,
    pub armor_vs: Vec<(String, i64)>,
    pub splash: bool,
    /// The `<stat>_bonus` values, by stat name.
    pub bonuses: Vec<(String, i64)>,
    /// Deposits: the resource it holds.
    pub resource_type: Option<i64>,
    /// Upgrades and abilities: the effect and its arguments.
    pub effect: Option<Vec<String>>,
    pub effect_target: Vec<String>,
}

impl UnitType {
    /// The entity class instances of this type have.
    pub fn entity_class(&self) -> Option<EntityClass> {
        Some(match self.base {
            BaseClass::Worker => EntityClass::Worker,
            BaseClass::Soldier => EntityClass::Soldier,
            BaseClass::Building => EntityClass::Building,
            BaseClass::Effect => EntityClass::Effect,
            BaseClass::Deposit => EntityClass::Deposit,
            BaseClass::Upgrade | BaseClass::Ability => return None,
        })
    }

    /// `getattr(type, stat + "_bonus", 0)`.
    pub fn bonus(&self, stat: &str) -> i64 {
        self.bonuses
            .iter()
            .find(|(k, _)| k == stat)
            .map_or(0, |(_, v)| *v)
    }

    pub fn is_upgrade(&self) -> bool {
        self.base == BaseClass::Upgrade
    }

    pub fn is_ability(&self) -> bool {
        self.base == BaseClass::Ability
    }

    fn new(name: &str, base: BaseClass, nb_res: usize) -> UnitType {
        let class = match base {
            BaseClass::Worker => EntityClass::Worker,
            BaseClass::Soldier => EntityClass::Soldier,
            BaseClass::Building => EntityClass::Building,
            BaseClass::Effect => EntityClass::Effect,
            BaseClass::Deposit => EntityClass::Deposit,
            BaseClass::Upgrade | BaseClass::Ability => EntityClass::Deposit,
        };
        let mut stats = class.default_stats();
        if matches!(base, BaseClass::Upgrade | BaseClass::Ability) {
            stats = Stats::default();
            if base == BaseClass::Ability {
                stats.effect_range = 6 * PRECISION;
                stats.effect_radius = 6 * PRECISION;
            }
        }
        UnitType {
            name: name.to_owned(),
            base,
            stats,
            airground_type: AirGround::Ground,
            cost: if base == BaseClass::Deposit {
                Vec::new()
            } else {
                vec![0; nb_res]
            },
            requirements: Vec::new(),
            is_a: Vec::new(),
            expanded_is_a: Vec::new(),
            can_build: Vec::new(),
            can_train: Vec::new(),
            can_use: Vec::new(),
            can_research: Vec::new(),
            can_upgrade_to: Vec::new(),
            target_types: vec!["ground".into()],
            harm_target_type: Vec::new(),
            speed_on_terrain: Vec::new(),
            storable_resource_types: Vec::new(),
            storage_bonus: Vec::new(),
            damage_vs: Vec::new(),
            armor_vs: Vec::new(),
            splash: false,
            bonuses: Vec::new(),
            resource_type: None,
            effect: None,
            effect_target: if base == BaseClass::Ability {
                vec!["self".into()]
            } else {
                Vec::new()
            },
        }
    }
}

/// An index into [`Rules`] unit types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TypeId(pub u32);

/// The game rules: `rules.txt`, then the mods' and the map's rules.
#[derive(Clone, Debug, Default)]
pub struct Rules {
    raw: Definitions,
    types: Vec<UnitType>,
    by_name: BTreeMap<String, TypeId>,
    /// What went wrong while reading the rules. The original logs these.
    pub warnings: Vec<String>,
}

impl Rules {
    /// Reads rules from the texts of the layers in order: `rules.txt`, then
    /// the mods', the campaign's and the map's. As the original's resource
    /// loader does, the texts are joined with a line break and read as one,
    /// with any line end accepted.
    ///
    /// Rules over one of the limits ([`crate::defs::MAX_TEXT_BYTES`],
    /// [`MAX_DEFINITIONS`], [`MAX_ANCESTORS`]...) are refused: they load as
    /// empty rules with a warning that says which limit.
    pub fn load<S: AsRef<str>>(texts: &[S]) -> Rules {
        let mut warnings = Vec::new();
        let mut raw = Definitions::default();
        let read = check_size(layers_size(texts)).and_then(|()| {
            let s = replace_class_race(&join_layers(texts));
            read_rules_text(&s, &mut raw, &mut warnings)?;
            raw.check_inheritance()
        });
        if let Err(e) = read {
            return Rules {
                warnings: vec![format!("the rules are refused: {e}")],
                ..Rules::default()
            };
        }
        raw.apply_inheritance(true, &mut warnings);
        if let Some(RawValue::Int(n)) = raw.get("parameters", "nb_of_resource_types")
            && n > MAX_NB_OF_RESOURCE_TYPES as i64
        {
            warnings.push(format!(
                "nb_of_resource_types {n}, more than {MAX_NB_OF_RESOURCE_TYPES}"
            ));
        }
        let nb_res = nb_of_resource_types(&raw);
        let mut types = Vec::new();
        let mut by_name = BTreeMap::new();
        for (name, d) in &mut raw.defs {
            let class_name = match def_get(d, "class") {
                Some(RawValue::Words(w)) => w.first().cloned(),
                _ => None,
            };
            let Some(base) = class_name.as_deref().and_then(BaseClass::from_name) else {
                continue;
            };
            interpret(d, base, name, nb_res, &mut warnings);
            let t = build_type(name, base, d, nb_res, &mut warnings);
            by_name.insert(name.clone(), TypeId(types.len() as u32));
            types.push(t);
        }
        Rules {
            raw,
            types,
            by_name,
            warnings,
        }
    }

    /// `rules.get(obj, attr)`: a property of a class, inherited if needed.
    /// `attr` ending in `_level_<digit>` falls back to the property without
    /// the suffix.
    pub fn get(&self, obj: &str, attr: &str) -> Option<RawValue> {
        self.raw.get(obj, attr)
    }

    /// The words of a list property, or `None`.
    pub fn get_words(&self, obj: &str, attr: &str) -> Option<Vec<String>> {
        match self.get(obj, attr) {
            Some(RawValue::Words(w)) => Some(w),
            _ => None,
        }
    }

    /// `rules.get(obj, attr)` is a true value.
    pub fn get_is_truthy(&self, obj: &str, attr: &str) -> bool {
        self.get(obj, attr).is_some_and(|v| v.is_truthy())
    }

    /// The class names in definition order, without `parameters`.
    pub fn classnames(&self) -> Vec<&str> {
        self.raw
            .defs
            .iter()
            .map(|(n, _)| n.as_str())
            .filter(|n| *n != "parameters")
            .collect()
    }

    /// Whether `name` is a class name (`name in rules.classnames()`).
    pub fn has_class(&self, name: &str) -> bool {
        name != "parameters" && self.raw.index(name).is_some()
    }

    /// The classes whose `class` is `faction`, in definition order.
    pub fn factions(&self) -> Vec<String> {
        self.classnames()
            .into_iter()
            .filter(|c| self.get(c, "class") == Some(RawValue::Words(vec!["faction".into()])))
            .map(str::to_owned)
            .collect()
    }

    /// The number of resource types (`parameters nb_of_resource_types`,
    /// 2 by default, [`MAX_NB_OF_RESOURCE_TYPES`] at most).
    pub fn nb_of_resource_types(&self) -> usize {
        nb_of_resource_types(&self.raw)
    }

    /// Pads with zeros or truncates to the number of resource types.
    pub fn normalized_cost_or_resources(&self, lst: &[i64]) -> Vec<i64> {
        normalized(lst, self.nb_of_resource_types())
    }

    pub fn type_id(&self, name: &str) -> Option<TypeId> {
        self.by_name.get(name).copied()
    }

    /// `rules.unit_class(name)`.
    pub fn unit_type(&self, name: &str) -> Option<&UnitType> {
        self.type_id(name).map(|id| self.ty(id))
    }

    /// The unit type called `name`, to change it (tests change rules).
    pub fn unit_type_mut(&mut self, name: &str) -> Option<&mut UnitType> {
        let id = self.type_id(name)?;
        self.types.get_mut(id.0 as usize)
    }

    /// The unit type of an id this rules gave.
    #[expect(
        clippy::indexing_slicing,
        reason = "a TypeId comes from the same rules, whose types do not change in number"
    )]
    pub fn ty(&self, id: TypeId) -> &UnitType {
        &self.types[id.0 as usize]
    }

    /// All unit types in definition order.
    pub fn types(&self) -> &[UnitType] {
        &self.types
    }

    /// The type a faction uses instead of `type_name`, or `type_name`.
    pub fn equivalent_type(&self, type_name: &str, faction: &str) -> String {
        match self.get_words(faction, type_name) {
            Some(w) => w.first().cloned().unwrap_or_else(|| type_name.to_owned()),
            None => type_name.to_owned(),
        }
    }

    /// The classes able to build, train, upgrade to, research or summon
    /// `type_name`, in definition order.
    pub fn get_makers(&self, type_name: &str) -> Vec<String> {
        let t = type_name.to_owned();
        let mut result = Vec::new();
        for c in self.classnames() {
            let Some(uc) = self.unit_type(c) else {
                continue;
            };
            let mut can = [
                &uc.can_build,
                &uc.can_train,
                &uc.can_upgrade_to,
                &uc.can_research,
            ]
            .iter()
            .any(|l| l.contains(&t));
            if !can {
                for ability in &uc.can_use {
                    if let Some(effect) = self.get_words(ability, "effect")
                        && effect.first().map(String::as_str) == Some("summon")
                        && effect.contains(&t)
                    {
                        can = true;
                        break;
                    }
                }
            }
            if can {
                result.push(c.to_owned());
            }
        }
        result
    }

    /// `rules.get("parameters", "minimal_damage")`, or 0.17.
    pub fn minimal_damage(&self) -> i64 {
        match self.get("parameters", "minimal_damage") {
            Some(RawValue::Int(n)) => n,
            _ => 17 * PRECISION / 100,
        }
    }
}

/// `parameters nb_of_resource_types`: 2 if missing or negative, and
/// [`MAX_NB_OF_RESOURCE_TYPES`] at most.
fn nb_of_resource_types(raw: &Definitions) -> usize {
    match raw.get("parameters", "nb_of_resource_types") {
        Some(RawValue::Int(n)) if n >= 0 => usize::try_from(n)
            .map_or(MAX_NB_OF_RESOURCE_TYPES, |n| {
                n.min(MAX_NB_OF_RESOURCE_TYPES)
            }),
        _ => 2,
    }
}

/// The bytes of the texts together.
fn layers_size<S: AsRef<str>>(texts: &[S]) -> usize {
    texts
        .iter()
        .map(|s| s.as_ref().len())
        .fold(0, usize::saturating_add)
}

/// The texts joined with a line break, line ends converted to `\n`.
fn join_layers<S: AsRef<str>>(texts: &[S]) -> String {
    texts
        .iter()
        .map(|s| s.as_ref().replace("\r\n", "\n").replace('\r', "\n"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn normalized(lst: &[i64], n: usize) -> Vec<i64> {
    let mut l = lst.to_vec();
    l.resize(n, 0);
    l
}

/// `Rules.interpret` and `Creature.interpret`, applied to the definition
/// itself, like the original.
fn interpret(d: &mut Def, base: BaseClass, name: &str, nb_res: usize, warnings: &mut Vec<String>) {
    if base.is_creature() {
        for vs_attr in ["damage_vs", "armor_vs"] {
            let words = match def_get(d, vs_attr) {
                Some(RawValue::Words(w)) => w.clone(),
                _ => Vec::new(),
            };
            let mut map: Vec<(String, i64)> = Vec::new();
            let mut targets: Vec<String> = Vec::new();
            for s in words {
                match decimal(&s) {
                    Some(n) => {
                        for t in targets.drain(..) {
                            match map.iter_mut().find(|(k, _)| *k == t) {
                                Some(e) => e.1 = n,
                                None => map.push((t, n)),
                            }
                        }
                    }
                    None => targets.push(s),
                }
            }
            def_set(d, vs_attr, RawValue::Versus(map));
        }
    }
    if def_get(d, "cost").is_none() && !matches!(base, BaseClass::Deposit) {
        def_set(d, "cost", RawValue::IntList(vec![0; nb_res]));
    }
    // _update_old_definitions
    if def_get(d, "sight_range") == Some(&RawValue::Int(PRECISION)) {
        def_set(d, "sight_range", RawValue::Int(12 * PRECISION));
        def_set(d, "bonus_height", RawValue::Int(1));
    }
    if def_get(d, "special_range").is_some() {
        def_remove(d, "special_range");
        def_set(d, "range", RawValue::Int(12 * PRECISION));
        def_set(d, "minimal_range", RawValue::Int(4 * PRECISION));
        def_set(d, "is_ballistic", RawValue::Int(1));
    }
    let keys: Vec<String> = d.iter().map(|(k, _)| k.clone()).collect();
    for k in keys {
        if k == "class" {
            continue;
        }
        let known = base.has_attribute(&k)
            || k.strip_suffix("_bonus")
                .is_some_and(|s| base.has_attribute(s));
        if !known {
            def_remove(d, &k);
            warnings.push(format!(
                "in {name}: {base:?} doesn't have any attribute called '{k}'"
            ));
        } else if k == "cost"
            && let Some(RawValue::IntList(l)) = def_get(d, "cost")
        {
            let l = normalized(l, nb_res);
            def_set(d, "cost", RawValue::IntList(l));
        }
    }
}

fn build_type(
    name: &str,
    base: BaseClass,
    d: &Def,
    nb_res: usize,
    warnings: &mut Vec<String>,
) -> UnitType {
    let mut t = UnitType::new(name, base, nb_res);
    for (k, v) in d {
        let k = k.as_str();
        let words = || v.words().map(<[String]>::to_vec).unwrap_or_default();
        match k {
            "class" => {}
            "airground_type" => match v {
                RawValue::Str(s) => match AirGround::parse(s) {
                    Some(a) => t.airground_type = a,
                    None => warnings.push(format!("in {name}: unknown airground_type {s}")),
                },
                _ => warnings.push(format!("in {name}: unexpected airground_type")),
            },
            "cost" => {
                if let RawValue::IntList(l) = v {
                    t.cost = l.clone();
                }
            }
            "requirements" => t.requirements = words(),
            "is_a" => t.is_a = words(),
            "expanded_is_a" => t.expanded_is_a = words(),
            "can_build" => t.can_build = words(),
            "can_train" => t.can_train = words(),
            "can_use" => t.can_use = words(),
            "can_research" => t.can_research = words(),
            "can_upgrade_to" => t.can_upgrade_to = words(),
            "target_types" => t.target_types = words(),
            "harm_target_type" => t.harm_target_type = words(),
            "speed_on_terrain" => t.speed_on_terrain = words(),
            "effect" => t.effect = Some(words()),
            "effect_target" => t.effect_target = words(),
            "storable_resource_types" | "storage_bonus" => {
                let l = match v {
                    RawValue::IntList(l) => l.clone(),
                    _ => Vec::new(),
                };
                if k == "storage_bonus" {
                    t.storage_bonus = l;
                } else {
                    t.storable_resource_types = l;
                }
            }
            "damage_vs" | "armor_vs" => {
                if let RawValue::Versus(m) = v {
                    if k == "damage_vs" {
                        t.damage_vs = m.clone();
                    } else {
                        t.armor_vs = m.clone();
                    }
                }
            }
            "splash" => t.splash = v.is_truthy(),
            "resource_type" => {
                if let RawValue::Int(n) = v {
                    t.resource_type = Some(*n);
                }
            }
            _ => {
                if let Some(stat) = t.stats.get_mut(k) {
                    *stat = match v {
                        RawValue::Int(n) => *n,
                        other => i64::from(other.is_truthy()),
                    };
                } else if let (Some(s), RawValue::Int(n)) = (k.strip_suffix("_bonus"), v) {
                    t.bonuses.push((s.to_owned(), *n));
                } else {
                    warnings.push(format!(
                        "in {name}: the property '{k}' is not supported by this port"
                    ));
                }
            }
        }
    }
    t
}

/// The computer player scripts of `ai.txt`, by name, in definition order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AiScripts {
    scripts: Vec<(String, Vec<String>)>,
    /// What went wrong while reading.
    pub warnings: Vec<String>,
}

impl AiScripts {
    /// `load_ai`: reads the `ai.txt` texts of the layers, joined with a line
    /// break as the original's resource loader does. A second `def` of the
    /// same name restarts that script.
    ///
    /// Scripts over one of the limits ([`crate::defs::MAX_TEXT_BYTES`],
    /// [`MAX_DEFINITIONS`]...) are refused: they load as no scripts with a
    /// warning that says which limit.
    pub fn load<S: AsRef<str>>(texts: &[S]) -> AiScripts {
        let mut ai = AiScripts::default();
        let read = check_size(layers_size(texts))
            .and_then(|()| preprocess_within_limits(&join_layers(texts)))
            .and_then(|s| ai.read(&s));
        if let Err(e) = read {
            return AiScripts {
                scripts: Vec::new(),
                warnings: vec![format!("the computer player scripts are refused: {e}")],
            };
        }
        ai
    }

    fn read(&mut self, s: &str) -> Result<(), String> {
        let mut name: Option<String> = None;
        for line in s.split('\n') {
            let words: Vec<&str> = line.split_whitespace().collect();
            let Some(&w0) = words.first() else {
                continue;
            };
            if w0 == "def" {
                let Some(n) = words.get(1) else {
                    self.warnings.push(format!("error in ai.txt: {line}"));
                    continue;
                };
                let n = (*n).to_owned();
                let count = self.scripts.len();
                match self.scripts.iter_mut().find(|(k, _)| *k == n) {
                    Some(e) => e.1.clear(),
                    None if count >= MAX_DEFINITIONS => {
                        return Err(format!("more than {MAX_DEFINITIONS} scripts"));
                    }
                    None => self.scripts.push((n.clone(), Vec::new())),
                }
                name = Some(n);
            } else if let Some(n) = &name {
                if let Some(e) = self.scripts.iter_mut().find(|(k, _)| k == n) {
                    e.1.push(line.to_owned());
                }
            } else {
                self.warnings
                    .push("'def <AI_name>' is missing (check ai.txt)".to_owned());
            }
        }
        Ok(())
    }

    /// `get_ai(name)`: the lines of a script.
    pub fn get(&self, name: &str) -> Option<&[String]> {
        self.scripts
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_slice())
    }

    /// `get_ai_names()`, in definition order.
    pub fn names(&self) -> Vec<&str> {
        self.scripts.iter().map(|(k, _)| k.as_str()).collect()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn a_class_inherits_through_is_a_and_expands_its_ancestors() {
        let r = Rules::load(&[
            "def a\nclass soldier\nhp_max 3\ndef b\nis_a a\nspeed 2\ndef c\nis_a b\n",
        ]);
        assert_eq!(r.unit_type("c").unwrap().stats.hp_max, 3000);
        assert_eq!(r.unit_type("c").unwrap().stats.speed, 2000);
        assert_eq!(r.unit_type("c").unwrap().expanded_is_a, ["b", "a"]);
        assert_eq!(r.get("c", "hp_max"), Some(RawValue::Int(3000)));
    }

    #[test]
    fn an_unknown_attribute_is_dropped_with_a_warning() {
        let r = Rules::load(&["def p\nclass worker\nextraction_qty 1\nmana_cost 2\n"]);
        assert_eq!(r.get("p", "extraction_qty"), None);
        assert_eq!(r.warnings.len(), 2);
    }

    #[test]
    fn class_race_means_class_faction() {
        let r = Rules::load(&["def human\nclass race\n"]);
        assert_eq!(r.factions(), ["human"]);
    }

    #[test]
    fn effect_bonus_values_of_decimal_stats_are_converted() {
        let r = Rules::load(&["def u\nclass upgrade\neffect bonus damage 1.5\n"]);
        assert_eq!(
            r.unit_type("u").unwrap().effect.as_deref(),
            Some(&["bonus".to_owned(), "damage".to_owned(), "1500".to_owned()][..])
        );
    }

    #[test]
    fn damage_vs_lists_targets_before_their_value() {
        let r = Rules::load(&["def p\nclass soldier\ndamage_vs a b 2 c 3\n"]);
        assert_eq!(
            r.unit_type("p").unwrap().damage_vs,
            [("a".into(), 2000), ("b".into(), 2000), ("c".into(), 3000)]
        );
    }

    #[test]
    fn the_number_of_resource_types_is_cut_to_the_maximum_with_a_warning() {
        let r = Rules::load(&[
            "def parameters\nnb_of_resource_types 1000\ndef p\nclass soldier\ncost 1 2\n",
        ]);
        assert_eq!(r.nb_of_resource_types(), 10);
        assert_eq!(r.unit_type("p").unwrap().cost.len(), 10);
        assert_eq!(r.warnings, ["nb_of_resource_types 1000, more than 10"]);
    }

    #[test]
    fn a_class_listed_twice_as_its_own_parent_is_searched_once() {
        let r = Rules::load(&["def a\nclass soldier\nhp_max 3\ndef b\nis_a b b a b a\n"]);
        assert_eq!(r.get("b", "hp_max"), Some(RawValue::Int(3000)));
        assert_eq!(r.get("b", "speed"), None);
    }

    /// The rules refused, with the only warning.
    fn refused(text: &str) -> String {
        let r = Rules::load(&[text]);
        assert!(r.classnames().is_empty() && r.types().is_empty(), "{text}");
        let [w] = &r.warnings[..] else {
            panic!("{:?}", r.warnings);
        };
        w.clone()
    }

    #[test]
    fn rules_over_a_limit_are_refused_whole() {
        assert_eq!(
            refused(&format!("def a\nclass soldier\n; {}", "x".repeat(300_000))),
            "the rules are refused: 300022 bytes, more than 262144"
        );
        assert_eq!(
            refused(&(0..501).map(|i| format!("def c{i}\n")).collect::<String>()),
            "the rules are refused: more than 500 classes"
        );
        let properties: String = (0..201).map(|i| format!("p{i} 1\n")).collect();
        assert_eq!(
            refused(&format!("def a\n{properties}")),
            "the rules are refused: a has more than 200 properties"
        );
    }

    #[test]
    fn inheritance_too_broad_or_too_deep_is_refused() {
        assert_eq!(
            refused("def a\ndef b\nis_a a a a a a a a a a\n"),
            "the rules are refused: b has 9 parents, more than 8"
        );
        let chain: String = (0..34)
            .map(|i| format!("def c{i}\nis_a c{}\n", i + 1))
            .collect();
        assert_eq!(
            refused(&chain),
            "the rules are refused: c0 has more than 32 ancestors"
        );
        let properties: String = (0..150).map(|i| format!("p{i} 1\n")).collect();
        assert_eq!(
            refused(&format!("def a\n{properties}def b\n{properties}is_a a\n")),
            "the rules are refused: b has 301 properties with those it inherits, more than 200"
        );
        let many: String = (0..100).map(|i| format!("def b{i}\nis_a a\n")).collect();
        assert_eq!(
            refused(&format!("def a\n{properties}{many}")),
            "the rules are refused: the classes have more than 10000 properties with those they inherit"
        );
        // A cycle within the limits loads, as in the original.
        let r = Rules::load(&["def a\nclass soldier\nis_a b\nhp_max 1\ndef b\nis_a a\nspeed 2\n"]);
        assert_eq!(r.get("b", "hp_max"), Some(RawValue::Int(1000)));
        assert_eq!(r.unit_type("a").unwrap().stats.speed, 2000);
    }

    /// The simulation sums `food_provided` in 64 bits, and this value made
    /// the sum overflow.
    #[test]
    fn a_number_beyond_the_limit_is_an_error_in_its_line() {
        let r = Rules::load(&[
            "def farm\nclass building\nfood_provided 9223372036854775807\nhp_max 3000000\n",
        ]);
        assert_eq!(r.unit_type("farm").unwrap().stats.food_provided, 0);
        assert_eq!(r.unit_type("farm").unwrap().stats.hp_max, 0);
        assert_eq!(
            r.warnings,
            [
                "error in definition of farm: food_provided 9223372036854775807",
                "error in definition of farm: hp_max 3000000"
            ]
        );
    }

    /// The simulation adds an integer bonus to the stat when the upgrade
    /// applies, and this value made the sum overflow.
    #[test]
    fn an_integer_bonus_beyond_the_limit_is_an_error_in_its_line() {
        let r = Rules::load(&[
            "def boost\nclass upgrade\neffect bonus collision 9223372036854775807\n\
             def plus\nclass upgrade\neffect bonus collision 1 2\n",
        ]);
        assert_eq!(r.unit_type("boost").unwrap().effect, None);
        assert_eq!(
            r.unit_type("plus").unwrap().effect.as_deref(),
            Some(
                &[
                    "bonus".to_owned(),
                    "collision".to_owned(),
                    "1".to_owned(),
                    "2".to_owned()
                ][..]
            )
        );
        assert_eq!(
            r.warnings,
            ["error in definition of boost: effect bonus collision 9223372036854775807"]
        );
    }

    #[test]
    fn scripts_over_a_limit_are_refused_whole() {
        let ai = AiScripts::load(&[(0..501).map(|i| format!("def s{i}\n")).collect::<String>()]);
        assert!(ai.names().is_empty());
        assert_eq!(
            ai.warnings,
            ["the computer player scripts are refused: more than 500 scripts"]
        );
        let ai = AiScripts::load(&["def a\n".repeat(10_001)]);
        assert_eq!(
            ai.warnings,
            ["the computer player scripts are refused: 10002 lines, more than 10000"]
        );
    }

    #[test]
    fn a_second_ai_def_restarts_the_script() {
        let ai = AiScripts::load(&["def easy\na\nb\ndef hard\nc\n", "def easy\nd\n"]);
        assert_eq!(ai.names(), ["easy", "hard"]);
        assert_eq!(ai.get("easy").unwrap(), ["d"]);
    }
}
