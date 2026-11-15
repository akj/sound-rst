//! An entity as one player knows it: what a turn's message
//! ([`TurnMessage`](crate::TurnMessage)) and an event
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
use crate::orders::{Keyword, Order, OrderKind};
use crate::questions;
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
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
    /// Whether it is a gate: one of the player or an ally lets the player
    /// through the exit it blocks.
    pub is_a_gate: bool,
    /// Whether it is a creature that can harm or carry others: one that
    /// counts as a soldier when a fight is lost or won.
    pub is_a_threat: bool,
    /// For an exit, what closes it.
    pub exit: Option<KnownExit>,
    /// What it is visibly busy with: for a perceived unit, as it is now;
    /// for a remembered one, as it was when last seen, with what it
    /// gathers from named as the player knows it when the projection is
    /// made.
    pub activity: Option<Activity>,
    /// The upgrades of its owner that apply to it, for a perceived entity
    /// with an owner. Empty for a remembered entity: its owner's upgrades
    /// now are not what the player saw, and the interface keeps those it
    /// was told when it last perceived the entity.
    pub upgrades: Vec<String>,
    /// When the player last perceived it, for a remembered entity. `None`
    /// for an entity the player perceives now.
    pub last_seen: Option<i64>,
    /// What only a player who controls the entity knows: `None` for any
    /// other entity.
    pub own: Option<OwnPart>,
}

/// An exit as a player knows it: what closes it. The square it is on and
/// its other side are part of the map, sent once
/// ([`MapExit`](crate::MapExit)).
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct KnownExit {
    /// A dense forest closes the path.
    pub is_blocked_by_forests: bool,
    /// The units and buildings blocking this side, in the order they came.
    pub blockers: Vec<EntityId>,
}

/// What a unit is visibly busy with: the noise it makes.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Activity {
    /// Building or repairing.
    Building,
    /// Gathering from a deposit or the woods of a square: the deposit's
    /// type (`goldmine`), or the square's terrain (`_forest`).
    Exploiting(String),
}

impl Activity {
    /// The style key of its noise: `building`, `exploiting_goldmine`...
    pub fn name(&self) -> String {
        match self {
            Activity::Building => "building".into(),
            Activity::Exploiting(t) => format!("exploiting_{t}"),
        }
    }
}

/// What only a player who controls an entity knows of it: see
/// [`KnownEntity::own`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
    /// Whether it is doing nothing: it has no target for an action. A unit
    /// that is not idle may be stopped.
    pub is_idle: bool,
    /// For a worker, whether it gathers by itself when idle.
    pub auto_gather: Option<bool>,
    /// For a worker, whether it repairs by itself when idle.
    pub auto_repair: Option<bool>,
    /// The room it has for other units, as a transport or a shelter. The
    /// units inside take some of it.
    pub transport_capacity: i64,
    /// The room it takes in a transport or a shelter.
    pub transport_volume: i64,
    /// The food it uses: upgrading it to another type costs the
    /// difference.
    pub food_cost: i64,
}

/// An order of an entity the player controls, as its interface describes
/// it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct KnownOrder {
    pub keyword: Keyword,
    /// The type of an order that takes one: `footman` in `train footman`.
    pub type_name: Option<String>,
    /// Production waiting for food.
    pub is_deferred: bool,
    /// Given as imperative: the unit carries it out whatever it meets.
    pub is_imperative: bool,
    /// For a training, a research or an upgrade, the game time left before
    /// it is done, in milliseconds: what the sounds of its progress tell.
    /// `None` for any other order.
    pub time_left: Option<i64>,
    pub target: Option<KnownTarget>,
    /// For a patrol, the places patrolled, in order.
    pub patrol: Option<Vec<Option<Place>>>,
}

/// What an order aims at, as the player who controls the unit may know it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
    Remembered(
        #[cfg_attr(feature = "serde", serde(with = "crate::wire::nested_entity"))] Box<KnownEntity>,
    ),
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
            is_a_gate: s.is_a_gate != 0,
            is_a_threat: e.menace() != 0,
            exit: e.exit.as_ref().map(|x| KnownExit {
                is_blocked_by_forests: x.is_blocked_by_forests,
                blockers: x.blockers.clone(),
            }),
            activity: match last_seen {
                None => w.activity_of(e),
                Some(_) => remembered_activity(w, viewer, e),
            },
            upgrades: match (last_seen, e.player) {
                (None, Some(p)) => questions::upgrades_applying(
                    e.type_id.map(|t| w.rules.ty(t)),
                    &w.player(p).upgrades,
                ),
                _ => Vec::new(),
            },
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
        let upgrading = e.orders.iter().any(|o| o.keyword() == Keyword::UpgradeTo);
        OwnPart {
            orders: e
                .orders
                .iter()
                .map(|o| KnownOrder::of(w, viewer, o))
                .collect(),
            has_basic_abilities: !questions::basic_abilities(e.class, upgrading).is_empty(),
            distance_to_goal: e.distance_to_goal,
            cargo: e.cargo,
            inside: e.inside.clone(),
            stance: e.ai_mode,
            rallying_point: e.rallying_point.clone(),
            group: e.group.clone(),
            is_idle: e.is_idle(),
            auto_gather: e.auto_gather,
            auto_repair: e.auto_repair,
            transport_capacity: e.stats.transport_capacity,
            transport_volume: e.stats.transport_volume,
            food_cost: e.stats.food_cost,
        }
    }
}

impl KnownOrder {
    fn of(w: &World, viewer: PlayerId, o: &Order) -> KnownOrder {
        let (time_left, patrol) = match &o.kind {
            OrderKind::Production(p) => (Some(p.time_left.max(0)), None),
            OrderKind::Patrol { route, .. } => (None, route.as_ref().map(|r| r.places.clone())),
            OrderKind::Go { .. }
            | OrderKind::Attack { .. }
            | OrderKind::Gather { .. }
            | OrderKind::AutoAttack
            | OrderKind::AutoExplore
            | OrderKind::Wait { .. }
            | OrderKind::Block { .. }
            | OrderKind::Repair { .. }
            | OrderKind::BuildPhaseTwo { .. }
            | OrderKind::Build { .. }
            | OrderKind::Use { .. }
            | OrderKind::Load { .. }
            | OrderKind::LoadAll { .. }
            | OrderKind::UnloadAll { .. } => (None, None),
        };
        KnownOrder {
            keyword: o.keyword(),
            type_name: o.kind.type_id().map(|t| w.rules.ty(t).name.clone()),
            is_deferred: o.is_deferred(),
            is_imperative: o.is_imperative,
            time_left,
            target: o.target().map(|t| KnownTarget::of(w, viewer, t)),
            patrol,
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

/// What a remembered entity was busy with when `viewer` last saw it, from
/// the remembered copy's first order. What it gathers from is named as
/// `viewer` knows it now: perceived, else remembered.
pub(crate) fn remembered_activity(w: &World, viewer: PlayerId, e: &Entity) -> Option<Activity> {
    questions::activity(e.orders.first(), |t| match t {
        Target::Entity(id) => {
            let pl = w.player(viewer);
            let perceived = pl.perception.contains(id).then(|| w.entity(*id)).flatten();
            let known = perceived.or_else(|| pl.memory.get(id).map(|r| &*r.snapshot))?;
            Some(&*known.type_name)
        }
        Target::Memory(m) => Some(&*m.snapshot.type_name),
        _ => None,
    })
}

#[cfg(test)]
mod tests;
