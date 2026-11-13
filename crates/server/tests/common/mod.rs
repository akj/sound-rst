//! A real server on a free local port, and WebSocket clients that speak
//! the protocol to it.

#![allow(dead_code)]

use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use soundrts_protocol::*;
use soundrts_server::{ServerConfig, ServerHandle};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

/// Long enough for a slow machine; only a failing test waits this long.
pub const TIMEOUT: Duration = Duration::from_secs(10);

/// Starts a server on 127.0.0.1 and a free port, with `tweak` applied to
/// the defaults.
pub fn server(tweak: impl FnOnce(&mut ServerConfig)) -> ServerHandle {
    let mut config = ServerConfig {
        name: "test".into(),
        ip: Ipv4Addr::LOCALHOST.into(),
        port: 0,
        ..ServerConfig::default()
    };
    tweak(&mut config);
    soundrts_server::start(config).expect("server starts")
}

pub fn login(s: &str) -> Login {
    Login::new(s).unwrap()
}

pub fn content() -> ContentHash {
    ContentHash::new("content1").unwrap()
}

pub fn chat(s: &str) -> ChatText {
    ChatText::clean(s).unwrap()
}

/// A map for `min` to `max` players, and its file.
pub fn map(min: u8, max: u8) -> (MapInfo, MapData) {
    let bytes = format!("title 1000\nnb_players_min {min}\nnb_players_max {max}\n").into_bytes();
    let file_name = MapFileName::new("test.txt").unwrap();
    let info = MapInfo {
        file_name: file_name.clone(),
        hash: MapHash::of(&bytes),
        title_sounds: vec![1000],
        nb_players_min: min,
        nb_players_max: max,
    };
    let data = MapData {
        file_name,
        bytes: MapBytes::new(bytes).unwrap(),
    };
    (info, data)
}

pub fn lobby(change: LobbyChange) -> ServerMsg {
    ServerMsg::Lobby { change }
}

pub fn moved(login: &Login, whereabouts: Whereabouts) -> ServerMsg {
    lobby(LobbyChange::PlayerMoved {
        player: LobbyPlayer {
            login: login.clone(),
            whereabouts,
        },
    })
}

pub fn refused(reason: Refusal) -> ServerMsg {
    ServerMsg::Refused { reason }
}

pub fn human(login: &Login) -> SlotPlayer {
    SlotPlayer::Human(login.clone())
}

pub struct Client {
    ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
    pub login: Login,
    /// The turns received by the methods that skip them, in order.
    pub turns: Vec<(Turn, Vec<SlotOrders>)>,
    /// The last turn this client reported done.
    pub done: Option<Turn>,
}

impl Client {
    /// Connects without saying hello.
    pub async fn connect(addr: SocketAddr) -> Client {
        let (ws, _) = tokio::time::timeout(
            TIMEOUT,
            tokio_tungstenite::connect_async(format!("ws://{addr}")),
        )
        .await
        .expect("connects in time")
        .expect("connects");
        Client {
            ws,
            login: login("nobody"),
            turns: Vec::new(),
            done: None,
        }
    }

    /// Connects and says hello with `name`, and returns the lobby the
    /// server sent back. The client's login is the one the server gave.
    pub async fn hello(addr: SocketAddr, name: &str) -> (Client, LobbySnapshot) {
        Client::hello_with(addr, name, content()).await
    }

    pub async fn hello_with(
        addr: SocketAddr,
        name: &str,
        content: ContentHash,
    ) -> (Client, LobbySnapshot) {
        let mut c = Client::connect(addr).await;
        c.send(ClientMsg::Hello {
            protocol: PROTOCOL_VERSION,
            login: login(name),
            content,
        })
        .await;
        match c.recv().await {
            ServerMsg::Welcome { lobby } => {
                c.login = lobby.you.clone();
                (c, lobby)
            }
            other => panic!("expected a welcome, got {other:?}"),
        }
    }

    pub async fn send(&mut self, msg: ClientMsg) {
        self.send_text(&msg.to_json()).await;
    }

