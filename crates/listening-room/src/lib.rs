//! The listening room: a console program that plays the game's sounds
//! through several renderers and asks the player what they hear, so the
//! player's ears can decide the game's sound design.
//!
//! A renderer takes a [`scene::Scene`] (a listener and sources placed in
//! metres) and produces what each ear hears. [`render::Current`] is the
//! game's maths as it is now; [`render::StereoPlus`] adds smooth volume and
//! a duller tone behind; `binaural::Binaural` (feature `binaural`) uses
//! Steam Audio's head model, alone or with the plain cues of `stereo-plus`
//! for behind. [`script`] is the numbered script.

#[cfg(feature = "binaural")]
pub mod binaural;
pub mod render;
pub mod rng;
pub mod scene;
pub mod script;
pub mod sounds;
pub mod wav;
