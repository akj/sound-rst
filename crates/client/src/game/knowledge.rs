//! What the player knows of the game, in one place: the seat's view,
//! current from the moment a turn is taken; the interface's copy of the
//! entities and squares, which the end of a turn brings up to date a few
//! passes of the loop later, so that a turn's events are said in the
//! original's order; and the step from a unit's old id to its new one,
//! when a unit takes the place of another (an upgrade).
//!
//! A unit replaced in a turn that also tells the player of the new unit
//! (of the last one, when the turn replaced the new one in turn) is
//! followed as soon as the turn is taken
//! ([`GameInterface::follow_replaced`]): everything the interface names a
//! unit by (the group, the selected object, the orders picked, the
//! commands not yet sent, the selection of a game loaded) names the new
//! unit from then on, as the game does. A unit replaced by one the player
//! does not know is lost like any unit that goes out of the player's
//! knowledge. The copy still shows the old unit until the end of the turn
//! is handled: meanwhile [`Knowledge::unit`] finds it by the new id, and
//! [`Knowledge::entities`] gives each entity shown with the id the game
//! gives it now. Only the end of a turn
//! ([`GameInterface::update_fog_of_war`]) changes the copy.
//!
//! The production, workers and time questions and the selection of units
//! read what the seat knows now ([`Knowledge::seat`],
//! [`Knowledge::current`]). The news, the sounds, the fight question, and
//! what is said of the targets and squares read the copy
//! ([`Knowledge::unit`], [`Knowledge::views`]), so that they keep the
//! original's order and each sentence describes one moment.
//!
//! Nothing here reads the world: the seat's messages are all it knows.

use std::collections::{BTreeSet, HashMap, HashSet};

use indexmap::IndexMap;
use soundrts_world::{
    Command, EntityId, Event, KnownEntity, Order, OrderTarget, SeatView, SquareId, TurnMessage,
};

use super::GameInterface;
use super::entity::EntityView;

