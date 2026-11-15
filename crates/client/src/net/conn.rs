//! The connection to a server: a WebSocket served by its own thread, so
//! that the menus and the game only ever poll a channel and never wait on
//! the network.

use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use soundrts_protocol::game::{SeatFrame, decode_frame};
use soundrts_protocol::{
    ClientMsg, DEFAULT_PORT, Invalid, LobbySnapshot, MAX_MESSAGE_BYTES, Rejection, ServerMsg, Wire,
};
use tokio::sync::mpsc as tokio_mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

/// What comes from the server.
#[derive(Clone, Debug, PartialEq)]
pub enum NetEvent {
    Msg(ServerMsg),
    /// A frame of the game's stream, decoded but not yet checked against
    /// the game; or why it could not be decoded.
    Frame(Result<Box<SeatFrame>, Invalid>),
    /// The connection is over, for this reason.
    Closed(String),
}

/// What goes to the server.
#[derive(Clone, Debug, PartialEq)]
pub enum Outgoing {
    /// A message of the lobby, as JSON text.
    Msg(ClientMsg),
    /// A command of the player, encoded ([`soundrts_protocol::game::encode_command`]).
    Command(Vec<u8>),
}

/// Both ends of a link to a server: a real socket, or a fake one in tests.
pub trait Link {
    fn send(&mut self, out: Outgoing);
    /// The next event, if one has come. Never blocks.
    fn poll(&mut self) -> Option<NetEvent>;
}

/// A connection to a server. Dropping it closes it.
pub struct Connection(Box<dyn Link>);

impl Connection {
    pub fn new(link: Box<dyn Link>) -> Connection {
        Connection(link)
    }

    pub fn send(&mut self, msg: ClientMsg) {
        self.0.send(Outgoing::Msg(msg));
    }

    /// Sends an encoded command of the player.
    pub fn send_command(&mut self, bytes: Vec<u8>) {
        self.0.send(Outgoing::Command(bytes));
    }

    pub fn poll(&mut self) -> Option<NetEvent> {
        self.0.poll()
    }

    /// Closes the connection: nothing more is sent or received.
    pub fn close(&mut self) {
        self.0 = Box::new(ClosedLink);
    }
}

/// The link of a connection the client closed.
struct ClosedLink;

impl Link for ClosedLink {
    fn send(&mut self, _: Outgoing) {}

    fn poll(&mut self) -> Option<NetEvent> {
        None
    }
}

/// Why a connection could not be opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectError {
    /// The address is not one.
    BadAddress,
    /// Nothing answered, or not in time.
    Unreachable(String),
    /// Something answered, but not as a SoundRTS server does.
    NotAServer(String),
    Rejected(Rejection),
}

/// The WebSocket URL of an address typed by a player: a host name or an
/// IP address, with or without a port.
pub fn ws_url(address: &str) -> Option<String> {
    let a = address.trim();
    if a.is_empty() {
        return None;
    }
    if let Ok(addr) = a.parse::<std::net::SocketAddr>() {
        return Some(format!("ws://{addr}"));
    }
    if let Ok(ip) = a.parse::<std::net::Ipv6Addr>() {
        return Some(format!("ws://[{ip}]:{DEFAULT_PORT}"));
    }
    let (host, port) = match a.rsplit_once(':') {
        Some((h, p)) => (h, p.parse::<u16>().ok()?),
        None => (a, DEFAULT_PORT),
    };
    let ok = !host.is_empty()
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
    ok.then(|| format!("ws://{host}:{port}"))
}

/// The most server messages waiting to be read. Beyond it the connection
/// stops reading the socket, so a server that floods the client fills its
/// own buffers rather than the client's memory.
pub const EVENT_QUEUE: usize = 1024;

/// How a connection notices a server that no longer answers, as when a
/// cable is pulled: after `ping` with nothing heard it pings the server,
/// which answers at once, and after `silence` it gives the connection up as
/// lost.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Liveness {
    pub ping: Duration,
    pub silence: Duration,
}

impl Default for Liveness {
    fn default() -> Liveness {
        Liveness {
            ping: Duration::from_secs(5),
            silence: Duration::from_secs(15),
        }
    }
}

/// What opens a connection to one server: the first time, and again after
/// it is lost.
#[derive(Clone, Debug)]
pub struct Dialer {
    pub address: String,
    pub hello: ClientMsg,
    /// How long to wait for the server, then as long for its welcome.
    pub timeout: Duration,
    pub liveness: Liveness,
}

