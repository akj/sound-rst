//! One task per connection: the WebSocket handshake, the hello, the rate
//! limits and decoding, the heartbeat, then messages to the lobby and from
//! the connection's [`Outbox`] to the socket.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use soundrts_protocol::{
    ClientMsg, ContentHash, Login, MAX_MESSAGE_BYTES, PROTOCOL_VERSION, Refusal, Rejection,
    ServerMsg, Wire, game::decode_command,
};
use tokio::net::TcpStream;
use tokio::sync::{Notify, OwnedSemaphorePermit, mpsc, watch};
use tokio::time::Instant;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, Role, WebSocketConfig};
use tokio_tungstenite::tungstenite::{Bytes, Message, Utf8Bytes};
use tracing::{debug, warn};

use crate::lobby::{LobbyInput, LobbyTx};

/// Identifies a connection for the life of the server. Never reused.
pub(crate) type ConnId = u64;

/// Messages waiting for a slow client. A client that falls this far behind
/// is disconnected rather than buffered without end.
const OUTBOX_CAPACITY: usize = 1024;

/// Undecodable messages in a row before the server hangs up.
const MAX_INVALID_IN_A_ROW: u32 = 5;

/// The general message rate limit: a burst, then a steady rate. A client
/// at the fastest game speed sends about 15 messages a second.
const MESSAGE_BURST: f64 = 100.0;
const MESSAGES_PER_SECOND: f64 = 50.0;

/// The chat rate limit: this many lines in this window.
const CHAT_LINES: usize = 5;
const CHAT_WINDOW: Duration = Duration::from_secs(5);

/// The invitation rate limit: this many in this window. Each is said to
/// the player invited. A host invites at most one player for each free
/// slot of a room: 15 on the largest maps.
const INVITATIONS: usize = 20;
const INVITATION_WINDOW: Duration = Duration::from_secs(60);

/// The map transfer rate limit: this many maps asked for or sent in this
/// window. A client asks for a room's map once, on entering a room whose
/// map it lacks, and a host sends one when the server lacks its map.
const MAP_TRANSFERS: usize = 10;
const MAP_WINDOW: Duration = Duration::from_secs(60);

/// The largest hello, and the largest frame before it. A hello is under
/// 200 bytes. (The WebSocket handshake before it has its own limit, 64 KB.)
const MAX_HELLO_BYTES: usize = 16 * 1024;

/// The most bytes of messages other than maps that one connection can
/// have handed to the lobby or its game and they have not handled yet.
/// An honest client sends a few hundred bytes at a time: a chat line, a
/// command of at most 4 KB. The lobby and games handle each message in far
/// less than a millisecond.
const MAX_QUEUED_BYTES: usize = 1024 * 1024;

/// Maps have their own allowance: one map, the largest message there is.
/// A host sends one map, when the server asks for it.
const MAX_QUEUED_MAP_BYTES: usize = MAX_MESSAGE_BYTES;

/// How long to wait for the peer to answer a close.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(2);

/// How long a connection may take, how often it is pinged, and its
/// socket's send buffer.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ConnSettings {
    /// To say hello.
    pub hello: Duration,
    /// Between two pings.
    pub ping: Duration,
    /// To be heard from at all, a ping's answer included, before the
    /// connection counts as dead.
    pub silence: Duration,
    /// The bytes the system may hold for the client before writes wait.
    pub send_buffer: Option<usize>,
}

pub(crate) enum Outbound {
    Text(Utf8Bytes),
    /// A frame of a hosted game's stream.
    Binary(Bytes),
    /// Close the connection after what came before.
    Close,
}

/// Where the lobby and games put messages for one client.
#[derive(Clone)]
pub(crate) struct Outbox {
    tx: mpsc::Sender<Outbound>,
    kick: Arc<Notify>,
}

impl Outbox {
    pub(crate) fn send(&self, msg: &ServerMsg) {
        self.send_text(encode(msg));
    }

    /// Sends a message already encoded with [`encode`], to share one
    /// encoding between many clients.
    pub(crate) fn send_text(&self, text: Utf8Bytes) {
        self.push(Outbound::Text(text));
    }

    /// Sends a frame of a hosted game's stream.
    pub(crate) fn send_binary(&self, bytes: Bytes) {
        self.push(Outbound::Binary(bytes));
    }

    /// Closes the connection once the messages before are sent.
    pub(crate) fn close(&self) {
        self.push(Outbound::Close);
    }

    /// How many messages wait to be written to the socket.
    pub(crate) fn queued(&self) -> usize {
        self.tx.max_capacity().saturating_sub(self.tx.capacity())
    }

