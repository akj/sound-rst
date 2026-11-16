//! What the client knows of the server's lobby and of the room it is in,
//! and the few words that tell the player what changed.

use std::sync::Arc;

use soundrts_protocol::{
    Computer, Income, LobbyChange, LobbyPlayer, LobbySnapshot, Login, MapInfo, RoomChange, RoomId,
    RoomState, RoomSummary, ServerName, Skill, Slot, SlotIndex, SlotPlayer, Whereabouts,
};
use soundrts_world::msg::{Msg, MsgPart, Phrase, comma, msg, nb2msg, period};

use super::words;
use crate::game::style_msg;
use crate::mapfile::{Map, title_of};
use crate::message::RoomTopic;
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
    } else if !map.title_is_valid() {
        Err(TooMany(
            "parts in a map title, or a part that is no name and no count",
        ))
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
                (arrivals && login != self.you).then(|| {
                    [
                        vec![MsgPart::literal(login.as_str())],
                        msg(Phrase::HasJustLoggedIn),
                    ]
                    .concat()
                })
            }
            LobbyChange::PlayerLeft { login } => {
                self.players.retain(|p| p.login != login);
                arrivals.then(|| {
                    [
                        vec![MsgPart::literal(login.as_str())],
                        msg(Phrase::HasJustLoggedOut),
                    ]
                    .concat()
                })
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
            LobbyChange::PlayerRenamed { from, to } => {
                if let Some(p) = self.players.iter_mut().find(|p| p.login == from) {
                    p.login = to.clone();
                }
                if self.you == from {
                    self.you = to;
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

/// How a player of a room is named: the login, or the kind of computer
/// with its level.
pub fn player_name(p: &SlotPlayer) -> Msg {
    match p {
        SlotPlayer::Human(login) => vec![MsgPart::literal(login.as_str())],
        SlotPlayer::Computer(computer) => computer_name(*computer),
    }
}

/// The world's name of a computer: "quiet computer, level 3".
pub fn computer_name(computer: Computer) -> Msg {
    soundrts_world::computer_name(computer.ai_type(), computer.skill().unwrap_or_default())
}

/// An income as said: "150 percent income".
pub fn income_words(income: Income) -> Msg {
    [
        nb2msg(i64::from(income.percent())),
        msg(Phrase::Percent),
        msg(Phrase::Income),
    ]
    .concat()
}

/// How a seat is named: its player's name, then its income when it is
/// not the full income, so that everyone in the room hears it: "bob, 150
/// percent income".
pub fn seat_name(slot: &Slot) -> Msg {
    let mut name = player_name(&slot.player);
    if slot.income != Income::FULL {
        name.extend(comma());
        name.extend(income_words(slot.income));
    }
    name
}

/// A seat's name inside a sentence: followed by a comma when it ends with
/// a level or an income, so that what comes next is heard apart from it:
/// "set quiet computer, level 3, to level 2".
pub fn seat_name_in_sentence(slot: &Slot) -> Msg {
    let mut name = seat_name(slot);
    let listed = slot.income != Income::FULL
        || matches!(slot.player, SlotPlayer::Computer(c) if c.skill().is_some());
    if listed {
        name.extend(comma());
    }
    name
}

/// What a seat's level or income becomes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeatSetting {
    Skill(Skill),
    Income(Income),
}

impl SeatSetting {
    /// Gives the seat this level or income: a level only to a computer
    /// that has one.
    pub fn apply(self, slot: &mut Slot) {
        match (self, &mut slot.player) {
            (SeatSetting::Skill(skill), SlotPlayer::Computer(c)) => {
                *c = c.at(skill).unwrap_or(*c);
            }
            (SeatSetting::Skill(_), SlotPlayer::Human(_)) => {}
            (SeatSetting::Income(income), _) => slot.income = income,
        }
    }
}

/// The choices of a seat's level, for a quiet or an aggressive computer,
/// and of its income, for any seat but the random computer: one for each
/// value the seat does not have, as the organizer of a room or of a
/// training game hears them: "set quiet computer, level 3, to level 2",
/// "set bob to 150 percent income".
pub fn seat_settings(slot: &Slot) -> Vec<(Msg, SeatSetting)> {
    let name = seat_name_in_sentence(slot);
    let set = |value: Msg| [msg(Phrase::Set), name.clone(), msg(Phrase::SetTo), value].concat();
    let mut choices = Vec::new();
    if let SlotPlayer::Computer(c) = slot.player
        && let Some(current) = c.skill()
    {
        for skill in Skill::ALL.into_iter().filter(|s| *s != current) {
            let level = [msg(Phrase::Level), nb2msg(skill.number().into())].concat();
            choices.push((set(level), SeatSetting::Skill(skill)));
        }
    }
    if slot.player.takes_income() {
        for income in Income::CHOICES.into_iter().filter(|i| *i != slot.income) {
            choices.push((set(income_words(income)), SeatSetting::Income(income)));
        }
    }
    choices
}

/// Names separated by short silences (`insert_silences`).
fn names(list: impl IntoIterator<Item = Msg>) -> Msg {
    let mut out = Vec::new();
    for (i, n) in list.into_iter().enumerate() {
        if i > 0 {
            out.extend(period());
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
            msg(Phrase::WatchingList),
            names(
                self.state
                    .watchers
                    .iter()
                    .map(|l| vec![MsgPart::literal(l.as_str())]),
            ),
        ]
        .concat()
    }

    /// What is said on entering the room: the map, whether it takes
    /// watchers, and the seats that do not have the full income.
    pub fn welcome(&self) -> Msg {
        let watchers = if self.state.takes_watchers {
            Phrase::WatchersWelcome
        } else {
            Phrase::NoWatchers
        };
        let mut m = [self.title(), comma(), msg(watchers)].concat();
        if let Some(incomes) = self.incomes() {
            m.extend(period());
            m.extend(incomes);
        }
        m
    }

    /// The seats that do not have the full income, named with it, or
    /// none: an income is never a hidden advantage.
    pub fn incomes(&self) -> Option<Msg> {
        let seats: Vec<Msg> = self
            .state
            .slots
            .iter()
            .filter(|s| s.income != Income::FULL)
            .map(seat_name)
            .collect();
        (!seats.is_empty()).then(|| names(seats))
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
            msg(Phrase::PlayersOn),
            nb2msg(i64::from(map.nb_players_max)),
        ]
        .concat();
        if n >= i64::from(map.nb_players_min) {
            m.extend(msg(Phrase::TheGameWillStartWhenOrganizerIsReady));
        } else {
            m.extend(msg(Phrase::NotEnoughPlayers));
            m.extend(nb2msg(i64::from(map.nb_players_min)));
        }
        m.extend(period());
        m.extend(names(self.state.slots.iter().map(seat_name)));
        let watchers = self.watchers();
        if !watchers.is_empty() {
            m.extend(period());
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
                .map(seat_name_in_sentence)
                .unwrap_or_default()
        };
        Some(match change {
            RoomChange::Entered => return None,
            RoomChange::Joined { player } => (
                [
                    player_name(player),
                    msg(Phrase::HasJustJoined),
                    self.status(),
                ]
                .concat(),
                RoomTopic::Players,
            ),
            RoomChange::Left { player } => (
                [
                    player_name(player),
                    msg(Phrase::HasJustQuitGame),
                    period(),
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
                        msg(Phrase::Move),
                        seat_name_in_sentence(s),
                        msg(Phrase::ToAlliance),
                        nb2msg(i64::from(s.alliance)),
                    ]
                    .concat(),
                    RoomTopic::Alliance(*slot),
                )
            }
            // The computer's name says its new level.
            RoomChange::Skill { slot } => (
                player_name(&self.state.slots.get(*slot)?.player),
                RoomTopic::Skill(*slot),
            ),
            // Said even when it is back to the full income.
            RoomChange::Income { slot } => {
                let s = self.state.slots.get(*slot)?;
                (
                    [player_name(&s.player), comma(), income_words(s.income)].concat(),
                    RoomTopic::Income(*slot),
                )
            }
            RoomChange::Ready { slot } => {
                let s = self.state.slots.get(*slot)?;
                let word = if s.ready {
                    Phrase::IsReady
                } else {
                    Phrase::IsNotReady
                };
                (
                    [slot_name(*slot), msg(word)].concat(),
                    RoomTopic::Ready(*slot),
                )
            }
            RoomChange::Host { host } => (words::new_host(host), RoomTopic::Host),
            RoomChange::Watching { login } => (
                [
                    vec![MsgPart::literal(login.as_str())],
                    msg(Phrase::IsWatching),
                ]
                .concat(),
                RoomTopic::Watcher(login.clone()),
            ),
            RoomChange::StoppedWatching { login } => (
                [
                    vec![MsgPart::literal(login.as_str())],
                    msg(Phrase::StoppedWatching),
                ]
                .concat(),
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
            .map(seat_name)
            .collect()
    }
}
