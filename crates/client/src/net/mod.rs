//! Playing on a server: the connection, what the client knows of the
//! lobby and its room, the chat, and the frames of a game the server runs.
//!
//! Server messages are read by one hook, [`pump`], that every waiting loop
//! of the client calls: menus, text entry and the game loop. It updates
//! the models, says what changed, and counts: [`Net::version`] changes
//! when the multiplayer menus must be built again, and [`Net::interrupts`]
//! goes up when the game starts, the connection is lost, or the host
//! removes the player; a menu or prompt opened before it went up gives up,
//! as Escape would.

pub mod chat;
pub mod conn;
pub mod discovery;
pub mod lobby;
pub mod servers;
pub mod words;

use std::collections::VecDeque;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use soundrts_protocol::game::{Role, SeatFrame};
use soundrts_protocol::{
    Channel, ChatText, ClientMsg, ContentHash, Invalid, LobbySnapshot, Login, MapData, MapHash,
    MapInfo, Refusal, RoomExit, RoomId, SeatToken, ServerMsg, Speed,
};
use soundrts_world::msg::{Cue, Msg, MsgPart, Phrase, cue, msg, period};

use self::chat::{ChatLog, line_msg, line_news};
use self::conn::{Connection, NetEvent, Opening};
use self::lobby::{LobbyModel, MapState, RoomModel, TooMany, check_room, check_watchers};
use crate::client::Client;
use crate::mapfile::{Map, name_from_path, title_of};
use crate::menu::{Exit, input_string};
use crate::message::{Category, News, Subject};
use crate::package::{NETWORK_MAP_LIMITS, check_expansion};

/// How the multiplayer menus reach the network. The defaults are those of
/// a player; tests change them.
#[derive(Clone, Debug)]
pub struct NetConfig {
    /// The address a hosted server listens on.
    pub host_ip: IpAddr,
    /// The port of a hosted server: 0 picks a free one.
    pub host_port: u16,
    /// Ask the router to forward the port of a hosted server.
    pub upnp: bool,
    /// Where to ask for servers on the local network.
    pub discovery_targets: Vec<SocketAddr>,
    /// Where the public list is; `None` to skip it.
    pub public_list_url: Option<String>,
    /// How a connection notices a server that stopped answering.
    pub liveness: conn::Liveness,
    /// What an encrypted connection trusts.
    pub trust: conn::Trust,
}

impl Default for NetConfig {
    fn default() -> NetConfig {
        NetConfig {
            host_ip: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            host_port: soundrts_protocol::DEFAULT_PORT,
            upnp: true,
            discovery_targets: vec![discovery::broadcast_target()],
            public_list_url: Some(servers::PUBLIC_LIST_URL.to_owned()),
            liveness: conn::Liveness::default(),
            trust: conn::Trust::built_in(),
        }
    }
}

/// An invitation to a room.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invitation {
    pub room: RoomId,
    pub by: Login,
}

/// The seat a server gave the player in a game it runs, to play or to
/// watch: the game starts for the player.
#[derive(Clone, Debug)]
pub struct Seated {
    /// The player's slot in the room, or that the player watches.
    pub you: Role,
    pub map: MapInfo,
    pub speed: Speed,
    pub paused: bool,
    /// How long the server holds the seat once it notices the connection
    /// is lost.
    pub hold: Duration,
    /// What takes the seat back after a lost connection: a secret, never
    /// said, logged or written.
    pub token: SeatToken,
}

/// The most frames of a game received and not yet taken by the game. The
/// game takes every frame waiting at each pass of its loop, and a server
/// sends one a turn: a server that sends more is not playing a game.
pub const MAX_QUEUED_FRAMES: usize = 1024;

/// A frame of the game's stream as it came: decoded, or why it could not
/// be.
pub type Incoming = Result<Box<SeatFrame>, Invalid>;

