//! What the multiplayer menus and messages say beyond the phrases of
//! `msgparts`: more sound numbers of the original's `tts.txt`, and the
//! phrases the original never recorded, in English, kept here together so
//! they can be translated later.

use soundrts_world::msg::{Msg, MsgPart};

// Phrases of the original's `tts.txt` that `msgparts` does not name.

/// "game"
pub const GAME: &[u64] = &[4000];
/// "Internet"
pub const INTERNET: &[u64] = &[4001];
/// "local"
pub const LOCAL: &[u64] = &[4002];
/// "processing"
pub const PROCESSING: &[u64] = &[4004];
/// "join"
pub const JOIN: &[u64] = &[4019];
/// "error: the server couldn't start"
pub const SERVER_COULD_NOT_START: &[u64] = &[4080];
/// "Downloading..."
pub const DOWNLOADING: &[u64] = &[4328];

// Phrases the original never recorded.

pub const READY: &str = "is ready";
pub const NOT_READY: &str = "is not ready";
pub const I_AM_READY: &str = "I am ready";
pub const I_AM_NOT_READY: &str = "I am not ready";
pub const REMOVE: &str = "remove";
pub const IS_NOW_THE_ORGANIZER: &str = "is now the organizer";
pub const YOU_HAVE_BEEN_REMOVED: &str = "you have been removed from the game";
pub const WAITING_FOR: &str = "waiting for";
pub const GAME_RESUMES: &str = "the game resumes";
pub const OUT_OF_SYNC: &str = "the game is out of sync: the players no longer play the same game";
pub const SHARE_ADDRESS: &str = "the address to give to the other players is";
pub const ADDRESS_COPIED: &str = "it is copied to the clipboard";
pub const RECENT: &str = "recent";
pub const FAVORITE: &str = "favorite";
pub const ADD_TO_FAVORITES: &str = "add this server to your favorites";
pub const REMOVE_FROM_FAVORITES: &str = "remove this server from your favorites";
pub const TO_YOUR_ALLIES: &str = "to your allies";
pub const ANNOUNCE_ARRIVALS: &str = "say when players arrive on the server or leave it";
pub const ON: &str = "on";
pub const OFF: &str = "off";
pub const SERVER_FULL: &str = "the server is full";
pub const NO_SERVER_FOUND: &str = "no server found";

/// A phrase as a message.
pub fn text(s: &str) -> Msg {
    vec![MsgPart::from(s)]
}
