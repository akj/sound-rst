//! An entity as one player knows it: what a [`View`](crate::View), the end
//! of a turn ([`Voila`](crate::Voila)) and an event
//! ([`Event::Entity`](crate::Event::Entity)) tell an interface about an
//! entity.
//!
//! A [`KnownEntity`] is plain owned data with no reference into the world,
//! so that a host can send it. It has two parts:
//!
//! - **The observable part**, which every known entity has: what any player
//!   who perceives or remembers the entity knows of it.
//! - **The own part** ([`OwnPart`]), only for an entity of a player whose
//!   units the player controls: its own, and an ally's that shares
//!   control. Any other entity has none, an enemy's above all, so its
//!   orders, cargo and the units inside it cannot be read.
//!
//! A projection is made from the world as it is when it is asked for: the
//! view's methods when they are called, the end of a turn when the turn
//! ends, an event when it happens. It is not updated afterwards.

use std::sync::Arc;

use crate::entity::{AiMode, Entity, EntityId, Place, PlayerId, SquareId, Target};
use crate::orders::{Keyword, Order};
use crate::rules::{AirGround, EntityClass, TypeId};
use crate::world::World;

/// An entity as one player knows it: perceived now, or remembered as it
/// was when the player last perceived it. Every field but [`own`] is the
/// observable part (see the [module](self)).
///
/// Lengths are in millimeters, hit points, mana and quantities in
/// thousandths, and times in milliseconds of game time, as in the world.
///
/// [`own`]: KnownEntity::own
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KnownEntity {
    pub id: EntityId,
    /// What kind of thing it is: a deposit, a meadow, an exit, a unit, a
    /// building...
    pub class: EntityClass,
    /// Its type in the rules and the style: `footman`, `goldmine`,
    /// `path`...
    pub type_name: Arc<str>,
    /// Its owner. Deposits, meadows and exits have none.
    pub player: Option<PlayerId>,
    /// Where it is. `None` once it has left the map.
    pub place: Option<Place>,
    pub x: i64,
    pub y: i64,
    /// The direction it faces, in degrees: 0 is east, 90 north.
    pub o: i64,
    /// Hit points, for a creature or anything that can be repaired.
    pub hp: i64,
    pub hp_max: i64,
    /// Mana, for a creature whose `mana_max` is not 0.
    pub mana: i64,
    pub mana_max: i64,
    /// What a deposit holds.
    pub qty: i64,
    /// For a building site, the type of the building it becomes.
    pub site_type: Option<TypeId>,
    /// The unit's number among its owner's units of its type: the `3` of
    /// `footman 3`. The owner's interface names its units with it.
    ///
    /// A leak, kept so that nothing said changes: only the owner should
    /// know it, since it follows the owner's other units of the type, but
    /// the interface reads it for any unit a map adds, to decide whether
    /// the arrival is worth saying. A host should not send it for another
    /// player's unit once that decision no longer needs it.
    pub number: Option<i64>,
    /// The exit it blocks: it also stands on the square on the other side.
    pub blocked_exit: Option<EntityId>,
    pub airground: AirGround,
    /// How fast it moves on the terrain it is on, in thousandths of the
    /// rules' `speed`: the pace of its footsteps.
    pub actual_speed: i64,
    /// Whether it is walking now.
    pub is_moving: bool,
    /// Whether it can move at all: its speed is not 0.
    pub is_mobile: bool,
    /// Whether its type is invisible.
    pub is_invisible: bool,
    /// Whether a cloaker hides it.
    pub is_cloaked: bool,
    pub is_a_detector: bool,
    pub is_a_cloaker: bool,
    /// Whether a building can be built on it: meadows.
    pub is_a_building_land: bool,
    pub is_repairable: bool,
    pub is_healable: bool,
    /// Whether it is a creature that can harm or carry others: one that
    /// counts as a soldier when a fight is lost or won.
    pub is_a_threat: bool,
    /// When the player last perceived it, for a remembered entity. `None`
    /// for an entity the player perceives now.
    pub last_seen: Option<i64>,
    /// What only a player who controls the entity knows: `None` for any
    /// other entity.
    pub own: Option<OwnPart>,
}

/// What only a player who controls an entity knows of it: see
/// [`KnownEntity::own`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnPart {
    /// Its orders, the one it carries out first.
    pub orders: Vec<KnownOrder>,
    /// Whether it may always receive some orders (go, attack...): a
    /// creature not being upgraded, or a building site, which may be
    /// cancelled.
    pub has_basic_abilities: bool,
    /// How far it still has to go to its goal. [`crate::INF`] without
    /// one.
    pub distance_to_goal: i64,
    /// The resource a worker carries: the resource's index and the
    /// quantity.
    pub cargo: Option<(i64, i64)>,
    /// The units inside a transport or a shelter. `None` for an entity that
    /// cannot hold any.
    pub inside: Option<Vec<EntityId>>,
    /// Whether it fights (offensive) or flees (defensive) when attacked.
    pub stance: Option<AiMode>,
    /// The id given to its `rallying_point` order.
    pub rallying_point: Option<String>,
    /// The name of the group it joined.
    pub group: Option<String>,
}

