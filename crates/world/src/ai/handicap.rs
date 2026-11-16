//! What a computer type may do that a human may not, and its name, which
//! says so. A fair computer has no handicap.

use super::Skill;
use crate::msg::{Msg, MsgPart, msg, nb2msg};
use crate::msgparts as mp;
use crate::nofloat::PRECISION;

/// The AI type of the random computer, which gives random orders.
pub const RANDOM_AI_TYPE: &str = "ai2";

/// A computer's privileges over a human, granted each time it plays.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Handicap {
    /// Its resources are set to this many of each, in whole units.
    pub resources: Option<i64>,
    /// It may give any order without its requirements.
    pub ignores_requirements: bool,
}

impl Handicap {
    /// A fair computer's.
    pub const NONE: Handicap = Handicap {
        resources: None,
        ignores_requirements: false,
    };

    /// The handicap of an AI type: the random computer's resources are set
    /// to 1000 of each, and it ignores requirements.
    pub fn of(ai_type: &str) -> Handicap {
        if ai_type == RANDOM_AI_TYPE {
            Handicap {
                resources: Some(1000),
                ignores_requirements: true,
            }
        } else {
            Handicap::NONE
        }
    }

    /// The resources it sets, with [`PRECISION`].
    pub(crate) fn resources_with_precision(self) -> Option<i64> {
        self.resources.map(|r| r * PRECISION)
    }

    /// The words that follow the computer's name, if it has a handicap.
    pub fn words(self) -> Option<&'static str> {
        match (self.resources.is_some(), self.ignores_requirements) {
            (true, true) => Some("with unlimited resources and no requirements"),
            (true, false) => Some("with unlimited resources"),
            (false, true) => Some("with no requirements"),
            (false, false) => None,
        }
    }
}

/// The name of a computer of an AI type at a level, with its handicap, as
/// players choose it and hear it: "quiet computer, level 3", "aggressive
/// computer, level 1", "random computer with unlimited resources and no
/// requirements". Another type of `ai.txt` is named by its login, `ai_` and
/// the type, then its level. The random computer's level changes nothing,
/// so its name has none.
pub fn computer_name(ai_type: &str, skill: Skill) -> Msg {
    handicapped_name(ai_type, skill, Handicap::of(ai_type))
}

/// The name of a computer of an AI type at a level with this handicap.
pub(crate) fn handicapped_name(ai_type: &str, skill: Skill, handicap: Handicap) -> Msg {
    let mut name = match ai_type {
        "easy" => msg(mp::QUIET_COMPUTER),
        "aggressive" => msg(mp::AGGRESSIVE_COMPUTER),
        RANDOM_AI_TYPE => vec![MsgPart::from("random computer")],
        other => vec![MsgPart::Literal(format!("ai_{other}"))],
    };
    if ai_type != RANDOM_AI_TYPE {
        name.extend(msg(mp::COMMA));
        name.extend(msg(mp::LEVEL));
        name.extend(nb2msg(skill.number().into()));
    }
    name.extend(handicap.words().map(MsgPart::from));
    name
}
