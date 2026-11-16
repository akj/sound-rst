//! What an upgrade or an ability does (`effect`), read when the rules
//! load.

use super::{BaseClass, Stat, TypeId, decimal, int};
use crate::map::MAX_START_UNITS;
use crate::nofloat::py_int;

/// The effect of an upgrade or an ability. The rules give it as a kind
/// and its arguments: `effect summon 15 meteors`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// An upgrade: `bonus <stat> <value> [<per level>]` adds the value to
    /// the statistic of each unit it applies to, and the value per level
    /// times the level the unit starts at.
    Bonus {
        stat: Stat,
        value: i64,
        per_level: i64,
    },
    /// An upgrade: `apply_bonus <stat>` adds the `<stat>_bonus` of each
    /// unit's own type.
    ApplyBonus(Stat),
    /// Moves the user's units around it to the target square.
    Teleportation,
    /// Brings the user's units around the target to the user's square.
    Recall,
    /// Gives the target unit to the user's player.
    Conversion,
    /// `summon <decay> <items>`: adds the units the items name at the
    /// target, for a time, in thousandths of a second.
    Summon { decay: i64, items: Vec<Summoned> },
    /// `raise_dead <decay> <items>`: raises the units the items name from
    /// the corpses around the target, for a time.
    RaiseDead { decay: i64, items: Vec<Summoned> },
    /// `resurrection <count>`: brings back to life up to that many of the
    /// player's dead units around the target.
    Resurrection { count: i64 },
}

/// An item of a summon: a type, and how many units of it. The rules
/// write `2 dragon`, or `dragon` for one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Summoned {
    pub count: i64,
    pub ty: TypeId,
}

impl Effect {
    /// The effect written as `words`: `None` if a type of `base` cannot
    /// have it or its arguments cannot be read, and the item that names no
    /// type of `type_id` as an error. The words of a `bonus` value are
    /// already read: a decimal statistic's multiplied by
    /// [`crate::nofloat::PRECISION`].
    pub(super) fn parse(
        base: BaseClass,
        words: &[String],
        type_id: impl Fn(&str) -> Option<TypeId>,
    ) -> Result<Option<Effect>, String> {
        let Some(effect) = Effect::parse_words(base, words) else {
            return Ok(None);
        };
        Ok(Some(match effect {
            Parsed::Done(e) => e,
            Parsed::Summon(decay, items) => Effect::Summon {
                decay,
                items: summoned(items, type_id)?,
            },
            Parsed::RaiseDead(decay, items) => Effect::RaiseDead {
                decay,
                items: summoned(items, type_id)?,
            },
        }))
    }

    fn parse_words(base: BaseClass, words: &[String]) -> Option<Parsed<'_>> {
        let (kind, args) = words.split_first()?;
        let arg = |i: usize| args.get(i).map(String::as_str);
        let stat = |i: usize| arg(i).and_then(Stat::parse);
        Some(Parsed::Done(match (base, kind.as_str()) {
            (BaseClass::Upgrade, "bonus") => Effect::Bonus {
                stat: stat(0)?,
                value: int(arg(1)?)?,
                per_level: match arg(2) {
                    Some(w) => int(w)?,
                    None => 0,
                },
            },
            (BaseClass::Upgrade, "apply_bonus") => Effect::ApplyBonus(stat(0)?),
            (BaseClass::Ability, "teleportation") => Effect::Teleportation,
            (BaseClass::Ability, "recall") => Effect::Recall,
            (BaseClass::Ability, "conversion") => Effect::Conversion,
            (BaseClass::Ability, "summon") => {
                return Some(Parsed::Summon(
                    decimal(arg(0)?)?,
                    args.get(1..).unwrap_or_default(),
                ));
            }
            (BaseClass::Ability, "raise_dead") => {
                return Some(Parsed::RaiseDead(
                    decimal(arg(0)?)?,
                    args.get(1..).unwrap_or_default(),
                ));
            }
            (BaseClass::Ability, "resurrection") => Effect::Resurrection {
                count: int(arg(0)?)?,
            },
            _ => return None,
        }))
    }

    /// Whether a unit may use an ability with this effect: an upgrade's
    /// effect is never used.
    pub fn is_used(&self) -> bool {
        !matches!(self, Effect::Bonus { .. } | Effect::ApplyBonus(_))
    }
}

/// An effect read but for the items of a summon.
enum Parsed<'a> {
    Done(Effect),
    Summon(i64, &'a [String]),
    RaiseDead(i64, &'a [String]),
}

/// The items of a summon: each type name, with the number before it, or
/// 1. A number that no type follows counts for nothing.
fn summoned(
    words: &[String],
    type_id: impl Fn(&str) -> Option<TypeId>,
) -> Result<Vec<Summoned>, String> {
    let mut items = Vec::new();
    let mut count = 1;
    for w in words {
        if !w.is_empty() && w.bytes().all(|b| b.is_ascii_digit()) {
            count = py_int(w).unwrap_or(1).min(MAX_START_UNITS);
        } else {
            let ty = type_id(w).ok_or_else(|| w.clone())?;
            items.push(Summoned { count, ty });
            count = 1;
        }
    }
    Ok(items)
}
