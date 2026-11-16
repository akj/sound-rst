//! How well a computer plays, apart from how it plays: its skill level.

/// How well a computer plays, from level 1, the weakest, to level 5. A
/// level applies to any AI type. At [`Skill::NORMAL`] a computer plays as
/// the original's computers do.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Skill {
    Level1,
    Level2,
    #[default]
    Level3,
    Level4,
    Level5,
}

impl Skill {
    /// The level that plays as the original's computers do.
    pub const NORMAL: Skill = Skill::Level3;

    /// Every level, from the weakest.
    pub const ALL: [Skill; 5] = [
        Skill::Level1,
        Skill::Level2,
        Skill::Level3,
        Skill::Level4,
        Skill::Level5,
    ];

    /// The number players hear: 1 to 5.
    pub fn number(self) -> u8 {
        match self {
            Skill::Level1 => 1,
            Skill::Level2 => 2,
            Skill::Level3 => 3,
            Skill::Level4 => 4,
            Skill::Level5 => 5,
        }
    }

    /// The level of a number from 1 to 5.
    pub fn from_number(n: u8) -> Option<Skill> {
        Skill::ALL.into_iter().find(|s| s.number() == n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_level_is_its_number_and_normal_is_three() {
        assert_eq!(Skill::default(), Skill::NORMAL);
        assert_eq!(Skill::NORMAL.number(), 3);
        assert_eq!(Skill::ALL.map(Skill::number), [1, 2, 3, 4, 5]);
        assert_eq!(Skill::from_number(1), Some(Skill::Level1));
        assert_eq!(Skill::from_number(5), Some(Skill::Level5));
        assert_eq!(Skill::from_number(0), None);
        assert_eq!(Skill::from_number(6), None);
    }
}
