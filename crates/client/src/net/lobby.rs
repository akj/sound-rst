//! What the client knows of the server's lobby and of the room it is in,
//! and the few words that tell the player what changed.

use std::sync::Arc;

use soundrts_protocol::{
    AiLevel, LobbyChange, LobbyPlayer, LobbySnapshot, Login, MAX_TITLE_SOUNDS, MapInfo, RoomChange,
    RoomId, RoomState, RoomSummary, ServerName, SlotIndex, SlotPlayer, Whereabouts,
};
use soundrts_world::msg::{Msg, msg, nb2msg};
use soundrts_world::msgparts as mp;

use super::words::{self, text};
use crate::game::style_msg;
use crate::mapfile::{Map, title_of};
use crate::message::RoomTopic;
use crate::sound_cache::literal;
use crate::style::Style;

/// The most players a client keeps for a server's lobby. A server takes
/// 40 by default; its owner can raise that, but not to anywhere near this.
pub const MAX_LOBBY_PLAYERS: usize = 1000;

/// The most rooms a client keeps for a server. A server opens at most 10
/// by default.
pub const MAX_LOBBY_ROOMS: usize = 1000;

/// The most players in a room or a game: a map names at most 255
/// (`nb_players_max` is a byte), and the largest shipped with the game 16.
pub const MAX_ROOM_SLOTS: usize = 255;

/// What a server sent beyond what a client keeps: the client closes the
/// connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TooMany(pub &'static str);

impl std::fmt::Display for TooMany {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "too many {}", self.0)
    }
}

/// The most watchers a client keeps for a room. A server takes 8 by
/// default.
pub const MAX_ROOM_WATCHERS: usize = 255;

/// Whether the watchers of a room from the server are within what a
/// client keeps.
pub fn check_watchers(watchers: usize) -> Result<(), TooMany> {
    if watchers > MAX_ROOM_WATCHERS {
        Err(TooMany("watchers in a room"))
    } else {
        Ok(())
    }
}

/// Whether a room from the server is within what a client keeps.
pub fn check_room(map: &MapInfo, slots: usize) -> Result<(), TooMany> {
    if slots > MAX_ROOM_SLOTS {
        Err(TooMany("players in a room"))
    } else if map.title_sounds.len() > MAX_TITLE_SOUNDS {
        Err(TooMany("sounds in a map title"))
    } else {
        Ok(())
    }
}

/// Whether a server's lobby, as it sends it on arrival, is within what a
/// client keeps.
pub fn check_snapshot(snapshot: &LobbySnapshot) -> Result<(), TooMany> {
    if snapshot.players.len() > MAX_LOBBY_PLAYERS {
        return Err(TooMany("players"));
    }
    if snapshot.rooms.len() > MAX_LOBBY_ROOMS {
        return Err(TooMany("rooms"));
    }
    for r in &snapshot.rooms {
        check_room(&r.map, r.players.len())?;
    }
    Ok(())
}

/// The players and rooms of the server.
#[derive(Clone, Debug)]
pub struct LobbyModel {
    pub server_name: ServerName,
    /// The player's own login on this server.
    pub you: Login,
    pub players: Vec<LobbyPlayer>,
    pub rooms: Vec<RoomSummary>,
}

impl LobbyModel {
    pub fn new(snapshot: &LobbySnapshot) -> LobbyModel {
        LobbyModel {
            server_name: snapshot.server_name.clone(),
            you: snapshot.you.clone(),
            players: snapshot.players.clone(),
            rooms: snapshot.rooms.clone(),
        }
    }

