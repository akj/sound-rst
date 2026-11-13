//! One task per connection: the WebSocket handshake, the hello, the rate
//! limits and decoding, then messages to the lobby and from the
//! connection's [`Outbox`] to the socket.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use soundrts_protocol::{
    ClientMsg, ContentHash, Login, MAX_MESSAGE_BYTES, PROTOCOL_VERSION, Refusal, Rejection,
    ServerMsg, Wire,
};
use tokio::net::TcpStream;
use tokio::sync::{Notify, mpsc, watch};
use tokio::time::Instant;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::{Message, Utf8Bytes};
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

/// How long to wait for the peer to answer a close.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(2);

pub(crate) enum Outbound {
    Text(Utf8Bytes),
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

    /// Closes the connection once the messages before are sent.
    pub(crate) fn close(&self) {
        self.push(Outbound::Close);
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

pub(crate) async fn serve(
    stream: TcpStream,
    peer: SocketAddr,
    conn: ConnId,
    lobby: LobbyTx,
    mut stop: watch::Receiver<bool>,
    hello_timeout: Duration,
) {
    if let Err(e) = stream.set_nodelay(true) {
        debug!(%peer, error = %e, "cannot set TCP_NODELAY");
    }
    let deadline = Instant::now() + hello_timeout;
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_MESSAGE_BYTES));
    let mut ws = match tokio::time::timeout_at(
        deadline,
        tokio_tungstenite::accept_async_with_config(stream, Some(config)),
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
    let mut invalid_in_a_row = 0;
    loop {
        tokio::select! {
            incoming = ws.next() => {
                let text = match incoming {
                    Some(Ok(Message::Text(text))) => Some(text),
                    Some(Ok(Message::Binary(_))) => None,
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
                match text.and_then(|t| ClientMsg::from_json(&t).ok()) {
                    Some(msg) => {
                        invalid_in_a_row = 0;
                        if matches!(msg, ClientMsg::Chat { .. }) && !limits.chat(now) {
                            outbox.send(&ServerMsg::Refused { reason: Refusal::SlowDown });
                            continue;
                        }
                        let _ = lobby.send(LobbyInput::Msg { conn, msg });
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
            out = rx.recv() => match out {
                Some(Outbound::Text(text)) => {
                    // A peer that stops reading blocks the write; a kick or a
                    // shutdown must still end the connection.
                    tokio::select! {
                        sent = ws.send(Message::Text(text)) => if sent.is_err() { break },
                        _ = kick.notified() => {
                            warn!(conn, "client too slow to read its messages; disconnecting");
                            break;
                        }
                        _ = stop.changed() => break,
                    }
                }
                Some(Outbound::Close) | None => {
                    close(&mut ws).await;
                    break;
                }
            },
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
    let _ = tokio::time::timeout(CLOSE_TIMEOUT, async {
        if ws.close(None).await.is_ok() {
            while let Some(Ok(_)) = ws.next().await {}
        }
    })
    .await;
}

struct Limits {
    tokens: f64,
    refilled: Instant,
    chat: VecDeque<Instant>,
}

impl Limits {
    fn new(now: Instant) -> Limits {
        Limits {
            tokens: MESSAGE_BURST,
            refilled: now,
            chat: VecDeque::new(),
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

    /// Whether one more chat line is allowed now.
    fn chat(&mut self, now: Instant) -> bool {
        while self
            .chat
            .front()
            .is_some_and(|&t| now.duration_since(t) >= CHAT_WINDOW)
        {
            self.chat.pop_front();
        }
        if self.chat.len() < CHAT_LINES {
            self.chat.push_back(now);
            true
        } else {
            false
        }
    }
}
