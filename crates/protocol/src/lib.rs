//! The messages between a SoundRTS client and a SoundRTS server.
//!
//! A server is a lobby, a chat and a turn clock. It never runs the game:
//! every client runs the same simulation from the same seed, and the server
//! tells them which orders to apply at each turn (lockstep). The server
//! knows nothing about rules or maps beyond what a host tells it, so it
//! needs no game data.
//!
//! Each message is one WebSocket text frame holding one JSON object. Every
//! type that crosses the wire checks its own limits when it is read, so a
//! value of these types is always valid.

use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Changes whenever a message changes shape. Client and server must agree.
pub const PROTOCOL_VERSION: u32 = 1;

/// The TCP port of the game, and the UDP port of the discovery on the
/// local network. Not the original's 2500, so that an original client
/// never reaches this server by mistake.
pub const DEFAULT_PORT: u16 = 2501;

/// The game time a turn advances, as the simulation's
/// `VIRTUAL_TIME_INTERVAL`.
pub const TURN_GAME_MS: u64 = 300;

/// The longest login, in characters.
pub const MAX_LOGIN_CHARS: usize = 20;

/// The longest chat message, in characters. Longer messages are cut.
pub const MAX_CHAT_CHARS: usize = 400;

/// The most chat lines a server keeps for the lobby and sends on arrival.
pub const CHAT_HISTORY_LINES: usize = 50;

/// The longest order, in bytes.
pub const MAX_ORDER_BYTES: usize = 4096;

/// The most orders in one [`ClientMsg::Orders`].
pub const MAX_ORDERS_PER_MESSAGE: usize = 64;

/// The most bytes of orders one player can have waiting for one turn. The
/// server refuses orders beyond it with [`Refusal::SlowDown`], which keeps
/// every [`ServerMsg::Turn`] far below [`MAX_MESSAGE_BYTES`].
pub const MAX_TURN_ORDER_BYTES: usize = 32 * 1024;

/// The most sounds in [`MapInfo::title_sounds`].
pub const MAX_TITLE_SOUNDS: usize = 16;

/// The largest map file.
pub const MAX_MAP_BYTES: usize = 16 * 1024 * 1024;

/// The largest message either side accepts: a map in base64, with room to
/// spare for the rest of the message.
pub const MAX_MESSAGE_BYTES: usize = MAX_MAP_BYTES / 3 * 4 + 64 * 1024;

/// What a client broadcasts on UDP [`DEFAULT_PORT`] to find servers on the
/// local network. Servers answer with a [`ServerAnnouncement`] in JSON.
pub const DISCOVERY_REQUEST: &[u8] = b"soundrts-rs discover";

// ---------------------------------------------------------------------------
// Validated values
// ---------------------------------------------------------------------------

/// Why a value from the wire was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invalid(pub String);

impl fmt::Display for Invalid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Invalid {}

/// A player's name: 1 to 20 ASCII letters and digits, as in the original.
/// Unique on a server: the server adds a number to a name already taken.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Login(String);