    pub async fn send_text(&mut self, text: &str) {
        self.ws
            .send(Message::Text(text.into()))
            .await
            .expect("sends");
    }

    pub async fn send_binary(&mut self, bytes: &[u8]) {
        self.ws
            .send(Message::Binary(bytes.to_vec().into()))
            .await
            .expect("sends");
    }

    /// The next message; fails the test if none comes in time.
    pub async fn recv(&mut self) -> ServerMsg {
        loop {
            let msg = tokio::time::timeout(TIMEOUT, self.ws.next())
                .await
                .unwrap_or_else(|_| panic!("{}: no message in time", self.login))
                .unwrap_or_else(|| panic!("{}: connection closed", self.login))
                .unwrap_or_else(|e| panic!("{}: connection error: {e}", self.login));
            match msg {
                Message::Text(text) => {
                    return ServerMsg::from_json(&text)
                        .unwrap_or_else(|e| panic!("cannot read {text}: {e}"));
                }
                Message::Close(_) => panic!("{}: connection closed", self.login),
                _ => {}
            }
        }
    }

    pub async fn expect(&mut self, expected: ServerMsg) {
        let got = self.recv().await;
        assert_eq!(got, expected, "{} got the wrong message", self.login);
    }

    /// Expects the server to close the connection, with nothing before.
    pub async fn expect_closed(&mut self) {
        loop {
            let msg = tokio::time::timeout(TIMEOUT, self.ws.next())
                .await
                .expect("closed in time");
            match msg {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => return,
                Some(Ok(Message::Text(text))) => panic!("expected a close, got {text}"),
                Some(Ok(_)) => {}
            }
        }
    }

    /// Skips messages until `expected`.
    pub async fn skip_to(&mut self, expected: &ServerMsg) {
        while self.recv().await != *expected {}
    }

    /// Skips messages until one that `pred` accepts, and returns it.
    pub async fn skip_until(&mut self, pred: impl Fn(&ServerMsg) -> bool) -> ServerMsg {
        loop {
            let msg = self.recv().await;
            if pred(&msg) {
                return msg;
            }
        }
    }

    pub async fn close(mut self) {
        let _ = self.ws.close(None).await;
        while let Ok(Some(Ok(_))) = tokio::time::timeout(TIMEOUT, self.ws.next()).await {}
    }

    // -- Games ---------------------------------------------------------------

    /// The next message that is not a turn. Turns are recorded, and
    /// reported done with `digest` when `ack` is set.
    pub async fn next_event(&mut self, ack: bool) -> ServerMsg {
        loop {
            match self.recv().await {
                ServerMsg::Turn { turn, orders } => {
                    self.turns.push((turn, orders));
                    if ack {
                        self.ack_turns(0).await;
                    }
                }
                other => return other,
            }
        }
    }

    pub async fn expect_event(&mut self, ack: bool, expected: ServerMsg) {
        let got = self.next_event(ack).await;
        assert_eq!(got, expected, "{} got the wrong message", self.login);
    }

    /// The next turn, recorded; other messages fail the test.
    pub async fn next_turn(&mut self) -> (Turn, Vec<SlotOrders>) {
        match self.recv().await {
            ServerMsg::Turn { turn, orders } => {
                self.turns.push((turn, orders.clone()));
                (turn, orders)
            }
            other => panic!("{}: expected a turn, got {other:?}", self.login),
        }
    }

    /// Reports done every recorded turn not yet reported, with `digest`.
    pub async fn ack_turns(&mut self, digest: Digest) {
        let last = match self.turns.last() {
            Some((t, _)) => *t,
            None => return,
        };
        let first = self.done.map_or(0, |t| t + 1);
        for turn in first..=last {
            self.send(ClientMsg::TurnDone { turn, digest }).await;
        }
        self.done = Some(last);
    }

    /// Receives turns, reporting each done with `digest`, until `turn`.
    pub async fn play_to(&mut self, turn: Turn, digest: Digest) {
        while self.done.is_none_or(|d| d < turn) {
            let (t, _) = self.next_turn().await;
            if t <= turn {
                self.send(ClientMsg::TurnDone { turn: t, digest }).await;
                self.done = Some(t);
            }
        }
    }
}

