//! Playing on a server: the connection, what the client knows of the
//! lobby and its room, the chat, and the turns of a game.
//!
//! Server messages are read by one hook, [`pump`], that every waiting loop
//! of the client calls: menus, text entry and the game loop. It updates
//! the models, says what changed, and sets flags: [`Net::version`] changes
//! when the multiplayer menus must be built again, and [`Net::interrupt`]
//! makes the menu or prompt that is open give up, as Escape would, when
//! the game starts, the connection is lost, or the host removes the player.

pub mod chat;
pub mod conn;
pub mod discovery;
pub mod lobby;
pub mod servers;
pub mod words;

use std::collections::VecDeque;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use soundrts_protocol::{
    Channel, ChatText, ClientMsg, ContentHash, LobbySnapshot, Login, MAX_ORDER_BYTES,
    MAX_ORDERS_PER_MESSAGE, MAX_TURN_ORDER_BYTES, MapData, MapHash, MapInfo, Order, Orders,
    Refusal, RoomExit, RoomId, ServerMsg, Slot, SlotIndex, SlotOrders, Speed, Turn,
};
use soundrts_world::msg::{Msg, msg};
use soundrts_world::msgparts as mp;

use self::chat::{CHAT_SOUND, ChatLog, line_msg};
use self::conn::{Connection, NetEvent};
use self::lobby::{LobbyModel, MapState, RoomModel};
use self::words::text;
use crate::client::Client;
use crate::mapfile::{Map, name_from_path, title_of};
use crate::menu::{Exit, input_string};
use crate::message::InfoOptions;
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
}

impl Default for NetConfig {
    fn default() -> NetConfig {
        NetConfig {
            host_ip: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            host_port: soundrts_protocol::DEFAULT_PORT,
            upnp: true,
            discovery_targets: vec![discovery::broadcast_target()],
            public_list_url: Some(servers::PUBLIC_LIST_URL.to_owned()),
        }
    }
}

/// An invitation to a room.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invitation {
    pub room: RoomId,
    pub by: Login,
}

/// A game the server started, waiting to be played.
#[derive(Clone, Debug)]
pub struct GameStart {
    pub seed: i64,
    pub slots: Vec<Slot>,
    pub you: SlotIndex,
    pub map: MapInfo,
    pub speed: Speed,
}

/// What the server says to a game being played.
#[derive(Default)]
pub struct GameLink {
    /// The turns received and not run yet, in order.
    pub turns: VecDeque<(Turn, Vec<SlotOrders>)>,
    /// The game is over for this player (the server said so).
    pub ended: bool,
    /// Bytes of orders sent since the last turn received.
    sent_since_turn: usize,
    out_of_sync_said: bool,
}

/// The connection to a server, and what the client knows of it.
pub struct Net {
    conn: Connection,
    pub lobby: LobbyModel,
    pub room: Option<RoomModel>,
    pub chat: ChatLog,
    pub invitations: Vec<Invitation>,
    pub started: Option<GameStart>,
    /// While a game runs.
    pub game: Option<GameLink>,
    /// Changes whenever the multiplayer menus must be built again.
    pub version: u64,
    /// The open prompt or static menu must give up.
    pub interrupt: bool,
    /// Why the connection ended, once it has.
    pub closed: Option<String>,
    pub content: ContentHash,
    /// The map of the room being created, for the server to ask for.
    pub creating: Option<Arc<Map>>,
}

impl Net {
    /// The state after the server's welcome. The chat history goes into
    /// the log without being said.
    pub fn new(conn: Connection, snapshot: LobbySnapshot, content: ContentHash) -> Net {
        let mut chat = ChatLog::default();
        for line in &snapshot.chat {
            chat.push(line.clone());
        }
        Net {
            conn,
            lobby: LobbyModel::new(&snapshot),
            room: None,
            chat,
            invitations: Vec::new(),
            started: None,
            game: None,
            version: 0,
            interrupt: false,
            closed: None,
            content,
            creating: None,
        }
    }

