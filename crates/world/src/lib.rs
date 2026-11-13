//! The SoundRTS game simulation.
//!
//! This crate is deterministic and headless. It takes the rules, the computer
//! player scripts and a map as text, and it never touches audio, speech, a
//! window or the file system.
//!
//! # Driving a game
//!
//! ```no_run
//! use soundrts_world::{AiScripts, Controller, Rules, World, multiplayer_default_triggers};
//! # let (rules_txt, ai_txt, map_txt) = ("", "", "");
//! let rules = Rules::load(&[rules_txt]);
//! let ai = AiScripts::load(&[ai_txt]);
//! let mut world = World::new(rules, ai, multiplayer_default_triggers(), 42);
//! world.load_and_build_map(map_txt).expect("a valid map");
//! let me = world
//!     .populate_map(&[Controller::human("me", true), Controller::computer("easy")], true, true)
//!     .expect("enough starting places")[0];
//! loop {
//!     world.queue_command(me, "order 0 0 go 12"); // from the interface
//!     world.update(); // one turn: 300 ms of game time
//!     for _event in world.drain_events(me) { /* speak, play sounds */ }
//!     # break;
//! }
//! ```
//!
//! The rules and scripts of a mod, a campaign or a map are more layers of
//! the same files: pass them all, in order, to [`Rules::load`] and
//! [`AiScripts::load`]. Commands are the original's text lines (`control 4
//! 7-9`, `order 0 1 go 12`, `cmd add_units a1 footman`, `quit`...); they run
//! at the start of the next [`World::update`]. Only a local human
//! ([`Controller::human`] with `local`) receives events.
//!
//! # Reading the state
//!
//! - The map: [`World::squares`] (grid, names, neighbors, exits, terrain),
//!   [`World::map`] (title, objective, sizes, starts), [`World::time`],
//!   [`World::turn`].
//! - Entities: [`World::entity`] by id. An [`Entity`] has its type, place,
//!   position, hit points, mana, owner, orders (keyword, type, target) and
//!   the units inside it. [`World::order_menu`] and
//!   [`World::full_order_menu`] list the orders a unit may receive,
//!   [`World::order_info`] their costs, [`World::activity`] what it is busy
//!   with.
//! - Players: [`World::players`] (in the game), [`World::player`]: resources,
//!   food, units, upgrades, allies, perception, memory (copies of entities
//!   seen before, with the time they were seen), observed squares,
//!   statistics, objectives, victory, defeat, cheat mode. A player in cheat
//!   mode has observed every square before.
//! - [`World::get_objects_string`] and [`World::get_digest`] describe the
//!   whole state, to compare two runs.
//!
//! # Computer players
//!
//! A computer player is a [`Player`] of kind `Computer` (scripted by the
//! `ai.txt` script of its AI type: `easy`, `aggressive`...; `timers` does
//! nothing) or `Computer2` (the `ai2` type, which gives random orders and
//! has unlimited resources). The kind selects the AI: a world plays its
//! computers without anything to install.
#![forbid(unsafe_code)]

pub mod msg;
pub mod msgparts;

pub mod defs;

mod ai;
pub mod collision;
pub mod entity;
pub mod events;
pub mod group;
pub mod map;
pub mod nofloat;
pub mod orders;
pub mod pathfinding;
pub mod player;
mod playerlogic;
pub mod pyrandom;
pub mod rules;
pub mod square;
mod squarelogic;
pub mod triggers;
mod unit;
#[cfg(test)]
mod upstream_tests;
pub mod world;

pub use entity::{Entity, EntityId, Place, PlayerId, SquareId, Target};
pub use events::{EntityEvent, Event, Reason, Voila};
pub use map::MapError;
pub use player::{Controller, ControllerKind, Player, PlayerKind};
pub use rules::{AiScripts, Rules, VIRTUAL_TIME_INTERVAL};
pub use world::{Abort, DefaultTrigger, INF, World, multiplayer_default_triggers};
