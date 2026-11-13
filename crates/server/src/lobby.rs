//! The lobby task: it owns the players, the rooms, the lobby chat history
//! and the map cache, and answers every message that is not a game's.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use soundrts_protocol::{
    AiLevel, CHAT_HISTORY_LINES, Channel, ChatLine, ChatText, ClientMsg, ContentHash, Faction,
    LobbyChange, LobbyPlayer, LobbySnapshot, Login, MAX_LOGIN_CHARS, MapData, MapHash, MapInfo,
    Refusal, Rejection, RoomChange, RoomExit, RoomId, ServerMsg, SlotIndex, Speed, Whereabouts,
};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Utf8Bytes;
use tracing::{info, warn};

use crate::ServerConfig;
use crate::connection::{ConnId, Outbox, encode};
use crate::game::{self, GameInput};
use crate::room::{Member, Occupant, Phase, Room};

pub(crate) type LobbyTx = mpsc::UnboundedSender<LobbyInput>;

pub(crate) enum LobbyInput {
    /// A connection said a good hello.
    Hello {
        conn: ConnId,
        login: Login,
        content: ContentHash,
        outbox: Outbox,
    },
    /// A message after the hello.
    Msg { conn: ConnId, msg: ClientMsg },
    /// The connection is closed.
    Gone { conn: ConnId },
    /// A game took this human out (they asked, disconnected, or lagged).
    LeftGame { room: RoomId, conn: ConnId },
    /// No human is left in this game.
    GameOver { room: RoomId },
}

/// The most bytes of maps the server keeps.
const MAP_CACHE_BYTES: usize = 64 * 1024 * 1024;

/// The alliances a host can choose.
const ALLIANCES: std::ops::RangeInclusive<u8> = 1..=8;

pub(crate) fn spawn(config: &ServerConfig, player_count: Arc<AtomicU32>) -> LobbyTx {
    let (tx, rx) = mpsc::unbounded_channel();
    let lobby = Lobby {
        name: config.name.clone(),
        max_clients: config.max_clients,
        max_rooms: config.max_rooms,
        game_settings: (config.max_lead, config.lag_timeout),
        players: BTreeMap::new(),
        rooms: BTreeMap::new(),
        next_room: 1,
        chat: VecDeque::new(),
        maps: MapCache::default(),
        player_count,
        me: tx.downgrade(),
    };
    tokio::spawn(lobby.run(rx));
    tx
}

struct Player {
    login: Login,
    content: ContentHash,
    outbox: Outbox,
    place: Place,
}

enum Place {
    Lobby {
        /// A room this player asked to create, waiting for its map.
        creating: Option<NewRoom>,
    },
    InRoom(RoomId),
    Playing(RoomId),
}

impl Place {
    fn whereabouts(&self) -> Whereabouts {
        match self {
            Place::Lobby { .. } => Whereabouts::Lobby,
            Place::InRoom(id) => Whereabouts::InRoom(*id),
            Place::Playing(id) => Whereabouts::Playing(*id),
        }
    }
}

const IN_LOBBY: Place = Place::Lobby { creating: None };

struct NewRoom {
    map: MapInfo,
    speed: Speed,
    open: bool,
}

/// Maps by hash, as the `ServerMsg::Map` that sends them, while a room
/// uses them.
#[derive(Default)]
struct MapCache {
    maps: HashMap<MapHash, Utf8Bytes>,
    bytes: usize,
}

impl MapCache {
    /// False when the cache has no room for it.
    fn insert(&mut self, hash: MapHash, msg: Utf8Bytes) -> bool {
        if self.maps.contains_key(&hash) {
            return true;
        }
        if self.bytes + msg.len() > MAP_CACHE_BYTES {
            return false;
        }
        self.bytes += msg.len();
        self.maps.insert(hash, msg);
        true
    }

    fn get(&self, hash: &MapHash) -> Option<Utf8Bytes> {
        self.maps.get(hash).cloned()
    }

    fn remove(&mut self, hash: &MapHash) {
        if let Some(msg) = self.maps.remove(hash) {
            self.bytes -= msg.len();
        }
    }
}

