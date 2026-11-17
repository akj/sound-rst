//! What a computer player knows of the battlefield beyond the menace by
//! square, which every player's units read: where it knows of enemies,
//! corpses and friends, where a harmful thing stands that will end, and
//! where its side's units were hurt. Only a computer player has it.

use std::collections::{BTreeMap, BTreeSet};

use crate::entity::{Place, SquareId};

use super::door::Door;

/// The tactical knowledge of a computer player. Its door fills the places
/// with corpses and friends from what it perceives and remembers; the world
/// adds where it knows of enemies, the cataclysms and the hits its side
/// takes.
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

    /// What the world finds of what the player perceives and remembers,
    /// and its door cannot show: where it knows of enemies, a by-product of
    /// the enemy menace, which reads a thing's range after its owner's
    /// upgrade levels; and the squares of a harmful thing that will end,
    /// since no player is told when a thing will end.
    pub(crate) fn set_from_the_world(
        &mut self,
        enemy_presence: Vec<SquareId>,
        cataclysmic_places: BTreeSet<SquareId>,
    ) {
        self.enemy_presence = enemy_presence;
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

impl Door<'_> {
    /// Finds again where it knows of corpses and of vulnerable units of
    /// its side, from what it perceives and remembers.
    pub(super) fn update_tactical_knowledge(&mut self) {
        if self.tactical().is_none() {
            return;
        }
        let mut corpses = BTreeSet::new();
        let mut friends = BTreeSet::new();
        let known = self.perceived().chain(self.remembered().map(|(s, _)| s));
        for o in known {
            let Some(Place::Square(place)) = o.place() else {
                continue;
            };
            if o.is_corpse() {
                corpses.insert(place);
            } else if o.owner().is_some_and(|op| self.allied().contains(&op)) && o.is_vulnerable() {
                friends.insert(place);
            }
        }
        if let Some(k) = self.tactical_mut() {
            k.places_with_corpses = corpses;
            k.places_with_friends = friends;
        }
    }
}
