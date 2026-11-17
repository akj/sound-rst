//! The game stream: what a seat is sent while it plays a game that a host
//! runs, and what the game is built from and told.
//!
//! - The game's set-up ([`GameSetup`]): the hosting client sends it, the
//!   rules, scripts and map as text, and the host builds the game from it.
//! - The start ([`StartMessage`]): the host sends it to every seat once.
//!   It leaves out the rules: each client loads the same layers from its
//!   own files, and supplies them with
//!   [`GameStart::from_message`](soundrts_world::GameStart::from_message). The
//!   start's [`RulesHash`] names the rules the game was built from, and a
//!   client reads the stream only with rules of the same hash.
//! - Each turn, a seat's [`TurnMessage`]: whole for the first message of
//!   a game and to a seat that has just joined or returned, and after that
//!   as the change since the seat's last message ([`TurnChange`]).
//! - The end ([`GameEnd`]), once.
//! - A player's [`Command`], from its client.
//!
//! On a server, the start, each message, change and end travel as one
//! binary WebSocket frame each, a [`SeatFrame`], beside the lobby's JSON
//! text frames; a command travels as one binary frame from the client.
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
//! could hold. A seat's frames are checked against the game, in order,
//! before they are trusted ([`SeatStream`]): a
//! [`SeatView`](soundrts_world::SeatView) made from what they give answers
//! without panicking.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::fmt;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use soundrts_world::{
    Command, GameEnd, GameSetup, PlayerId, SquareId, StartMessage, TurnChange, TurnMessage,
};

use crate::{
    ClockChange, Invalid, Login, MAX_COMMAND_BYTES, MAX_MESSAGE_BYTES, MapInfo, SeatChange,
    SeatToken, SlotIndex, Speed, StartRefusal, StopReason,
};

/// The largest message of the game stream either side reads: as large as
/// the lobby's largest, which holds a map.
pub const MAX_GAME_MESSAGE_BYTES: usize = MAX_MESSAGE_BYTES;

mod stream;
pub use stream::{
    Back, Checked, JoinRefused, JoinStream, Joined, Joining, Reseated, ReturnStream, Returning,
    SeatStream,
};

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

pub fn encode_setup(setup: &GameSetup) -> Result<Vec<u8>, Invalid> {
    encode(setup)
}

/// The set-up the hosting client sent. Its texts are untrusted still: the
/// world's loaders refuse what they cannot read.
pub fn decode_setup(bytes: &[u8]) -> Result<GameSetup, Invalid> {
    decode(bytes)
}

/// The SHA-256 of the rules text a game is built from: every layer's
/// `rules.txt`, the map's included, joined in order, as
/// [`GameSetup::rules`] holds it and as a client gives it to
/// [`Rules::load`](soundrts_world::Rules::load). The stream names unit
/// types, upgrades and factions by their place in the rules, so a seat
/// reads it right only with rules of the same text.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RulesHash([u8; 32]);

impl RulesHash {
    pub fn of(rules: &str) -> RulesHash {
        use sha2::Digest;
        RulesHash(sha2::Sha256::digest(rules.as_bytes()).into())
    }
}

impl fmt::Debug for RulesHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

/// One binary frame of a seat's game stream, from the server: what a
/// game sends a seat, in this order. That it has a seat, the start,
/// then the whole message, when the seat joins the game or returns to it; a change from
/// the last message the seat was sent at each turn it is sent one; the end
/// once.
///
/// A watcher's seat is sent the stream of the player it follows, exactly
/// as that player's own seat is. When it follows another player, it is
/// sent that player's whole message, and changes from it after: a whole
/// message during the game is only ever a watcher's, and says whom it
/// now follows.
///
/// A turn's change is the end of the seat's turn, with the turn's events.
/// When something came after the end of the turn (the players leaving,
/// when the turn ended the game), a second change of the same game time
/// follows: what the seat knows after the turn, with those events. A seat
/// that missed turns is told, in its next change, their events that are
/// not about an entity, first. A decoded frame is not yet checked: a seat
/// reads its frames through [`JoinStream`], [`SeatStream`] and
/// [`ReturnStream`], which check them in order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SeatFrame {
    /// The start of the game, and the hash of the rules the server built
    /// it from: a seat reads the stream only with rules of that hash.
    Start {
        start: StartMessage,
        rules: RulesHash,
    },
    Whole(TurnMessage),
    Change(TurnChange),
    /// The game is over for the seat: its statistics and result. The
    /// lobby's [`ServerMsg::GameEnded`](crate::ServerMsg::GameEnded)
    /// follows.
    End(GameEnd),
    /// You have a seat in a game: when it starts, when you join it to
    /// watch, and when you return to it. Keep `token` to return if your
    /// connection is lost: the game holds the seat for `hold` after it
    /// notices the loss. The players' names and alliances come with the
    /// game.
    Seated {
        you: Role,
        map: MapInfo,
        speed: Speed,
        paused: bool,
        hold: Duration,
        token: SeatToken,
    },
    /// Something happened to a seat of the game.
    Seat {
        slot: SlotIndex,
        change: SeatChange,
    },
    /// The room's host changed the game's clock.
    Clock {
        by: Login,
        change: ClockChange,
    },
    /// The player of this slot is the room's host from now on, with its
    /// powers over the game: the host before them left it.
    Host {
        slot: SlotIndex,
    },
    /// `from`, an ally of the seat's side, points at `square`; to the
    /// seat of `from`, that its flare was sent.
    Flare {
        from: PlayerId,
        square: SquareId,
    },
    /// The server stopped the game. The lobby's
    /// [`ServerMsg::GameEnded`](crate::ServerMsg::GameEnded) follows.
    Stopped(StopReason),
    /// To the room's host: the game did not start, and the room goes on.
    NotStarted(StartRefusal),
}

/// Whose seat a seat of a game is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    /// The player of this slot of the room plays.
    Player(SlotIndex),
    /// Someone watches: no units, no part in the game, and nothing it
    /// sends changes the game.
    Watcher,
}

pub fn encode_frame(frame: &SeatFrame) -> Result<Vec<u8>, Invalid> {
    encode(frame)
}

/// A frame as sent, not yet checked against the game: see [`SeatStream`].
pub fn decode_frame(bytes: &[u8]) -> Result<SeatFrame, Invalid> {
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

/// A command as its player sends it: at most [`MAX_COMMAND_BYTES`].
pub fn encode_command(command: &Command) -> Result<Vec<u8>, Invalid> {
    check_command(command)?;
    let bytes = encode(command)?;
    if bytes.len() > MAX_COMMAND_BYTES {
        return Err(Invalid(format!(
            "a command of {} bytes is too long",
            bytes.len()
        )));
    }
    Ok(bytes)
}

/// A player's command, as typed as the world takes it: refused beyond
/// [`MAX_COMMAND_BYTES`]. Whether the player may give it is the world's to
/// say when it runs.
pub fn decode_command(bytes: &[u8]) -> Result<Command, Invalid> {
    if bytes.len() > MAX_COMMAND_BYTES {
        return Err(Invalid(format!(
            "a command of {} bytes is too long",
            bytes.len()
        )));
    }
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
