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
            Abort::Unclassified(s) => f.write_str(s),
        }
    }
}

/// The result of work that may stop part-way.
pub type Res<T = ()> = Result<T, Abort>;
