//! Checks of what a seat receives, before it is trusted: that it keeps
//! the promises of the types the world makes, so that a
//! [`SeatView`](crate::SeatView) built from it answers without panicking.
//! What the world makes passes them.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::collections::BTreeMap;

use crate::change::Malformed;
use crate::entity::{EntityId, Place, SquareId};
use crate::events::Event;
use crate::known::{KnownEntity, KnownTarget};
use crate::message::{GameEnd, GameStart, StartMessage, TurnMessage};
use crate::rules::TypeId;

/// The largest food cost, transport capacity or volume, either side of 0,
/// in thousandths. The rules give far smaller ones; within it, a cost less
/// another, or a capacity less the volumes of every unit a message could
/// name, cannot overflow.
const MAX_OWN_NUMBER: u64 = 1 << 32;

/// The largest amount a seat is told, either side of 0: a resource, food,
/// a statistic, a time. Far beyond any game's, and small enough that the
/// interface's sums and differences of them cannot overflow.
const MAX_AMOUNT: u64 = 1 << 48;

/// The most objectives a seat has at once, and the longest number and
/// description of one. A campaign's chapter has a few short ones.
pub const MAX_OBJECTIVES: usize = 64;
const MAX_OBJECTIVE_LENGTH: usize = 64;

fn amount(n: i64, what: &str) -> Result<(), Malformed> {
    if n.unsigned_abs() > MAX_AMOUNT {
        return malformed(format!("{what} out of range"));
    }
    Ok(())
}

fn malformed(why: impl Into<String>) -> Result<(), Malformed> {
    Err(Malformed(why.into()))
}

/// Whether the entities are by id: in increasing order, each once.
fn by_id(list: &[KnownEntity], what: &str) -> Result<(), Malformed> {
    if list.windows(2).any(|w| matches!(w, [a, b] if a.id >= b.id)) {
        return malformed(format!("{what} out of order"));
    }
    Ok(())
}

impl StartMessage {
    /// Fails unless every square and exit the map names is one of its own,
    /// and the exits are by id.
    pub fn check(&self) -> Result<(), Malformed> {
        let squares = self.map.squares.len();
        let known = |s: SquareId| (s.0 as usize) < squares;
        for (i, s) in self.map.squares.iter().enumerate() {
            if !s.neighbors.iter().all(|&n| known(n)) {
                return malformed(format!("square {i} has a neighbor not on the map"));
            }
        }
        if !self.map.exits.iter().all(|x| known(x.square)) {
            return malformed("an exit is not on the map");
        }
        if self
            .map
            .exits
            .windows(2)
            .any(|w| matches!(w, [a, b] if a.id >= b.id))
        {
            return malformed("exits out of order");
        }
        Ok(())
    }
}

/// Fails if an entity is inside itself, or inside one that is, however
/// far: the interface finds the square of an entity through its
/// containers.
fn inside_nothing_it_contains(m: &TurnMessage) -> Result<(), Malformed> {
    let mut container: BTreeMap<EntityId, Option<EntityId>> = BTreeMap::new();
    for e in m
        .unperceived_units
        .iter()
        .chain(&m.remembered)
        .chain(&m.perceived)
    {
        let inside = match e.place {
            Some(Place::Inside(c)) => Some(c),
            _ => None,
        };
        container.insert(e.id, inside);
    }
    // Each chain is walked once: `true` for an entity on the chain being
    // walked, `false` for one whose chain ends.
    let mut walked: BTreeMap<EntityId, bool> = BTreeMap::new();
    for &start in container.keys() {
        let mut chain = Vec::new();
        let mut at = Some(start);
        while let Some(e) = at {
            match walked.get(&e) {
                Some(true) => return malformed(format!("entity {e} is inside itself")),
                Some(false) => break,
                None => {}
            }
            walked.insert(e, true);
            chain.push(e);
            at = container.get(&e).copied().flatten();
        }
        for e in chain {
            walked.insert(e, false);
        }
    }
    Ok(())
}

impl GameEnd {
    /// Fails unless every statistic is within what a game counts.
    pub fn check(&self) -> Result<(), Malformed> {
        for n in self.stats.numbers() {
            amount(n, "a statistic")?;
        }
        Ok(())
    }
}

