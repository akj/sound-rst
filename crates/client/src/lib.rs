//! The SoundRTS client: everything the player hears and presses.
//!
//! [`client::Client`] is the context the menus and the game interface
//! receive: settings, resources, sound, voice and keyboard. It is built from
//! [`client::Devices`]; [`testkit::Harness`] builds one with recording
//! fakes so a test can run real menus and check what was said.

pub mod audio;
pub mod bindings;
pub mod campaign;
pub mod client;
pub mod clock;
pub mod config;
pub mod input;
pub mod log;
pub mod mainmenu;
pub mod mapfile;
pub mod menu;
pub mod message;
pub mod options;
pub mod package;
pub mod parameters;
pub mod paths;
pub mod probe;
pub mod resource;
pub mod rodio_output;
pub mod sound;
pub mod sound_cache;
pub mod style;
pub mod testkit;
pub mod tts;
pub mod voice;
pub mod window;
