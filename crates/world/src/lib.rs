//! The SoundRTS game simulation.
//!
//! This crate is deterministic and headless. It takes the rules, the computer
//! player scripts and a map as text, and it never touches audio, speech, a
//! window or the file system.
//!
//! # Driving a game
//!
//! ```no_run
//! use soundrts_world::{AiScripts, Command, Controller, Rules, World, multiplayer_default_triggers};
//! # let (rules_txt, ai_txt, map_txt) = ("", "", "");
//! let rules = Rules::load(&[rules_txt]);
//! let ai = AiScripts::load(&[ai_txt]);
//! let mut world = World::new(rules, ai, multiplayer_default_triggers(), 42);
//! world.load_and_build_map(map_txt).expect("a valid map");
//! let me = world
//!     .populate_map(&[Controller::human("me", true), Controller::computer("easy")], true, true)
//!     .expect("enough starting places")[0];
//! loop {
//!     world.queue(me, Command::Quit); // a command of the player
//!     world.update(); // one turn: 300 ms of game time
//!     for _event in world.drain_events(me) { /* speak, play sounds */ }
//!     # break;
//! }
//! ```
//!
//! The rules and scripts of a mod, a campaign or a map are more layers of
//! the same files: pass them all, in order, to [`Rules::load`] and
//! [`AiScripts::load`]. A command is a [`Command`], given with
//! [`World::queue`]; it runs at the start of the next [`World::update`].
//! Only a local human ([`Controller::human`] with `local`) receives
//! events.
//!
//! # Reading the state
//!
//! The [`Host`] tells each player what it knows as plain data, made to be
//! sent: a [`GameStart`] once, a [`TurnMessage`] with each step and a
//! [`GameEnd`] at the end (see [`message`]). An interface reads them
//! through a [`SeatView`], which answers its questions from those alone
//! (see [`seat_view`]). It is told about an entity as that player knows
//! it, a [`KnownEntity`]: see [`known`]. The world itself has the whole
//! state:
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
//! - With the `truth` feature, `World::digest_u64` sums up the whole
//!   state, to compare two runs.
//!
//! # Records
//!
//! A [`Host`] keeps the [`Record`] of its game: its set-up and every
//! command it took, turn by turn. [`Host::replay`] builds the game again
//! from a record; with the `serde` feature, a record is written and read
//! as a file (see [`record`]).
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

mod abort;
mod ai;
pub mod change;
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
pub mod message;
pub mod nofloat;
pub mod orders;
pub mod pathfinding;
pub mod player;
mod playerlogic;
pub mod pyrandom;
mod questions;
mod received;
pub mod record;
pub mod rules;
pub mod seat_view;
pub mod setup;
pub mod square;
mod squarelogic;
#[cfg(feature = "truth")]
pub mod test_games;
pub mod triggers;
mod unit;
#[cfg(test)]
mod upstream_tests;
#[cfg(feature = "serde")]
mod wire;
pub mod world;

pub use ai::{Handicap, computer_name};
pub use change::{
    ById, EntitiesChange, EntityFields, Ids, Malformed, OwnFields, SeenAgain, SquaresChange,
    StateChange, TurnChange,
};
pub use command::{Cheat, Command, KeywordOrder, Order, OrderTarget, Queue, Refused};
#[cfg(feature = "truth")]
pub use entity::Entity;
pub use entity::{AiMode, EntityId, Place, PlayerId, SquareId, Target};
pub use events::{EntityEvent, Event, Reason};
pub use host::{Host, SeatTurn, TurnReport};
pub use known::{Activity, KnownEntity, KnownExit, KnownOrder, KnownTarget, OwnPart};
pub use map::MapError;
pub use message::{
    ControlledPlayer, GameEnd, GameMap, GameStart, KnownSquares, MapExit, SeatState, SquareInfo,
    StartMessage, TurnMessage,
};
pub use player::{Controller, ControllerKind, Player, PlayerKind};
pub use received::MAX_OBJECTIVES;
#[cfg(feature = "serde")]
pub use record::{Completeness, ReadError};
pub use record::{Record, RecordedTurn, ReplayError};
pub use rules::{AiScripts, Rules, VIRTUAL_TIME_INTERVAL};
pub use seat_view::{Comrades, MapSquare, SeatView};
pub use setup::GameSetup;
pub use world::{DefaultTrigger, INF, World, multiplayer_default_triggers};

/// The version of the simulation. Players can join the same game only with
/// the same version. Bump it whenever a change makes the simulation compute
/// a different result from the same inputs, which includes a change to
/// the digest of a world (`World::digest_u64`).
pub const SIM_VERSION: u32 = 12;

/// The hash of the recorded traces under `tests/traces/expected` for
/// [`SIM_VERSION`]. A test fails when the traces change and this does not.
#[cfg(test)]
const TRACES_HASH: u64 = 0x4970_7cc8_c855_fca4;

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
