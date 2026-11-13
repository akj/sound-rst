//! The seam where the computer player AI plugs in.
//!
//! In the original, `Computer` and `Computer2` subclass `Player` and
//! override `play` (called during the player's update), `on_unit_attacked`
//! and `time_has_come`. Here a computer player is a [`crate::Player`] of
//! kind `Computer` or `Computer2` that holds a [`ComputerAi`]. The world
//! calls the AI at those points. Until an AI is installed with
//! [`crate::World::set_ai_factory`], every computer player gets
//! [`IdleAi`], which does what the original's `timers` AI does: nothing.

use crate::entity::{EntityId, PlayerId, SquareId};
use crate::player::PlayerKind;
use crate::world::World;

/// The behavior of a computer player.
///
/// The world takes the AI out of its player while calling it, so the AI may
/// freely read and change the world, including its own player.
pub trait ComputerAi: Send {
    /// Called once per world update, during the player's update, after
    /// perception, memory and menace are updated (`Player.play`).
    fn play(&mut self, _world: &mut World, _me: PlayerId) {}

    /// Called when one of the player's units is hit or killed. For
    /// `Computer` players this replaces the base reaction (counterattacks
    /// of nearby units), as the original's override does.
    fn on_unit_attacked(
        &mut self,
        _world: &mut World,
        _me: PlayerId,
        _unit: EntityId,
        _attacker: EntityId,
    ) {
    }

    /// Whether units waiting with a `wait` order on `place` may go.
    fn time_has_come(&self, _world: &World, _me: PlayerId, _place: SquareId) -> bool {
        false
    }

    /// Called when the map changes the AI type with the `ai` trigger,
    /// after the player's AI type is updated.
    fn set_ai(&mut self, _world: &mut World, _me: PlayerId, _ai_type: &str) {}
}

/// An AI that does nothing.
#[derive(Clone, Copy, Debug, Default)]
pub struct IdleAi;

impl ComputerAi for IdleAi {}

/// Creates the AI of a new computer player, given its kind (with its AI
/// type) and the lines of its `ai.txt` script, if the type has one.
pub type AiFactory = dyn Fn(&PlayerKind, Option<&[String]>) -> Box<dyn ComputerAi> + Send;