/// What the server sends a game being played or watched.
pub struct GameLink {
    /// The player's slot, or that the player watches.
    pub you: Role,
    /// The frames received and not taken by the game yet, in order. At
    /// most [`MAX_QUEUED_FRAMES`].
    pub frames: VecDeque<Incoming>,
    /// The game is over for this player: the server said so, and they are
    /// back in the lobby.
    pub ended: bool,
}

impl GameLink {
    /// The link of a game where the player has the seat `you`.
    pub fn new(you: Role) -> GameLink {
        GameLink {
            you,
            frames: VecDeque::new(),
            ended: false,
        }
    }
}

/// Opens a connection to the same server again, as the player first
/// opened it, without waiting.
pub type Redial = Box<dyn FnMut() -> Opening>;

/// The connection to a server, and what the client knows of it.
pub struct Net {
    conn: Connection,
    redial: Redial,
    pub lobby: LobbyModel,
    pub room: Option<RoomModel>,
    pub chat: ChatLog,
    pub invitations: Vec<Invitation>,
    /// A game the server started for the player, waiting to be played.
    pub started: Option<Seated>,
    /// While a game runs.
    pub game: Option<GameLink>,
    /// Changes whenever the multiplayer menus must be built again.
    pub version: u64,
    /// How many events have made the open prompts and static menus give
    /// up. Each remembers the count when it opens.
    pub interrupts: u64,
    /// Why the connection ended, once it has.
    pub closed: Option<String>,
    pub content: ContentHash,
    /// The map of the room being created, for the server to ask for.
    pub creating: Option<Arc<Map>>,
}

impl Net {
    /// The state after the server's welcome. The chat history goes into
    /// the log without being said.
    pub fn new(
        conn: Connection,
        snapshot: LobbySnapshot,
        content: ContentHash,
        redial: Redial,
    ) -> Net {
        let mut chat = ChatLog::default();
        for line in &snapshot.chat {
            chat.push(line.clone());
        }
        Net {
            conn,
            redial,
            lobby: LobbyModel::new(&snapshot),
            room: None,
            chat,
            invitations: Vec::new(),
            started: None,
            game: None,
            version: 0,
            interrupts: 0,
            closed: None,
            content,
            creating: None,
        }
    }

    pub fn send(&mut self, msg: ClientMsg) {
        self.conn.send(msg);
    }

    /// Starts opening a new connection to the server.
    pub fn redial(&mut self) -> Opening {
        (self.redial)()
    }

    /// The player is back in the game they played or watched, in the seat
    /// `you`, on `conn`, a new connection, with the lobby as it now is:
    /// what comes next on it goes to the game.
    pub fn returned(&mut self, conn: Connection, lobby: LobbyModel, you: Role) {
        self.conn = conn;
        self.lobby = lobby;
        self.closed = None;
        self.game = Some(GameLink::new(you));
        self.changed();
    }

    fn changed(&mut self) {
        self.version += 1;
    }

    fn handle(&mut self, cx: &mut Client, event: NetEvent) {
        match event {
            NetEvent::Msg(m) => self.handle_msg(cx, m),
            NetEvent::Frame(f) => self.handle_frame(cx, f),
            NetEvent::Closed(reason) => {
                cx.log.info(format!("connection closed: {reason}"));
                if self.closed.is_none() {
                    self.closed = Some(reason);
                }
                self.interrupts += 1;
                self.changed();
            }
        }
    }

    /// Closes the connection to a server that sent more than a client
    /// keeps, and says so.
    fn give_up(&mut self, cx: &mut Client, too_many: TooMany) {
        cx.log
            .warning(format!("the server sent {too_many}; disconnecting"));
        cx.voice.alert(&msg(Phrase::UnexpectedReply));
        self.conn.close();
        self.handle(cx, NetEvent::Closed(too_many.to_string()));
    }

