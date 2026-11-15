//! Entities: everything placed on the map (`worldentity.py`, and the data
//! of `worldresource.py`, `worldexit.py` and `worldunit.py`).
//!
//! An [`Entity`] is plain data owned by the [`crate::World`] arena and
//! addressed by [`EntityId`]. It is cheap to clone: memory of the fog of war
//! and events hold clones, like `copy.copy(o)` in the original.
//!
//! An entity has what every thing on the map has (its type's name and
//! attributes, its place and position) and a [`Kind`], which carries only
//! the state of its kind: a deposit its quantity, an exit what blocks it, a
//! corpse the unit it was, a creature its owner, hit points and orders. The
//! state that only some creatures have is in [`CreatureKind`].

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::nofloat::PRECISION;
use crate::orders::{Order, TargetRef};
use crate::rules::{AirGround, EntityClass, Stats, TypeId};

/// The identifier of an entity, also used for squares and players in the
/// text protocol: all of them share one counter, as in the original.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EntityId(pub u32);

impl fmt::Display for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// An index into [`crate::World::squares`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SquareId(pub u32);

/// An index into the players of a [`crate::World`]. A player keeps its id
/// after quitting the game.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PlayerId(pub u32);

/// Where an entity is: on a square, or inside a transport or a building.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ZoomTarget {
    pub place: SquareId,
    pub x: i64,
    pub y: i64,
    /// Given by the player, rather than a subsquare the simulation chose:
    /// only a given point has an identity (see [`Target::identity`]).
    pub given: bool,
}

/// What tells two targets apart when orders are compared: the same
/// square, the same entity (perceived or remembered), or the same point
/// given by the player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TargetIdentity {
    Square(SquareId),
    Entity(EntityId),
    Point(SquareId, i64, i64),
}

/// A remembered entity: a copy of the entity made when it went out of
/// sight, owned by one player's memory.
#[derive(Clone, Debug, Hash)]
pub struct MemoryRef {
    pub owner: PlayerId,
    pub snapshot: Arc<Entity>,
}

/// What an order or an action aims at.
#[derive(Clone, Debug, Hash)]
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
    /// Identity: the same square, entity, remembrance or point.
    pub fn same(&self, other: &Target) -> bool {
        match (self, other) {
            (Target::Square(a), Target::Square(b)) => a == b,
            (Target::Entity(a), Target::Entity(b)) => a == b,
            (Target::Memory(a), Target::Memory(b)) => Arc::ptr_eq(&a.snapshot, &b.snapshot),
            (Target::Xy(a, b), Target::Xy(c, d)) => a == c && b == d,
            (Target::Zoom(a), Target::Zoom(b)) => (a.x, a.y) == (b.x, b.y),
            _ => false,
        }
    }

    /// What tells the target apart when orders are compared (the
    /// original's `id`): a subsquare and a point have none.
    pub(crate) fn identity(&self) -> Option<TargetIdentity> {
        match self {
            Target::Square(s) => Some(TargetIdentity::Square(*s)),
            Target::Zoom(z) => z.given.then_some(TargetIdentity::Point(z.place, z.x, z.y)),
            Target::Entity(e) => Some(TargetIdentity::Entity(*e)),
            Target::Memory(m) => Some(TargetIdentity::Entity(m.snapshot.id)),
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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

#[derive(Clone, Debug, Hash)]
pub struct Action {
    pub kind: ActionKind,
    pub target: Option<Target>,
}

/// `"offensive"` or `"defensive"`: whether a unit flees.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum AiMode {
    Offensive,
    Defensive,
}

/// An exit: one side of a path, bridge or portal between two squares.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ExitData {
    pub other_side: Option<EntityId>,
    pub is_a_portal: bool,
    /// Units and buildings blocking this side.
    pub blockers: Vec<EntityId>,
    /// A dense forest closes the path.
    pub is_blocked_by_forests: bool,
}

/// A building under construction.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SiteData {
    /// The building it will become.
    pub building_type: TypeId,
    pub starting_hp: i64,
    /// Updates left until completion.
    pub timer: i64,
    pub damage_during_construction: i64,
}