    pub fn send(&mut self, msg: ClientMsg) {
        self.conn.send(msg);
    }

    fn changed(&mut self) {
        self.version += 1;
    }

    fn handle(&mut self, cx: &mut Client, event: NetEvent) {
        match event {
            NetEvent::Msg(m) => self.handle_msg(cx, m),
            NetEvent::Closed(reason) => {
                cx.log.info(format!("connection closed: {reason}"));
                if self.closed.is_none() {
                    self.closed = Some(reason);
                }
                self.interrupt = true;
                self.changed();
            }
        }
    }

    fn handle_msg(&mut self, cx: &mut Client, m: ServerMsg) {
        match m {
            ServerMsg::Lobby { change } => {
                if let soundrts_protocol::LobbyChange::RoomClosed { id } = &change {
                    self.invitations.retain(|i| i.room != *id);
                }
                let said = self.lobby.apply(change, cx.config.announce_arrivals != 0);
                if let Some(said) = said
                    && self.game.is_none()
                {
                    cx.voice.info(&said);
                }
                self.changed();
            }
            ServerMsg::Chat { line } => {
                let m = [msg(&[CHAT_SOUND]), line_msg(&line)].concat();
                self.chat.push(line);
                cx.voice.chat(&m);
            }
            ServerMsg::Invited { room, by } => {
                let title = self
                    .lobby
                    .room(room)
                    .map(|r| title_of(&r.map))
                    .unwrap_or_default();
                cx.voice
                    .chat(&[vec![by.as_str().into()], msg(mp::INVITES_YOU), title].concat());
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
                            cx.voice.info(&msg(words::DOWNLOADING));
                            MapState::Downloading
                        }
                    };
                    self.room = Some(RoomModel { state: room, map });
                } else if let Some(r) = &mut self.room
                    && r.state.id == room.id
                {
                    r.state = room;
                    if let Some((said, kind)) = r.announce(&change, cx.res.style()) {
                        cx.voice.info_with(
                            &said,
                            InfoOptions {
                                update_type: Some(kind),
                                ..InfoOptions::default()
                            },
                        );
                    }
                }
                self.changed();
            }
            ServerMsg::RoomExited { reason } => {
                self.room = None;
                if reason == RoomExit::Removed {
                    cx.voice.chat(&text(words::YOU_HAVE_BEEN_REMOVED));
                    self.interrupt = true;
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
            ServerMsg::GameStarted {
                seed,
                slots,
                you,
                map,
                speed,
            } => {
                self.started = Some(GameStart {
                    seed,
                    slots,
                    you,
                    map,
                    speed,
                });
                self.interrupt = true;
                self.changed();
            }
            ServerMsg::Turn { turn, orders } => {
                if let Some(g) = &mut self.game {
                    g.turns.push_back((turn, orders));
                    g.sent_since_turn = 0;
                }
            }
            ServerMsg::Waiting { players } => {
                if self.game.is_some() {
                    let mut m = text(words::WAITING_FOR);
                    for (i, p) in players.iter().enumerate() {
                        if i > 0 {
                            m.extend(msg(mp::COMMA));
                        }
                        m.push(p.as_str().into());
                    }
                    cx.voice.chat(&m);
                }
            }
            ServerMsg::Resumed => {
                if self.game.is_some() {
                    cx.voice.info(&text(words::GAME_RESUMES));
                }
            }
            ServerMsg::PlayerLeftGame { slot } => {
                // The world says it when the player's `quit` order runs.
                cx.log.info(format!("slot {slot} left the game"));
            }
            ServerMsg::OutOfSync { turn, slots } => {
                if let Some(g) = &mut self.game
                    && !g.out_of_sync_said
                {
                    g.out_of_sync_said = true;
                    cx.log
                        .warning(format!("out of sync after turn {turn}: slots {slots:?}"));
                    cx.voice.chat(&text(words::OUT_OF_SYNC));
                }
            }
            ServerMsg::GameEnded => {
                if let Some(g) = &mut self.game {
                    g.ended = true;
                }
                self.changed();
            }
            ServerMsg::Refused { reason } => {
                cx.log.info(format!("refused by the server: {reason:?}"));
                let m = match reason {
                    Refusal::TooManyRooms => msg(mp::TOO_MANY_GAMES),
                    Refusal::CannotStart => {
                        let mut m = msg(mp::BEEP);
                        if let Some(r) = &self.room {
                            for name in r.not_ready() {
                                m.extend(name);
                                m.extend(text(words::NOT_READY));
                                m.extend(msg(mp::PERIOD));
                            }
                        }
                        m
                    }
                    _ => msg(mp::BEEP),
                };
                cx.voice.info(&m);
            }
            ServerMsg::Welcome { .. } | ServerMsg::Rejected { .. } => {
                cx.log.warning("unexpected welcome or rejection");
            }
        }
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
            cx.voice
                .chat(&[msg(mp::BEEP), text(words::MAP_REFUSED)].concat());
            return;
        }
        let map = match Map::from_bytes(bytes.clone(), &file_name) {
            Ok(m) => Arc::new(m),
            Err(e) => {
                cx.log.warning(format!("cannot read the map received: {e}"));
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

/// Reads what the server sent and acts on it. Every waiting loop calls it.
pub fn pump(cx: &mut Client) {
    let Some(mut net) = cx.net.take() else {
        return;
    };
    while let Some(event) = net.conn.poll() {
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

/// Whether the open prompt or static menu must give up.
pub fn interrupted(cx: &Client) -> bool {
    cx.net.as_ref().is_some_and(|n| n.interrupt)
}

/// The characters of a chat line: any but control characters.
pub fn chat_char(c: char) -> bool {
    !c.is_control()
}

/// Asks for a chat line and sends it: to the game's players in a game
/// (from the game menu), to the room in a room, else to the lobby. A beep
/// without a server.
pub fn chat_in_menus(cx: &mut Client) -> Result<(), Exit> {
    let Some(net) = &cx.net else {
        cx.voice.item(&msg(mp::BEEP));
        return Ok(());
    };
    let channel = if net.game.is_some() {
        Channel::Game
    } else if net.room.is_some() {
        Channel::Room
    } else {
        Channel::Lobby
    };
    if let Some(line) = input_string(cx, &msg(mp::ENTER_MESSAGE), chat_char, "", false)? {
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
        None => cx.voice.item(&msg(mp::BEEP)),
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

/// Sends the commands written since the last call as orders, as many as
/// fit in what the server takes for one turn; the others wait for the
/// next turn. A command too long to be an order is dropped.
pub fn send_orders(cx: &mut Client, pending: &mut Vec<String>) {
    let Some(net) = &mut cx.net else {
        return;
    };
    let Some(game) = &mut net.game else {
        return;
    };
    // Orders sent just before a turn arrived may still wait at the server
    // for the next one: half of what it takes leaves room for them.
    let budget = MAX_TURN_ORDER_BYTES / 2;
    let mut batch = Vec::new();
    let mut taken = 0;
    for command in pending.iter() {
        let len = command.len();
        if len > MAX_ORDER_BYTES {
            cx.log
                .warning(format!("command too long to send: {len} bytes"));
            taken += 1;
            continue;
        }
        if game.sent_since_turn + len > budget {
            break;
        }
        match Order::new(command.clone()) {
            Ok(o) => {
                game.sent_since_turn += len;
                batch.push(o);
            }
            Err(_) => cx.log.warning(format!("not an order: {command:?}")),
        }
        taken += 1;
    }
    pending.drain(..taken);
    for chunk in batch.chunks(MAX_ORDERS_PER_MESSAGE) {
        let orders = Orders::new(chunk.to_vec()).expect("a chunk fits in a message");
        net.send(ClientMsg::Orders { orders });
    }
}
