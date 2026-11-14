//! The game stream: what a seat is sent while it plays a game that a host
//! runs, and what the game is built from and told.
//!
//! - The game's set-up ([`GameSetup`]): the hosting client sends it, the
//!   rules, scripts and map as text, and the host builds the game from it.
//! - The start ([`StartMessage`]): the host sends it to every seat once.
//!   It leaves out the rules: each client has loaded the same layers from
//!   its own files, and supplies them with [`GameStart::from_message`].
//! - Each turn, a seat's [`TurnMessage`]: whole for the first message of
//!   a game and to a seat that has just joined or returned, and after that
//!   as the change since the seat's last message ([`TurnChange`]).
//! - The end ([`GameEnd`]), once.
//! - A player's [`Command`], from its client.
//!
//! Each is encoded with postcard, a compact binary form of serde's data
//! model: field values in order, integers as variable-length numbers, no
//! names. The same value gives the same bytes on every machine. The lobby
//! keeps JSON. Measured on whole games (`tests/stream_measure.rs`), a
//! player hosting 15 other seats of the largest game would send about 6.8
//! megabits a second of JSON at the 99th percentile, and 0.8 of postcard;
//! with the WebSocket's per-message compression, 0.37 against 0.23.
//!
//! Decoding takes bytes from a stranger. It refuses more than
//! [`MAX_GAME_MESSAGE_BYTES`], bytes left over, and anything the encoder
//! could not have written; it never panics. A list states its length
//! first, and postcard allocates for no more items than the bytes left
//! could hold. What is decoded is checked against the game before it is
//! trusted: a [`SeatView`](soundrts_world::SeatView) made from it answers
//! without panicking.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use serde::Serialize;
use serde::de::DeserializeOwned;
use soundrts_world::{
    Command, GameEnd, GameSetup, GameStart, StartMessage, TurnChange, TurnMessage,
};

use crate::{Invalid, MAX_MESSAGE_BYTES};

/// The largest message of the game stream either side reads: as large as
/// the lobby's largest, which holds a map.
pub const MAX_GAME_MESSAGE_BYTES: usize = MAX_MESSAGE_BYTES;

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, Invalid> {
    let bytes = postcard::to_allocvec(value).map_err(|e| Invalid(format!("cannot encode: {e}")))?;
    if bytes.len() > MAX_GAME_MESSAGE_BYTES {
        return Err(Invalid(format!("{} bytes is too long", bytes.len())));
    }
    Ok(bytes)
}

fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, Invalid> {
    if bytes.len() > MAX_GAME_MESSAGE_BYTES {
        return Err(Invalid(format!("{} bytes is too long", bytes.len())));
    }
    let (value, rest) =
        postcard::take_from_bytes(bytes).map_err(|e| Invalid(format!("cannot decode: {e}")))?;
    if !rest.is_empty() {
        return Err(Invalid(format!("{} bytes left over", rest.len())));
    }
    Ok(value)
}

fn checked<T>(r: Result<T, soundrts_world::Malformed>) -> Result<T, Invalid> {
    r.map_err(|e| Invalid(e.0))
}

pub fn encode_setup(setup: &GameSetup) -> Result<Vec<u8>, Invalid> {
    encode(setup)
}

/// The set-up the hosting client sent. Its texts are untrusted still: the
/// world's loaders refuse what they cannot read.
pub fn decode_setup(bytes: &[u8]) -> Result<GameSetup, Invalid> {
    decode(bytes)
}

pub fn encode_start(start: &StartMessage) -> Result<Vec<u8>, Invalid> {
    encode(start)
}

/// The start of the game, checked: every square it names is on its map.
pub fn decode_start(bytes: &[u8]) -> Result<StartMessage, Invalid> {
    let start: StartMessage = decode(bytes)?;
    checked(start.check())?;
    Ok(start)
}

pub fn encode_turn(message: &TurnMessage) -> Result<Vec<u8>, Invalid> {
    encode(message)
}

/// A whole message, checked against the game it is of.
pub fn decode_turn(bytes: &[u8], start: &GameStart) -> Result<TurnMessage, Invalid> {
    let message = decode(bytes)?;
    checked(start.check(&message))?;
    Ok(message)
}

pub fn encode_change(change: &TurnChange) -> Result<Vec<u8>, Invalid> {
    encode(change)
}

/// A change, not yet applied: [`apply_change`] applies and checks it.
pub fn decode_change(bytes: &[u8]) -> Result<TurnChange, Invalid> {
    decode(bytes)
}

/// The seat's new message: the change decoded and applied to `last`, the
/// message it last had, and checked against the game.
pub fn apply_change(
    bytes: &[u8],
    last: &TurnMessage,
    start: &GameStart,
) -> Result<TurnMessage, Invalid> {
    let message = checked(decode_change(bytes)?.apply(last))?;
    checked(start.check(&message))?;
    Ok(message)
}

pub fn encode_end(end: &GameEnd) -> Result<Vec<u8>, Invalid> {
    encode(end)
}

pub fn decode_end(bytes: &[u8]) -> Result<GameEnd, Invalid> {
    decode(bytes)
}

/// Refuses a speed that is not a finite number above 0, which no
/// interface asks for.
fn check_command(command: &Command) -> Result<(), Invalid> {
    match command {
        Command::Speed(s) if !(s.is_finite() && *s > 0.0) => {
            Err(Invalid(format!("not a speed: {s}")))
        }
        _ => Ok(()),
    }
}

pub fn encode_command(command: &Command) -> Result<Vec<u8>, Invalid> {
    check_command(command)?;
    encode(command)
}

/// A player's command, as typed as the world takes it. Whether the player
/// may give it is the world's to say when it runs.
pub fn decode_command(bytes: &[u8]) -> Result<Command, Invalid> {
    let command = decode(bytes)?;
    check_command(&command)?;
    Ok(command)
}

// The tests may panic: that is how they fail.
#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
mod tests;
