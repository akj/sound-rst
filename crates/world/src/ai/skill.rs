//! How well a computer plays, apart from how it plays: its skill level.

/// How well a computer plays, from level 1, the weakest, to level 3. A
/// level applies to any AI type. At [`Skill::NORMAL`], the top level, a
/// computer plays as the original's computers do.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Skill {
    Level1,
    Level2,
    #[default]
    Level3,
}

impl Skill {
    /// The level that plays as the original's computers do.
    pub const NORMAL: Skill = Skill::Level3;

    /// Every level, from the weakest.
    pub const ALL: [Skill; 3] = [Skill::Level1, Skill::Level2, Skill::Level3];

    /// The number players hear: 1 to 3.
    pub fn number(self) -> u8 {
        match self {
            Skill::Level1 => 1,
            Skill::Level2 => 2,
            Skill::Level3 => 3,
        }
    }

    /// The level of a number from 1 to 3.
    pub fn from_number(n: u8) -> Option<Skill> {
        Skill::ALL.into_iter().find(|s| s.number() == n)
    }

    /// How the computer plays at this level. The engine applies these
    /// settings around any script of `ai.txt`, so that a mod's scripts
    /// have every level too.
    pub(super) fn settings(self) -> Settings {
        match self {
            Skill::Level1 => Settings {
                grace: 600_000,
                workers: Workers::AtMost(6),
                turns_between_decisions: 40,
            },
            Skill::Level2 => Settings {
                grace: 300_000,
                workers: Workers::AtMost(8),
                turns_between_decisions: 30,
            },
            Skill::Level3 => Settings {
                grace: 0,
                workers: Workers::AtLeast(10),
                turns_between_decisions: 10,
            },
        }
    }
}

/// How a computer plays at a skill level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Settings {
    /// The game time, in milliseconds, before which the computer starts no
    /// attack: neither the attacks of its script nor its constant attacks.
    /// It still defends a place of its own that is attacked, and still
    /// explores.
    pub(super) grace: i64,
    /// How many workers it gets.
    pub(super) workers: Workers,
    /// It decides once in this many turns. The computers that decide at
    /// the same rate share these turns between them.
    pub(super) turns_between_decisions: i64,
}

/// How many workers a computer gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Workers {
    /// It keeps this many, and gets more when its script asks for more.
    AtLeast(i64),
    /// It keeps this many, and gets no more when its script asks for more.
    AtMost(i64),
}

impl Workers {
    /// The number it keeps getting.
    pub(super) fn kept(self) -> i64 {
        match self {
            Workers::AtLeast(n) | Workers::AtMost(n) => n,
        }
    }

    /// The number of workers it gets when its script asks for `asked`.
    pub(super) fn asked(self, asked: i64) -> i64 {
        match self {
            Workers::AtLeast(_) => asked,
            Workers::AtMost(n) => asked.min(n),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_level_is_its_number_and_normal_is_three() {
        assert_eq!(Skill::default(), Skill::NORMAL);
        assert_eq!(Skill::NORMAL.number(), 3);
        assert_eq!(Skill::ALL.map(Skill::number), [1, 2, 3]);
        assert_eq!(Skill::from_number(1), Some(Skill::Level1));
        assert_eq!(Skill::from_number(3), Some(Skill::Level3));
        assert_eq!(Skill::from_number(0), None);
        assert_eq!(Skill::from_number(4), None);
    }
}
