//! Entities: everything placed on the map (`worldentity.py`, and the data
//! of `worldresource.py`, `worldexit.py` and `worldunit.py`).
//!
//! An [`Entity`] is plain data owned by the [`crate::World`] arena and
//! addressed by [`EntityId`]. It is cheap to clone: memory of the fog of war
//! and events hold clones, like `copy.copy(o)` in the original.
//!
//! The Python classes read each other's attributes freely, with class
//! defaults standing in for missing ones. The entity keeps that shape: the
//! attributes every class shares live in one struct with per-class defaults
//! (see [`EntityClass::default_stats`]), and the data only one class has
//! lives in [`ExitData`], [`SiteData`] and the corpse's unit.

use std::cmp::Ordering;
use std::fmt;
use std::sync::Arc;

use crate::nofloat::PRECISION;
use crate::orders::Order;
use crate::rules::{AirGround, EntityClass, Stats, TypeId};

/// The identifier of an entity, also used for squares and players in the
/// text protocol: all of them share one counter, as in the original.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntityId(pub u32);

impl EntityId {
    /// Compares as the original does where it sorts by `id`: ids are
    /// strings there, so "10" comes before "9".
    pub fn cmp_as_text(self, other: EntityId) -> Ordering {
        self.0.to_string().cmp(&other.0.to_string())
    }
}

impl fmt::Display for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// An index into [`crate::World::squares`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SquareId(pub u32);

/// An index into the players of a [`crate::World`]. A player keeps its id
/// after quitting the game.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlayerId(pub u32);

/// Where an entity is: on a square, or inside a transport or a building.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Place {
    Square(SquareId),
    /// Inside the entity with this id.
    Inside(EntityId),
}

impl Place {
    pub fn square(self) -> Option<SquareId> {
        match self {
            Place::Square(s) => Some(s),
            Place::Inside(_) => None,
        }
    }
}

/// A point of a square given by the player (`zoom-<square>-<x>-<y>`), or one
/// of the nine subsquares of a square.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZoomTarget {
    pub place: SquareId,
    pub x: i64,
    pub y: i64,
    /// The text id it was given with, if any.
    pub id: Option<String>,
}

/// A remembered entity: a copy of the entity made when it went out of
/// sight, owned by one player's memory.
#[derive(Clone, Debug)]
pub struct MemoryRef {
    pub owner: PlayerId,
    pub snapshot: Arc<Entity>,
}

/// What an order or an action aims at.
#[derive(Clone, Debug)]
pub enum Target {
    Square(SquareId),
    Zoom(ZoomTarget),
    Entity(EntityId),
    /// A remembered entity, as the owner of the order remembers it.
    Memory(MemoryRef),
    /// A point.
    Xy(i64, i64),
}

impl Target {
    /// Identity, as `is` in the original: the same square, entity or
    /// remembrance. Two subsquare targets are always equal, because the
    /// original's `ZoomTarget.__eq__` returns a non-empty tuple.
    pub fn same(&self, other: &Target) -> bool {
        match (self, other) {
            (Target::Square(a), Target::Square(b)) => a == b,
            (Target::Entity(a), Target::Entity(b)) => a == b,
            (Target::Memory(a), Target::Memory(b)) => Arc::ptr_eq(&a.snapshot, &b.snapshot),
            (Target::Xy(a, b), Target::Xy(c, d)) => a == c && b == d,
            (Target::Zoom(_), Target::Zoom(_)) => true,
            _ => false,
        }
    }

    /// `getattr(target, "id", None)` as text.
    pub fn id_text(&self, square_ids: impl Fn(SquareId) -> u32) -> Option<String> {
        match self {
            Target::Square(s) => Some(square_ids(*s).to_string()),
            Target::Zoom(z) => z.id.clone(),
            Target::Entity(e) => Some(e.0.to_string()),
            Target::Memory(m) => Some(m.snapshot.id.0.to_string()),
            Target::Xy(..) => None,
        }
    }

    pub fn entity_id(&self) -> Option<EntityId> {
        match self {
            Target::Entity(e) => Some(*e),
            Target::Memory(m) => Some(m.snapshot.id),
            _ => None,
        }
    }
}

/// What a unit is doing right now (`worldaction.py`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionKind {
    /// No movement: the action of a unit told to stop.
    Plain,
    /// Moving to an exit, a square or an entity.
    Move,
    /// Moving to a point.
    MoveXy,
    /// Attacking.
    Attack,
}