    fn handle_msg(&mut self, cx: &mut Client, m: ServerMsg) {
        if let Err(too_many) = self.check(&m) {
            self.give_up(cx, too_many);
            return;
        }
        match m {
            ServerMsg::Lobby { change } => {
                if let soundrts_protocol::LobbyChange::RoomClosed { id } = &change {
                    self.invitations.retain(|i| i.room != *id);
                }
                let said = match self.lobby.apply(change, cx.config.announce_arrivals) {
                    Ok(said) => said,
                    Err(too_many) => return self.give_up(cx, too_many),
                };
                if let Some(said) = said
                    && self.game.is_none()
                {
                    cx.news(News::new(Category::System, said));
                }
                self.changed();
            }
            ServerMsg::Chat { line } => {
                cx.news(line_news(&line));
                self.chat.push(line);
            }
            ServerMsg::Invited { room, by } => {
                // A server tells of a room before inviting to it.
                let Some(title) = self.lobby.room(room).map(|r| title_of(&r.map)) else {
                    cx.log
                        .warning(format!("invited to an unknown room: {}", room.0));
                    return;
                };
                let m = [
                    vec![MsgPart::literal(by.as_str())],
                    msg(Phrase::InvitesYou),
                    title,
                ]
                .concat();
                cx.news(News::new(Category::System, m));
                self.invitations.retain(|i| i.room != room);
                self.invitations.push(Invitation { room, by });
                self.changed();
            }
            ServerMsg::Room { change, room } => {
                if change == soundrts_protocol::RoomChange::Entered {
                    self.invitations.retain(|i| i.room != room.id);
                    let local = match self.creating.take() {
                        Some(m) if m.hash().as_ref() == Some(&room.map.hash) => Some(m),
                        _ => cx.res.find_multiplayer_map(&room.map.hash),
                    };
                    let map = match local {
                        Some(m) => MapState::Ready(m),
                        None => {
                            self.send(ClientMsg::GetMap {
                                hash: room.map.hash.clone(),
                            });
                            cx.news(News::new(Category::System, msg(Phrase::Downloading)));
                            MapState::Downloading
                        }
                    };
                    self.room = Some(RoomModel { state: room, map });
                } else if let Some(r) = &mut self.room
                    && r.state.id == room.id
                {
                    r.state = room;
                    if let Some((said, topic)) = r.announce(&change, cx.res.style()) {
                        cx.news(News::new(Category::System, said).about(Subject::Room(topic)));
                    }
                }
                self.changed();
            }
            ServerMsg::RoomExited { reason } => {
                self.room = None;
                let said = match reason {
                    RoomExit::Left => None,
                    RoomExit::Removed => Some(Phrase::YouHaveBeenRemoved),
                    RoomExit::Closed => Some(Phrase::RoomClosed),
                };
                if let Some(said) = said {
                    cx.news(News::new(Category::System, msg(said)));
                    self.interrupts += 1;
                }
                self.changed();
            }
            ServerMsg::SendMap { hash } => match self.creating.as_ref().and_then(|m| m.data()) {
                Some(data) if data.hash() == hash => self.send(ClientMsg::Map { map: data }),
                _ => cx
                    .log
                    .warning(format!("the server asked for an unknown map: {hash}")),
            },
            ServerMsg::Map { map } => {
                self.map_received(cx, map);
                self.changed();
            }
            ServerMsg::GameEnded => {
                if let Some(g) = &mut self.game {
                    g.ended = true;
                }
                self.changed();
            }
            ServerMsg::Refused { reason } => {
                cx.log.info(format!("refused by the server: {reason:?}"));
                // The answer to the map asked for when entering the room.
                if reason == Refusal::WrongMap
                    && let Some(r) = &mut self.room
                    && matches!(r.map, MapState::Downloading)
                {
                    r.map = MapState::Refused;
                    self.changed();
                }
                let m = match reason {
                    Refusal::TooManyRooms => msg(Phrase::TooManyGames),
                    Refusal::CannotStart => {
                        let mut m = cue(Cue::Beep);
                        if let Some(r) = &self.room {
                            for name in r.not_ready() {
                                m.extend(name);
                                m.extend(msg(Phrase::IsNotReady));
                                m.extend(period());
                            }
                        }
                        m
                    }
                    Refusal::NoWatchers => {
                        [cue(Cue::Beep), msg(Phrase::GameTakesNoWatchers)].concat()
                    }
                    Refusal::WatchersFull => [cue(Cue::Beep), msg(Phrase::WatchersFull)].concat(),
                    Refusal::SlowDown => [cue(Cue::Beep), msg(Phrase::TooFast)].concat(),
                    _ => cue(Cue::Beep),
                };
                cx.news(News::new(Category::System, m));
            }
            ServerMsg::Welcome { .. } | ServerMsg::Rejected { .. } => {
                cx.log.warning("unexpected welcome or rejection");
            }
        }
    }

