//! What a player has paid for orders that have not delivered yet.
//!
//! An order pays when it is queued. Its payment is then either spent
//! (committed) when the order delivers, or given back (refunded) when it
//! cannot. The ledger holds each payment once, under what it was paid for:
//! a payment is made at most once for the same thing, and given back at
//! most once. Money moved without the ledger is spent at once and never
//! given back: a repair, and a building site's own cost when it is
//! cancelled.

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

/// What a `build` order asks for: the building, where, and the command
/// that gave it. The units of a group given one command share a project;
/// so do orders without a command for the same building at the same place.
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

/// A payment taken out of the ledger, to be committed or refunded. Only
/// [`Ledger::take`] makes one, from a payment [`Ledger::reserve`] made;
/// refunding or committing it uses it up.
#[must_use = "a payment taken out of the ledger is committed or refunded"]
#[derive(Debug)]
pub(crate) struct Payment {
    cost: Vec<i64>,
}

impl Payment {
    /// Gives the payment back to `resources`.
    pub(crate) fn refund(self, resources: &mut [i64]) {
        for (r, c) in resources.iter_mut().zip(self.cost) {
            *r += c;
        }
    }

    /// Spends the payment for good.
    pub(crate) fn commit(self) {}
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

    /// Takes the payment for `purpose` out of the ledger; `None` when none
    /// is held: it was never made, or was already committed or refunded.
    pub(crate) fn take(&mut self, purpose: &Purpose) -> Option<Payment> {
        self.held
            .remove(purpose)
            .map(|Held { cost, .. }| Payment { cost })
    }

    /// Gives back the payment for `purpose`, if one is held.
    pub(crate) fn refund(&mut self, resources: &mut [i64], purpose: &Purpose) {
        if let Some(payment) = self.take(purpose) {
            payment.refund(resources);
        }
    }

    /// Spends the payment for `purpose` for good, if one is held.
    pub(crate) fn commit(&mut self, purpose: &Purpose) {
        if let Some(payment) = self.take(purpose) {
            payment.commit();
        }
    }

    /// Each building project paid for, with the uid of the order that paid.
    pub(crate) fn projects(&self) -> impl Iterator<Item = (Project, u64)> + '_ {
        self.held
            .iter()
            .filter_map(|(purpose, held)| match purpose {
                Purpose::Building(project) => Some((*project, held.payer)),
                Purpose::Production(_) => None,
            })
    }

    /// Forgets every payment, spent: the player has left the game.
    pub(crate) fn forget(&mut self) {
        self.held.clear();
    }
}

/// A building project names the entity it is built on, for as long as it
/// is paid for.
impl Names for Ledger {
    fn names(&self, f: &mut impl FnMut(EntityId)) {
        for (project, _) in self.projects() {
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

    #[test]
    fn an_order_is_refunded_once() {
        let mut resources = [10000, 500];
        let mut ledger = ledger_with_a_peasant_paid(&mut resources);
        assert_eq!(resources, [6000, 500]);
        ledger.refund(&mut resources, &PEASANT);
        assert_eq!(resources, [10000, 500]);
        ledger.refund(&mut resources, &PEASANT);
        assert_eq!(resources, [10000, 500]);
        assert!(ledger.take(&PEASANT).is_none());
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
        ledger.commit(&PEASANT);
        ledger.refund(&mut resources, &PEASANT);
        assert_eq!(resources, [6000, 500]);
    }

    #[test]
    fn a_project_is_paid_once_by_every_order_of_it() {
        let farm = Purpose::Building(Project {
            building: TypeId(3),
            target: Some(TargetIdentity::Entity(EntityId(40))),
            command: Some(2),
        });
        let mut resources = [10000, 500];
        let mut ledger = Ledger::default();
        ledger.reserve(&mut resources, farm, 11, vec![1000, 200]);
        ledger.reserve(&mut resources, farm, 12, vec![1000, 200]);
        assert_eq!(resources, [9000, 300]);
        assert_eq!(
            ledger.projects().collect::<Vec<_>>(),
            [(
                Project {
                    building: TypeId(3),
                    target: Some(TargetIdentity::Entity(EntityId(40))),
                    command: Some(2),
                },
                11
            )]
        );
        let mut named = Vec::new();
        ledger.names(&mut |e| named.push(e));
        assert_eq!(named, [EntityId(40)]);
        ledger.refund(&mut resources, &farm);
        ledger.refund(&mut resources, &farm);
        assert_eq!(resources, [10000, 500]);
    }
}