impl GameStart {
    /// Fails unless the message keeps the promises of a [`TurnMessage`]
    /// for this game: lists by id, an entity perceived or remembered and
    /// not both, and every square and unit type it names one of the
    /// game's.
    pub fn check(&self, m: &TurnMessage) -> Result<(), Malformed> {
        by_id(&m.perceived, "perceived entities")?;
        by_id(&m.remembered, "remembered entities")?;
        by_id(&m.unperceived_units, "unperceived units")?;
        if m.perceived.iter().any(KnownEntity::is_memory) {
            return malformed("a perceived entity is a memory");
        }
        if !m.remembered.iter().all(KnownEntity::is_memory) {
            return malformed("a remembered entity is not a memory");
        }
        let perceived = |id: EntityId| m.perceived.binary_search_by_key(&id, |e| e.id).is_ok();
        if m.remembered.iter().any(|e| perceived(e.id)) {
            return malformed("an entity is both perceived and remembered");
        }
        let squares = &m.squares;
        let named = squares
            .observed
            .iter()
            .chain(&squares.observed_before)
            .chain(&squares.partly_observed)
            .chain(squares.terrain.keys());
        for &s in named {
            self.square(s)?;
        }
        for e in &m.events {
            if let Event::Entity { entity, .. } = e {
                self.entity(entity)?;
            }
        }
        let lists = [&m.perceived, &m.remembered, &m.unperceived_units];
        for e in lists.into_iter().flatten() {
            self.entity(e)?;
        }
        amount(m.time, "the time")?;
        let s = &m.state;
        for &n in s.resources.iter().chain([&s.available_food, &s.used_food]) {
            amount(n, "an amount")?;
        }
        if s.objectives.len() > MAX_OBJECTIVES
            || s.objectives.iter().any(|o| {
                o.number.len() > MAX_OBJECTIVE_LENGTH || o.description.len() > MAX_OBJECTIVE_LENGTH
            })
        {
            return malformed("too many objectives, or too long");
        }
        for c in &s.controlled {
            let types = c.upgrades.iter().map(|(t, _)| t);
            for t in types.chain(c.forbidden_techs.iter().copied()) {
                self.unit_type(t)?;
            }
        }
        inside_nothing_it_contains(m)
    }

    fn unit_type(&self, t: TypeId) -> Result<(), Malformed> {
        if self.rules.types().get(t.0 as usize).is_none() {
            return malformed(format!("unknown unit type {}", t.0));
        }
        Ok(())
    }

    fn square(&self, s: SquareId) -> Result<(), Malformed> {
        if self.map.square(s).is_none() {
            return malformed(format!("square {} is not on the map", s.0));
        }
        Ok(())
    }

    fn place(&self, p: Option<Place>) -> Result<(), Malformed> {
        match p {
            Some(Place::Square(s)) => self.square(s),
            _ => Ok(()),
        }
    }

    /// Fails unless every square and type the entity names is the game's,
    /// and the numbers of its own part that the order menus and costs
    /// compute with are within [`MAX_OWN_NUMBER`].
    fn entity(&self, e: &KnownEntity) -> Result<(), Malformed> {
        self.place(e.place)?;
        if let Some(own) = &e.own {
            let numbers = [own.food_cost, own.transport_capacity, own.transport_volume];
            if numbers.iter().any(|n| n.unsigned_abs() > MAX_OWN_NUMBER) {
                return malformed(format!("entity {} has a number out of range", e.id));
            }
        }
        if e.site_type
            .is_some_and(|t| self.rules.types().get(t.0 as usize).is_none())
        {
            return malformed(format!("entity {} is a site of an unknown type", e.id));
        }
        for o in e.own.iter().flat_map(|own| &own.orders) {
            if let Some(t) = o.type_id {
                self.unit_type(t)?;
            }
            for &p in o.patrol.iter().flatten() {
                self.place(p)?;
            }
            match &o.target {
                Some(KnownTarget::Square(s)) => self.square(*s)?,
                Some(KnownTarget::Remembered(r)) => self.entity(r)?,
                _ => {}
            }
        }
        Ok(())
    }
}