struct Lobby {
    name: String,
    max_clients: usize,
    max_rooms: usize,
    game_settings: (u32, std::time::Duration),
    /// In order of arrival.
    players: BTreeMap<ConnId, Player>,
    rooms: BTreeMap<RoomId, Room>,
    next_room: u32,
    /// The last lobby chat lines, oldest first.
    chat: VecDeque<ChatLine>,
    maps: MapCache,
    player_count: Arc<AtomicU32>,
    /// For games to report back; weak so that the lobby stops when the
    /// server does.
    me: mpsc::WeakUnboundedSender<LobbyInput>,
}

type Answer = Result<(), Refusal>;

impl Lobby {
    async fn run(mut self, mut inputs: mpsc::UnboundedReceiver<LobbyInput>) {
        while let Some(input) = inputs.recv().await {
            match input {
                LobbyInput::Hello {
                    conn,
                    login,
                    content,
                    outbox,
                } => self.hello(conn, login, content, outbox),
                LobbyInput::Msg { conn, msg } => {
                    if let Some(player) = self.players.get(&conn) {
                        let outbox = player.outbox.clone();
                        if let Err(reason) = self.on_msg(conn, msg) {
                            outbox.send(&ServerMsg::Refused { reason });
                        }
                    }
                }
                LobbyInput::Gone { conn } => self.gone(conn),
                LobbyInput::LeftGame { room, conn } => self.left_game(room, conn),
                LobbyInput::GameOver { room } => self.game_over(room),
            }
        }
    }

    // -- Arrivals and departures -------------------------------------------

    fn hello(&mut self, conn: ConnId, wanted: Login, content: ContentHash, outbox: Outbox) {
        if self.players.len() >= self.max_clients {
            info!(login = %wanted, "rejected: server full");
            outbox.send(&ServerMsg::Rejected {
                reason: Rejection::ServerFull,
            });
            outbox.close();
            return;
        }
        let login = self.unique_login(&wanted);
        info!(%login, %content, "player arrived");
        let player = Player {
            login: login.clone(),
            content,
            outbox: outbox.clone(),
            place: IN_LOBBY,
        };
        let arrived = LobbyChange::PlayerArrived {
            player: lobby_player(&player),
        };
        self.broadcast(arrived);
        self.players.insert(conn, player);
        self.count_players();
        outbox.send(&ServerMsg::Welcome {
            lobby: LobbySnapshot {
                server_name: self.name.clone(),
                you: login,
                players: self.players.values().map(lobby_player).collect(),
                rooms: self.rooms.values().map(Room::summary).collect(),
                chat: self.chat.iter().cloned().collect(),
            },
        });
    }

    /// The login asked for, or with the lowest number from 2 added that
    /// makes it unique. Logins that differ only in case would sound the
    /// same, so they count as taken.
    fn unique_login(&self, wanted: &Login) -> Login {
        let taken = |l: &str| {
            self.players
                .values()
                .any(|p| p.login.as_str().eq_ignore_ascii_case(l))
        };
        if !taken(wanted.as_str()) {
            return wanted.clone();
        }
        (2u32..)
            .find_map(|n| {
                let suffix = n.to_string();
                let base =
                    &wanted.as_str()[..wanted.as_str().len().min(MAX_LOGIN_CHARS - suffix.len())];
                let login = format!("{base}{suffix}");
                (!taken(&login)).then(|| Login::new(login).expect("letters and digits"))
            })
            .expect("fewer players than numbers")
    }

    fn gone(&mut self, conn: ConnId) {
        let Some(player) = self.players.remove(&conn) else {
            return;
        };
        self.count_players();
        info!(login = %player.login, "player left");
        // Before the room or game hears of it, so that everyone hears of
        // the departure first.
        self.broadcast(LobbyChange::PlayerLeft {
            login: player.login,
        });
        match player.place {
            Place::Lobby { .. } => {}
            Place::InRoom(id) => self.take_out_of_room(id, conn),
            Place::Playing(id) => {
                let room = &self.rooms[&id];
                if let (Phase::Playing { game }, Some(slot)) = (&room.phase, room.slot_of(conn)) {
                    let _ = game.send(GameInput::Leave { slot });
                }
            }
        }
    }

    fn count_players(&self) {
        let n = u32::try_from(self.players.len()).unwrap_or(u32::MAX);
        self.player_count.store(n, Ordering::Relaxed);
    }

    // -- Messages ----------------------------------------------------------

