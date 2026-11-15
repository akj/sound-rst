//! What the client says beyond the phrases of `msgparts`: more sound
//! numbers of the original's `tts.txt`, and the phrases the original never
//! recorded, in English, kept here together so they can be translated
//! later.

use soundrts_protocol::{ClockChange, Login, SeatChange, Speed, StartRefusal, StopReason};
use soundrts_world::msg::{Msg, MsgPart, nb2msg};

use crate::game::server::Failure;
use crate::sound_cache::literal;

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
/// "the list is empty"
pub const THE_LIST_IS_EMPTY: &[u64] = &[4321];

// Phrases the original never recorded.

pub const READY: &str = "is ready";
pub const NOT_READY: &str = "is not ready";
pub const I_AM_READY: &str = "I am ready";
pub const I_AM_NOT_READY: &str = "I am not ready";
pub const REMOVE: &str = "remove";
pub const IS_NOW_THE_ORGANIZER: &str = "is now the organizer";
pub const YOU_HAVE_BEEN_REMOVED: &str = "you have been removed from the game";
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
/// After the server's name in the welcome: the connection is encrypted.
pub const ENCRYPTED: &str = "encrypted";
// Why an encrypted connection was refused.
pub const CERTIFICATE_FOR_ANOTHER_NAME: &str =
    "not connected: the server's certificate is for another name";
pub const CERTIFICATE_OUT_OF_DATE: &str = "not connected: the server's certificate is out of date";
pub const CERTIFICATE_NOT_TRUSTED: &str = "not connected: the server's certificate is not trusted";
pub const MAP_REFUSED: &str = "the map of this game is too large once unpacked";
pub const GAME_CANNOT_START: &str = "this game cannot start here";
pub const NO_PORT_FORWARDING: &str =
    "the router did not open the port: only the players of your local network can join";
pub const IDLE: &str = "idle";
pub const BADLY_HURT: &str = "badly hurt";
pub const STRONGER: &str = "stronger";
pub const EVEN: &str = "evenly matched";
pub const WEAKER: &str = "weaker";
pub const DONE: &str = "done";
pub const NOT_DONE: &str = "not done";
/// As in "2 of 3".
pub const OF: &str = "of";
pub const DAMAGE: &str = "damage";
/// As in "damage 6 every 1.5 seconds".
pub const EVERY: &str = "every";
pub const NO_ATTACK: &str = "no attack";
pub const RANGE: &str = "range";
pub const ARMOR: &str = "armor";
pub const HIT_POINTS: &str = "hit points";

// Saved games.

pub const SAVE_GAME: &str = "save game";
pub const SAVED: &str = "saved";
pub const CANNOT_SAVE: &str = "cannot save";
pub const LOAD_A_GAME: &str = "load a game";
/// The game saved by itself when the player last quit.
pub const LAST_GAME: &str = "last game";
pub const LOADED: &str = "loaded";
pub const CANNOT_LOAD: &str = "cannot load";
pub const ANOTHER_VERSION: &str = "made by another version of the game";
pub const DAMAGED: &str = "the file is damaged";
pub const LOADING_STOPPED: &str = "loading stopped";
pub const SAVED_JUST_NOW: &str = "saved just now";
/// As in "saved 3 hours ago".
pub const SAVED_WORD: &str = "saved";
pub const MINUTES_AGO: &str = "minutes ago";
pub const HOURS_AGO: &str = "hours ago";
pub const DAYS_AGO: &str = "days ago";
pub const DELETE_THIS_SAVE: &str = "delete this saved game?";
pub const DELETE: &str = "delete";
pub const DELETED: &str = "deleted";

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

// A game on a server: what happens to the players and the clock, and why
// a game stops or does not start. The most important word comes first.

pub const LOST_THE_CONNECTION: &str = "lost the connection";
pub const IS_BACK: &str = "is back";
pub const GIVEN_UP: &str = "did not come back, and is out of the game";
pub const LEFT_THE_GAME: &str = "left the game";
pub const REMOVED_BY: &str = "removed from the game by";
pub const PAUSED_BY: &str = "paused by";
pub const RESUMED_BY: &str = "resumed by";
pub const SPEED: &str = "speed";
pub const SLOW: &str = "slow";
pub const NORMAL: &str = "normal";
pub const FAST: &str = "fast";
pub const SET_BY: &str = "set by";
pub const STOPPED: &str = "game stopped by the server:";
pub const IT_FAILED: &str = "it failed";
pub const A_TURN_TOOK_TOO_LONG: &str = "a turn took too long";
pub const NOT_STARTED: &str = "game not started:";
pub const CANNOT_BUILD: &str = "the server cannot build it:";
pub const TOOK_TOO_LONG_TO_BUILD: &str = "it took too long to build";
pub const SERVER_BUSY: &str = "the server is busy, try again later";
pub const CANNOT_BE_RIGHT: &str = "game ended: the server sent what cannot be right";
/// The game's rules are not the ones this client has, so its unit names
/// would be wrong: the game is not played.
pub const RULES_DIFFER: &str = "cannot play: your game data differs from this game's";
pub const ONLY_THE_ORGANIZER_SETS_THE_SPEED: &str = "only the organizer can change the speed";
/// The game menu's item that stops the clock of a game on a server.
pub const PAUSE: &str = "pause";
/// The game menu's item that starts it again.
pub const RESUME: &str = "resume";

// Returning to a game on a server after the connection is lost.