/// Anything placed on the map: deposit, meadow, corpse, exit, unit,
/// building or building site. `K` is its kind: [`Kind`] for any entity,
/// [`Creature`] for the unit a corpse was.
#[derive(Clone, Debug)]
pub struct Entity<K = Kind> {
    pub id: EntityId,
    pub type_name: Arc<str>,
    pub place: Option<Place>,
    pub x: i64,
    pub y: i64,
    /// Orientation in degrees.
    pub o: i64,
    /// The numeric attributes: its class's defaults, or its type's, then
    /// changed by upgrades.
    pub stats: Stats,
    pub airground_type: AirGround,
    /// Moved since its last update: anything whose type has a speed.
    pub is_moving: bool,
    /// `_previous_square`: the place before the last change of place.
    pub previous_place: Option<Place>,
    pub kind: K,
}

/// What an entity is, with the state only that kind has.
#[derive(Clone, Debug)]
#[allow(
    clippy::large_enum_variant,
    reason = "a box would add an allocation to each copy of a creature that a memory takes"
)]
pub enum Kind {
    /// Land a building can be built on. It has no state of its own.
    Meadow,
    Deposit(Deposit),
    Exit(ExitData),
    Corpse(Corpse),
    /// A unit, a building or a building site: what a player can own, give
    /// orders to and damage.
    Creature(Creature),
}

/// A deposit of a resource.
#[derive(Clone, Debug, Hash)]
pub struct Deposit {
    pub type_id: TypeId,
    /// The quantity it starts with: regeneration stops there. The quantity
    /// it holds is `stats.qty`.
    pub qty_max: i64,
    pub resource_regen: i64,
    /// The meadow it covers, put back when it is exhausted.
    pub building_land: Option<EntityId>,
}

/// What is left where a unit died.
#[derive(Clone, Debug)]
pub struct Corpse {
    /// The unit as it was when it died.
    pub unit: Box<Entity<Creature>>,
    /// When it disappears, in world time.
    pub time_limit: i64,
}

/// What every unit, building and building site has.
#[derive(Clone, Debug)]
pub struct Creature {
    pub player: Option<PlayerId>,
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
    /// The units inside, for transports and shelters: those whose type had
    /// a transport capacity when they were created.
    pub inside: Option<Vec<EntityId>>,
    pub position_to_hold: Option<Target>,
    pub minimal_damage: i64,
    pub next_attack_time: i64,
    /// How many times damage upgrades applied.
    pub damage_level: i64,
    /// `float("inf")` is [`crate::INF`].
    pub distance_to_goal: i64,
    /// Where the units it trains go, as the `rallying_point` order gave it.
    pub rallying_point: Option<TargetRef>,
    /// The name of the group the unit joined.
    pub group: Option<String>,
    pub(crate) destination: Option<SquareId>,
    /// The speed on the current terrain.
    pub actual_speed: i64,
    /// When it disappears, in world time.
    pub time_limit: Option<i64>,
    pub is_cloaked: bool,
    pub blocked_exit: Option<EntityId>,
    /// The meadow it covers, put back when it is destroyed.
    pub building_land: Option<EntityId>,
    pub kind: CreatureKind,
}

/// What a creature is, with the state only that kind has. Units and
/// buildings have a rules type; a building site has the type of the
/// building it will become.
#[derive(Clone, Debug)]
pub enum CreatureKind {
    /// A unit that gathers, builds and repairs.
    Worker {
        type_id: TypeId,
        stance: AiMode,
        work: Work,
    },
    /// A unit that fights.
    Soldier {
        type_id: TypeId,
        stance: AiMode,
    },
    /// A unit that is a spell's effect: it never flees.
    Effect {
        type_id: TypeId,
    },
    /// A building, finished: it never flees.
    Building {
        type_id: TypeId,
    },
    BuildingSite(SiteData),
}

/// What only a worker has.
#[derive(Clone, Debug, Hash)]
pub struct Work {
    pub auto_gather: bool,
    pub auto_repair: bool,
    /// The resource it carries: type and quantity.
    pub cargo: Option<(i64, i64)>,
}

/// The radius of every entity with collision, in millimeters.
pub const COLLISION_RADIUS: i64 = 175;

impl CreatureKind {
    /// The kind of a new creature of a rules type of this class, or `None`
    /// for a class that is not a creature's.
    pub(crate) fn of_type(type_id: TypeId, class: EntityClass) -> Option<CreatureKind> {
        Some(match class {
            EntityClass::Worker => CreatureKind::Worker {
                type_id,
                stance: AiMode::Defensive,
                work: Work {
                    auto_gather: true,
                    auto_repair: true,
                    cargo: None,
                },
            },
            EntityClass::Soldier => CreatureKind::Soldier {
                type_id,
                stance: AiMode::Offensive,
            },
            EntityClass::Effect => CreatureKind::Effect { type_id },
            EntityClass::Building => CreatureKind::Building { type_id },
            EntityClass::Deposit
            | EntityClass::Meadow
            | EntityClass::Corpse
            | EntityClass::Exit
            | EntityClass::BuildingSite => return None,
        })
    }