    /// Whether a message keeps what the client holds within its bounds;
    /// lobby changes are checked as they are applied.
    fn check(&self, m: &ServerMsg) -> Result<(), TooMany> {
        match m {
            ServerMsg::Room { room, .. } => {
                check_room(&room.map, room.slots.len())?;
                check_watchers(room.watchers.len())
            }
            _ => Ok(()),
        }
    }

    /// A frame of a game's stream. The seat starts the game; the start of
    /// a game that did not start is said; any other frame waits for the
    /// game, which checks it.
    fn handle_frame(&mut self, cx: &mut Client, frame: Incoming) {
        let frame = match frame {
            Ok(f) => f,
            Err(e) => {
                cx.log
                    .warning(format!("a frame of the game cannot be read: {e}"));
                return self.for_the_game(cx, Err(e));
            }
        };
        match *frame {
            SeatFrame::Seated {
                you,
                map,
                speed,
                paused,
                hold,
                token,
            } if self.game.is_none() => {
                if let Err(too_many) = check_room(&map, 0) {
                    return self.give_up(cx, too_many);
                }
                self.started = Some(Seated {
                    you,
                    map,
                    speed,
                    paused,
                    hold,
                    token,
                });
                self.game = Some(GameLink::new(you));
                self.interrupts += 1;
                self.changed();
            }
            SeatFrame::NotStarted(refusal) if self.game.is_none() => {
                cx.log
                    .warning(format!("the game did not start: {refusal:?}"));
                cx.news(News::new(Category::System, words::not_started(&refusal)));
            }
            other => self.for_the_game(cx, Ok(Box::new(other))),
        }
    }

    /// Keeps a frame for the game being played.
    fn for_the_game(&mut self, cx: &mut Client, frame: Incoming) {
        let Some(game) = &mut self.game else {
            cx.log.warning("a frame of a game while no game is played");
            return;
        };
        if game.frames.len() >= MAX_QUEUED_FRAMES {
            return self.give_up(cx, TooMany("frames ahead of the game"));
        }
        game.frames.push_back(frame);
    }

    /// A map asked for: checked, saved with the downloaded maps as the
    /// original did (`<name>_<hash>.<ext>`), and given to the room.
    fn map_received(&mut self, cx: &mut Client, data: MapData) {
        let Some(room) = &mut self.room else {
            return;
        };
        let hash = data.hash();
        if !matches!(room.map, MapState::Downloading) || hash != room.state.map.hash {
            cx.log.warning("received a map that was not asked for");
            return;
        }
        let file_name = data.file_name.as_str().to_owned();
        let bytes = data.bytes.into_vec();
        if !file_name.ends_with(".txt")
            && let Err(e) = check_expansion(&bytes, NETWORK_MAP_LIMITS)
        {
            cx.log.warning(format!("refused the map received: {e}"));
            let m = [cue(Cue::Beep), msg(Phrase::MapRefused)].concat();
            cx.news(News::new(Category::System, m));
            room.map = MapState::Refused;
            return;
        }
        let map = match Map::from_server(bytes.clone(), &file_name) {
            Ok(m) => Arc::new(m),
            Err(e) => {
                cx.log.warning(format!("cannot read the map received: {e}"));
                let m = [cue(Cue::Beep), msg(Phrase::GameCannotStart)].concat();
                cx.news(News::new(Category::System, m));
                room.map = MapState::Refused;
                return;
            }
        };
        if cx.res.find_multiplayer_map(&hash).is_none() {
            save_downloaded_map(cx, &file_name, &hash, &bytes);
        }
        room.map = MapState::Ready(map);
    }
}