    fn push(&self, out: Outbound) {
        match self.tx.try_send(out) {
            Ok(()) | Err(mpsc::error::TrySendError::Closed(_)) => {}
            Err(mpsc::error::TrySendError::Full(_)) => self.kick.notify_one(),
        }
    }
}

pub(crate) fn encode(msg: &ServerMsg) -> Utf8Bytes {
    msg.to_json().into()
}

type Ws = WebSocketStream<TcpStream>;

/// `pending` is this connection's place among those that have not said
/// hello; it is given back once the hello is read.
pub(crate) async fn serve(
    stream: TcpStream,
    peer: SocketAddr,
    conn: ConnId,
    pending: OwnedSemaphorePermit,
    lobby: LobbyTx,
    mut stop: watch::Receiver<bool>,
    timeouts: ConnSettings,
) {
    if let Err(e) = stream.set_nodelay(true) {
        debug!(%peer, error = %e, "cannot set TCP_NODELAY");
    }
    if let Some(bytes) = timeouts.send_buffer
        && let Err(e) = socket2::SockRef::from(&stream).set_send_buffer_size(bytes)
    {
        debug!(%peer, error = %e, "cannot set the send buffer");
    }
    let deadline = crate::hosted::after(timeouts.hello);
    // The WebSocket reserves room for a whole frame as soon as its header
    // comes, so the hello is read with frames no larger than a hello.
    let hello_config = WebSocketConfig::default()
        .max_message_size(Some(MAX_HELLO_BYTES))
        .max_frame_size(Some(MAX_HELLO_BYTES));
    let mut ws = match tokio::time::timeout_at(
        deadline,
        tokio_tungstenite::accept_async_with_config(stream, Some(hello_config)),
    )
    .await
    {
        Ok(Ok(ws)) => ws,
        Ok(Err(e)) => {
            debug!(%peer, error = %e, "WebSocket handshake failed");
            return;
        }
        Err(_) => {
            debug!(%peer, "WebSocket handshake timed out");
            return;
        }
    };

    let hello = tokio::select! {
        first = tokio::time::timeout_at(deadline, first_text(&mut ws)) => match first {
            Ok(Some(text)) => read_hello(&text),
            Ok(None) => return,
            Err(_) => Err(Rejection::NoHello),
        },
        _ = stop.changed() => {
            close(&mut ws).await;
            return;
        }
    };
    let (login, content) = match hello {
        Ok(hello) => hello,
        Err(reason) => {
            debug!(%peer, ?reason, "rejected");
            let msg = encode(&ServerMsg::Rejected { reason });
            if ws.send(Message::Text(msg)).await.is_ok() {
                close(&mut ws).await;
            }
            return;
        }
    };
    // A client sends nothing between its hello and the answer, so nothing
    // read past the hello is lost when the socket gets its full limits.
    let stream = ws.into_inner();
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_MESSAGE_BYTES));
    let mut ws = WebSocketStream::from_raw_socket(stream, Role::Server, Some(config)).await;
    drop(pending);

    let (tx, mut rx) = mpsc::channel(OUTBOX_CAPACITY);
    let kick = Arc::new(Notify::new());
    let outbox = Outbox {
        tx,
        kick: kick.clone(),
    };
    let _ = lobby.send(LobbyInput::Hello {
        conn,
        login,
        content,
        outbox: outbox.clone(),
    });

    let mut limits = Limits::new(Instant::now());
    let maps = Allowance::new(MAX_QUEUED_MAP_BYTES);
    let others = Allowance::new(MAX_QUEUED_BYTES);
    let mut invalid_in_a_row = 0;
    let mut heard = Instant::now();
    let mut ping = tokio::time::interval_at(crate::hosted::after(timeouts.ping), timeouts.ping);
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        let silent = heard.checked_add(timeouts.silence).unwrap_or(heard);
        tokio::select! {
            incoming = ws.next() => {
                heard = Instant::now();
                let (bytes, incoming) = match incoming {
                    Some(Ok(Message::Text(text))) => {
                        (text.len(), ClientMsg::from_json(&text).ok().map(Incoming::Msg))
                    }
                    Some(Ok(Message::Binary(bytes))) => {
                        (bytes.len(), decode_command(&bytes).ok().map(Incoming::Command))
                    }
                    Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_))) => continue,
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(e)) => {
                        debug!(conn, error = %e, "connection error");
                        break;
                    }
                };
                let now = Instant::now();
                if !limits.message(now) {
                    outbox.send(&ServerMsg::Refused { reason: Refusal::SlowDown });
                    continue;
                }
                match incoming {
                    Some(incoming) => {
                        invalid_in_a_row = 0;
                        if let Incoming::Msg(msg) = &incoming
                            && !limits.kind(msg, now)
                        {
                            outbox.send(&ServerMsg::Refused { reason: Refusal::SlowDown });
                            continue;
                        }
                        let large = matches!(
                            incoming,
                            Incoming::Msg(ClientMsg::Map { .. } | ClientMsg::StartHosted { .. })
                        );
                        let allowance = if large { &maps } else { &others };
                        let Some(queued) = allowance.take(bytes) else {
                            warn!(conn, bytes, "client sent more than the server can hold; disconnecting");
                            close_with(&mut ws, Some(CloseFrame {
                                code: CloseCode::Policy,
                                reason: "too much sent at once".into(),
                            }))
                            .await;
                            break;
                        };
                        let input = match incoming {
                            Incoming::Msg(msg) => LobbyInput::Msg { conn, msg, queued },
                            Incoming::Command(command) => LobbyInput::Command {
                                conn,
                                command,
                                bytes,
                                queued,
                            },
                        };
                        let _ = lobby.send(input);
                    }
                    None => {
                        invalid_in_a_row += 1;
                        outbox.send(&ServerMsg::Refused { reason: Refusal::Invalid });
                        if invalid_in_a_row >= MAX_INVALID_IN_A_ROW {
                            debug!(conn, "too many invalid messages");
                            outbox.close();
                        }
                    }
                }
            }
            out = rx.recv() => {
                let msg = match out {
                    Some(Outbound::Text(text)) => Message::Text(text),
                    Some(Outbound::Binary(bytes)) => Message::Binary(bytes),
                    Some(Outbound::Close) | None => {
                        close(&mut ws).await;
                        break;
                    }
                };
                if write(&mut ws, msg, &kick, &mut stop, silent, conn).await.is_err() {
                    break;
                }
            }
            _ = ping.tick() => {
                if Instant::now() >= silent {
                    debug!(conn, "heard nothing in time; the connection is dead");
                    break;
                }
                if write(&mut ws, Message::Ping(Bytes::new()), &kick, &mut stop, silent, conn)
                    .await
                    .is_err()
                {
                    break;
                }
            }
            _ = tokio::time::sleep_until(silent) => {
                debug!(conn, "heard nothing in time; the connection is dead");
                break;
            }
            _ = kick.notified() => {
                warn!(conn, "client too slow to read its messages; disconnecting");
                break;
            }
            _ = stop.changed() => {
                close(&mut ws).await;
                break;
            }
        }
    }
    let _ = lobby.send(LobbyInput::Gone { conn });
}

