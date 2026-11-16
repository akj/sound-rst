//! Why work in progress stops part-way.
//!
//! The work is the update of one unit or one player, one command, or one
//! map trigger. The world catches the stop, logs a warning that names the
//! unit or the player, and the game goes on. What the work changed before
//! the stop stays changed, so a function that can stop does its checks
//! before its first change, or says why what it leaves is harmless.
//!
//! A stop that an ordinary game can reach is not here: a target that died,
//! a square without a path, or a unit converted in the middle of an order
//! has an outcome of its own where it happens, such as an order marked
//! impossible, and a mod's effect that cannot be applied is dropped when
//! the rules load. The computer player skips a step it cannot take, and
//! goes on with its turn. What is left is a map's trigger that cannot be
//! read, and a broken invariant of the simulation, which is a defect to
//! fix.
//!
//! `tests/stops.rs` counts the places that raise each kind.

use std::fmt;

/// Why the work in progress stops.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Abort {
    /// A trigger of the map, or one typed in the console, cannot be read.
    /// The trigger's action stops; a trigger not done is tried again at
    /// the next check.
    Trigger(TriggerError),
    /// The simulation broke one of its own rules: a defect to fix.
    Invariant(Invariant),
}

impl fmt::Display for Abort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Abort::Trigger(t) => write!(f, "trigger: {t}"),
            Abort::Invariant(i) => write!(f, "broken invariant: {i}"),
        }
    }
}

impl From<TriggerError> for Abort {
    fn from(t: TriggerError) -> Abort {
        Abort::Trigger(t)
    }
}

/// What a trigger's words lack to be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TriggerError {
    /// A list in parentheses where a word is expected.
    ListForWord,
    /// A word where a list in parentheses is expected.
    WordForList,
    /// An empty list where a condition or an action is expected.
    Empty,
    /// A word of the trigger is missing after its name.
    MissingWord(String),
    /// A word that should be a number is not one.
    NotANumber(String),
    /// A word that should name a square names none.
    NoSuchSquare(String),
    /// `order` takes the units to choose, then the orders, each a list.
    OrderWithoutTwoLists,
}

impl fmt::Display for TriggerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TriggerError::ListForWord => f.write_str("a list where a word is expected"),
            TriggerError::WordForList => f.write_str("a word where a list is expected"),
            TriggerError::Empty => f.write_str("an empty list"),
            TriggerError::MissingWord(name) => write!(f, "{name} lacks a word"),
            TriggerError::NotANumber(w) => write!(f, "{w:?} is not a number"),
            TriggerError::NoSuchSquare(w) => write!(f, "no square {w:?}"),
            TriggerError::OrderWithoutTwoLists => {
                f.write_str("order takes the units, then the orders, each a list")
            }
        }
    }
}

impl From<Invariant> for Abort {
    fn from(i: Invariant) -> Abort {
        Abort::Invariant(i)
    }
}

/// A rule of the simulation that its own code keeps, found broken. Each
/// says what was found and where.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Invariant {
    /// A path that avoids danger was asked for no player, who would judge
    /// the danger. Units that avoid danger have an owner.
    AvoidingDangerForNobody,
    /// An order aims at a point given by its coordinates, which only an
    /// action has: an order aims at a square, a point of a square or an
    /// entity.
    OrderAimsAtCoordinates,
    /// A project in a player's budget is not a `build` order. Only a
    /// `build` order reserves its building's cost there.
    ProjectIsNotABuild,
    /// An entity without a place, or inside one without a place, was asked
    /// its height or its chance to be hit.
    Unplaced,
    /// A unit inside another carries out an order or a step of one, or
    /// moves on its own. A unit loaded has its orders cancelled, and a unit
    /// inside takes none.
    ActingInside,
    /// A hit aims at a unit inside another. Only what is on a square is
    /// aimed at.
    HitInside,
    /// An attack aims at something that is not an entity.
    AimAtNoEntity,
    /// Land (a meadow or an exit) was told to die. Only creatures and
    /// deposits die.
    LandDies,
    /// A building site being built has no place.
    SiteWithoutPlace,
    /// A new unit found no room where room was just found or freed: where
    /// its building site, the building it replaces or the free space found
    /// for it stood.
    NoRoomWhereRoomWas,
    /// The map has no square to choose from.
    NoSquare,
    /// An order that needs a type is given without one, or with one the
    /// rules lack. `is_order_allowed` refuses both.
    OrderWithoutItsType,
    /// `cancel_building` is given to a unit that is not a building site.
    /// Only a site may receive it.
    CancelOfNoSite,
    /// A starting square of the map names no square. The map is refused
    /// when it loads with such a name.
    UnknownStartingSquare,
    /// A worker brings back a cargo it does not carry.
    BringingBackNothing,
    /// A building that needs land is placed on something other than a
    /// meadow it can remove.
    LandIsNotAnEntity,
    /// An ability whose effect the game does not know is used.
    /// `is_order_allowed` refuses such an ability.
    UnknownAbility,
    /// An order that needs a target, or a group's name, was taken without
    /// one. `take_order` refuses such an order before anything changes.
    OrderWithoutItsTarget,
    /// The effect of an upgrade or an ability cannot be read. The rules
    /// drop such an effect when they load, with a warning.
    EffectUnread,
}

impl fmt::Display for Invariant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Invariant::AvoidingDangerForNobody => {
                "a path avoiding danger was asked for no player (pathfinding)"
            }
            Invariant::OrderAimsAtCoordinates => {
                "an order aims at coordinates, which only an action does (updated_target)"
            }
            Invariant::ProjectIsNotABuild => {
                "a project in the budget is not a build order (free_resources)"
            }
            Invariant::Unplaced => "an entity without a place was asked its height (height_of)",
            Invariant::ActingInside => "a unit inside another acts on its own",
            Invariant::HitInside => "a hit aims at a unit inside another (chance_to_hit)",
            Invariant::AimAtNoEntity => "an attack aims at no entity (near_enough_to_aim)",
            Invariant::LandDies => "a meadow or an exit was told to die (die)",
            Invariant::SiteWithoutPlace => "a building site being built has no place (be_built)",
            Invariant::NoRoomWhereRoomWas => {
                "a new unit found no room where room was just found or freed"
            }
            Invariant::NoSquare => "the map has no square to choose from",
            Invariant::OrderWithoutItsType => "an order needing a type has none (taken)",
            Invariant::CancelOfNoSite => "cancel_building given to no building site",
            Invariant::UnknownStartingSquare => "a starting square names no square (auto_explore)",
            Invariant::BringingBackNothing => "a worker brings back no cargo (gather)",
            Invariant::LandIsNotAnEntity => {
                "a building is put on land that is not an entity (put_building_site)"
            }
            Invariant::UnknownAbility => "an ability without a known effect is used (use)",
            Invariant::OrderWithoutItsTarget => "an order was taken without its target",
            Invariant::EffectUnread => "an effect the rules should have dropped is applied",
        })
    }
}

/// The result of work that may stop part-way.
pub type Res<T = ()> = Result<T, Abort>;
