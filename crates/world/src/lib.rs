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
//!     world.queue_command(me, "order 0 0 go 12"); // from a server or a record
//!     world.update(); // one turn: 300 ms of game time
//!     for _event in world.drain_events(me) { /* speak, play sounds */ }
//!     # break;
//! }
//! ```
//!
//! The rules and scripts of a mod, a campaign or a map are more layers of
//! the same files: pass them all, in order, to [`Rules::load`] and
//! [`AiScripts::load`]. A command is a [`Command`], given with
//! [`World::queue`], or the original's text line (`control 4 7-9`, `order 0
//! 1 go 12`, `cmd add_units a1 footman`, `quit`...), given with
//! [`World::queue_command`]; it runs at the start of the next
//! [`World::update`]. Only a local human
//! ([`Controller::human`] with `local`) receives events.
//!
//! # Reading the state
//!
//! An interface reads what one player knows through [`Host::view`]: see
//! [`view`]. It is told about an entity as that player knows it, a
//! [`KnownEntity`]: see [`known`]. The world itself has the whole state:
//!
//! - The map: [`World::squares`] (grid, names, neighbors, exits, terrain),
//!   [`World::map`] (title, objective, sizes, starts), [`World::time`],
//!   [`World::turn`].
//! - Entities: [`World::entity`] by id. An entity has its type, place,
//!   position, hit points, mana, owner, orders (keyword, type, target) and
//!   the units inside it. Only the `truth` feature can name its type,
//!   `entity::Entity`: an interface is built without it.
//!   [`World::order_menu`] and [`World::full_order_menu`] list the orders
//!   a unit may receive, [`World::order_info`] their costs,
//!   [`World::activity`] what it is busy with.
//! - Players: [`World::players`] (in the game), [`World::player`]: resources,
//!   food, units, upgrades, allies, perception, memory (copies of entities
//!   seen before, with the time they were seen), observed squares,
//!   statistics, objectives, victory, defeat, cheat mode. A player in cheat
//!   mode has observed every square before.
//! - [`World::digest_u64`] and [`World::get_digest`] sum up the whole
//!   state, to compare two runs.
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
pub mod command;
// The simulation's entity, whole. The interface is built without `truth`
// and so cannot name it: it is told about entities as `KnownEntity`.
#[cfg(feature = "truth")]
pub mod entity;
#[cfg(not(feature = "truth"))]
mod entity;
pub mod events;
pub mod group;
pub mod host;
pub mod known;
pub mod map;
pub mod nofloat;
pub mod orders;
pub mod pathfinding;
pub mod player;
mod playerlogic;
pub mod pyrandom;
pub mod rules;
pub mod setup;
pub mod square;
mod squarelogic;
pub mod triggers;
mod unit;
#[cfg(test)]
mod upstream_tests;
pub mod view;
pub mod world;

pub use command::{Cheat, Command, KeywordOrder, Order, OrderTarget, Queue};
#[cfg(feature = "truth")]
pub use entity::Entity;
pub use entity::{AiMode, EntityId, Place, PlayerId, SquareId, Target};
pub use events::{EntityEvent, Event, Reason, Voila};
pub use host::{Host, TurnReport};
pub use known::{KnownEntity, KnownOrder, KnownTarget, OwnPart};
pub use map::MapError;
pub use player::{Controller, ControllerKind, Player, PlayerKind};
pub use rules::{AiScripts, Rules, VIRTUAL_TIME_INTERVAL};
pub use setup::GameSetup;
pub use view::{Comrades, MapSquare, View};
pub use world::{Abort, DefaultTrigger, INF, World, multiplayer_default_triggers};

/// The version of the simulation. Players can join the same game only with
/// the same version. Bump it whenever a change makes the simulation compute
/// a different result from the same inputs, which includes a change to
/// [`World::digest_u64`].
pub const SIM_VERSION: u32 = 4;

/// The hash of the recorded traces under `tests/traces/expected` for
/// [`SIM_VERSION`]. A test fails when the traces change and this does not.
#[cfg(test)]
const TRACES_HASH: u64 = 0xd716_57f0_6583_39d6;

#[cfg(test)]
mod sim_version {
    use std::hash::{Hash, Hasher};

    #[test]
    fn the_traces_change_only_with_the_sim_version() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/traces/expected");
        let mut names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        let mut h = crate::world::Fnv1a::default();
        for name in names {
            let text = std::fs::read_to_string(dir.join(&name)).unwrap();
            name.hash(&mut h);
            text.replace('\r', "").hash(&mut h);
        }
        assert_eq!(
            h.finish(),
            super::TRACES_HASH,
            "the traces changed: bump SIM_VERSION and set TRACES_HASH to {:#018x}",
            h.finish()
        );
    }
}