impl Login {
    pub fn new(s: impl Into<String>) -> Result<Login, Invalid> {
        let s = s.into();
        let ok = (1..=MAX_LOGIN_CHARS).contains(&s.len())
            && s.bytes().all(|b| b.is_ascii_alphanumeric());
        if ok {
            Ok(Login(s))
        } else {
            Err(Invalid(format!("bad login: {s:?}")))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Login {
    type Error = Invalid;
    fn try_from(s: String) -> Result<Login, Invalid> {
        Login::new(s)
    }
}

impl From<Login> for String {
    fn from(l: Login) -> String {
        l.0
    }
}

impl fmt::Display for Login {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A short identifier: 1 to 64 ASCII letters, digits, `_`, `-` and `.`.
fn check_identifier(what: &str, s: &str) -> Result<(), Invalid> {
    let ok = (1..=64).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'));
    if ok {
        Ok(())
    } else {
        Err(Invalid(format!("bad {what}: {s:?}")))
    }
}

macro_rules! identifier {
    ($(#[$doc:meta])* $name:ident, $what:literal) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn new(s: impl Into<String>) -> Result<$name, Invalid> {
                let s = s.into();
                check_identifier($what, &s)?;
                Ok($name(s))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = Invalid;
            fn try_from(s: String) -> Result<$name, Invalid> {
                $name::new(s)
            }
        }

        impl From<$name> for String {
            fn from(v: $name) -> String {
                v.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

identifier!(
    /// What decides whether two clients can play together: a hash of the
    /// rules and the AI the client loaded (mods included), as the
    /// original's compatibility version. Opaque to the server.
    ContentHash,
    "content hash"
);

identifier!(
    /// A faction name from the rules, or `random_faction`. The server
    /// cannot check it against the rules; clients with the same
    /// [`ContentHash`] share the same factions.
    Faction,
    "faction"
);

impl Faction {
    pub fn random() -> Faction {
        Faction("random_faction".into())
    }
}

identifier!(
    /// The SHA-256 of a map file, in lower case hex. Two maps with the same
    /// hash are the same map.
    MapHash,
    "map hash"
);

impl MapHash {
    pub fn of(bytes: &[u8]) -> MapHash {
        use sha2::Digest;
        let digest = sha2::Sha256::digest(bytes);
        MapHash(digest.iter().map(|b| format!("{b:02x}")).collect())
    }
}

identifier!(
    /// A map's file name with its extension: `jl1.txt` for a text map,
    /// anything else for a zipped map package.
    MapFileName,
    "map file name"
);

/// A chat message: no control characters, no space at either end, not
/// empty, at most [`MAX_CHAT_CHARS`] characters. Any language.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ChatText(String);

impl ChatText {
    /// Cleans what a player typed: drops control characters, trims, and
    /// cuts to [`MAX_CHAT_CHARS`]. `None` when nothing is left.
    pub fn clean(s: &str) -> Option<ChatText> {
        let s: String = s.chars().filter(|c| !c.is_control()).collect();
        let s: String = s.trim().chars().take(MAX_CHAT_CHARS).collect();
        let s = s.trim_end();
        (!s.is_empty()).then(|| ChatText(s.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ChatText {
    type Error = Invalid;
    fn try_from(s: String) -> Result<ChatText, Invalid> {
        match ChatText::clean(&s) {
            Some(t) if t.0 == s => Ok(t),
            _ => Err(Invalid("bad chat text".into())),
        }
    }
}

impl From<ChatText> for String {
    fn from(t: ChatText) -> String {
        t.0
    }
}

/// A command for the simulation, as `World::queue_command` takes it: one
/// line, at most [`MAX_ORDER_BYTES`] bytes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Order(String);

impl Order {
    pub fn new(s: impl Into<String>) -> Result<Order, Invalid> {
        let s = s.into();
        let ok = !s.is_empty() && s.len() <= MAX_ORDER_BYTES && !s.chars().any(char::is_control);
        if ok {
            Ok(Order(s))
        } else {
            Err(Invalid("bad order".into()))
        }
    }

    /// The order a server gives in place of a player who left the game.
    pub fn quit() -> Order {
        Order("quit".into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Order {
    type Error = Invalid;
    fn try_from(s: String) -> Result<Order, Invalid> {
        Order::new(s)
    }
}

impl From<Order> for String {
    fn from(o: Order) -> String {
        o.0
    }
}

/// At most [`MAX_ORDERS_PER_MESSAGE`] orders.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Vec<Order>", into = "Vec<Order>")]
pub struct Orders(Vec<Order>);

impl Orders {
    pub fn new(orders: Vec<Order>) -> Result<Orders, Invalid> {
        if orders.len() <= MAX_ORDERS_PER_MESSAGE {
            Ok(Orders(orders))
        } else {
            Err(Invalid("too many orders".into()))
        }
    }

    pub fn as_slice(&self) -> &[Order] {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl TryFrom<Vec<Order>> for Orders {
    type Error = Invalid;
    fn try_from(v: Vec<Order>) -> Result<Orders, Invalid> {
        Orders::new(v)
    }
}

impl From<Orders> for Vec<Order> {
    fn from(o: Orders) -> Vec<Order> {
        o.0
    }
}

/// The bytes of a map file. Base64 on the wire; at most [`MAX_MAP_BYTES`].
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct MapBytes(Vec<u8>);

impl MapBytes {
    pub fn new(bytes: Vec<u8>) -> Result<MapBytes, Invalid> {
        if bytes.len() <= MAX_MAP_BYTES {
            Ok(MapBytes(bytes))
        } else {
            Err(Invalid("map too large".into()))
        }
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }

    pub fn into_vec(self) -> Vec<u8> {
        self.0
    }
}

impl fmt::Debug for MapBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MapBytes({} bytes)", self.0.len())
    }
}

impl TryFrom<String> for MapBytes {
    type Error = Invalid;
    fn try_from(s: String) -> Result<MapBytes, Invalid> {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(s)
            .map_err(|_| Invalid("bad base64".into()))?;
        MapBytes::new(bytes)
    }
}

impl From<MapBytes> for String {
    fn from(b: MapBytes) -> String {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.encode(b.0)
    }
}

// ---------------------------------------------------------------------------
// Game settings
// ---------------------------------------------------------------------------

/// The speeds a game can be created with, as the original's menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Speed {
    Slow,
    Normal,
    Fast2,
    Fast4,
}

impl Speed {
    /// How many times faster than real time.
    pub fn factor(self) -> f64 {
        match self {
            Speed::Slow => 0.5,
            Speed::Normal => 1.0,
            Speed::Fast2 => 2.0,
            Speed::Fast4 => 4.0,
        }
    }

    /// The real time between two turns.
    pub fn turn_interval(self) -> Duration {
        Duration::from_secs_f64(TURN_GAME_MS as f64 / 1000.0 / self.factor())
    }
}

/// The computer players of the original.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiLevel {
    Easy,
    Aggressive,
    Ai2,
}

impl AiLevel {
    /// The name the simulation knows it by (`Controller::computer`).
    pub fn ai_type(self) -> &'static str {
        match self {
            AiLevel::Easy => "easy",
            AiLevel::Aggressive => "aggressive",
            AiLevel::Ai2 => "ai2",
        }
    }
}

/// What a host tells the server about a map. The host's client read it
/// from the file; the server checks only the hash.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapInfo {
    pub file_name: MapFileName,
    pub hash: MapHash,
    /// The sound numbers of the map's `title` line, said after its name.
    /// At most [`MAX_TITLE_SOUNDS`]; a server refuses a room with more.
    pub title_sounds: Vec<i64>,
    pub nb_players_min: u8,
    pub nb_players_max: u8,
}

/// A map file, sent to the server by a host or to a client that lacks it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapData {
    pub file_name: MapFileName,
    pub bytes: MapBytes,
}

impl MapData {
    pub fn hash(&self) -> MapHash {
        MapHash::of(self.bytes.as_slice())
    }
}

// ---------------------------------------------------------------------------
// Lobby
// ---------------------------------------------------------------------------

/// A game room on a server: from its creation until its game ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RoomId(pub u32);

/// Where a player is on the server.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "room")]
pub enum Whereabouts {
    Lobby,
    InRoom(RoomId),
    Playing(RoomId),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LobbyPlayer {
    pub login: Login,
    pub whereabouts: Whereabouts,
}

/// A room as the lobby sees it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomSummary {
    pub id: RoomId,
    pub host: Login,
    pub map: MapInfo,
    pub speed: Speed,
    /// Anyone compatible can join an open room; a closed room needs an
    /// invitation.
    pub open: bool,
    /// Only clients with the same content can join.
    pub content: ContentHash,
    pub players: Vec<SlotPlayer>,
    pub started: bool,
}

/// Who is in a room: the lobby's whole state, sent on arrival.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LobbySnapshot {
    pub server_name: String,
    /// Your login on this server: the one you asked for, or with a number
    /// added if it was taken.
    pub you: Login,
    pub players: Vec<LobbyPlayer>,
    pub rooms: Vec<RoomSummary>,
    /// The last lobby chat lines, oldest first.
    pub chat: Vec<ChatLine>,
}

/// A change to the lobby, sent to every client on the server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "change")]
pub enum LobbyChange {
    PlayerArrived { player: LobbyPlayer },
    PlayerLeft { login: Login },
    PlayerMoved { player: LobbyPlayer },
    RoomOpened { room: RoomSummary },
    RoomChanged { room: RoomSummary },
    RoomClosed { id: RoomId },
}

// ---------------------------------------------------------------------------
// Rooms
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "who")]
pub enum SlotPlayer {
    Human(Login),
    Computer(AiLevel),
}

/// A place in a room, and in the game that starts from it. The slots' order
/// is the players' order in the simulation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Slot {
    pub player: SlotPlayer,
    pub faction: Faction,
    /// Players with the same alliance are allies. From 1.
    pub alliance: u8,
    /// Humans say they are ready; the host and computers always are.
    pub ready: bool,
}

/// An index in [`RoomState::slots`].
pub type SlotIndex = usize;

/// A room as its members see it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomState {
    pub id: RoomId,
    pub host: Login,
    pub map: MapInfo,
    pub speed: Speed,
    pub open: bool,
    pub content: ContentHash,
    pub slots: Vec<Slot>,
}

/// What changed in a room, so that a client can say it in a few words.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "change")]
pub enum RoomChange {
    /// You created or joined the room.
    Entered,
    Joined {
        player: SlotPlayer,
    },
    Left {
        player: SlotPlayer,
    },
    Faction {
        slot: SlotIndex,
    },
    Alliance {
        slot: SlotIndex,
    },
    Ready {
        slot: SlotIndex,
    },
    Host {
        host: Login,
    },
}

/// Why you are no longer in a room.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomExit {
    /// You asked to leave.
    Left,
    /// The host removed you.
    Removed,
}

// ---------------------------------------------------------------------------
// Chat
// ---------------------------------------------------------------------------

/// Who hears a chat message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    /// Everyone on the server who is not playing a game.
    Lobby,
    /// The members of your room, before its game starts.
    Room,
    /// Every human player of your game.
    Game,
    /// Your allies in your game, and you.
    Allies,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatLine {
    pub channel: Channel,
    pub from: Login,
    pub text: ChatText,
}

// ---------------------------------------------------------------------------
// Games
// ---------------------------------------------------------------------------

/// A turn number. The first turn is 0.
pub type Turn = u32;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotOrders {
    pub slot: SlotIndex,
    pub orders: Orders,
}

/// A digest of the simulation's state after a turn (`World::get_digest`).
/// Clients in step have the same digest after the same turn.
pub type Digest = u64;

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

/// From a client to the server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ClientMsg {
    /// The first message. Answered by [`ServerMsg::Welcome`] or
    /// [`ServerMsg::Rejected`].
    Hello {
        protocol: u32,
        login: Login,
        content: ContentHash,
    },

    Chat {
        channel: Channel,
        text: ChatText,
    },

    /// Creates a room with you as host. The server asks for the map with
    /// [`ServerMsg::SendMap`] if it does not have it.
    CreateRoom {
        map: MapInfo,
        speed: Speed,
        open: bool,
    },
    /// A map file: the one the server asked for.
    Map {
        map: MapData,
    },
    /// Asks for a room's map, when you do not have a file with its hash.
    GetMap {
        hash: MapHash,
    },
    JoinRoom {
        room: RoomId,
    },
    LeaveRoom,

    // The host only.
    Invite {
        login: Login,
    },
    AddComputer {
        level: AiLevel,
    },
    /// Removes a computer, or a human who then returns to the lobby.
    RemoveSlot {
        slot: SlotIndex,
    },
    SetAlliance {
        slot: SlotIndex,
        alliance: u8,
    },
    /// Starts the game. Every human must be ready, and the number of
    /// players must suit the map.
    Start,

    /// Your own slot, or a computer's if you are the host.
    SetFaction {
        slot: SlotIndex,
        faction: Faction,
    },
    SetReady {
        ready: bool,
    },

    /// You have built the world and can run turn 0. The server starts the
    /// game's clock once every human has sent it.
    Loaded,
    /// Your orders, for the next turn the server sends.
    Orders {
        orders: Orders,
    },
    /// You have run `turn`, and this is your state after it.
    TurnDone {
        turn: Turn,
        digest: Digest,
    },
    /// You leave the game: the others get a `quit` order for you, and you
    /// return to the lobby.
    LeaveGame,
}

/// From the server to a client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ServerMsg {
    Welcome {
        lobby: LobbySnapshot,
    },
    /// The server closes the connection after this.
    Rejected {
        reason: Rejection,
    },

    Lobby {
        change: LobbyChange,
    },
    Chat {
        line: ChatLine,
    },
    Invited {
        room: RoomId,
        by: Login,
    },

    /// The room you are in changed, or you entered it.
    Room {
        change: RoomChange,
        room: RoomState,
    },
    /// You are no longer in a room.
    RoomExited {
        reason: RoomExit,
    },
    /// To a host: send this map with [`ClientMsg::Map`].
    SendMap {
        hash: MapHash,
    },
    /// The map you asked for.
    Map {
        map: MapData,
    },

    /// The game starts. Run it from `seed`, with the room's slots as
    /// players; `you` is your slot.
    GameStarted {
        seed: i64,
        slots: Vec<Slot>,
        you: SlotIndex,
        map: MapInfo,
        speed: Speed,
    },
    /// The orders to queue before running `turn`, by slot order. Turns come
    /// in order, one per turn interval, and slots with no orders are left
    /// out.
    Turn {
        turn: Turn,
        orders: Vec<SlotOrders>,
    },
    /// The game waits for these players to catch up.
    Waiting {
        players: Vec<Login>,
    },
    Resumed,
    /// A human left the game; their `quit` order is in a coming turn.
    PlayerLeftGame {
        slot: SlotIndex,
    },
    /// Clients disagree about the state after `turn`. `slots` are the
    /// players whose digest differs from the most common one.
    OutOfSync {
        turn: Turn,
        slots: Vec<SlotIndex>,
    },
    /// The game is over for you: you are back in the lobby.
    GameEnded,

    /// The server did not do what you asked.
    Refused {
        reason: Refusal,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "reason")]
pub enum Rejection {
    ProtocolMismatch {
        server: u32,
    },
    ServerFull,
    /// The first message was not a [`ClientMsg::Hello`].
    NoHello,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Refusal {
    /// Not allowed where you are (lobby, room, game) or as who you are.
    NotNow,
    NoSuchRoom,
    NoSuchPlayer,
    RoomFull,
    /// Your content differs from the room's.
    Incompatible,
    TooManyRooms,
    /// Someone is not ready, or the number of players does not suit the
    /// map.
    CannotStart,
    /// The map is not the one asked for, or the server has no such map.
    WrongMap,
    /// Too many messages too fast.
    SlowDown,
    /// The message could not be read.
    Invalid,
}

/// What a server answers to [`DISCOVERY_REQUEST`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerAnnouncement {
    pub protocol: u32,
    pub name: String,
    pub port: u16,
    pub players: u32,
}

// ---------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------

/// Encodes and decodes a message as one JSON text.
pub trait Wire: Serialize + for<'de> Deserialize<'de> {
    fn to_json(&self) -> String {
        serde_json::to_string(self).expect("messages always serialize")
    }

    fn from_json(s: &str) -> Result<Self, Invalid> {
        serde_json::from_str(s).map_err(|e| Invalid(e.to_string()))
    }
}

impl Wire for ClientMsg {}
impl Wire for ServerMsg {}
impl Wire for ServerAnnouncement {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logins_are_ascii_letters_and_digits() {
        assert!(Login::new("Andrew2").is_ok());
        assert!(Login::new("").is_err());
        assert!(Login::new("a b").is_err());
        assert!(Login::new("é").is_err());
        assert!(Login::new("a".repeat(21)).is_err());
    }

    #[test]
    fn a_bad_login_on_the_wire_is_refused() {
        let json = r#"{"type":"hello","protocol":1,"login":"bad login","content":"abc"}"#;
        assert!(ClientMsg::from_json(json).is_err());
    }

    #[test]
    fn chat_keeps_any_language_and_drops_control_characters() {
        let t = ChatText::clean("  bonjour à tous\u{7}\n ").unwrap();
        assert_eq!(t.as_str(), "bonjour à tous");
        assert!(ChatText::clean(" \n\t").is_none());
        let long = ChatText::clean(&"é".repeat(500)).unwrap();
        assert_eq!(long.as_str().chars().count(), MAX_CHAT_CHARS);
    }

    #[test]
    fn unclean_chat_on_the_wire_is_refused() {
        let json = r#"{"type":"chat","channel":"lobby","text":" hi"}"#;
        assert!(ClientMsg::from_json(json).is_err());
    }

    #[test]
    fn orders_have_limits() {
        assert!(Order::new("order 0 0 go 12").is_ok());
        assert!(Order::new("a\nb").is_err());
        assert!(Order::new("").is_err());
        let many = vec![Order::quit(); MAX_ORDERS_PER_MESSAGE + 1];
        assert!(Orders::new(many).is_err());
    }

    #[test]
    fn a_turn_reads_back_the_same() {
        let m = ServerMsg::Turn {
            turn: 7,
            orders: vec![SlotOrders {
                slot: 1,
                orders: Orders::new(vec![Order::new("order 0 0 go 12").unwrap()]).unwrap(),
            }],
        };
        let json = m.to_json();
        assert_eq!(
            json,
            r#"{"type":"turn","turn":7,"orders":[{"slot":1,"orders":["order 0 0 go 12"]}]}"#
        );
        assert_eq!(ServerMsg::from_json(&json).unwrap(), m);
    }

    #[test]
    fn map_bytes_travel_in_base64() {
        let data = MapData {
            file_name: MapFileName::new("jl1.txt").unwrap(),
            bytes: MapBytes::new(b"title 1\n".to_vec()).unwrap(),
        };
        let json = ClientMsg::Map { map: data.clone() }.to_json();
        assert!(json.contains(r#""bytes":"dGl0bGUgMQo=""#), "{json}");
        assert_eq!(
            ClientMsg::from_json(&json).unwrap(),
            ClientMsg::Map { map: data }
        );
    }

    #[test]
    fn the_map_hash_is_sha256() {
        assert_eq!(
            MapHash::of(b"").as_str(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn speeds_set_the_turn_interval() {
        assert_eq!(Speed::Normal.turn_interval(), Duration::from_millis(300));
        assert_eq!(Speed::Fast4.turn_interval(), Duration::from_millis(75));
        assert_eq!(Speed::Slow.turn_interval(), Duration::from_millis(600));
    }

    #[test]
    fn whereabouts_read_back() {
        for w in [
            Whereabouts::Lobby,
            Whereabouts::InRoom(RoomId(3)),
            Whereabouts::Playing(RoomId(4)),
        ] {
            let p = LobbyPlayer {
                login: Login::new("a").unwrap(),
                whereabouts: w,
            };
            let json = serde_json::to_string(&p).unwrap();
            assert_eq!(serde_json::from_str::<LobbyPlayer>(&json).unwrap(), p);
        }
    }
}
