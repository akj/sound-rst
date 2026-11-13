//! The SoundRTS client: everything the player hears and presses.
//!
//! [`client::Client`] is the context the menus and the game interface
//! receive: settings, resources, sound, voice and keyboard. It is built from
//! [`client::Devices`]; `testkit::Harness` (feature `testkit`) builds one
//! with recording fakes so a test can run real menus and games and check
//! what was said.
//!
//! A game is a [`session::Session`] (a training game or a campaign
//! mission) that owns the world and runs the [`game::GameInterface`].

pub mod audio;
pub mod bindings;
pub mod campaign;
pub mod client;
pub mod clipboard;
pub mod clock;
pub mod config;
pub mod game;
pub mod input;
pub mod log;
pub mod mainmenu;
pub mod mapfile;
pub mod menu;
pub mod message;
pub mod notice;
pub mod options;
pub mod package;
pub mod parameters;
pub mod paths;
pub mod probe;
pub mod resource;
pub mod rng;
pub mod rodio_output;
pub mod session;
pub mod singleplayer;
pub mod sound;
pub mod sound_cache;
pub mod style;
#[cfg(any(test, feature = "testkit"))]
pub mod testkit;
pub mod tts;
pub mod voice;
pub mod window;