#[derive(Clone, Debug)]
pub struct Action {
    pub kind: ActionKind,
    pub target: Option<Target>,
}

/// `"offensive"` or `"defensive"`: whether a unit flees.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AiMode {
    Offensive,
    Defensive,
}

/// An exit: one side of a path, bridge or portal between two squares.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExitData {
    pub other_side: Option<EntityId>,
    pub is_a_portal: bool,
    /// Units and buildings blocking this side.
    pub blockers: Vec<EntityId>,
    /// A dense forest closes the path.
    pub is_blocked_by_forests: bool,
}

/// A building under construction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SiteData {
    /// The building it will become.
    pub building_type: TypeId,
    pub starting_hp: i64,
    /// Updates left until completion.
    pub timer: i64,
    pub damage_during_construction: i64,
}

/// Anything placed on the map: deposit, meadow, corpse, exit, unit,
/// building or building site.
#[derive(Clone, Debug)]
pub struct Entity {
    pub id: EntityId,
    pub class: EntityClass,
    /// The rules class, for units, buildings and deposits.
    pub type_id: Option<TypeId>,
    pub type_name: Arc<str>,
    pub place: Option<Place>,
    pub x: i64,
    pub y: i64,
    /// Orientation in degrees.
    pub o: i64,
    pub player: Option<PlayerId>,
    /// The numeric attributes, copied from the type when created, then
    /// changed by upgrades.
    pub stats: Stats,
    pub airground_type: AirGround,
    pub blocked_exit: Option<EntityId>,
    /// The meadow under a building or a deposit, kept to put it back.
    pub building_land: Option<EntityId>,
    /// When the entity disappears, in world time.
    pub time_limit: Option<i64>,
    pub is_cloaked: bool,
    pub is_moving: bool,
    /// `_previous_square`: the place before the last change of place.
    pub previous_place: Option<Place>,

    pub hp: i64,
    pub mana: i64,
    pub orders: Vec<Order>,
    pub action: Option<Action>,
    /// Dead ends recently met: place, x, y, weight.
    pub walked: Vec<(Option<Place>, i64, i64, i64)>,
    pub(crate) rotations: Vec<(i64, i64)>,
    pub(crate) smooth_rotations: Vec<i64>,
    /// The number of the unit among the units of the same type of its
    /// player.
    pub number: Option<i64>,
    /// The units inside, for transports and shelters.
    pub inside: Option<Vec<EntityId>>,
    pub position_to_hold: Option<Target>,
    pub minimal_damage: i64,
    pub next_attack_time: i64,
    /// How many times damage upgrades applied.
    pub damage_level: i64,
    /// `float("inf")` is [`crate::INF`].
    pub distance_to_goal: i64,
    /// The id given to the `rallying_point` order.
    pub rallying_point: Option<String>,
    /// The name of the group the unit joined.
    pub group: Option<String>,
    pub ai_mode: Option<AiMode>,
    /// Workers only.
    pub auto_gather: Option<bool>,
    /// Workers only.
    pub auto_repair: Option<bool>,
    /// The resource a worker carries: type and quantity.
    pub cargo: Option<(i64, i64)>,
    pub(crate) destination: Option<SquareId>,
    /// The speed on the current terrain.
    pub actual_speed: i64,

    /// Deposits.
    pub qty_max: i64,
    pub resource_regen: i64,
    pub exit: Option<ExitData>,
    /// Corpses: the unit as it was when it died.
    pub corpse_unit: Option<Box<Entity>>,
    pub site: Option<SiteData>,
}

/// The radius of every entity with collision, in millimeters.
pub const COLLISION_RADIUS: i64 = 175;

impl Entity {
    pub(crate) fn new(class: EntityClass, type_name: &str) -> Entity {
        let stats = class.default_stats();
        let ai_mode = match class {
            EntityClass::Worker => Some(AiMode::Defensive),
            EntityClass::Soldier | EntityClass::Building | EntityClass::BuildingSite => {
                Some(AiMode::Offensive)
            }
            _ => None,
        };
        Entity {
            id: EntityId(0),
            class,
            type_id: None,
            type_name: Arc::from(type_name),
            place: None,
            x: 0,
            y: 0,
            o: 0,
            player: None,
            actual_speed: stats.speed,
            stats,
            airground_type: AirGround::Ground,
            blocked_exit: None,
            building_land: None,
            time_limit: None,
            is_cloaked: false,
            is_moving: false,
            previous_place: None,
            hp: 0,
            mana: 0,
            orders: Vec::new(),
            action: None,
            walked: Vec::new(),
            rotations: Vec::new(),
            smooth_rotations: Vec::new(),
            number: None,
            inside: None,
            position_to_hold: None,
            minimal_damage: 0,
            next_attack_time: 0,
            damage_level: 0,
            distance_to_goal: crate::INF,
            rallying_point: None,
            group: None,
            ai_mode,
            auto_gather: (class == EntityClass::Worker).then_some(true),
            auto_repair: (class == EntityClass::Worker).then_some(true),
            cargo: None,
            destination: None,
            qty_max: 0,
            resource_regen: 0,
            exit: None,
            corpse_unit: None,
            site: None,
        }
    }