    pub fn class(&self) -> EntityClass {
        match self {
            CreatureKind::Worker { .. } => EntityClass::Worker,
            CreatureKind::Soldier { .. } => EntityClass::Soldier,
            CreatureKind::Effect { .. } => EntityClass::Effect,
            CreatureKind::Building { .. } => EntityClass::Building,
            CreatureKind::BuildingSite(_) => EntityClass::BuildingSite,
        }
    }
}

impl Creature {
    /// A creature not yet in the world: no owner, no hit points.
    pub(crate) fn new(kind: CreatureKind) -> Creature {
        Creature {
            player: None,
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
            destination: None,
            actual_speed: 0,
            time_limit: None,
            is_cloaked: false,
            blocked_exit: None,
            building_land: None,
            kind,
        }
    }

    /// Its rules type: none for a building site.
    pub fn type_id(&self) -> Option<TypeId> {
        match &self.kind {
            CreatureKind::Worker { type_id, .. }
            | CreatureKind::Soldier { type_id, .. }
            | CreatureKind::Effect { type_id }
            | CreatureKind::Building { type_id } => Some(*type_id),
            CreatureKind::BuildingSite(_) => None,
        }
    }

    /// Whether it fights or flees when attacked: workers and soldiers
    /// switch, buildings and sites always fight, effects have no stance.
    pub fn stance(&self) -> Option<AiMode> {
        match &self.kind {
            CreatureKind::Worker { stance, .. } | CreatureKind::Soldier { stance, .. } => {
                Some(*stance)
            }
            CreatureKind::Building { .. } | CreatureKind::BuildingSite(_) => {
                Some(AiMode::Offensive)
            }
            CreatureKind::Effect { .. } => None,
        }
    }

    /// The work of a worker.
    pub fn work(&self) -> Option<&Work> {
        match &self.kind {
            CreatureKind::Worker { work, .. } => Some(work),
            CreatureKind::Soldier { .. }
            | CreatureKind::Effect { .. }
            | CreatureKind::Building { .. }
            | CreatureKind::BuildingSite(_) => None,
        }
    }

    pub fn work_mut(&mut self) -> Option<&mut Work> {
        match &mut self.kind {
            CreatureKind::Worker { work, .. } => Some(work),
            CreatureKind::Soldier { .. }
            | CreatureKind::Effect { .. }
            | CreatureKind::Building { .. }
            | CreatureKind::BuildingSite(_) => None,
        }
    }

    pub fn site(&self) -> Option<&SiteData> {
        match &self.kind {
            CreatureKind::BuildingSite(site) => Some(site),
            CreatureKind::Worker { .. }
            | CreatureKind::Soldier { .. }
            | CreatureKind::Effect { .. }
            | CreatureKind::Building { .. } => None,
        }
    }

    /// Sets the stance of a worker or a soldier: the other kinds cannot
    /// switch.
    pub(crate) fn set_stance(&mut self, mode: AiMode) {
        match &mut self.kind {
            CreatureKind::Worker { stance, .. } | CreatureKind::Soldier { stance, .. } => {
                *stance = mode;
            }
            CreatureKind::Effect { .. }
            | CreatureKind::Building { .. }
            | CreatureKind::BuildingSite(_) => {}
        }
    }

    /// `is_idle`: no action target.
    pub fn is_idle(&self) -> bool {
        self.action_target().is_none()
    }

    pub fn action_target(&self) -> Option<&Target> {
        self.action.as_ref().and_then(|a| a.target.as_ref())
    }
}

impl Kind {
    pub fn class(&self) -> EntityClass {
        match self {
            Kind::Meadow => EntityClass::Meadow,
            Kind::Deposit(_) => EntityClass::Deposit,
            Kind::Exit(_) => EntityClass::Exit,
            Kind::Corpse(_) => EntityClass::Corpse,
            Kind::Creature(c) => c.kind.class(),
        }
    }
}

