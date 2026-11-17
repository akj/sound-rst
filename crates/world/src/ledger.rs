//! What a player has paid for orders that have not delivered yet.
//!
//! An order pays when it is queued. Its payment is then either spent
//! (committed) when the order delivers, or given back (refunded) when it
//! cannot. The ledger holds each payment once, under what it was paid for:
//! a payment is made at most once for the same thing, and given back at
//! most once. Money moved without the ledger is spent at once and never
//! given back: a repair; a building site's own cost when it is cancelled;
//! and the cost of a site put down after its project's payment was given
//! back, because an order of the project that came first was refused at
//! the site.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use crate::entity::{EntityId, Names, TargetIdentity};
use crate::rules::TypeId;

/// What a payment is for: the ledger's key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Purpose {
    /// A production order (train, research, upgrade), by its uid.
    Production(u64),
    /// A building project. Every `build` order of the same project shares
    /// the payment of the first.
    Building(Project),
}

/// What a `build` order asks for: the building, where it aims now, and
/// the command that gave it. The units of a group given one command share
/// a project; so do orders without a command for the same building at the
/// same place.
///
/// The land it names may have left the map since, taken by another
/// building: the ledger only compares its id and never looks it up, and an
/// id is never given to another entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct Project {
    pub(crate) building: TypeId,
    pub(crate) target: Option<TargetIdentity>,
    pub(crate) command: Option<u64>,
}

/// A payment held in the ledger.
#[derive(Clone, Debug, Hash)]
struct Held {
    cost: Vec<i64>,
    /// The uid of the order that paid.
    payer: u64,
}

/// The payments of one player, by what each was paid for, in the order
/// of their keys.
#[derive(Clone, Debug, Default, Hash)]
pub(crate) struct Ledger {
    held: BTreeMap<Purpose, Held>,
}

impl Ledger {
    /// Pays `cost` out of `resources` for `purpose`, by the order `payer`,
    /// unless a payment for it is already held.
    pub(crate) fn reserve(
        &mut self,
        resources: &mut [i64],
        purpose: Purpose,
        payer: u64,
        cost: Vec<i64>,
    ) {
        if let Entry::Vacant(slot) = self.held.entry(purpose) {
            for (r, c) in resources.iter_mut().zip(&cost) {
                *r -= c;
            }
            slot.insert(Held { cost, payer });
        }
    }

    /// Whether a payment for `purpose` is held.
    pub(crate) fn is_reserved(&self, purpose: &Purpose) -> bool {
        self.held.contains_key(purpose)
    }

    /// Gives back the payment for `purpose` to `resources`, if one is held.
    pub(crate) fn refund(&mut self, resources: &mut [i64], purpose: &Purpose) {
        if let Some(Held { cost, .. }) = self.held.remove(purpose) {
            for (r, c) in resources.iter_mut().zip(cost) {
                *r += c;
            }
        }
    }

    /// Spends the payment for `purpose` for good. False when none was held:
    /// it was never made, or was already committed or refunded.
    pub(crate) fn commit(&mut self, purpose: &Purpose) -> bool {
        self.held.remove(purpose).is_some()
    }

    /// The order `payer`, a `build` order of `from`, now aims elsewhere, so
    /// its project is `to`: the payment it made follows it. Another order of
    /// `from` then shares it no more, and only an order of `to` holds it.
    /// Two payments that come to the same project, which happens when the
    /// orders that made them both lost their land, are held together and
    /// given back together.
    pub(crate) fn follow(&mut self, payer: u64, from: Project, to: Project) {
        let Entry::Occupied(paid) = self.held.entry(Purpose::Building(from)) else {
            return;
        };
        if paid.get().payer != payer {
            return;
        }
        let moved = paid.remove();
        match self.held.entry(Purpose::Building(to)) {
            Entry::Vacant(slot) => {
                slot.insert(moved);
            }
            Entry::Occupied(mut slot) => {
                let held = slot.get_mut();
                for (h, m) in held.cost.iter_mut().zip(&moved.cost) {
                    *h += m;
                }
            }
        }
    }

    /// Each building project paid for.
    pub(crate) fn projects(&self) -> impl Iterator<Item = Project> + '_ {
        self.held.keys().filter_map(|purpose| match purpose {
            Purpose::Building(project) => Some(*project),
            Purpose::Production(_) => None,
        })
    }

    /// Forgets every payment, spent: the player has left the game.
    pub(crate) fn forget(&mut self) {
        self.held.clear();
    }
}