    fn on_msg(&mut self, conn: ConnId, msg: ClientMsg) -> Answer {
        match msg {
            ClientMsg::Hello { .. } => Err(Refusal::NotNow),
            ClientMsg::Chat { channel, text } => self.chat(conn, channel, text),
            ClientMsg::CreateRoom { map, speed, open } => {
                self.create_room(conn, NewRoom { map, speed, open })
            }
            ClientMsg::Map { map } => self.map_uploaded(conn, map),
            ClientMsg::GetMap { hash } => {
                let msg = self.maps.get(&hash).ok_or(Refusal::WrongMap)?;
                self.players[&conn].outbox.send_text(msg);
                Ok(())
            }
            ClientMsg::JoinRoom { room } => self.join(conn, room),
            ClientMsg::LeaveRoom => {
                let id = self.gathering_room(conn)?;
                self.players[&conn].outbox.send(&ServerMsg::RoomExited {
                    reason: RoomExit::Left,
                });
                self.take_out_of_room(id, conn);
                self.move_player(conn, IN_LOBBY);
                Ok(())
            }
            ClientMsg::Invite { login } => self.invite(conn, login),
            ClientMsg::AddComputer { level } => self.add_computer(conn, level),
            ClientMsg::RemoveSlot { slot } => self.remove_slot(conn, slot),
            ClientMsg::SetAlliance { slot, alliance } => self.set_alliance(conn, slot, alliance),
            ClientMsg::Start => self.start(conn),
            ClientMsg::SetFaction { slot, faction } => self.set_faction(conn, slot, faction),
            ClientMsg::SetReady { ready } => self.set_ready(conn, ready),
            ClientMsg::Orders { orders } => {
                self.pass_to_game(conn, |slot| GameInput::Orders { slot, orders })
            }
            ClientMsg::TurnDone { turn, digest } => {
                self.pass_to_game(conn, |slot| GameInput::TurnDone { slot, turn, digest })
            }
            ClientMsg::LeaveGame => self.pass_to_game(conn, |slot| GameInput::Leave { slot }),
        }
    }

    // -- Chat --------------------------------------------------------------

    fn chat(&mut self, conn: ConnId, channel: Channel, text: ChatText) -> Answer {
        let player = &self.players[&conn];
        let line = ChatLine {
            channel,
            from: player.login.clone(),
            text,
        };
        let to: Vec<&Outbox> = match (channel, &player.place) {
            (Channel::Lobby, Place::Lobby { .. } | Place::InRoom(_)) => self
                .players
                .values()
                .filter(|p| matches!(p.place, Place::Lobby { .. } | Place::InRoom(_)))
                .map(|p| &p.outbox)
                .collect(),
            (Channel::Room, Place::InRoom(id)) => {
                self.rooms[id].members().map(|(_, m)| &m.outbox).collect()
            }
            (Channel::Game | Channel::Allies, Place::Playing(id)) => {
                let room = &self.rooms[id];
                let alliance =
                    room.slots[room.slot_of(conn).expect("a player is in their game")].alliance;
                room.members()
                    .filter(|(_, m)| {
                        matches!(self.players.get(&m.conn), Some(p) if matches!(p.place, Place::Playing(r) if r == *id))
                    })
                    .filter(|(i, _)| channel == Channel::Game || room.slots[*i].alliance == alliance)
                    .map(|(_, m)| &m.outbox)
                    .collect()
            }
            _ => return Err(Refusal::NotNow),
        };
        let text = encode(&ServerMsg::Chat { line: line.clone() });
        for outbox in to {
            outbox.send_text(text.clone());
        }
        if channel == Channel::Lobby {
            if self.chat.len() == CHAT_HISTORY_LINES {
                self.chat.pop_front();
            }
            self.chat.push_back(line);
        }
        Ok(())
    }

    // -- Rooms -------------------------------------------------------------

    fn create_room(&mut self, conn: ConnId, new: NewRoom) -> Answer {
        let player = &self.players[&conn];
        if !matches!(player.place, Place::Lobby { .. }) {
            return Err(Refusal::NotNow);
        }
        if new.map.nb_players_min == 0 || new.map.nb_players_min > new.map.nb_players_max {
            return Err(Refusal::Invalid);
        }
        if self.rooms.len() >= self.max_rooms {
            return Err(Refusal::TooManyRooms);
        }
        if self.maps.get(&new.map.hash).is_some() {
            self.open_room(conn, new);
        } else {
            let hash = new.map.hash.clone();
            let player = self.players.get_mut(&conn).expect("checked above");
            player.place = Place::Lobby {
                creating: Some(new),
            };
            player.outbox.send(&ServerMsg::SendMap { hash });
        }
        Ok(())
    }