impl Entity {
    /// A new entity of a kind, with its class's default attributes, not yet
    /// placed.
    pub(crate) fn new(type_name: &str, kind: Kind) -> Entity {
        Entity {
            id: EntityId(0),
            type_name: Arc::from(type_name),
            place: None,
            x: 0,
            y: 0,
            o: 0,
            stats: kind.class().default_stats(),
            airground_type: AirGround::Ground,
            is_moving: false,
            previous_place: None,
            kind,
        }
    }

    pub fn class(&self) -> EntityClass {
        self.kind.class()
    }

    /// The rules type of a unit, a building or a deposit.
    pub fn type_id(&self) -> Option<TypeId> {
        match &self.kind {
            Kind::Deposit(d) => Some(d.type_id),
            Kind::Creature(c) => c.type_id(),
            Kind::Meadow | Kind::Exit(_) | Kind::Corpse(_) => None,
        }
    }

    pub fn creature(&self) -> Option<&Creature> {
        match &self.kind {
            Kind::Creature(c) => Some(c),
            Kind::Meadow | Kind::Deposit(_) | Kind::Exit(_) | Kind::Corpse(_) => None,
        }
    }

    pub fn creature_mut(&mut self) -> Option<&mut Creature> {
        match &mut self.kind {
            Kind::Creature(c) => Some(c),
            Kind::Meadow | Kind::Deposit(_) | Kind::Exit(_) | Kind::Corpse(_) => None,
        }
    }

    pub fn exit(&self) -> Option<&ExitData> {
        match &self.kind {
            Kind::Exit(x) => Some(x),
            Kind::Meadow | Kind::Deposit(_) | Kind::Corpse(_) | Kind::Creature(_) => None,
        }
    }

    pub fn exit_mut(&mut self) -> Option<&mut ExitData> {
        match &mut self.kind {
            Kind::Exit(x) => Some(x),
            Kind::Meadow | Kind::Deposit(_) | Kind::Corpse(_) | Kind::Creature(_) => None,
        }
    }

    pub fn deposit(&self) -> Option<&Deposit> {
        match &self.kind {
            Kind::Deposit(d) => Some(d),
            Kind::Meadow | Kind::Exit(_) | Kind::Corpse(_) | Kind::Creature(_) => None,
        }
    }

    pub fn deposit_mut(&mut self) -> Option<&mut Deposit> {
        match &mut self.kind {
            Kind::Deposit(d) => Some(d),
            Kind::Meadow | Kind::Exit(_) | Kind::Corpse(_) | Kind::Creature(_) => None,
        }
    }

    pub fn corpse(&self) -> Option<&Corpse> {
        match &self.kind {
            Kind::Corpse(c) => Some(c),
            Kind::Meadow | Kind::Deposit(_) | Kind::Exit(_) | Kind::Creature(_) => None,
        }
    }

    /// Its owner: only a creature has one.
    pub fn player(&self) -> Option<PlayerId> {
        self.creature().and_then(|c| c.player)
    }

    /// Its orders: only a creature has any.
    pub fn orders(&self) -> &[Order] {
        self.creature().map_or(&[], |c| &c.orders)
    }

    pub fn site(&self) -> Option<&SiteData> {
        self.creature().and_then(Creature::site)
    }

    pub fn work(&self) -> Option<&Work> {
        self.creature().and_then(Creature::work)
    }

    /// The units inside a transport or a shelter.
    pub fn inside(&self) -> Option<&[EntityId]> {
        self.creature().and_then(|c| c.inside.as_deref())
    }

    /// When it disappears: a corpse always does, a creature may.
    pub fn time_limit(&self) -> Option<i64> {
        match &self.kind {
            Kind::Corpse(c) => Some(c.time_limit),
            Kind::Creature(c) => c.time_limit,
            Kind::Meadow | Kind::Deposit(_) | Kind::Exit(_) => None,
        }
    }

    /// The meadow it covers: a deposit's or a creature's.
    pub fn building_land(&self) -> Option<EntityId> {
        match &self.kind {
            Kind::Deposit(d) => d.building_land,
            Kind::Creature(c) => c.building_land,
            Kind::Meadow | Kind::Exit(_) | Kind::Corpse(_) => None,
        }
    }

    /// Whether it fights or flees: see [`Creature::stance`].
    pub fn stance(&self) -> Option<AiMode> {
        self.creature().and_then(Creature::stance)
    }

    /// The exit it blocks: only a creature blocks one.
    pub fn blocked_exit(&self) -> Option<EntityId> {
        self.creature().and_then(|c| c.blocked_exit)
    }

