//! What the player knows of the game, in one place: the seat's view,
//! current from the moment a turn is taken; the interface's copy of the
//! entities and squares, which the end of a turn brings up to date a few
//! passes of the loop later, so that a turn's events are said in the
//! original's order; and the step from a unit's old id to its new one,
//! when a unit takes the place of another (an upgrade).
//!
//! Everything the interface names a unit by (the group, the selected
//! object, the orders picked, the commands not yet sent, the selection of
//! a game loaded) names it as the game names it now: a unit replaced is
//! followed as soon as the turn that replaced it is taken
//! ([`GameInterface::follow_replaced`]), and every command is followed
//! again when it is given ([`Knowledge::name_now`]). The copy still shows
//! the old unit until the end of that turn is handled: meanwhile
//! [`Knowledge::unit`] finds it by the new id, and [`Knowledge::entities`]
//! gives each entity shown with the id the game gives it now. Only the end
//! of a turn ([`GameInterface::update_fog_of_war`]) changes the copy.
//!
//! The questions and the selection read what the seat knows now
//! ([`Knowledge::seat`], [`Knowledge::current`]). The news, the sounds and
//! what is said of the targets and squares read the copy
//! ([`Knowledge::unit`], [`Knowledge::entities`]), so that they keep the
//! original's order.
//!
//! Nothing here reads the world: the seat's messages are all it knows.

use std::collections::BTreeSet;

use indexmap::IndexMap;
use soundrts_world::{
    Command, EntityId, Event, Order, OrderTarget, SeatView, SquareId, TurnMessage,
};

use super::GameInterface;
use super::entity::EntityView;

/// What the player knows of the game.
pub struct Knowledge {
    /// What the seat was told in the last turn taken: everything the
    /// interface reads of the game now.
    seat: SeatView,
    /// The entities the player perceives or remembers, as of the last end
    /// of turn handled, in arrival order, by the id each had then.
    shown: IndexMap<EntityId, EntityView>,
    /// The squares observed at the last end of turn handled.
    observed: BTreeSet<SquareId>,
    /// The squares observed before then (all of them in cheat mode).
    observed_before: BTreeSet<SquareId>,
    /// The game time of the last end of turn handled, in seconds.
    time: f64,
    /// The number of turns whose ends have been handled.
    turns_handled: u64,
    /// The units replaced, old and new, in order, whose new unit the copy
    /// does not show yet.
    replaced: Vec<(EntityId, EntityId)>,
}

impl Knowledge {
    /// What the seat is told when it joins: nothing shown yet.
    pub(super) fn new(seat: SeatView) -> Knowledge {
        Knowledge {
            seat,
            shown: IndexMap::new(),
            observed: BTreeSet::new(),
            observed_before: BTreeSet::new(),
            time: 0.0,
            turns_handled: 0,
            replaced: Vec::new(),
        }
    }

    /// Starts again from `seat`, a player's view the interface follows
    /// from now on: nothing is shown until its first end of turn. The game
    /// time and the count of turns handled go on.
    pub(super) fn restart(&mut self, seat: SeatView) {
        let (time, turns_handled) = (self.time, self.turns_handled);
        *self = Knowledge {
            time,
            turns_handled,
            ..Knowledge::new(seat)
        };
    }

    /// What the player knows of the game now.
    pub fn seat(&self) -> &SeatView {
        &self.seat
    }

    /// As [`Knowledge::seat`], to install what the seat is told.
    pub(super) fn seat_mut(&mut self) -> &mut SeatView {
        &mut self.seat
    }

    /// The unit the game names `id` now, as the copy shows it: while the
    /// end of the turn that replaced it waits, the unit it replaced.
    pub fn unit(&self, id: EntityId) -> Option<&EntityView> {
        self.shown.get(&self.shown_id(id)?)
    }

    /// The entity `id` as the player knows it now, from the last turn
    /// taken.
    pub fn current(&self, id: EntityId) -> Option<EntityView> {
        self.seat.known(id).map(EntityView::new)
    }

    /// The entities the player perceives now, from the last turn taken.
    pub fn perceived_now(&self) -> impl Iterator<Item = EntityView> {
        self.seat
            .message()
            .perceived
            .iter()
            .cloned()
            .map(EntityView::new)
    }

    /// As [`Knowledge::unit`], to change how it is presented.
    pub(super) fn unit_mut(&mut self, id: EntityId) -> Option<&mut EntityView> {
        let id = self.shown_id(id)?;
        self.shown.get_mut(&id)
    }

    /// The id under which the copy shows the unit the game names `id` now.
    fn shown_id(&self, mut id: EntityId) -> Option<EntityId> {
        for _ in 0..=self.replaced.len() {
            if self.shown.contains_key(&id) {
                return Some(id);
            }
            id = self.replaced.iter().rev().find(|(_, new)| *new == id)?.0;
        }
        None
    }

    /// The id the game gives now to the unit `id` named when it was shown.
    fn now(&self, mut id: EntityId) -> EntityId {
        for &(old, new) in &self.replaced {
            if id == old {
                id = new;
            }
        }
        id
    }