    fn map_uploaded(&mut self, conn: ConnId, map: MapData) -> Answer {
        let player = self.players.get_mut(&conn).expect("a player");
        let Place::Lobby { creating } = &mut player.place else {
            return Err(Refusal::NotNow);
        };
        let new = creating.take().ok_or(Refusal::NotNow)?;
        let hash = map.hash();
        if hash != new.map.hash {
            return Err(Refusal::WrongMap);
        }
        if self.rooms.len() >= self.max_rooms {
            return Err(Refusal::TooManyRooms);
        }
        if !self.maps.insert(hash, encode(&ServerMsg::Map { map })) {
            warn!("map cache full");
            return Err(Refusal::TooManyRooms);
        }
        self.open_room(conn, new);
        Ok(())
    }

    fn open_room(&mut self, conn: ConnId, new: NewRoom) {
        let id = RoomId(self.next_room);
        self.next_room += 1;
        let player = &self.players[&conn];
        let host = Member {
            conn,
            login: player.login.clone(),
            outbox: player.outbox.clone(),
            ready: true,
        };
        let room = Room::new(
            id,
            host,
            new.map,
            new.speed,
            new.open,
            player.content.clone(),
        );
        info!(room = id.0, host = %player.login, map = %room.map.file_name, "room opened");
        player.outbox.send(&ServerMsg::Room {
            change: RoomChange::Entered,
            room: room.state(),
        });
        self.broadcast(LobbyChange::RoomOpened {
            room: room.summary(),
        });
        self.rooms.insert(id, room);
        self.move_player(conn, Place::InRoom(id));
    }

    fn join(&mut self, conn: ConnId, id: RoomId) -> Answer {
        let player = &self.players[&conn];
        if !matches!(player.place, Place::Lobby { .. }) {
            return Err(Refusal::NotNow);
        }
        let room = self.rooms.get_mut(&id).ok_or(Refusal::NoSuchRoom)?;
        let Phase::Gathering { invited } = &room.phase else {
            return Err(Refusal::NotNow);
        };
        if room.content != player.content {
            return Err(Refusal::Incompatible);
        }
        if !room.open && !invited.contains(&conn) {
            return Err(Refusal::NotNow);
        }
        if room.is_full() {
            return Err(Refusal::RoomFull);
        }
        room.add(Occupant::Human(Member {
            conn,
            login: player.login.clone(),
            outbox: player.outbox.clone(),
            ready: false,
        }));
        info!(room = id.0, login = %player.login, "joined room");
        player.outbox.send(&ServerMsg::Room {
            change: RoomChange::Entered,
            room: room.state(),
        });
        let joined = RoomChange::Joined {
            player: soundrts_protocol::SlotPlayer::Human(player.login.clone()),
        };
        notify_others(room, conn, joined);
        self.room_changed(id);
        self.move_player(conn, Place::InRoom(id));
        Ok(())
    }

    /// Takes a human out of a room whose game has not started, makes
    /// another human host if they were, and closes the room if no human is
    /// left. Leaves the player's place to the caller.
    fn take_out_of_room(&mut self, id: RoomId, conn: ConnId) {
        let Some(room) = self.rooms.get_mut(&id) else {
            return;
        };
        let Some(slot) = room.slot_of(conn) else {
            return;
        };
        let left = room.slots.remove(slot);
        if let Phase::Gathering { invited } = &mut room.phase {
            invited.remove(&conn);
        }
        info!(room = id.0, player = ?left.player(), "left room");
        let Some(next_host) = room.members().next().map(|(_, m)| m.conn) else {
            self.close_room(id);
            return;
        };
        let host_left = room.host == conn;
        if host_left {
            room.host = next_host;
        }
        room.notify(RoomChange::Left {
            player: left.player(),
        });
        if host_left {
            room.notify(RoomChange::Host {
                host: room.host_login(),
            });
        }
        self.room_changed(id);
    }