    /// Applies a change, and gives what to say of it, if anything: who
    /// arrived or left the server (unless `arrivals` is off). A change that
    /// makes the lobby more than a client keeps is not applied.
    pub fn apply(&mut self, change: LobbyChange, arrivals: bool) -> Result<Option<Msg>, TooMany> {
        let player_fits = |players: &[LobbyPlayer], login: &Login| {
            players.len() < MAX_LOBBY_PLAYERS || players.iter().any(|p| p.login == *login)
        };
        Ok(match change {
            LobbyChange::PlayerArrived { player } => {
                let login = player.login.clone();
                if !player_fits(&self.players, &login) {
                    return Err(TooMany("players"));
                }
                self.players.retain(|p| p.login != login);
                self.players.push(player);
                (arrivals && login != self.you)
                    .then(|| [vec![literal(login.as_str())], msg(mp::HAS_JUST_LOGGED_IN)].concat())
            }
            LobbyChange::PlayerLeft { login } => {
                self.players.retain(|p| p.login != login);
                arrivals
                    .then(|| [vec![literal(login.as_str())], msg(mp::HAS_JUST_LOGGED_OUT)].concat())
            }
            LobbyChange::PlayerMoved { player } => {
                if !player_fits(&self.players, &player.login) {
                    return Err(TooMany("players"));
                }
                match self.players.iter_mut().find(|p| p.login == player.login) {
                    Some(p) => *p = player,
                    None => self.players.push(player),
                }
                None
            }
            LobbyChange::RoomOpened { room } | LobbyChange::RoomChanged { room } => {
                check_room(&room.map, room.players.len())?;
                match self.rooms.iter().position(|r| r.id == room.id) {
                    Some(i) => self.rooms[i] = room,
                    None if self.rooms.len() < MAX_LOBBY_ROOMS => self.rooms.push(room),
                    None => return Err(TooMany("rooms")),
                }
                None
            }
            LobbyChange::RoomClosed { id } => {
                self.rooms.retain(|r| r.id != id);
                None
            }
        })
    }

    pub fn room(&self, id: RoomId) -> Option<&RoomSummary> {
        self.rooms.iter().find(|r| r.id == id)
    }

    /// The other players in the lobby, out of rooms and games.
    pub fn available_players(&self) -> Vec<Login> {
        self.players
            .iter()
            .filter(|p| p.whereabouts == Whereabouts::Lobby && p.login != self.you)
            .map(|p| p.login.clone())
            .collect()
    }
}

/// The map of the room: here, still coming from the server, or refused
/// (the server does not have it, or it cannot be unpacked or read).
#[derive(Clone, Debug)]
pub enum MapState {
    Ready(Arc<Map>),
    Downloading,
    Refused,
}

/// The room the player is in, before its game starts.
#[derive(Clone, Debug)]
pub struct RoomModel {
    pub state: RoomState,
    pub map: MapState,
}

/// How a player of a room is named: the login, or the kind of computer.
pub fn player_name(p: &SlotPlayer) -> Msg {
    match p {
        SlotPlayer::Human(login) => vec![literal(login.as_str())],
        SlotPlayer::Computer(level) => computer_name(*level),
    }
}

pub fn computer_name(level: AiLevel) -> Msg {
    match level {
        AiLevel::Easy => msg(mp::QUIET_COMPUTER),
        AiLevel::Aggressive => msg(mp::AGGRESSIVE_COMPUTER),
        AiLevel::Ai2 => [msg(mp::AGGRESSIVE_COMPUTER), nb2msg(2)].concat(),
    }
}

/// Names separated by short silences (`insert_silences`).
fn names(list: impl IntoIterator<Item = Msg>) -> Msg {
    let mut out = Vec::new();
    for (i, n) in list.into_iter().enumerate() {
        if i > 0 {
            out.extend(msg(mp::PERIOD));
        }
        out.extend(n);
    }
    out
}

impl RoomModel {
    pub fn title(&self) -> Msg {
        title_of(&self.state.map)
    }

    pub fn slot_of(&self, login: &Login) -> Option<SlotIndex> {
        self.state
            .slots
            .iter()
            .position(|s| s.player == SlotPlayer::Human(login.clone()))
    }

    pub fn is_host(&self, login: &Login) -> bool {
        self.state.host == *login
    }

    /// Whether `login` watches the room or its game.
    pub fn is_watcher(&self, login: &Login) -> bool {
        self.state.watchers.contains(login)
    }

    /// Who watches, as said: "watching:" and their names; nothing if
    /// nobody does.
    pub fn watchers(&self) -> Msg {
        if self.state.watchers.is_empty() {
            return Vec::new();
        }
        [
            text(words::WATCHING_LIST),
            names(
                self.state
                    .watchers
                    .iter()
                    .map(|l| vec![literal(l.as_str())]),
            ),
        ]
        .concat()
    }

