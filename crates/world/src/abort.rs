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
//! impossible. What is left is a mod's rules or a map's trigger that the
//! work cannot read, and a broken invariant of the simulation, which is a
//! defect to fix.
//!
//! `tests/stops.rs` counts the places that raise each kind.

use std::fmt;

/// Why the work in progress stops.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Abort {
    /// The simulation broke one of its own rules: a defect to fix.
    Invariant(Invariant),
    /// A stop not yet given a kind of its own. The text says what went
    /// wrong.
    Unclassified(String),
}

impl Abort {
    /// A stop not yet given a kind of its own, with its text.
    pub(crate) fn new(s: impl Into<String>) -> Abort {
        Abort::Unclassified(s.into())
    }
}

impl fmt::Display for Abort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Abort::Invariant(i) => write!(f, "broken invariant: {i}"),
            Abort::Unclassified(s) => f.write_str(s),
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
        })
    }
}

/// The result of work that may stop part-way.
pub type Res<T = ()> = Result<T, Abort>;