/// A unit that took the place of another in a turn taken.
#[derive(Clone, Copy)]
struct Replaced {
    /// The end of turn that told it, counted as the ends are taken.
    end: u64,
    old: EntityId,
    new: EntityId,
}

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
    /// The number of ends of turn taken: those not handled wait in the
    /// interface's queue.
    ends_taken: u64,
    /// The units replaced in the ends of turn taken, from the last one
    /// handled on, in order.
    replaced: Vec<Replaced>,
    /// The replacements read so far, to find units by the ids they have
    /// now and to give each entity shown its id now.
    #[cfg(any(test, feature = "testkit"))]
    steps_read: std::cell::Cell<usize>,
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
            ends_taken: 0,
            replaced: Vec::new(),
            #[cfg(any(test, feature = "testkit"))]
            steps_read: std::cell::Cell::new(0),
        }
    }

    /// Starts again from `seat`, a player's view the interface follows
    /// from now on: nothing is shown until its first end of turn, and no
    /// end of turn waits. The game time and the count of turns handled go
    /// on.
    pub(super) fn restart(&mut self, seat: SeatView) {
        let (time, turns_handled) = (self.time, self.turns_handled);
        *self = Knowledge {
            time,
            turns_handled,
            ends_taken: turns_handled,
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

    /// The entity `id` as the player knows it now, from the last turn
    /// taken: `None` if the player neither perceives nor remembers it.
    pub fn current(&self, id: EntityId) -> Option<EntityView> {
        self.seat.known(id).map(EntityView::new)
    }

    /// The unit the game names `id` now, as the copy shows it: while the
    /// end of the turn that replaced it waits, the unit it replaced.
    pub fn unit(&self, id: EntityId) -> Option<&EntityView> {
        self.shown.get(&self.shown_id(id)?)
    }

    /// As [`Knowledge::unit`], to change how it is presented.
    pub(super) fn unit_mut(&mut self, id: EntityId) -> Option<&mut EntityView> {
        let id = self.shown_id(id)?;
        self.shown.get_mut(&id)
    }

    /// The id under which the copy shows the unit the game names `id` now:
    /// back through the replacements, newest first, in one pass.
    fn shown_id(&self, mut id: EntityId) -> Option<EntityId> {
        if self.shown.contains_key(&id) {
            return Some(id);
        }
        for r in self.replaced.iter().rev() {
            self.read_a_step();
            if r.new == id {
                id = r.old;
                if self.shown.contains_key(&id) {
                    return Some(id);
                }
            }
        }
        None
    }

    /// Counts a replacement read.
    fn read_a_step(&self) {
        #[cfg(any(test, feature = "testkit"))]
        self.steps_read.set(self.steps_read.get() + 1);
    }

    /// The id the game gives now to each entity shown that a replacement
    /// moved, by the id it was shown with, in one pass over the
    /// replacements.
    fn ids_now(&self) -> HashMap<EntityId, EntityId> {
        let steps = self.replaced.iter().map(|r| {
            self.read_a_step();
            (r.old, r.new)
        });
        let starts = |id| self.shown.contains_key(&id);
        chain_ends(steps, starts).into_iter().collect()
    }

    /// The entities shown, in arrival order, each with the id the game
    /// gives it now: what the player may pick as the target.
    pub fn entities(&self) -> impl Iterator<Item = (EntityId, &EntityView)> {
        let now = self.ids_now();
        self.shown
            .iter()
            .map(move |(&id, v)| (now.get(&id).copied().unwrap_or(id), v))
    }

    /// The entities shown, in arrival order.
    pub fn views(&self) -> impl Iterator<Item = &EntityView> {
        self.shown.values()
    }

    /// The entities shown, to change how they are presented.
    pub(super) fn views_mut(&mut self) -> impl Iterator<Item = &mut EntityView> {
        self.shown.values_mut()
    }

    /// The copy, by the id each entity was shown with: only the end of a
    /// turn, which brings it up to date, reads it so.
    pub(super) fn end_of_turn_copy(&self) -> &IndexMap<EntityId, EntityView> {
        &self.shown
    }

    /// As [`Knowledge::end_of_turn_copy`], for the end of a turn to change it.
    pub(super) fn end_of_turn_copy_mut(&mut self) -> &mut IndexMap<EntityId, EntityView> {
        &mut self.shown
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

    /// The number of replacements kept.
    pub fn replacements(&self) -> usize {
        self.replaced.len()
    }

    /// The replacements read so far, to find units by the ids they have
    /// now and to give each entity shown its id now.
    #[cfg(any(test, feature = "testkit"))]
    pub fn steps_read(&self) -> usize {
        self.steps_read.get()
    }

    /// The end of the turn `m` is handled: its time and squares are the
    /// copy's. Its entities follow ([`GameInterface::update_fog_of_war`]),
    /// which shows the units that turn put in place of others: the
    /// replacements of the ends handled before it are no longer needed.
    pub(super) fn end_turn(&mut self, m: &TurnMessage) {
        self.turns_handled += 1;
        self.ends_taken = self.ends_taken.max(self.turns_handled);
        self.time = m.time as f64 / 1000.0;
        self.observed = m.squares.observed.clone();
        self.observed_before = m.squares.observed_before.clone();
        let handled = self.turns_handled;
        self.replaced.retain(|r| r.end >= handled);
    }

    /// The ends of turn waiting are dropped, untold: so are their
    /// replacements.
    pub(super) fn forget_turns_waiting(&mut self) {
        self.replaced.clear();
        self.ends_taken = self.turns_handled;
    }
}

/// Where the chains of `steps` end, in one pass: each step moves every id
/// that has reached its old id to its new one, in order, as the world
/// replaces units; an id that `starts` joins the first time it is a
/// step's old id. Each id that moved, with the last id it reached, in the
/// order of the steps that moved them last.
fn chain_ends(
    steps: impl Iterator<Item = (EntityId, EntityId)>,
    starts: impl Fn(EntityId) -> bool,
) -> Vec<(EntityId, EntityId)> {
    // The ids that have reached each id, with the step that moved them
    // there; and the ids that have joined.
    let mut at: HashMap<EntityId, (usize, Vec<EntityId>)> = HashMap::new();
    let mut joined = HashSet::new();
    for (i, (old, new)) in steps.enumerate() {
        let mut moving = at.remove(&old).map(|(_, ids)| ids).unwrap_or_default();
        if starts(old) && joined.insert(old) {
            moving.push(old);
        }
        if moving.is_empty() {
            continue;
        }
        let there = at.entry(new).or_insert((i, Vec::new()));
        there.0 = i;
        if there.1.len() < moving.len() {
            std::mem::swap(&mut there.1, &mut moving);
        }
        there.1.extend(moving);
    }
    let mut ends: Vec<(usize, EntityId, EntityId)> = at
        .into_iter()
        .flat_map(|(end, (i, ids))| ids.into_iter().map(move |id| (i, id, end)))
        .filter(|&(_, id, end)| id != end)
        .collect();
    ends.sort_unstable();
    ends.into_iter().map(|(_, id, end)| (id, end)).collect()
}

/// Whether `list`, by id, has the entity `id`.
fn lists(list: &[KnownEntity], id: EntityId) -> bool {
    list.binary_search_by_key(&id, |e| e.id).is_ok()
}

impl GameInterface {
    /// Takes the end of the turn `end` as it comes, before it waits in the
    /// queue, and follows each unit it replaced to the last unit of its
    /// chain, when the turn tells the player of that one and the copy does
    /// not show it already: everything the interface names the old unit by
    /// names the last one, as the game does from now on. A turn that
    /// catches up others may replace a unit by one that another replaced
    /// in turn: the units between, which the player is not told of, are
    /// never named.
    pub(super) fn follow_replaced(&mut self, end: &TurnMessage) {
        self.known.ends_taken += 1;
        let at = self.known.ends_taken;
        let steps: Vec<(EntityId, EntityId)> = end
            .events
            .iter()
            .filter_map(|e| match *e {
                Event::Replaced { old, new } => Some((old, new)),
                _ => None,
            })
            .collect();
        // A unit a turn put in place of another starts no chain of its own
        // in that turn: it is in the middle of one.
        let mut first_new: HashMap<EntityId, usize> = HashMap::new();
        let mut first_old: HashMap<EntityId, usize> = HashMap::new();
        for (i, &(old, new)) in steps.iter().enumerate() {
            first_old.entry(old).or_insert(i);
            first_new.entry(new).or_insert(i);
        }
        let starts = |id| match (first_new.get(&id), first_old.get(&id)) {
            (Some(n), Some(o)) => n > o,
            _ => true,
        };
        for (old, new) in chain_ends(steps.iter().copied(), starts) {
            let told = lists(&end.perceived, new) || lists(&end.remembered, new);
            if !told || self.known.shown.contains_key(&new) {
                continue;
            }
            self.known.replaced.push(Replaced { end: at, old, new });
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
            self.target.iter_mut().for_each(follow);
            for c in &mut self.pending_commands {
                replace_in_command(c, old, new);
            }
        }
    }

    /// Drops what waits of the turns taken, untold.
    pub(super) fn drop_turns_waiting(&mut self) {
        self.srv_queue.clear();
        self.known.forget_turns_waiting();
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
