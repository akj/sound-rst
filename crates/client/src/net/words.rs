//! What the client says beyond the phrases of `msgparts`: more sound
//! numbers of the original's `tts.txt`, and the phrases the original never
//! recorded, in English, kept here together so they can be translated
//! later.

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
/// "on", as in "2 on gold"
pub const ON2: &[u64] = &[119];
/// "against"
pub const AGAINST: &[u64] = &[121];
/// "building", what a worker is busy with
pub const BUILDING: &[u64] = &[128];

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
pub const OUT_OF_SYNC: &str =
    "the game is out of sync: you no longer play the same game as the others";
pub const OTHERS_OUT_OF_SYNC: &str = "other players are out of sync: they leave the game";
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
pub const MAP_REFUSED: &str = "the map of this game is too large once unpacked";
pub const GAME_CANNOT_START: &str = "this game cannot start here";
pub const NO_PORT_FORWARDING: &str =
    "the router did not open the port: only the players of your local network can join";
pub const IDLE: &str = "idle";
pub const BADLY_HURT: &str = "badly hurt";
pub const STRONGER: &str = "stronger";
pub const EVEN: &str = "evenly matched";
pub const WEAKER: &str = "weaker";

// The sound and speech settings of the options menu.

pub const SOUND_AND_SPEECH: &str = "sound and speech";
pub const VOLUME: &str = "volume";
pub const PERCENT: &str = "percent";
pub const VOLUME_KEYS: &str = "Enter or Home raises it, End lowers it";
pub const SPOKEN: &str = "spoken";
pub const SOUND_ONLY: &str = "its sound only";
pub const LOG_ONLY: &str = "kept silently in the log";
pub const DROPPED: &str = "dropped";
pub const NVDA_DIRECT: &str = "NVDA tells the game when it has finished speaking";
pub const NVDA_DIRECT_EXPLANATION: &str = "this needs NVDA's current controller client and works from the next start: turn it off if messages stop, overlap, or wait too long";
pub const SPEECH_RATE: &str = "speech rate";
pub const CHARACTERS_A_SECOND: &str = "characters a second";
pub const MEASURE_IT: &str = "press Enter to measure it";
pub const MEASURE_HOW: &str =
    "press a key to hear a sentence, then press a key the moment it ends. Escape cancels.";
/// Said to measure the speech rate: no digits, which take longer.
pub const MEASURE_SENTENCE: &str = "This sentence measures how fast your screen reader speaks, so that each message of the game can follow the last one without a gap.";
pub const MEASURED: &str = "measured:";
pub const MEASURE_AGAIN: &str = "to measure again, press Enter";
pub const TOO_EARLY: &str = "the key came too early to be the end of the sentence";
pub const TOO_LATE: &str = "the key came too late to be the end of the sentence";
pub const NOTHING_CHANGED: &str = "nothing changed";
pub const TRY_AGAIN: &str = "to try again, press Enter";

// Key help in a game.

pub const KEY_HELP_ON: &str = "key help on: press a key to hear what it does, Escape to leave";
pub const KEY_HELP_OFF: &str = "key help off";
pub const NO_COMMAND: &str = "no command";

// Copying a message of the log.

pub const COPIED: &str = "copied";

// The categories of news, as the log's review keys name them. An alert is
// the original's own word.

pub const PRODUCTION: &str = "production";
pub const ECONOMY: &str = "economy";
pub const SCOUTING: &str = "scouting";
pub const CHAT: &str = "chat";
pub const SYSTEM: &str = "system";
pub const STORY: &str = "story";

/// A phrase as a message.
pub fn text(s: &str) -> Msg {
    vec![MsgPart::from(s)]
}
