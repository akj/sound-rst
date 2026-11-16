//! What an upgrade or an ability does (`effect`), read when the rules
//! load.

use super::{BaseClass, Stat, decimal, int};

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
    Summon { decay: i64, items: Vec<String> },
    /// `raise_dead <decay> <items>`: raises the units the items name from
    /// the corpses around the target, for a time.
    RaiseDead { decay: i64, items: Vec<String> },
    /// `resurrection <count>`: brings back to life up to that many of the
    /// player's dead units around the target.
    Resurrection { count: i64 },
}

impl Effect {
    /// The effect written as `words`, if a type of `base` can have it and
    /// its arguments can be read. The words of a `bonus` value are already
    /// read: a decimal statistic's multiplied by
    /// [`crate::nofloat::PRECISION`].
    pub(super) fn parse(base: BaseClass, words: &[String]) -> Option<Effect> {
        let (kind, args) = words.split_first()?;
        let arg = |i: usize| args.get(i).map(String::as_str);
        let stat = |i: usize| arg(i).and_then(Stat::parse);
        Some(match (base, kind.as_str()) {
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
            (BaseClass::Ability, "summon") => Effect::Summon {
                decay: decimal(arg(0)?)?,
                items: args.get(1..).unwrap_or_default().to_vec(),
            },
            (BaseClass::Ability, "raise_dead") => Effect::RaiseDead {
                decay: decimal(arg(0)?)?,
                items: args.get(1..).unwrap_or_default().to_vec(),
            },
            (BaseClass::Ability, "resurrection") => Effect::Resurrection {
                count: int(arg(0)?)?,
            },
            _ => return None,
        })
    }

    /// Whether a unit may use an ability with this effect: an upgrade's
    /// effect is never used.
    pub fn is_used(&self) -> bool {
        !matches!(self, Effect::Bonus { .. } | Effect::ApplyBonus(_))
    }
}