    /// Whether a cloaker hides it: only a creature can be hidden.
    pub fn is_cloaked(&self) -> bool {
        self.creature().is_some_and(|c| c.is_cloaked)
    }

    pub fn is_creature(&self) -> bool {
        self.class().is_creature()
    }

    pub fn is_unit(&self) -> bool {
        self.class().is_unit()
    }

    pub fn is_exit(&self) -> bool {
        matches!(self.kind, Kind::Exit(_))
    }

    pub fn is_deposit(&self) -> bool {
        matches!(self.kind, Kind::Deposit(_))
    }

    pub fn is_meadow(&self) -> bool {
        matches!(self.kind, Kind::Meadow)
    }

    pub fn is_corpse(&self) -> bool {
        matches!(self.kind, Kind::Corpse(_))
    }

    pub fn is_building_site(&self) -> bool {
        self.site().is_some()
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
        match self.class() {
            EntityClass::Worker | EntityClass::Soldier => Some(StatType::Unit),
            EntityClass::Building => Some(StatType::Building),
            EntityClass::Deposit
            | EntityClass::Meadow
            | EntityClass::Corpse
            | EntityClass::Exit
            | EntityClass::Effect
            | EntityClass::BuildingSite => None,
        }
    }

    /// `is_idle`: no action target. Only a creature has an action.
    pub fn is_idle(&self) -> bool {
        self.action_target().is_none()
    }

    pub fn action_target(&self) -> Option<&Target> {
        self.creature().and_then(Creature::action_target)
    }

    pub fn is_attacking(&self) -> bool {
        matches!(
            self.creature().and_then(|c| c.action.as_ref()),
            Some(a) if a.kind == ActionKind::Attack
        )
    }

    pub fn has_imperative_orders(&self) -> bool {
        self.orders().first().is_some_and(|o| o.is_imperative)
    }
}

impl<K> Entity<K> {
    /// A copy of what every entity has, with another kind.
    pub(crate) fn with_kind<L>(&self, kind: L) -> Entity<L> {
        Entity {
            id: self.id,
            type_name: self.type_name.clone(),
            place: self.place,
            x: self.x,
            y: self.y,
            o: self.o,
            stats: self.stats.clone(),
            airground_type: self.airground_type,
            is_moving: self.is_moving,
            previous_place: self.previous_place,
            kind,
        }
    }
}

impl Entity<Creature> {
    /// The creature as an entity of the world.
    pub(crate) fn into_entity(self) -> Entity {
        let Entity {
            id,
            type_name,
            place,
            x,
            y,
            o,
            stats,
            airground_type,
            is_moving,
            previous_place,
            kind,
        } = self;
        Entity {
            id,
            type_name,
            place,
            x,
            y,
            o,
            stats,
            airground_type,
            is_moving,
            previous_place,
            kind: Kind::Creature(kind),
        }
    }
}

/// An entity's kind seen through a reference, for the hash of an entity of
/// any kind and of the unit a corpse was.
#[derive(Clone, Copy)]
enum KindRef<'a> {
    Meadow,
    Deposit(&'a Deposit),
    Exit(&'a ExitData),
    Corpse(&'a Corpse),
    Creature(&'a Creature),
}

impl Hash for Entity {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let kind = match &self.kind {
            Kind::Meadow => KindRef::Meadow,
            Kind::Deposit(d) => KindRef::Deposit(d),
            Kind::Exit(x) => KindRef::Exit(x),
            Kind::Corpse(c) => KindRef::Corpse(c),
            Kind::Creature(c) => KindRef::Creature(c),
        };
        hash_entity(self, kind, state);
    }
}

impl Hash for Entity<Creature> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        hash_entity(self, KindRef::Creature(&self.kind), state);
    }
}