    fn close_room(&mut self, id: RoomId) {
        let Some(room) = self.rooms.remove(&id) else {
            return;
        };
        info!(room = id.0, "room closed");
        if !self.rooms.values().any(|r| r.map.hash == room.map.hash) {
            self.maps.remove(&room.map.hash);
        }
        self.broadcast(LobbyChange::RoomClosed { id });
    }

    fn invite(&mut self, conn: ConnId, login: Login) -> Answer {
        let id = self.hosted_room(conn)?;
        let (&target, guest) = self
            .players
            .iter()
            .find(|(_, p)| p.login == login)
            .ok_or(Refusal::NoSuchPlayer)?;
        let room = self.rooms.get_mut(&id).expect("hosted");
        if let Phase::Gathering { invited } = &mut room.phase {
            invited.insert(target);
        }
        guest.outbox.send(&ServerMsg::Invited {
            room: id,
            by: self.players[&conn].login.clone(),
        });
        Ok(())
    }

    fn add_computer(&mut self, conn: ConnId, level: AiLevel) -> Answer {
        let id = self.hosted_room(conn)?;
        let room = self.rooms.get_mut(&id).expect("hosted");
        if room.is_full() {
            return Err(Refusal::RoomFull);
        }
        room.add(Occupant::Computer(level));
        room.notify(RoomChange::Joined {
            player: soundrts_protocol::SlotPlayer::Computer(level),
        });
        self.room_changed(id);
        Ok(())
    }

    fn remove_slot(&mut self, conn: ConnId, slot: SlotIndex) -> Answer {
        let id = self.hosted_room(conn)?;
        let room = self.rooms.get_mut(&id).expect("hosted");
        let removed = match &room.slots.get(slot).ok_or(Refusal::NoSuchPlayer)?.occupant {
            Occupant::Human(m) if m.conn == conn => return Err(Refusal::NotNow),
            Occupant::Human(m) => Some(m.conn),
            Occupant::Computer(_) => None,
        };
        match removed {
            Some(target) => {
                self.players[&target].outbox.send(&ServerMsg::RoomExited {
                    reason: RoomExit::Removed,
                });
                self.take_out_of_room(id, target);
                self.move_player(target, IN_LOBBY);
            }
            None => {
                let left = room.slots.remove(slot);
                room.notify(RoomChange::Left {
                    player: left.player(),
                });
                self.room_changed(id);
            }
        }
        Ok(())
    }

    fn set_alliance(&mut self, conn: ConnId, slot: SlotIndex, alliance: u8) -> Answer {
        let id = self.hosted_room(conn)?;
        if !ALLIANCES.contains(&alliance) {
            return Err(Refusal::Invalid);
        }
        let room = self.rooms.get_mut(&id).expect("hosted");
        room.slots
            .get_mut(slot)
            .ok_or(Refusal::NoSuchPlayer)?
            .alliance = alliance;
        room.notify(RoomChange::Alliance { slot });
        Ok(())
    }

    fn set_faction(&mut self, conn: ConnId, slot: SlotIndex, faction: Faction) -> Answer {
        let id = self.gathering_room(conn)?;
        let room = self.rooms.get_mut(&id).expect("gathering");
        let is_host = room.host == conn;
        let target = room.slots.get_mut(slot).ok_or(Refusal::NoSuchPlayer)?;
        let allowed = match &target.occupant {
            Occupant::Human(m) => m.conn == conn,
            Occupant::Computer(_) => is_host,
        };
        if !allowed {
            return Err(Refusal::NotNow);
        }
        target.faction = faction;
        room.notify(RoomChange::Faction { slot });
        Ok(())
    }

    fn set_ready(&mut self, conn: ConnId, ready: bool) -> Answer {
        let id = self.gathering_room(conn)?;
        let room = self.rooms.get_mut(&id).expect("gathering");
        if room.host == conn {
            return Err(Refusal::NotNow);
        }
        let slot = room.slot_of(conn).expect("a member");
        let Occupant::Human(member) = &mut room.slots[slot].occupant else {
            unreachable!("slot_of finds humans");
        };
        member.ready = ready;
        room.notify(RoomChange::Ready { slot });
        Ok(())
    }

