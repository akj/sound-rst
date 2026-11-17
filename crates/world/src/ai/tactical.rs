//! What a computer player knows of the battlefield beyond the menace by
//! square, which every player's units read: where it knows of enemies,
//! corpses and friends, where a harmful thing stands that will end, and
//! where its side's units were hurt. Only a computer player has it.

use std::collections::{BTreeMap, BTreeSet};

use crate::entity::SquareId;

/// The tactical knowledge of a computer player. The world finds its places
/// at each update from what the player perceives and remembers, and adds
/// the hits its side takes. The computer reads it through its door.
#[derive(Clone, Debug, Default, Hash)]
pub(crate) struct TacticalKnowledge {
    /// The squares where it knows of an enemy, in the order found: each
    /// enemy's square that had no enemy menace yet when the enemy was
    /// counted. A square that ranged enemies around it made dangerous
    /// before an enemy was found on it is not here.
    enemy_presence: Vec<SquareId>,
    places_with_corpses: BTreeSet<SquareId>,
    /// The squares where it knows of a vulnerable unit of its side.
    places_with_friends: BTreeSet<SquareId>,
    /// The squares of a harmful thing that will end, where its units wait
    /// before they go.
    cataclysmic_places: BTreeSet<SquareId>,
    /// The damage its side's units took on each subsquare since the game
    /// began.
    subsquare_threat: BTreeMap<(i64, i64), i64>,
}

impl TacticalKnowledge {
    /// Whether it knows of an enemy on the square.
    pub(crate) fn knows_enemies_on(&self, s: SquareId) -> bool {
        self.enemy_presence.contains(&s)
    }

    pub(crate) fn is_cataclysmic(&self, s: SquareId) -> bool {
        self.cataclysmic_places.contains(&s)
    }

    /// The damage its side's units took on the subsquare.
    pub(crate) fn threat_on(&self, subsquare: (i64, i64)) -> i64 {
        self.subsquare_threat.get(&subsquare).copied().unwrap_or(0)
    }

    /// A unit of its side took damage on the subsquare.
    pub(crate) fn note_damage(&mut self, subsquare: (i64, i64), damage: i64) {
        *self.subsquare_threat.entry(subsquare).or_insert(0) += damage;
    }

    /// Its places, found again from what the player perceives and
    /// remembers. Three of them read a thing as it is, where a player is
    /// told less: where it knows of enemies, by the menace, reads a thing's
    /// range after its owner's upgrade levels; its friends, whether a thing
    /// can be hurt after them; the cataclysms, whether a thing will end.
    pub(crate) fn set_places(
        &mut self,
        enemy_presence: Vec<SquareId>,
        places_with_corpses: BTreeSet<SquareId>,
        places_with_friends: BTreeSet<SquareId>,
        cataclysmic_places: BTreeSet<SquareId>,
    ) {
        self.enemy_presence = enemy_presence;
        self.places_with_corpses = places_with_corpses;
        self.places_with_friends = places_with_friends;
        self.cataclysmic_places = cataclysmic_places;
    }

    pub(super) fn enemy_presence(&self) -> &[SquareId] {
        &self.enemy_presence
    }

    pub(super) fn has_corpses_on(&self, s: SquareId) -> bool {
        self.places_with_corpses.contains(&s)
    }

    pub(super) fn has_friends_on(&self, s: SquareId) -> bool {
        self.places_with_friends.contains(&s)
    }
}