/// What a client sent, decoded: a JSON message, or a hosted game's
/// command as a binary frame.
enum Incoming {
    Msg(ClientMsg),
    Command(soundrts_world::Command),
}

/// Writes one message. A peer that stops reading blocks the write; a kick,
/// a shutdown or the heartbeat's deadline still ends the connection.
async fn write(
    ws: &mut Ws,
    msg: Message,
    kick: &Notify,
    stop: &mut watch::Receiver<bool>,
    silent: Instant,
    conn: ConnId,
) -> Result<(), ()> {
    tokio::select! {
        sent = ws.send(msg) => sent.map_err(drop),
        _ = kick.notified() => {
            warn!(conn, "client too slow to read its messages; disconnecting");
            Err(())
        }
        _ = tokio::time::sleep_until(silent) => {
            debug!(conn, "heard nothing in time while writing; the connection is dead");
            Err(())
        }
        _ = stop.changed() => Err(()),
    }
}

/// The first text message, or `None` if the connection ends first. Any
/// other kind of message stands for a message that is not a hello.
async fn first_text(ws: &mut Ws) -> Option<String> {
    loop {
        match ws.next().await? {
            Ok(Message::Text(text)) => return Some(text.as_str().to_owned()),
            Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_)) => {}
            Ok(Message::Binary(_)) => return Some(String::new()),
            Ok(Message::Close(_)) | Err(_) => return None,
        }
    }
}

fn read_hello(text: &str) -> Result<(Login, ContentHash), Rejection> {
    let mismatch = Rejection::ProtocolMismatch {
        server: PROTOCOL_VERSION,
    };
    match ClientMsg::from_json(text) {
        Ok(ClientMsg::Hello {
            protocol,
            login,
            content,
        }) => {
            if protocol == PROTOCOL_VERSION {
                Ok((login, content))
            } else {
                Err(mismatch)
            }
        }
        Ok(_) => Err(Rejection::NoHello),
        Err(_) => {
            // The hello of another protocol version may not decode as ours.
            let value: serde_json::Value =
                serde_json::from_str(text).map_err(|_| Rejection::NoHello)?;
            match (value["type"].as_str(), value["protocol"].as_u64()) {
                (Some("hello"), Some(p)) if p != u64::from(PROTOCOL_VERSION) => Err(mismatch),
                _ => Err(Rejection::NoHello),
            }
        }
    }
}