    /// What is said on entering the room: the map, and whether it takes
    /// watchers.
    pub fn welcome(&self) -> Msg {
        let watchers = if self.state.takes_watchers {
            words::TAKES_WATCHERS
        } else {
            words::NO_WATCHERS_HERE
        };
        [self.title(), msg(mp::COMMA), text(watchers)].concat()
    }

    pub fn map(&self) -> Option<Arc<Map>> {
        match &self.map {
            MapState::Ready(m) => Some(m.clone()),
            MapState::Downloading | MapState::Refused => None,
        }
    }

    /// How many players, how many are needed, and who (`_game_status`).
    pub fn status(&self) -> Msg {
        let n = self.state.slots.len() as i64;
        let map = &self.state.map;
        let mut m = [
            nb2msg(n),
            msg(mp::PLAYERS_ON),
            nb2msg(i64::from(map.nb_players_max)),
        ]
        .concat();
        if n >= i64::from(map.nb_players_min) {
            m.extend(msg(mp::THE_GAME_WILL_START_WHEN_ORGANIZER_IS_READY));
        } else {
            m.extend(msg(mp::NOT_ENOUGH_PLAYERS));
            m.extend(nb2msg(i64::from(map.nb_players_min)));
        }
        m.extend(msg(mp::PERIOD));
        m.extend(names(
            self.state.slots.iter().map(|s| player_name(&s.player)),
        ));
        let watchers = self.watchers();
        if !watchers.is_empty() {
            m.extend(msg(mp::PERIOD));
            m.extend(watchers);
        }
        m
    }

    /// What to say of a change, and what it is about: newer news about
    /// the same topic replaces the unheard older one.
    pub fn announce(&self, change: &RoomChange, style: &Style) -> Option<(Msg, RoomTopic)> {
        let slot_name = |slot: SlotIndex| {
            self.state
                .slots
                .get(slot)
                .map(|s| player_name(&s.player))
                .unwrap_or_default()
        };
        Some(match change {
            RoomChange::Entered => return None,
            RoomChange::Joined { player } => (
                [player_name(player), msg(mp::HAS_JUST_JOINED), self.status()].concat(),
                RoomTopic::Players,
            ),
            RoomChange::Left { player } => (
                [
                    player_name(player),
                    msg(mp::HAS_JUST_QUIT_GAME),
                    msg(mp::PERIOD),
                    self.status(),
                ]
                .concat(),
                RoomTopic::Players,
            ),
            RoomChange::Faction { slot } => {
                let s = self.state.slots.get(*slot)?;
                (
                    [
                        slot_name(*slot),
                        style_msg(style, s.faction.as_str(), "title"),
                    ]
                    .concat(),
                    RoomTopic::Faction(*slot),
                )
            }
            RoomChange::Alliance { slot } => {
                let s = self.state.slots.get(*slot)?;
                (
                    [
                        msg(mp::MOVE),
                        slot_name(*slot),
                        msg(mp::TO_ALLIANCE),
                        nb2msg(i64::from(s.alliance)),
                    ]
                    .concat(),
                    RoomTopic::Alliance(*slot),
                )
            }
            RoomChange::Ready { slot } => {
                let s = self.state.slots.get(*slot)?;
                let word = if s.ready {
                    words::READY
                } else {
                    words::NOT_READY
                };
                (
                    [slot_name(*slot), text(word)].concat(),
                    RoomTopic::Ready(*slot),
                )
            }
            RoomChange::Host { host } => (words::new_host(host), RoomTopic::Host),
            RoomChange::Watching { login } => (
                [vec![literal(login.as_str())], text(words::IS_WATCHING)].concat(),
                RoomTopic::Watcher(login.clone()),
            ),
            RoomChange::StoppedWatching { login } => (
                [vec![literal(login.as_str())], text(words::STOPPED_WATCHING)].concat(),
                RoomTopic::Watcher(login.clone()),
            ),
        })
    }

    /// The humans who have not said they are ready.
    pub fn not_ready(&self) -> Vec<Msg> {
        self.state
            .slots
            .iter()
            .filter(|s| !s.ready)
            .map(|s| player_name(&s.player))
            .collect()
    }
}