    pub fn is_creature(&self) -> bool {
        self.class.is_creature()
    }

    pub fn is_unit(&self) -> bool {
        self.class.is_unit()
    }

    pub fn is_exit(&self) -> bool {
        self.class == EntityClass::Exit
    }

    pub fn is_deposit(&self) -> bool {
        self.class == EntityClass::Deposit
    }

    pub fn is_meadow(&self) -> bool {
        self.class == EntityClass::Meadow
    }

    pub fn is_corpse(&self) -> bool {
        self.class == EntityClass::Corpse
    }

    pub fn is_building_site(&self) -> bool {
        self.class == EntityClass::BuildingSite
    }

    /// `is_a_building_land`: meadows are, exits and buildings are not.
    pub fn is_a_building_land(&self) -> bool {
        self.stats.is_a_building_land != 0
    }

    pub fn is_inside(&self) -> bool {
        matches!(self.place, Some(Place::Inside(_)))
    }

    pub fn square(&self) -> Option<SquareId> {
        self.place.and_then(Place::square)
    }

    pub fn radius(&self) -> i64 {
        if self.stats.collision != 0 {
            COLLISION_RADIUS
        } else {
            0
        }
    }

    pub fn is_invisible(&self) -> bool {
        self.stats.is_invisible != 0
    }

    pub fn is_vulnerable(&self) -> bool {
        self.stats.is_vulnerable != 0
    }

    /// `menace`: the danger a unit represents.
    pub fn menace(&self) -> i64 {
        if !self.is_creature() {
            return 0;
        }
        if self.stats.damage != 0 {
            return self.stats.damage;
        }
        if self.stats.transport_capacity != 0 {
            return self.stats.transport_capacity * PRECISION * 2;
        }
        self.stats.damage
    }

    /// A melee unit has a range shorter than 2.
    pub fn is_melee(&self) -> bool {
        self.stats.range < 2 * PRECISION
    }

    /// `"unit"`, `"building"`, or nothing (effects, building sites).
    pub fn stat_type(&self) -> Option<StatType> {
        match self.class {
            EntityClass::Worker | EntityClass::Soldier => Some(StatType::Unit),
            EntityClass::Building => Some(StatType::Building),
            _ => None,
        }
    }

    pub fn can_switch_ai_mode(&self) -> bool {
        matches!(self.class, EntityClass::Worker | EntityClass::Soldier)
    }

    /// The orders a unit can always receive, by keyword.
    pub fn basic_abilities(&self) -> &'static [&'static str] {
        match self.class {
            EntityClass::Worker | EntityClass::Soldier | EntityClass::Effect => {
                if self
                    .orders
                    .iter()
                    .any(|o| o.keyword == crate::orders::Keyword::UpgradeTo)
                {
                    return &[];
                }
                match self.class {
                    EntityClass::Worker => {
                        &["go", "attack", "gather", "repair", "block", "join_group"]
                    }
                    EntityClass::Soldier => &["go", "attack", "patrol", "block", "join_group"],
                    _ => &[],
                }
            }
            EntityClass::BuildingSite => &["cancel_building"],
            _ => &[],
        }
    }

    /// `is_idle`: no action target.
    pub fn is_idle(&self) -> bool {
        self.action_target().is_none()
    }

    pub fn action_target(&self) -> Option<&Target> {
        self.action.as_ref().and_then(|a| a.target.as_ref())
    }

    pub fn is_attacking(&self) -> bool {
        matches!(&self.action, Some(a) if a.kind == ActionKind::Attack)
    }

    pub fn has_imperative_orders(&self) -> bool {
        self.orders.first().is_some_and(|o| o.is_imperative)
    }
}

/// What the statistics count a creature as.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StatType {
    Unit,
    Building,
}