/// A building project names the entity it is built on, for as long as it
/// is paid for, as the orders aiming at it do.
impl Names for Ledger {
    fn names(&self, f: &mut impl FnMut(EntityId)) {
        for project in self.projects() {
            if let Some(TargetIdentity::Entity(e)) = project.target {
                f(e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PEASANT: Purpose = Purpose::Production(7);

    fn ledger_with_a_peasant_paid(resources: &mut [i64]) -> Ledger {
        let mut ledger = Ledger::default();
        ledger.reserve(resources, PEASANT, 7, vec![4000, 0]);
        ledger
    }

    fn farm(target: Option<TargetIdentity>) -> Project {
        Project {
            building: TypeId(3),
            target,
            command: Some(2),
        }
    }

    const MEADOW: Option<TargetIdentity> = Some(TargetIdentity::Entity(EntityId(40)));

    #[test]
    fn an_order_is_refunded_once() {
        let mut resources = [10000, 500];
        let mut ledger = ledger_with_a_peasant_paid(&mut resources);
        assert_eq!(resources, [6000, 500]);
        ledger.refund(&mut resources, &PEASANT);
        assert_eq!(resources, [10000, 500]);
        ledger.refund(&mut resources, &PEASANT);
        assert_eq!(resources, [10000, 500]);
        assert!(!ledger.commit(&PEASANT));
    }

    #[test]
    fn an_order_never_paid_is_not_refunded() {
        let mut resources = [10000, 500];
        let mut ledger = ledger_with_a_peasant_paid(&mut resources);
        // A second peasant, refused for lack of gold, never paid.
        ledger.refund(&mut resources, &Purpose::Production(8));
        assert_eq!(resources, [6000, 500]);
        ledger.refund(&mut resources, &PEASANT);
        assert_eq!(resources, [10000, 500]);
    }

    #[test]
    fn a_committed_order_is_not_refunded() {
        let mut resources = [10000, 500];
        let mut ledger = ledger_with_a_peasant_paid(&mut resources);
        assert!(ledger.commit(&PEASANT));
        ledger.refund(&mut resources, &PEASANT);
        assert_eq!(resources, [6000, 500]);
        assert!(!ledger.commit(&PEASANT));
    }

    #[test]
    fn a_project_is_paid_once_by_every_order_of_it() {
        let paid = Purpose::Building(farm(MEADOW));
        let mut resources = [10000, 500];
        let mut ledger = Ledger::default();
        ledger.reserve(&mut resources, paid, 11, vec![1000, 200]);
        ledger.reserve(&mut resources, paid, 12, vec![1000, 200]);
        assert_eq!(resources, [9000, 300]);
        assert_eq!(ledger.projects().collect::<Vec<_>>(), [farm(MEADOW)]);
        let mut named = Vec::new();
        ledger.names(&mut |e| named.push(e));
        assert_eq!(named, [EntityId(40)]);
        ledger.refund(&mut resources, &paid);
        ledger.refund(&mut resources, &paid);
        assert_eq!(resources, [10000, 500]);
    }

    #[test]
    fn a_payment_follows_the_order_that_made_it_and_no_other() {
        let mut resources = [10000, 500];
        let mut ledger = Ledger::default();
        ledger.reserve(
            &mut resources,
            Purpose::Building(farm(MEADOW)),
            11,
            vec![1000, 200],
        );
        // Order 12 shares the project but did not pay: nothing moves.
        ledger.follow(12, farm(MEADOW), farm(None));
        assert_eq!(ledger.projects().collect::<Vec<_>>(), [farm(MEADOW)]);
        ledger.follow(11, farm(MEADOW), farm(None));
        assert_eq!(ledger.projects().collect::<Vec<_>>(), [farm(None)]);
        assert!(!ledger.is_reserved(&Purpose::Building(farm(MEADOW))));
        ledger.refund(&mut resources, &Purpose::Building(farm(None)));
        assert_eq!(resources, [10000, 500]);
    }

    #[test]
    fn two_payments_that_lose_their_land_are_given_back_together() {
        let other = Some(TargetIdentity::Entity(EntityId(41)));
        let mut resources = [10000, 500];
        let mut ledger = Ledger::default();
        ledger.reserve(
            &mut resources,
            Purpose::Building(farm(MEADOW)),
            11,
            vec![1000, 200],
        );
        ledger.reserve(
            &mut resources,
            Purpose::Building(farm(other)),
            13,
            vec![1000, 200],
        );
        ledger.follow(11, farm(MEADOW), farm(None));
        ledger.follow(13, farm(other), farm(None));
        assert_eq!(resources, [8000, 100]);
        assert_eq!(ledger.projects().collect::<Vec<_>>(), [farm(None)]);
        ledger.refund(&mut resources, &Purpose::Building(farm(None)));
        assert_eq!(resources, [10000, 500]);
    }
}