fn save_downloaded_map(cx: &mut Client, file_name: &str, hash: &MapHash, bytes: &[u8]) {
    let ext = if file_name.ends_with(".txt") {
        ".txt"
    } else {
        ".zip"
    };
    let name = format!("{}_{}{ext}", name_from_path(file_name), &hash.as_str()[..8]);
    let path = cx.paths.downloaded().join("multi").join(&name);
    match std::fs::write(&path, bytes) {
        Ok(()) => cx.res.forget_multiplayer_maps(),
        Err(e) => cx
            .log
            .warning(format!("couldn't write {}: {e}", path.display())),
    }
}

/// The most server messages one call to [`pump`] handles: the rest wait
/// for the next call, so that the player's keys are still read.
pub const PUMP_LIMIT: usize = 64;

/// Reads what the server sent and acts on it, at most [`PUMP_LIMIT`]
/// messages. Every waiting loop calls it.
pub fn pump(cx: &mut Client) {
    let Some(mut net) = cx.net.take() else {
        return;
    };
    for _ in 0..PUMP_LIMIT {
        let Some(event) = net.conn.poll() else {
            break;
        };
        net.handle(cx, event);
    }
    cx.net = Some(net);
}

/// Sends a message to the server, if connected.
pub fn send(cx: &mut Client, msg: ClientMsg) {
    if let Some(net) = &mut cx.net {
        net.send(msg);
    }
}

/// The count of [`Net::interrupts`], 0 off a server: a prompt or static
/// menu opened when it was `n` gives up once it is no longer `n`.
pub fn interrupts(cx: &Client) -> u64 {
    cx.net.as_ref().map_or(0, |n| n.interrupts)
}

/// The characters of a chat line: any but control characters.
pub fn chat_char(c: char) -> bool {
    !c.is_control()
}

/// Asks for a chat line and sends it: to everyone in the game in a game
/// (from the game menu), to the room in a room, else to the lobby. A beep
/// without a server.
pub fn chat_in_menus(cx: &mut Client) -> Result<(), Exit> {
    let Some(net) = &cx.net else {
        cx.voice.item(&cue(Cue::Beep));
        return Ok(());
    };
    let channel = if net.game.is_some() {
        Channel::Game
    } else if net.room.is_some() {
        Channel::Room
    } else {
        Channel::Lobby
    };
    if let Some(line) = input_string(cx, &msg(Phrase::EnterMessage), chat_char, "", false)? {
        send_chat(cx, channel, &line);
    }
    Ok(())
}

/// Sends a chat line, if anything is left of it once cleaned.
pub fn send_chat(cx: &mut Client, channel: Channel, line: &str) {
    if let Some(text) = ChatText::clean(line) {
        send(cx, ClientMsg::Chat { channel, text });
    }
}

fn say_chat_line(cx: &mut Client, line: Option<Msg>) {
    match line {
        Some(m) => cx.voice.item(&m),
        None => cx.voice.item(&cue(Cue::Beep)),
    }
}

/// Says the chat line before the one read last.
pub fn chat_previous(cx: &mut Client) {
    let line = cx
        .net
        .as_mut()
        .and_then(|n| n.chat.previous().map(line_msg));
    say_chat_line(cx, line);
}

/// Says the chat line after the one read last.
pub fn chat_next(cx: &mut Client) {
    let line = cx
        .net
        .as_mut()
        .and_then(|n| n.chat.following().map(line_msg));
    say_chat_line(cx, line);
}

/// The game link, while a game runs.
pub fn game_link(cx: &mut Client) -> Option<&mut GameLink> {
    cx.net.as_mut().and_then(|n| n.game.as_mut())
}

/// Sends an encoded command of the player, if connected.
pub fn send_command(cx: &mut Client, bytes: Vec<u8>) {
    if let Some(net) = &mut cx.net {
        net.conn.send_command(bytes);
    }
}
