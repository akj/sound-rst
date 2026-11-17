//! How well a computer plays, apart from how it plays: its skill level.

/// How well a computer plays, from level 1, the weakest, to level 4. A
/// level applies to any AI type. At [`Skill::NORMAL`], level 3, a computer
/// plays as the original's computers do. Level 4 aims to improve on level
/// 3, with the same resources and knowledge as level 3.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Skill {
    Level1,
    Level2,
    #[default]
    Level3,
    Level4,
}

impl Skill {
    /// The level that plays as the original's computers do.
    pub const NORMAL: Skill = Skill::Level3;

    /// Every level, from the weakest.
    pub const ALL: [Skill; 4] = [Skill::Level1, Skill::Level2, Skill::Level3, Skill::Level4];

    /// The number players hear: 1 to 4.
    pub fn number(self) -> u8 {
        match self {
            Skill::Level1 => 1,
            Skill::Level2 => 2,
            Skill::Level3 => 3,
            Skill::Level4 => 4,
        }
    }

    /// The level of a number from 1 to 4.
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
                ..Settings::NORMAL
            },
            Skill::Level2 => Settings {
                grace: 300_000,
                workers: Workers::AtMost(8),
                turns_between_decisions: 30,
                ..Settings::NORMAL
            },
            Skill::Level3 => Settings::NORMAL,
            Skill::Level4 => Settings {
                workers: Workers::WithSpare {
                    kept: 10,
                    spare: 15,
                },
                more_makers: MoreMakers::CountingSites(3),
                research: Research::AsSoonAsAffordable,
                defence: Defence::WhenOutnumbered,
                ..Settings::NORMAL
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
    /// When it builds one more maker of the units it trains.
    pub(super) more_makers: MoreMakers,
    /// When it researches an upgrade.
    pub(super) research: Research,
    /// What it does when it knows of more enemy fighters than it has.
    pub(super) defence: Defence,
}

impl Settings {
    /// The settings of [`Skill::NORMAL`]: the original's computer.
    const NORMAL: Settings = Settings {
        grace: 0,
        workers: Workers::AtLeast(10),
        turns_between_decisions: 10,
        more_makers: MoreMakers::AsTheOriginal,
        research: Research::AsTheOriginal,
        defence: Defence::AsTheOriginal,
    };
}

/// How many workers a computer gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Workers {
    /// It keeps this many, and gets more when its script asks for more.
    AtLeast(i64),
    /// It keeps this many, and gets no more when its script asks for more.
    AtMost(i64),
    /// It keeps `kept`, and gets more when its script asks for more. After
    /// the purchases of its turn, while it could still pay for two workers,
    /// it gets up to `spare`.
    WithSpare { kept: i64, spare: i64 },
}

impl Workers {
    /// The number it keeps getting.
    pub(super) fn kept(self) -> i64 {
        match self {
            Workers::AtLeast(n) | Workers::AtMost(n) | Workers::WithSpare { kept: n, .. } => n,
        }
    }

    /// The number of workers it gets when its script asks for `asked`.
    pub(super) fn asked(self, asked: i64) -> i64 {
        match self {
            Workers::AtLeast(_) | Workers::WithSpare { .. } => asked,
            Workers::AtMost(n) => asked.min(n),
        }
    }

    /// The number it gets from resources to spare, if it does.
    pub(super) fn spare(self) -> Option<i64> {
        match self {
            Workers::WithSpare { spare, .. } => Some(spare),
            Workers::AtLeast(_) | Workers::AtMost(_) => None,
        }
    }
}

/// When a computer builds one more maker, such as a barracks, of the units
/// it trains: when it wants more than three of the units for each maker,
/// and could pay for more than some number of them for each maker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MoreMakers {
    /// As in the original: for each maker it has built, it must be able to
    /// pay for more than 100 units.
    AsTheOriginal,
    /// For each maker it has or is building, it must be able to pay for
    /// more than this many units.
    CountingSites(i64),
}

/// When a computer whose script lets it research researches an upgrade, at
/// an idle building that can.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Research {
    /// As in the original: when it could pay for the upgrade more than
    /// three times over.
    AsTheOriginal,
    /// As soon as it can pay for the upgrade, unless it is outnumbered and
    /// defends (see [`Defence::WhenOutnumbered`]).
    AsSoonAsAffordable,
}

/// What a computer does when it knows of more enemy fighters than it has:
/// soldiers whose type can attack, of players that are not neutral.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Defence {
    /// As in the original: nothing more than its script says.
    AsTheOriginal,
    /// Before its script's purchases, it works toward as many fighters as
    /// the enemy fighters it knows of, of the cheapest type that a maker it
    /// has can train now. Meanwhile it gets no spare workers, and
    /// researches only as in the original.
    WhenOutnumbered,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_level_is_its_number_and_normal_is_three() {
        assert_eq!(Skill::default(), Skill::NORMAL);
        assert_eq!(Skill::NORMAL.number(), 3);
        assert_eq!(Skill::ALL.map(Skill::number), [1, 2, 3, 4]);
        assert_eq!(Skill::from_number(1), Some(Skill::Level1));
        assert_eq!(Skill::from_number(3), Some(Skill::Level3));
        assert_eq!(Skill::from_number(4), Some(Skill::Level4));
        assert_eq!(Skill::from_number(0), None);
        assert_eq!(Skill::from_number(5), None);
    }
}
