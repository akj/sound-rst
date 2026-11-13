//! The SoundRTS game simulation.
//!
//! This crate is deterministic and headless. It takes the rules, the computer
//! player scripts and a map as text, and it never touches audio, speech, a
//! window or the file system.
#![forbid(unsafe_code)]