pub const CONNECTION_LOST: &str = "connection lost, trying to return";
pub const BACK_IN_THE_GAME: &str = "back in the game";
pub const ORDERS_NOT_TAKEN: &str = "order not taken: not connected";
/// A chat line or a flare given while the connection is lost.
pub const NOT_SENT: &str = "not sent: not connected";
pub const COULD_NOT_RETURN: &str = "could not return:";
pub const THE_GAME_IS_OVER: &str = "the game is over";
pub const THE_SEAT_WAS_GIVEN_UP: &str = "your seat was given up";
pub const NO_ANSWER_IN_TIME: &str = "the server could not be reached in time";

// Watching a game: no units and no part in it, the side of one player at
// a time.

/// After someone's name: they joined the room or its game to watch.
pub const IS_WATCHING: &str = "is watching";
pub const STOPPED_WATCHING: &str = "stopped watching";
/// Before the names of those who watch.
pub const WATCHING_LIST: &str = "watching:";
/// Before the name of the player a watcher follows, and after a watcher's
/// name in a chat line.
pub const WATCHING: &str = "watching";
/// The lobby's item that watches a game, before its host and map.
pub const WATCH: &str = "watch";
pub const TAKES_WATCHERS: &str = "watchers welcome";
pub const NO_WATCHERS_HERE: &str = "no watchers";
/// The setting of a room being created: whether people may join it only
/// to watch, and its two values.
pub const WATCHERS: &str = "watchers";
pub const WELCOME: &str = "welcome";
pub const NOT_WELCOME: &str = "not welcome";
pub const NO_WATCHERS: &str = "this game takes no watchers";
pub const WATCHERS_FULL: &str = "no room for another watcher";
pub const ROOM_CLOSED: &str = "the room closed: no player is left";
pub const WATCHING_NO_ORDERS: &str = "watching: you give no orders";

// A flare that points the allies at a square.

/// After an ally's name, before the square: "bob: flare at b 2".
pub const FLARE_AT: &str = "flare at";
/// To the player whose flare the server sent.
pub const FLARE_SENT: &str = "flare sent";
pub const NO_ALLY: &str = "no ally in the game";
/// Off a server: a flare has nobody to reach.
pub const NOBODY_TO_TELL: &str = "nobody to tell";
/// The server refused a flare, a chat line or a command sent too soon
/// after others.
pub const TOO_FAST: &str = "too fast: wait a moment";
pub const NOT_WATCHING: &str = "only a watcher follows another player";

/// What happened to a player of the game, `who`.
pub fn seat_news(who: Msg, change: &SeatChange) -> Msg {
    let what = match change {
        SeatChange::Dropped => text(LOST_THE_CONNECTION),
        SeatChange::Returned => text(IS_BACK),
        SeatChange::GivenUp => text(GIVEN_UP),
        SeatChange::Left => text(LEFT_THE_GAME),
        SeatChange::Removed { by } => [text(REMOVED_BY), vec![literal(by.as_str())]].concat(),
    };
    [who, what].concat()
}

/// A speed of the game, as said.
pub fn speed(speed: Speed) -> Msg {
    match speed {
        Speed::Slow => text(SLOW),
        Speed::Normal => text(NORMAL),
        Speed::Fast2 => [text(FAST), nb2msg(2)].concat(),
        Speed::Fast4 => [text(FAST), nb2msg(4)].concat(),
    }
}

/// What the room's host, `by`, did to the clock.
pub fn clock_news(by: &Login, change: ClockChange) -> Msg {
    let what = match change {
        ClockChange::Paused => text(PAUSED_BY),
        ClockChange::Resumed => text(RESUMED_BY),
        ClockChange::Speed { speed: s } => [text(SPEED), speed(s), text(SET_BY)].concat(),
    };
    [what, vec![literal(by.as_str())]].concat()
}

/// Who is the room's host from now on.
pub fn new_host(host: &Login) -> Msg {
    [vec![literal(host.as_str())], text(IS_NOW_THE_ORGANIZER)].concat()
}

/// Why the server stopped the game.
pub fn stopped(reason: StopReason) -> Msg {
    let why = match reason {
        StopReason::Crashed => IT_FAILED,
        StopReason::TooSlow => A_TURN_TOOK_TOO_LONG,
    };
    [text(STOPPED), text(why)].concat()
}

/// Why the game did not start.
pub fn not_started(refusal: &StartRefusal) -> Msg {
    let why = match refusal {
        StartRefusal::CannotBuild { error } => {
            [text(CANNOT_BUILD), vec![literal(error.as_str())]].concat()
        }
        StartRefusal::TooSlow => text(TOOK_TOO_LONG_TO_BUILD),
        StartRefusal::Crashed => text(IT_FAILED),
        StartRefusal::Busy => text(SERVER_BUSY),
    };
    [text(NOT_STARTED), why].concat()
}

/// Why the player could not return to the game.
pub fn return_failed(why: Failure) -> Msg {
    let why = match why {
        Failure::GameOver => THE_GAME_IS_OVER,
        Failure::SeatGivenUp => THE_SEAT_WAS_GIVEN_UP,
        Failure::TimeUp => NO_ANSWER_IN_TIME,
    };
    [text(COULD_NOT_RETURN), text(why)].concat()
}

/// A phrase as a message.
pub fn text(s: &str) -> Msg {
    vec![MsgPart::from(s)]
}
