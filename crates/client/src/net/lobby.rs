//! What the client knows of the server's lobby and of the room it is in,
//! and the few words that tell the player what changed.

use std::sync::Arc;

use soundrts_protocol::{
    AiLevel, LobbyChange, LobbyPlayer, LobbySnapshot, Login, RoomChange, RoomId, RoomState,
    RoomSummary, SlotIndex, SlotPlayer, Whereabouts,
};
use soundrts_world::msg::{Msg, msg, nb2msg};
use soundrts_world::msgparts as mp;

use super::words::{self, text};
use crate::game::style_msg;
use crate::mapfile::{Map, title_of};
use crate::style::Style;

/// The players and rooms of the server.
#[derive(Clone, Debug)]
pub struct LobbyModel {
    pub server_name: String,
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
    /// arrived or left the server (unless `arrivals` is off).
    pub fn apply(&mut self, change: LobbyChange, arrivals: bool) -> Option<Msg> {
        match change {
            LobbyChange::PlayerArrived { player } => {
                let login = player.login.clone();
                self.players.retain(|p| p.login != login);
                self.players.push(player);
                (arrivals && login != self.you)
                    .then(|| [vec![login.as_str().into()], msg(mp::HAS_JUST_LOGGED_IN)].concat())
            }
            LobbyChange::PlayerLeft { login } => {
                self.players.retain(|p| p.login != login);
                arrivals
                    .then(|| [vec![login.as_str().into()], msg(mp::HAS_JUST_LOGGED_OUT)].concat())
            }
            LobbyChange::PlayerMoved { player } => {
                match self.players.iter_mut().find(|p| p.login == player.login) {
                    Some(p) => *p = player,
                    None => self.players.push(player),
                }
                None
            }
            LobbyChange::RoomOpened { room } | LobbyChange::RoomChanged { room } => {
                match self.rooms.iter_mut().find(|r| r.id == room.id) {
                    Some(r) => *r = room,
                    None => self.rooms.push(room),
                }
                None
            }
            LobbyChange::RoomClosed { id } => {
                self.rooms.retain(|r| r.id != id);
                None
            }
        }
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

/// The map of the room: here, or still coming from the server.
#[derive(Clone, Debug)]
pub enum MapState {
    Ready(Arc<Map>),
    Downloading,
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
        SlotPlayer::Human(login) => vec![login.as_str().into()],
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

    pub fn map(&self) -> Option<Arc<Map>> {
        match &self.map {
            MapState::Ready(m) => Some(m.clone()),
            MapState::Downloading => None,
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
        m
    }

    /// What to say of a change, and its kind: a newer message of the same
    /// kind replaces an unsaid older one.
    pub fn announce(&self, change: &RoomChange, style: &Style) -> Option<(Msg, String)> {
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
                "room players".to_owned(),
            ),
            RoomChange::Left { player } => (
                [
                    player_name(player),
                    msg(mp::HAS_JUST_QUIT_GAME),
                    msg(mp::PERIOD),
                    self.status(),
                ]
                .concat(),
                "room players".to_owned(),
            ),
            RoomChange::Faction { slot } => {
                let s = self.state.slots.get(*slot)?;
                (
                    [
                        slot_name(*slot),
                        style_msg(style, s.faction.as_str(), "title"),
                    ]
                    .concat(),
                    format!("room faction {slot}"),
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
                    format!("room alliance {slot}"),
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
                    format!("room ready {slot}"),
                )
            }
            RoomChange::Host { host } => (
                [
                    vec![host.as_str().into()],
                    text(words::IS_NOW_THE_ORGANIZER),
                ]
                .concat(),
                "room host".to_owned(),
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