/// An order of an entity the player controls, as its interface describes
/// it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KnownOrder {
    pub keyword: Keyword,
    /// The type of an order that takes one: `footman` in `train footman`.
    pub type_name: Option<String>,
    /// Production waiting for food.
    pub is_deferred: bool,
    pub target: Option<KnownTarget>,
    /// For a patrol, the places patrolled, in order.
    pub patrol: Option<Vec<Option<Place>>>,
}

/// What an order aims at, as the player who controls the unit may know it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KnownTarget {
    /// A square, or a point of a square given by the player: the square.
    Square(SquareId),
    /// An entity, which the player knows as it perceives or remembers it,
    /// or not at all. An order of an ally that aims at the ally's memory
    /// of an entity is here too: the ally's memory is not the player's.
    Entity(EntityId),
    /// The player's own memory of an entity, as it was when the order was
    /// given. Its `last_seen` is when the player last perceived the
    /// entity, by the player's memory when the projection was made, or the
    /// time then if the player no longer remembers it.
    Remembered(Box<KnownEntity>),
    /// A point that is not given as a square: the interface has no name
    /// for it.
    Point,
}

impl KnownEntity {
    /// `e` as `viewer` knows it: perceived now, or remembered as it was
    /// `last_seen`.
    pub(crate) fn of(w: &World, viewer: PlayerId, e: &Entity, last_seen: Option<i64>) -> Self {
        let controlled = e
            .player
            .is_some_and(|p| w.player(viewer).allied_control.contains(&p));
        let s = &e.stats;
        KnownEntity {
            id: e.id,
            class: e.class,
            type_name: e.type_name.clone(),
            player: e.player,
            place: e.place,
            x: e.x,
            y: e.y,
            o: e.o,
            hp: e.hp,
            hp_max: s.hp_max,
            mana: e.mana,
            mana_max: s.mana_max,
            qty: s.qty,
            site_type: e.site.as_ref().map(|site| site.building_type),
            number: e.number,
            blocked_exit: e.blocked_exit,
            airground: e.airground_type,
            actual_speed: e.actual_speed,
            is_moving: e.is_moving,
            is_mobile: s.speed != 0,
            is_invisible: e.is_invisible(),
            is_cloaked: e.is_cloaked,
            is_a_detector: s.is_a_detector != 0,
            is_a_cloaker: s.is_a_cloaker != 0,
            is_a_building_land: e.is_a_building_land(),
            is_repairable: s.is_repairable != 0,
            is_healable: s.is_healable != 0,
            is_a_threat: e.menace() != 0,
            last_seen,
            own: controlled.then(|| OwnPart::of(w, viewer, e)),
        }
    }

    /// Creatures have hit points, mana and upgrades.
    pub fn is_creature(&self) -> bool {
        self.class.is_creature()
    }

    pub fn is_exit(&self) -> bool {
        self.class == EntityClass::Exit
    }

    pub fn is_deposit(&self) -> bool {
        self.class == EntityClass::Deposit
    }

    pub fn is_building_site(&self) -> bool {
        self.class == EntityClass::BuildingSite
    }

    /// Whether it is inside a transport or a building.
    pub fn is_inside(&self) -> bool {
        matches!(self.place, Some(Place::Inside(_)))
    }

    /// Whether the player remembers it rather than perceives it now.
    pub fn is_memory(&self) -> bool {
        self.last_seen.is_some()
    }
}

impl OwnPart {
    fn of(w: &World, viewer: PlayerId, e: &Entity) -> OwnPart {
        OwnPart {
            orders: e
                .orders
                .iter()
                .map(|o| KnownOrder::of(w, viewer, o))
                .collect(),
            has_basic_abilities: !e.basic_abilities().is_empty(),
            distance_to_goal: e.distance_to_goal,
            cargo: e.cargo,
            inside: e.inside.clone(),
            stance: e.ai_mode,
            rallying_point: e.rallying_point.clone(),
            group: e.group.clone(),
        }
    }
}

impl KnownOrder {
    fn of(w: &World, viewer: PlayerId, o: &Order) -> KnownOrder {
        KnownOrder {
            keyword: o.keyword,
            type_name: o.type_name.clone(),
            is_deferred: o.is_deferred,
            target: o.target.as_ref().map(|t| KnownTarget::of(w, viewer, t)),
            patrol: o.patrol_targets.clone(),
        }
    }
}

impl KnownTarget {
    fn of(w: &World, viewer: PlayerId, t: &Target) -> KnownTarget {
        match t {
            Target::Square(s) => KnownTarget::Square(*s),
            Target::Zoom(z) => KnownTarget::Square(z.place),
            Target::Entity(id) => KnownTarget::Entity(*id),
            Target::Memory(m) if m.owner == viewer => {
                let memory = w.player(viewer).memory.get(&m.snapshot.id);
                let last_seen = memory.map_or(w.time, |r| r.time_stamp);
                let e = KnownEntity::of(w, viewer, &m.snapshot, Some(last_seen));
                KnownTarget::Remembered(Box::new(e))
            }
            Target::Memory(m) => KnownTarget::Entity(m.snapshot.id),
            Target::Xy(..) => KnownTarget::Point,
        }
    }
}

#[cfg(test)]
mod tests;