impl Dialer {
    /// Starts opening a connection: [`Opening::poll`] says when it is open.
    pub fn start(&self) -> Opening {
        match ws_url(&self.address) {
            Some(url) => start(url, self.hello.clone(), self.timeout, self.liveness),
            None => Opening::done(Err(ConnectError::BadAddress)),
        }
    }

    /// Connects, says hello, and waits for the server's welcome.
    pub fn open(&self) -> Opened {
        let mut opening = self.start();
        loop {
            if let Some(opened) = opening.poll() {
                return opened;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

/// Connects to `address`, says `hello`, and waits for the server's
/// welcome, at most `timeout`.
pub fn open(address: &str, hello: ClientMsg, timeout: Duration) -> Opened {
    Dialer {
        address: address.to_owned(),
        hello,
        timeout,
        liveness: Liveness::default(),
    }
    .open()
}

/// A connection with the server's welcome, or why there is none.
pub type Opened = Result<(Connection, LobbySnapshot), ConnectError>;

/// A connection being opened by its own thread: asking never waits.
pub struct Opening(Stage);

enum Stage {
    /// The thread connects and says hello, until `deadline`.
    Connecting {
        link: SocketLink,
        connected: mpsc::Receiver<Result<(), ConnectError>>,
        deadline: Instant,
    },
    /// The hello is said: the welcome is awaited until `deadline`.
    Welcoming { link: SocketLink, deadline: Instant },
    /// Over, and not yet told if `Some`.
    Done(Option<Opened>),
}

impl Opening {
    /// An opening already over, as a fake server gives.
    pub fn done(opened: Opened) -> Opening {
        Opening(Stage::Done(Some(opened)))
    }

    /// The connection with the server's welcome, or why there is none,
    /// once it is known; told once.
    pub fn poll(&mut self) -> Option<Opened> {
        if let Stage::Connecting {
            connected,
            deadline,
            ..
        } = &self.0
        {
            match connected.try_recv() {
                Err(mpsc::TryRecvError::Empty) if Instant::now() <= *deadline => return None,
                Ok(Ok(())) => {
                    let Stage::Connecting { link, deadline, .. } =
                        std::mem::replace(&mut self.0, Stage::Done(None))
                    else {
                        unreachable!("connecting");
                    };
                    // The welcome may take as long as the connection did.
                    let wait = deadline.saturating_duration_since(Instant::now());
                    self.0 = Stage::Welcoming {
                        link,
                        deadline: Instant::now() + wait.max(Duration::from_secs(1)),
                    };
                }
                Ok(Err(e)) => self.0 = Stage::Done(Some(Err(e))),
                Err(_) => {
                    let e = ConnectError::Unreachable("timed out".into());
                    self.0 = Stage::Done(Some(Err(e)));
                }
            }
        }
        if let Stage::Welcoming { link, deadline } = &mut self.0 {
            match link.poll() {
                Some(first) => {
                    let Stage::Welcoming { link, .. } =
                        std::mem::replace(&mut self.0, Stage::Done(None))
                    else {
                        unreachable!("welcoming");
                    };
                    self.0 = Stage::Done(Some(welcomed(link, first)));
                }
                None if Instant::now() > *deadline => {
                    let e = ConnectError::NotAServer("no welcome in time".into());
                    self.0 = Stage::Done(Some(Err(e)));
                }
                None => return None,
            }
        }
        match &mut self.0 {
            Stage::Done(opened) => opened.take(),
            Stage::Connecting { .. } | Stage::Welcoming { .. } => None,
        }
    }
}

/// Starts the thread that connects to `url`.
fn start(url: String, hello: ClientMsg, timeout: Duration, liveness: Liveness) -> Opening {
    let (events_tx, events_rx) = tokio_mpsc::channel(EVENT_QUEUE);
    let (out_tx, out_rx) = tokio_mpsc::unbounded_channel();
    let (connected_tx, connected_rx) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("soundrts-connection".into())
        .spawn(move || {
            let channels = Channels {
                out: out_rx,
                events: events_tx,
                connected: connected_tx,
            };
            serve(url, hello, timeout, liveness, channels);
        });
    let thread = match spawned {
        Ok(t) => t,
        Err(e) => return Opening::done(Err(ConnectError::Unreachable(e.to_string()))),
    };
    Opening(Stage::Connecting {
        link: SocketLink {
            out: Some(out_tx),
            events: Some(events_rx),
            thread: Some(thread),
        },
        connected: connected_rx,
        // The thread gives up on its own at the timeout; the margin covers
        // a slow thread start.
        deadline: Instant::now() + timeout + Duration::from_secs(1),
    })
}

/// The connection, if the server's first message is its welcome.
fn welcomed(link: SocketLink, first: NetEvent) -> Opened {
    match first {
        NetEvent::Msg(ServerMsg::Welcome { lobby }) => {
            super::lobby::check_snapshot(&lobby)
                .map_err(|e| ConnectError::NotAServer(e.to_string()))?;
            Ok((Connection::new(Box::new(link)), lobby))
        }
        NetEvent::Msg(ServerMsg::Rejected { reason }) => Err(ConnectError::Rejected(reason)),
        NetEvent::Msg(other) => Err(ConnectError::NotAServer(format!(
            "unexpected first message: {other:?}"
        ))),
        NetEvent::Frame(_) => Err(ConnectError::NotAServer(
            "a game's frame before the welcome".into(),
        )),
        NetEvent::Closed(reason) => Err(ConnectError::NotAServer(reason)),
    }
}

struct SocketLink {
    out: Option<tokio_mpsc::UnboundedSender<Outgoing>>,
    events: Option<tokio_mpsc::Receiver<NetEvent>>,
    thread: Option<JoinHandle<()>>,
}

impl Link for SocketLink {
    fn send(&mut self, out: Outgoing) {
        if let Some(tx) = &self.out {
            let _ = tx.send(out);
        }
    }

    fn poll(&mut self) -> Option<NetEvent> {
        self.events.as_mut()?.try_recv().ok()
    }
}

impl Drop for SocketLink {
    fn drop(&mut self) {
        // Without a receiver, a thread waiting to hand over a message, or to
        // write to a server that no longer reads, gives up at once; without
        // a sender, it closes the socket, within CLOSE_TIMEOUT.
        self.events = None;
        self.out = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// How long to wait for the server to answer a close.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(1);

/// Why the connection's loop ended.
enum End {
    /// The client dropped the connection: close the socket.
    Dropped,
    /// The connection is over, for this reason.
    Lost(String),
    /// The client no longer reads: just stop.
    Abandoned,
}

/// Why a connection to a server that stopped answering is lost.
const SILENT: &str = "the server stopped answering";

/// How the connection's thread talks with the client.
struct Channels {
    out: tokio_mpsc::UnboundedReceiver<Outgoing>,
    events: tokio_mpsc::Sender<NetEvent>,
    /// Whether the hello is said.
    connected: mpsc::Sender<Result<(), ConnectError>>,
}

fn serve(url: String, hello: ClientMsg, timeout: Duration, liveness: Liveness, channels: Channels) {
    let Channels {
        mut out,
        events,
        connected,
    } = channels;
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            let _ = connected.send(Err(ConnectError::Unreachable(e.to_string())));
            return;
        }
    };
    runtime.block_on(async move {
        let config = WebSocketConfig::default()
            .max_message_size(Some(MAX_MESSAGE_BYTES))
            .max_frame_size(Some(MAX_MESSAGE_BYTES));
        let connect = tokio_tungstenite::connect_async_with_config(url, Some(config), true);
        // A client that gives the connection up meanwhile waits for none of
        // it.
        let connecting = tokio::select! {
            c = tokio::time::timeout(timeout, connect) => c,
            _ = events.closed() => return,
        };
        let mut ws = match connecting {
            Ok(Ok((ws, _))) => ws,
            Ok(Err(e)) => {
                let error = match e {
                    tokio_tungstenite::tungstenite::Error::Io(e) => {
                        ConnectError::Unreachable(e.to_string())
                    }
                    e => ConnectError::NotAServer(e.to_string()),
                };
                let _ = connected.send(Err(error));
                return;
            }
            Err(_) => {
                let _ = connected.send(Err(ConnectError::Unreachable("timed out".into())));
                return;
            }
        };
        let hello = Message::Text(hello.to_json().into());
        let said = tokio::select! {
            s = tokio::time::timeout(timeout, ws.send(hello)) => s,
            _ = events.closed() => return,
        };
        match said {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                let _ = connected.send(Err(ConnectError::NotAServer(e.to_string())));
                return;
            }
            Err(_) => {
                let _ = connected.send(Err(ConnectError::Unreachable("timed out".into())));
                return;
            }
        }
        let _ = connected.send(Ok(()));
        // When the server was last heard, and whether it was pinged since.
        let mut heard = tokio::time::Instant::now();
        let mut pinged = false;
        let end = loop {
            let silent = heard + liveness.silence;
            tokio::select! {
                // A dropped connection is closed cleanly first.
                biased;
                msg = out.recv() => match msg {
                    Some(msg) => {
                        let message = match msg {
                            Outgoing::Msg(m) => Message::Text(m.to_json().into()),
                            Outgoing::Command(bytes) => Message::Binary(bytes.into()),
                        };
                        // A server that stops reading blocks the write.
                        tokio::select! {
                            sent = ws.send(message) => {
                                if let Err(e) = sent {
                                    break End::Lost(e.to_string());
                                }
                            }
                            _ = events.closed() => break End::Abandoned,
                            _ = tokio::time::sleep_until(silent) => break End::Lost(SILENT.into()),
                        }
                    }
                    None => break End::Dropped,
                },
                incoming = ws.next() => {
                    match incoming {
                        Some(Ok(Message::Text(text))) => match ServerMsg::from_json(&text) {
                            // Waits while the queue is full: the socket is
                            // not read meanwhile.
                            Ok(msg) => {
                                if events.send(NetEvent::Msg(msg)).await.is_err() {
                                    break End::Abandoned;
                                }
                            }
                            Err(e) => {
                                break End::Lost(format!("cannot read a message from the server: {e}"));
                            }
                        },
                        Some(Ok(Message::Binary(bytes))) => {
                            let frame = decode_frame(&bytes).map(Box::new);
                            if events.send(NetEvent::Frame(frame)).await.is_err() {
                                break End::Abandoned;
                            }
                        }
                        Some(Ok(Message::Close(_))) | None => break End::Lost("closed by the server".into()),
                        Some(Ok(_)) => {}
                        Some(Err(e)) => break End::Lost(e.to_string()),
                    }
                    // Once handed over: a client slow to read is not a
                    // silent server.
                    heard = tokio::time::Instant::now();
                    pinged = false;
                }
                _ = tokio::time::sleep_until(heard + liveness.ping), if !pinged => {
                    pinged = true;
                    tokio::select! {
                        sent = ws.send(Message::Ping(Default::default())) => {
                            if let Err(e) = sent {
                                break End::Lost(e.to_string());
                            }
                        }
                        _ = events.closed() => break End::Abandoned,
                        _ = tokio::time::sleep_until(silent) => break End::Lost(SILENT.into()),
                    }
                }
                _ = tokio::time::sleep_until(silent) => break End::Lost(SILENT.into()),
                _ = events.closed() => break End::Abandoned,
            }
        };
        match end {
            End::Dropped => {
                let _ = tokio::time::timeout(CLOSE_TIMEOUT, async {
                    if ws.close(None).await.is_ok() {
                        while let Some(Ok(_)) = ws.next().await {}
                    }
                })
                .await;
            }
            End::Lost(reason) => {
                // After the messages before it, unless the client drops
                // the connection meanwhile.
                tokio::select! {
                    _ = events.send(NetEvent::Closed(reason)) => {}
                    _ = events.closed() => {}
                }
            }
            End::Abandoned => {}
        }
    });
    // A host name still being looked up, on the runtime's blocking pool,
    // is not waited for.
    runtime.shutdown_background();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_addresses_become_websocket_urls() {
        assert_eq!(ws_url("1.2.3.4").as_deref(), Some("ws://1.2.3.4:2501"));
        assert_eq!(ws_url(" 1.2.3.4:99 ").as_deref(), Some("ws://1.2.3.4:99"));
        assert_eq!(
            ws_url("game.example.org").as_deref(),
            Some("ws://game.example.org:2501")
        );
        assert_eq!(ws_url("::1").as_deref(), Some("ws://[::1]:2501"));
        assert_eq!(ws_url("[::1]:7").as_deref(), Some("ws://[::1]:7"));
        assert_eq!(ws_url(""), None);
        assert_eq!(ws_url("a b"), None);
        assert_eq!(ws_url("host:port"), None);
    }
}