/// Sends a close and waits a little for the peer's, so that what was sent
/// before reaches it rather than being cut by a reset.
async fn close(ws: &mut Ws) {
    close_with(ws, None).await;
}

async fn close_with(ws: &mut Ws, frame: Option<CloseFrame>) {
    let _ = tokio::time::timeout(CLOSE_TIMEOUT, async {
        if ws.close(frame).await.is_ok() {
            while let Some(Ok(_)) = ws.next().await {}
        }
    })
    .await;
}

/// How many bytes of messages one connection may have handed to the lobby
/// or its game and they have not handled yet, counted as received.
struct Allowance {
    queued: Arc<AtomicUsize>,
    limit: usize,
}

impl Allowance {
    fn new(limit: usize) -> Allowance {
        Allowance {
            queued: Arc::new(AtomicUsize::new(0)),
            limit,
        }
    }

    /// Counts `bytes` as queued until the [`Queued`] given back is dropped;
    /// `None` if that would go over the limit.
    fn take(&self, bytes: usize) -> Option<Queued> {
        let before = self.queued.fetch_add(bytes, Ordering::Relaxed);
        if before + bytes > self.limit {
            self.queued.fetch_sub(bytes, Ordering::Relaxed);
            return None;
        }
        Some(Queued {
            bytes,
            queued: self.queued.clone(),
        })
    }
}

/// The bytes of one message, counted against its connection's
/// [`Allowance`] until the lobby or game that handles it drops this.
pub(crate) struct Queued {
    bytes: usize,
    queued: Arc<AtomicUsize>,
}

impl Drop for Queued {
    fn drop(&mut self) {
        self.queued.fetch_sub(self.bytes, Ordering::Relaxed);
    }
}

struct Limits {
    tokens: f64,
    refilled: Instant,
    chat: Window,
    invitations: Window,
    maps: Window,
}

impl Limits {
    fn new(now: Instant) -> Limits {
        Limits {
            tokens: MESSAGE_BURST,
            refilled: now,
            chat: Window::new(CHAT_LINES, CHAT_WINDOW),
            invitations: Window::new(INVITATIONS, INVITATION_WINDOW),
            maps: Window::new(MAP_TRANSFERS, MAP_WINDOW),
        }
    }

    /// Whether one more message is allowed now.
    fn message(&mut self, now: Instant) -> bool {
        let elapsed = now.duration_since(self.refilled).as_secs_f64();
        self.tokens = (self.tokens + elapsed * MESSAGES_PER_SECOND).min(MESSAGE_BURST);
        self.refilled = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Whether a message of this kind is allowed now: chat lines,
    /// invitations and map transfers have their own limits.
    fn kind(&mut self, msg: &ClientMsg, now: Instant) -> bool {
        let window = match msg {
            ClientMsg::Chat { .. } => &mut self.chat,
            ClientMsg::Invite { .. } => &mut self.invitations,
            ClientMsg::Map { .. } | ClientMsg::GetMap { .. } | ClientMsg::StartHosted { .. } => {
                &mut self.maps
            }
            _ => return true,
        };
        window.allow(now)
    }
}

/// At most `most` events in any `period`.
struct Window {
    most: usize,
    period: Duration,
    /// The events of the last period, oldest first.
    events: VecDeque<Instant>,
}

impl Window {
    fn new(most: usize, period: Duration) -> Window {
        Window {
            most,
            period,
            events: VecDeque::new(),
        }
    }

    /// Whether one more event is allowed now; if so, it counts.
    fn allow(&mut self, now: Instant) -> bool {
        while self
            .events
            .front()
            .is_some_and(|&t| now.duration_since(t) >= self.period)
        {
            self.events.pop_front();
        }
        if self.events.len() < self.most {
            self.events.push_back(now);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_allowance_counts_what_is_queued_until_it_is_handled() {
        let allowance = Allowance::new(100);
        let first = allowance.take(60).expect("within the allowance");
        assert!(allowance.take(41).is_none(), "over the allowance");
        let second = allowance.take(40).expect("exactly the allowance");
        drop(first);
        let third = allowance.take(60).expect("the first was handled");
        drop((second, third));
        assert_eq!(allowance.queued.load(Ordering::Relaxed), 0);
    }
}
