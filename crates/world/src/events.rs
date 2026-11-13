//! What the simulation tells a player's interface: the typed form of the
//! original's `player.push(...)` and `send_event(...)`.

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::entity::{Entity, EntityId, SquareId};
use crate::msg::Msg;

/// Why an order cannot be carried out.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Reason {
    TheQueueIsFull,
    NotEnoughFood,
    PopulationLimitReached,
    /// Not enough of the resource with this index.
    NotEnoughResource(usize),
    NotEnoughSpace,
    NotEnoughMana,
    CannotBuildHere,
    CountLimitReached,
}

impl Reason {
    /// The name the original uses, which is also the style key.
    pub fn name(&self) -> String {
        match self {
            Reason::TheQueueIsFull => "the_queue_is_full".into(),
            Reason::NotEnoughFood => "not_enough_food".into(),
            Reason::PopulationLimitReached => "population_limit_reached".into(),
            Reason::NotEnoughResource(i) => format!("not_enough_resource_{i}"),
            Reason::NotEnoughSpace => "not_enough_space".into(),
            Reason::NotEnoughMana => "not_enough_mana".into(),
            Reason::CannotBuildHere => "cannot_build_here".into(),
            Reason::CountLimitReached => "count_limit_reached".into(),
        }
    }
}

/// Something that happened to an entity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntityEvent {
    /// Hit by an attacker of this type, with its damage upgrade level.
    Wounded {
        attacker_type: String,
        attacker: EntityId,
        attacker_damage_level: i64,
    },
    Death,
    /// Killed by this attacker.
    DeathBy(EntityId),
    OrderOk,
    OrderImpossible(Option<Reason>),
    /// Starts attacking.
    Attack,
    /// Shoots or strikes.
    LaunchAttack,
    Missed,
    /// Blocked while moving.
    Collision,
    Flee,
    /// Just created by training, building or upgrading.
    Complete,
    /// Production progress, from 0 to 10.
    Completeness(i64),
    ProductionDeferred,
    ResearchComplete,
    /// Used the ability of this type.
    UseComplete(String),
    /// Stores a resource of this type.
    Store(i64),
    /// A deposit is empty.
    Exhausted,
    /// Enters a transport or a building.
    Enter,
    /// Leaves a transport or a building.
    Exit,
    /// Added by a map trigger.
    Added,
}

impl EntityEvent {
    /// The event as the original's comma separated text.
    pub fn text(&self) -> String {
        match self {
            EntityEvent::Wounded {
                attacker_type,
                attacker,
                attacker_damage_level,
            } => format!("wounded,{attacker_type},{attacker},{attacker_damage_level}"),
            EntityEvent::Death => "death".into(),
            EntityEvent::DeathBy(a) => format!("death_by,{a}"),
            EntityEvent::OrderOk => "order_ok".into(),
            EntityEvent::OrderImpossible(None) => "order_impossible".into(),
            EntityEvent::OrderImpossible(Some(r)) => format!("order_impossible,{}", r.name()),
            EntityEvent::Attack => "attack".into(),
            EntityEvent::LaunchAttack => "launch_attack".into(),
            EntityEvent::Missed => "missed".into(),
            EntityEvent::Collision => "collision".into(),
            EntityEvent::Flee => "flee".into(),
            EntityEvent::Complete => "complete".into(),
            EntityEvent::Completeness(c) => format!("completeness,{c}"),
            EntityEvent::ProductionDeferred => "production_deferred".into(),
            EntityEvent::ResearchComplete => "research_complete".into(),
            EntityEvent::UseComplete(t) => format!("use_complete,{t}"),
            EntityEvent::Store(r) => format!("store,{r}"),
            EntityEvent::Exhausted => "exhausted".into(),
            EntityEvent::Enter => "enter".into(),
            EntityEvent::Exit => "exit".into(),
            EntityEvent::Added => "added".into(),
        }
    }
}

/// An event for a player's interface. Drain them with
/// [`crate::World::drain_events`] after each update.
#[derive(Clone, Debug)]
pub enum Event {
    /// Something happened to an entity the player perceives (or to any
    /// entity, for universal notifications). The entity is a copy taken when
    /// it happened.
    Entity {
        entity: Box<Entity>,
        event: EntityEvent,
    },
    /// A message to say now.
    VoiceImportant(Msg),
    /// A cut scene: the sounds to play in order.
    Sequence(Vec<String>),
    /// The game speed changed.
    Speed(f64),
    /// The player left the game.
    Quit,
    /// The turn is over: what the player knows at its end, before the game
    /// may end.
    Voila(Box<Voila>),
}

/// What a player knows at the end of a turn: copies of what it remembers
/// and perceives, and the squares it observes.
#[derive(Clone, Debug)]
pub struct Voila {
    /// The world's time after the turn, in milliseconds.
    pub time: i64,
    /// The remembered entities, with when each one was last seen.
    pub memory: Vec<(Arc<Entity>, i64)>,
    pub perception: Vec<Arc<Entity>>,
    pub observed_squares: BTreeSet<SquareId>,
    /// Every square in cheat mode.
    pub observed_before_squares: BTreeSet<SquareId>,
}