/// `n` clients said hello, the earlier ones having seen the later ones
/// arrive.
pub async fn clients(addr: SocketAddr, names: &[&str]) -> Vec<Client> {
    let mut all: Vec<Client> = Vec::new();
    for name in names {
        let (c, _) = Client::hello(addr, name).await;
        for other in &mut all {
            other
                .expect(lobby(LobbyChange::PlayerArrived {
                    player: LobbyPlayer {
                        login: c.login.clone(),
                        whereabouts: Whereabouts::Lobby,
                    },
                }))
                .await;
        }
        all.push(c);
    }
    all
}

/// Skips every client's messages to `last`.
pub async fn sync(clients: &mut [Client], last: &ServerMsg) {
    for c in clients {
        c.skip_to(last).await;
    }
}

/// `clients[host]` creates a room on `map` and uploads the map if asked;
/// every client is then past the host's move into the room.
pub async fn open_room(
    clients: &mut [Client],
    host: usize,
    (info, data): &(MapInfo, MapData),
    open: bool,
) -> RoomId {
    let h = &mut clients[host];
    h.send(ClientMsg::CreateRoom {
        map: info.clone(),
        speed: Speed::Fast4,
        open,
    })
    .await;
    let mut msg = h.recv().await;
    if msg
        == (ServerMsg::SendMap {
            hash: info.hash.clone(),
        })
    {
        h.send(ClientMsg::Map { map: data.clone() }).await;
        msg = h.recv().await;
    }
    let ServerMsg::Room {
        change: RoomChange::Entered,
        room,
    } = msg
    else {
        panic!("expected to enter the room, got {msg:?}");
    };
    let last = moved(&h.login.clone(), Whereabouts::InRoom(room.id));
    sync(clients, &last).await;
    room.id
}

/// `clients[who]` joins `room`; every client is then past the move.
pub async fn join_room(clients: &mut [Client], who: usize, room: RoomId) {
    clients[who].send(ClientMsg::JoinRoom { room }).await;
    let last = moved(&clients[who].login.clone(), Whereabouts::InRoom(room));
    sync(clients, &last).await;
}

/// The host (`clients[0]`) adds a computer; every client is then past the
/// room's change.
pub async fn add_computer(clients: &mut [Client], room: RoomId) {
    clients[0]
        .send(ClientMsg::AddComputer {
            level: AiLevel::Easy,
        })
        .await;
    for c in clients.iter_mut() {
        loop {
            if let ServerMsg::Lobby {
                change: LobbyChange::RoomChanged { room: r },
            } = c.recv().await
                && r.id == room
            {
                break;
            }
        }
    }
}

/// `clients[who]` says they are ready; the room's members see it.
pub async fn ready(clients: &mut [Client], who: usize, members: &[usize]) {
    clients[who].send(ClientMsg::SetReady { ready: true }).await;
    for &m in members {
        loop {
            if let ServerMsg::Room {
                change: RoomChange::Ready { .. },
                ..
            } = clients[m].recv().await
            {
                break;
            }
        }
    }
}

/// Starts a game from `room` hosted by `clients[0]`, where every human
/// listed in `players` (by index, in slot order) is ready. Returns each
/// player's `GameStarted`; every client is then past the moves into the
/// game.
pub async fn start_game(clients: &mut [Client], room: RoomId, players: &[usize]) -> Vec<ServerMsg> {
    clients[0].send(ClientMsg::Start).await;
    let mut started = Vec::new();
    for &p in players {
        let msg = clients[p].recv().await;
        assert!(
            matches!(msg, ServerMsg::GameStarted { .. }),
            "expected the game to start, got {msg:?}"
        );
        started.push(msg);
    }
    let last_player = clients[*players.last().unwrap()].login.clone();
    let last = moved(&last_player, Whereabouts::Playing(room));
    for c in clients.iter_mut() {
        loop {
            match c.recv().await {
                ServerMsg::Turn { turn, orders } => c.turns.push((turn, orders)),
                msg if msg == last => break,
                _ => {}
            }
        }
    }
    started
}