    fn start(&mut self, conn: ConnId) -> Answer {
        let id = self.hosted_room(conn)?;
        let room = self.rooms.get_mut(&id).expect("hosted");
        let count = room.slots.len();
        let all_ready = room.slots.iter().all(|s| room.is_ready(s));
        if !all_ready
            || count < usize::from(room.map.nb_players_min)
            || count > usize::from(room.map.nb_players_max)
        {
            return Err(Refusal::CannotStart);
        }
        let seed = i64::from(rand::random::<u32>());
        let slots = room.slots();
        let mut players = Vec::new();
        for (you, m) in room.members() {
            m.outbox.send(&ServerMsg::GameStarted {
                seed,
                slots: slots.clone(),
                you,
                map: room.map.clone(),
                speed: room.speed,
            });
            players.push(game::Player {
                slot: you,
                conn: m.conn,
                login: m.login.clone(),
                outbox: m.outbox.clone(),
            });
        }
        let Some(me) = self.me.upgrade() else {
            return Ok(());
        };
        let (max_lead, lag_timeout) = self.game_settings;
        let settings = game::Settings {
            turn_interval: room.speed.turn_interval(),
            max_lead,
            lag_timeout,
        };
        info!(room = id.0, seed, players = count, "starting game");
        room.phase = Phase::Playing {
            game: game::spawn(id, players, settings, me),
        };
        let humans: Vec<ConnId> = room.members().map(|(_, m)| m.conn).collect();
        self.room_changed(id);
        for human in humans {
            self.move_player(human, Place::Playing(id));
        }
        Ok(())
    }

    // -- Games -------------------------------------------------------------

    fn pass_to_game(&mut self, conn: ConnId, input: impl FnOnce(SlotIndex) -> GameInput) -> Answer {
        let Place::Playing(id) = self.players[&conn].place else {
            return Err(Refusal::NotNow);
        };
        let room = &self.rooms[&id];
        let Phase::Playing { game } = &room.phase else {
            unreachable!("a player plays in a started room");
        };
        let slot = room.slot_of(conn).expect("a player is in their game");
        let _ = game.send(input(slot));
        Ok(())
    }

    fn left_game(&mut self, id: RoomId, conn: ConnId) {
        let Some(player) = self.players.get(&conn) else {
            return;
        };
        if !matches!(player.place, Place::Playing(r) if r == id) {
            return;
        }
        player.outbox.send(&ServerMsg::GameEnded);
        self.move_player(conn, IN_LOBBY);
    }

    fn game_over(&mut self, id: RoomId) {
        let stragglers: Vec<ConnId> = self
            .players
            .iter()
            .filter(|(_, p)| matches!(p.place, Place::Playing(r) if r == id))
            .map(|(&c, _)| c)
            .collect();
        for conn in stragglers {
            self.left_game(id, conn);
        }
        self.close_room(id);
    }

    // -- Helpers -----------------------------------------------------------

    /// The room this player is in, before its game starts.
    fn gathering_room(&self, conn: ConnId) -> Result<RoomId, Refusal> {
        match self.players[&conn].place {
            Place::InRoom(id) => Ok(id),
            _ => Err(Refusal::NotNow),
        }
    }

    /// The room this player hosts, before its game starts.
    fn hosted_room(&self, conn: ConnId) -> Result<RoomId, Refusal> {
        let id = self.gathering_room(conn)?;
        if self.rooms[&id].host == conn {
            Ok(id)
        } else {
            Err(Refusal::NotNow)
        }
    }

    fn move_player(&mut self, conn: ConnId, place: Place) {
        let player = self.players.get_mut(&conn).expect("a player");
        player.place = place;
        let moved = LobbyChange::PlayerMoved {
            player: lobby_player(player),
        };
        self.broadcast(moved);
    }

    fn room_changed(&self, id: RoomId) {
        self.broadcast(LobbyChange::RoomChanged {
            room: self.rooms[&id].summary(),
        });
    }

    /// Tells every player on the server.
    fn broadcast(&self, change: LobbyChange) {
        let text = encode(&ServerMsg::Lobby { change });
        for player in self.players.values() {
            player.outbox.send_text(text.clone());
        }
    }
}

fn lobby_player(player: &Player) -> LobbyPlayer {
    LobbyPlayer {
        login: player.login.clone(),
        whereabouts: player.place.whereabouts(),
    }
}

fn notify_others(room: &Room, conn: ConnId, change: RoomChange) {
    let text = encode(&ServerMsg::Room {
        change,
        room: room.state(),
    });
    for (_, m) in room.members() {
        if m.conn != conn {
            m.outbox.send_text(text.clone());
        }
    }
}
