//! The properties a rules class may have: how each is written, and which
//! base classes have it.
//!
//! A property no base class has is dropped with a warning when a type is
//! built, as the original drops an attribute its Python class lacks. A
//! property named `<p>_bonus` is written and held as `<p>` is.

use super::BaseClass;

/// How the value of a property is written in the rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Written {
    /// One word, kept as text.
    Text,
    /// An integer.
    Int,
    /// A decimal, multiplied by [`crate::nofloat::PRECISION`].
    Decimal,
    /// A list of integers.
    Ints,
    /// A list of decimals, each multiplied by
    /// [`crate::nofloat::PRECISION`].
    Decimals,
    /// Words, read by whatever uses the property.
    Words,
}

/// A property of the rules.
#[derive(Debug)]
pub(super) struct Property {
    pub name: &'static str,
    pub written: Written,
    /// The base classes that have it. Empty for a property of
    /// `parameters`.
    pub held_by: &'static [BaseClass],
}

use BaseClass::{Ability, Building, Deposit, Effect, Soldier, Upgrade, Worker};
use Written::{Decimal, Decimals, Int, Ints, Text, Words};

const UNIT: &[BaseClass] = &[Worker, Soldier, Effect];
const CREATURE: &[BaseClass] = &[Worker, Soldier, Building, Effect];
const ENTITY: &[BaseClass] = &[Worker, Soldier, Building, Effect, Deposit];
const BOUGHT: &[BaseClass] = &[Worker, Soldier, Building, Effect, Upgrade, Ability];
const TECHNOLOGY: &[BaseClass] = &[Upgrade, Ability];

macro_rules! properties {
    ($($held_by:expr => [$($name:literal: $written:ident),* $(,)?]),* $(,)?) => {
        &[$($(Property { name: $name, written: $written, held_by: $held_by },)*)*]
    };
}

/// Every property. Some name an attribute of the original's Python class
/// that no rule sets (`x`, `place`): the original keeps such a property,
/// and the port drops it with a warning when it builds the type.
const PROPERTIES: &[Property] = properties![
    &[] => ["minimal_damage": Decimal, "nb_of_resource_types": Int],
    ENTITY => [
        "_previous_square": Words, "activity": Words, "airground_type": Text,
        "any_land": Words, "blocked_exit": Words, "bonus_height": Int,
        "building_land": Words, "cloaking_range": Decimal, "collision": Int,
        "detection_range": Decimal, "harm_level": Int, "id": Words,
        "is_a_building_land": Int, "is_a_cloaker": Int, "is_a_detector": Int,
        "is_cloakable": Int, "is_cloaked": Words, "is_healable": Int, "is_inside": Words,
        "is_invisible": Int, "is_memory": Words, "is_moving": Words, "is_near_water": Words,
        "is_repairable": Int, "is_teleportable": Int, "is_undead": Int,
        "is_vulnerable": Int, "menace": Words, "o": Words, "place": Words, "player": Words,
        "qty": Decimal, "radius": Words, "sight_range": Decimal, "speed": Decimal,
        "speed_on_terrain": Words, "time_limit": Words, "transport_capacity": Int,
        "transport_volume": Int, "x": Words, "y": Words,
    ],
    CREATURE => [
        "_rotations": Words, "_smooth_rotations": Words, "action_target": Words,
        "ai_mode": Words, "armor": Decimal, "armor_vs": Words, "basic_abilities": Words,
        "can_build": Words, "can_research": Words, "can_switch_ai_mode": Words,
        "can_train": Words, "can_upgrade_to": Words, "can_use": Words, "cooldown": Decimal,
        "corpse": Int, "damage": Decimal, "damage_level": Words, "damage_radius": Decimal,
        "damage_vs": Words, "decay": Decimal, "distance_to_goal": Words,
        "expanded_is_a": Words, "food_provided": Int, "group": Words,
        "harm_target_type": Words, "heal_level": Int, "height": Words, "hp_delta": Words,
        "hp_max": Decimal, "hp_regen": Decimal, "inside": Words, "is_a": Words,
        "is_a_building": Words, "is_a_gate": Int, "is_a_unit": Words, "is_ballistic": Int,
        "is_buildable_anywhere": Int, "is_fully_repaired": Words, "is_idle": Words,
        "is_melee": Words, "mana_max": Decimal, "mana_regen": Decimal,
        "mana_start": Decimal, "minimal_range": Decimal, "next_attack_time": Words,
        "number": Words, "presence": Int, "provides_survival": Int,
        "rallying_point": Words, "range": Decimal, "repair_cost": Words, "splash": Words,
        "stat_type": Words, "storable_resource_types": Ints, "storage_bonus": Decimals,
        "target_types": Words, "type_name": Words, "upgrades": Words, "walked": Words,
    ],
    UNIT => ["_basic_abilities": Words, "_destination": Words],
    &[Worker] => ["auto_gather": Words, "auto_repair": Words, "cargo": Words],
    &[Building] => ["is_buildable_near_water_only": Int, "is_buildable_on_exits_only": Int],
    &[Deposit] => [
        "extraction_qty": Decimal, "extraction_time": Decimal, "resource_regen": Words,
        "resource_type": Int,
    ],
    BOUGHT => [
        "cost": Decimals, "count_limit": Int, "food_cost": Int, "requirements": Words,
        "time_cost": Decimal,
    ],
    TECHNOLOGY => ["effect": Words],
    &[Ability] => [
        "effect_radius": Decimal, "effect_range": Decimal, "effect_target": Words,
        "mana_cost": Decimal, "universal_notification": Int,
    ],
];

/// The property called `name`, or the one it is the bonus of.
pub(super) fn property(name: &str) -> Option<&'static Property> {
    let find = |n: &str| PROPERTIES.iter().find(|p| p.name == n);
    find(name).or_else(|| {
        let base = find(name.strip_suffix("_bonus")?)?;
        (base.written == Decimal).then_some(base)
    })
}

/// How the value of the property called `name` is written: as
/// [`Written::Words`] if the table does not have it.
pub(super) fn written(name: &str) -> Written {
    property(name).map_or(Words, |p| p.written)
}

/// Whether instances of `base` have the property called `name`, or the
/// property it is the bonus of.
pub(super) fn is_held_by(name: &str, base: BaseClass) -> bool {
    let held = |n: &str| {
        PROPERTIES
            .iter()
            .any(|p| p.name == n && p.held_by.contains(&base))
    };
    held(name) || name.strip_suffix("_bonus").is_some_and(held)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_property_is_listed_once() {
        for (i, p) in PROPERTIES.iter().enumerate() {
            assert!(
                PROPERTIES.iter().skip(i + 1).all(|q| q.name != p.name),
                "{}",
                p.name
            );
        }
    }

    #[test]
    fn a_bonus_is_written_as_its_property_when_that_is_a_decimal() {
        assert_eq!(written("damage_bonus"), Decimal);
        assert_eq!(written("collision_bonus"), Words);
        assert_eq!(written("storage_bonus"), Decimals);
        assert!(is_held_by("armor_bonus", Soldier));
        assert!(is_held_by("collision_bonus", Deposit));
        assert!(!is_held_by("armor_bonus", Deposit));
    }
}