/// Feeds the hasher what the single structure of every kind fed it before
/// the kinds were told apart: each of its fields in their order, a field a
/// kind lacks as the value that structure held for it.
fn hash_entity<K, H: Hasher>(e: &Entity<K>, kind: KindRef<'_>, state: &mut H) {
    let creature = match kind {
        KindRef::Creature(c) => Some(c),
        KindRef::Meadow | KindRef::Deposit(_) | KindRef::Exit(_) | KindRef::Corpse(_) => None,
    };
    let deposit = match kind {
        KindRef::Deposit(d) => Some(d),
        KindRef::Meadow | KindRef::Exit(_) | KindRef::Corpse(_) | KindRef::Creature(_) => None,
    };
    let class = match kind {
        KindRef::Meadow => EntityClass::Meadow,
        KindRef::Deposit(_) => EntityClass::Deposit,
        KindRef::Exit(_) => EntityClass::Exit,
        KindRef::Corpse(_) => EntityClass::Corpse,
        KindRef::Creature(c) => c.kind.class(),
    };
    let type_id = match kind {
        KindRef::Deposit(d) => Some(d.type_id),
        KindRef::Creature(c) => c.type_id(),
        KindRef::Meadow | KindRef::Exit(_) | KindRef::Corpse(_) => None,
    };
    let building_land = match kind {
        KindRef::Deposit(d) => d.building_land,
        KindRef::Creature(c) => c.building_land,
        KindRef::Meadow | KindRef::Exit(_) | KindRef::Corpse(_) => None,
    };
    let time_limit = match kind {
        KindRef::Corpse(c) => Some(c.time_limit),
        KindRef::Creature(c) => c.time_limit,
        KindRef::Meadow | KindRef::Deposit(_) | KindRef::Exit(_) => None,
    };
    let exit = match kind {
        KindRef::Exit(x) => Some(x),
        KindRef::Meadow | KindRef::Deposit(_) | KindRef::Corpse(_) | KindRef::Creature(_) => None,
    };
    let corpse_unit = match kind {
        KindRef::Corpse(c) => Some(&*c.unit),
        KindRef::Meadow | KindRef::Deposit(_) | KindRef::Exit(_) | KindRef::Creature(_) => None,
    };
    let no_walk: &[(Option<Place>, i64, i64, i64)] = &[];
    let no_rotation: &[(i64, i64)] = &[];
    let no_smooth_rotation: &[i64] = &[];
    let no_orders: &[Order] = &[];
    let work = creature.and_then(Creature::work);

    e.id.hash(state);
    class.hash(state);
    type_id.hash(state);
    e.type_name.hash(state);
    e.place.hash(state);
    e.x.hash(state);
    e.y.hash(state);
    e.o.hash(state);
    creature.and_then(|c| c.player).hash(state);
    e.stats.hash(state);
    e.airground_type.hash(state);
    creature.and_then(|c| c.blocked_exit).hash(state);
    building_land.hash(state);
    time_limit.hash(state);
    creature.is_some_and(|c| c.is_cloaked).hash(state);
    e.is_moving.hash(state);
    e.previous_place.hash(state);
    creature.map_or(0, |c| c.hp).hash(state);
    creature.map_or(0, |c| c.mana).hash(state);
    creature
        .map_or(no_orders, |c| c.orders.as_slice())
        .hash(state);
    creature.and_then(|c| c.action.as_ref()).hash(state);
    creature
        .map_or(no_walk, |c| c.walked.as_slice())
        .hash(state);
    creature
        .map_or(no_rotation, |c| c.rotations.as_slice())
        .hash(state);
    creature
        .map_or(no_smooth_rotation, |c| c.smooth_rotations.as_slice())
        .hash(state);
    creature.and_then(|c| c.number).hash(state);
    creature.and_then(|c| c.inside.as_deref()).hash(state);
    creature
        .and_then(|c| c.position_to_hold.as_ref())
        .hash(state);
    creature.map_or(0, |c| c.minimal_damage).hash(state);
    creature.map_or(0, |c| c.next_attack_time).hash(state);
    creature.map_or(0, |c| c.damage_level).hash(state);
    creature
        .map_or(crate::INF, |c| c.distance_to_goal)
        .hash(state);
    creature.and_then(|c| c.rallying_point.as_ref()).hash(state);
    creature.and_then(|c| c.group.as_ref()).hash(state);
    creature.and_then(Creature::stance).hash(state);
    work.map(|w| w.auto_gather).hash(state);
    work.map(|w| w.auto_repair).hash(state);
    work.and_then(|w| w.cargo).hash(state);
    creature.and_then(|c| c.destination).hash(state);
    creature.map_or(0, |c| c.actual_speed).hash(state);
    deposit.map_or(0, |d| d.qty_max).hash(state);
    deposit.map_or(0, |d| d.resource_regen).hash(state);
    exit.hash(state);
    corpse_unit.hash(state);
    creature.and_then(Creature::site).hash(state);
}

/// What the statistics count a creature as.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StatType {
    Unit,
    Building,
}