    /// The entities shown, in arrival order, each with the id the game
    /// gives it now.
    pub fn entities(&self) -> impl Iterator<Item = (EntityId, &EntityView)> {
        self.shown.iter().map(|(&id, v)| (self.now(id), v))
    }

    /// The entities shown, to change how they are presented.
    pub(super) fn entities_mut(&mut self) -> impl Iterator<Item = &mut EntityView> {
        self.shown.values_mut()
    }

    /// The copy itself, by the id each entity had when it was shown, for
    /// the end of a turn that brings it up to date.
    pub(super) fn copy_mut(&mut self) -> &mut IndexMap<EntityId, EntityView> {
        &mut self.shown
    }

    /// The squares observed now, in the last turn taken.
    pub fn observed_now(&self) -> &BTreeSet<SquareId> {
        &self.seat.message().squares.observed
    }

    /// The squares observed before the last turn taken.
    pub fn observed_before_now(&self) -> &BTreeSet<SquareId> {
        &self.seat.message().squares.observed_before
    }

    /// The squares observed at the last end of turn handled.
    pub fn observed(&self) -> &BTreeSet<SquareId> {
        &self.observed
    }

    /// The squares observed before the last end of turn handled.
    pub fn observed_before(&self) -> &BTreeSet<SquareId> {
        &self.observed_before
    }

    /// The game time of the last end of turn handled, in seconds.
    pub fn time(&self) -> f64 {
        self.time
    }

    #[cfg(test)]
    pub(super) fn set_time(&mut self, time: f64) {
        self.time = time;
    }

    /// The number of turns whose ends have been handled.
    pub(super) fn turns_handled(&self) -> u64 {
        self.turns_handled
    }

    /// The end of the turn `m` is handled: its time and squares are the
    /// copy's. Its entities follow ([`GameInterface::update_fog_of_war`]).
    pub(super) fn end_turn(&mut self, m: &TurnMessage) {
        self.turns_handled += 1;
        self.time = m.time as f64 / 1000.0;
        self.observed = m.squares.observed.clone();
        self.observed_before = m.squares.observed_before.clone();
        let still_old: Vec<bool> = self
            .replaced
            .iter()
            .map(|&(old, new)| !self.shown.contains_key(&new) && self.unit(old).is_some())
            .collect();
        let mut still_old = still_old.into_iter();
        self.replaced.retain(|_| still_old.next() == Some(true));
    }

    /// Makes `c` name each unit as the game names it now.
    pub(super) fn name_now(&self, c: &mut Command) {
        for &(old, new) in &self.replaced {
            replace_in_command(c, old, new);
        }
    }
}

/// [`Knowledge::unit`], for a unit the interface is known to show.
impl std::ops::Index<EntityId> for Knowledge {
    type Output = EntityView;

    fn index(&self, id: EntityId) -> &EntityView {
        self.unit(id).expect("a unit the interface shows")
    }
}

impl GameInterface {
    /// Follows each unit replaced in the turn `end` as soon as the turn is
    /// taken: everything the interface names it by names the unit that
    /// took its place, as the game does from now on. The selected object
    /// follows only if the player knows the new unit; otherwise it goes
    /// with the old one at the end of the turn.
    pub(super) fn follow_replaced(&mut self, end: &TurnMessage) {
        for e in &end.events {
            let &Event::Replaced { old, new } = e else {
                continue;
            };
            self.known.replaced.push((old, new));
            let follow = |u: &mut EntityId| {
                if *u == old {
                    *u = new;
                }
            };
            self.group.iter_mut().for_each(follow);
            self.previous_group.iter_mut().flatten().for_each(follow);
            for o in self.order.iter_mut().chain(&mut self.previous_order) {
                follow(&mut o.unit);
            }
            if let Some(l) = self.source.local_mut() {
                l.selection_to_restore.iter_mut().for_each(follow);
            }
            let known = end
                .perceived
                .iter()
                .chain(&end.remembered)
                .any(|e| e.id == new);
            if self.target == Some(old) && known {
                self.target = Some(new);
            }
            for c in &mut self.pending_commands {
                replace_in_command(c, old, new);
            }
        }
    }
}

/// Makes a command name `new` where it named `old`: the units it
/// controls, or the unit its order aims at.
fn replace_in_command(c: &mut Command, old: EntityId, new: EntityId) {
    let aimed = OrderTarget::Entity(old);
    match c {
        Command::Control(units) => {
            for u in units.iter_mut().filter(|u| **u == old) {
                *u = new;
            }
        }
        Command::Order { order, .. } => match order {
            Order::Default(t) if *t == aimed => *t = OrderTarget::Entity(new),
            Order::Keyword(o) if o.target() == Some(&aimed) => {
                *o = o.clone().at(OrderTarget::Entity(new));
            }
            Order::Default(_) | Order::Keyword(_) | Order::ResetGroup(_) | Order::JoinGroup(_) => {}
        },
        Command::Speed(_) | Command::Quit | Command::Cheat(_) => {}
    }
}
