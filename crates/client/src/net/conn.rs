//! The connection to a server: a WebSocket served by its own thread, so
//! that the menus and the game only ever poll a channel and never wait on
//! the network.

use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use soundrts_protocol::{
    ClientMsg, DEFAULT_PORT, LobbySnapshot, MAX_MESSAGE_BYTES, Rejection, ServerMsg, Wire,
};
use tokio::sync::mpsc as tokio_mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

/// What comes from the server.
#[derive(Clone, Debug, PartialEq)]
pub enum NetEvent {
    Msg(ServerMsg),
    /// The connection is over, for this reason.
    Closed(String),
}

/// Both ends of a link to a server: a real socket, or a fake one in tests.
pub trait Link {
    fn send(&mut self, msg: ClientMsg);
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
        self.0.send(msg);
    }

    pub fn poll(&mut self) -> Option<NetEvent> {
        self.0.poll()
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

/// Connects to `address`, says `hello`, and waits for the server's
/// welcome, at most `timeout`.
pub fn open(
    address: &str,
    hello: ClientMsg,
    timeout: Duration,
) -> Result<(Connection, LobbySnapshot), ConnectError> {
    let url = ws_url(address).ok_or(ConnectError::BadAddress)?;
    let (events_tx, events_rx) = mpsc::channel();
    let (out_tx, out_rx) = tokio_mpsc::unbounded_channel();
    let (connected_tx, connected_rx) = mpsc::channel();
    let thread = std::thread::Builder::new()
        .name("soundrts-connection".into())
        .spawn(move || serve(url, hello, timeout, out_rx, events_tx, connected_tx))
        .map_err(|e| ConnectError::Unreachable(e.to_string()))?;
    let mut link = SocketLink {
        out: Some(out_tx),
        events: events_rx,
        thread: Some(thread),
    };
    // The thread gives up on its own at the timeout; the margin covers a
    // slow thread start.
    let wait = timeout + Duration::from_secs(1);
    match connected_rx.recv_timeout(wait) {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e),
        Err(_) => return Err(ConnectError::Unreachable("timed out".into())),
    }
    match link.events.recv_timeout(wait) {
        Ok(NetEvent::Msg(ServerMsg::Welcome { lobby })) => {
            Ok((Connection::new(Box::new(link)), lobby))
        }
        Ok(NetEvent::Msg(ServerMsg::Rejected { reason })) => Err(ConnectError::Rejected(reason)),
        Ok(NetEvent::Msg(other)) => Err(ConnectError::NotAServer(format!(
            "unexpected first message: {other:?}"
        ))),
        Ok(NetEvent::Closed(reason)) => Err(ConnectError::NotAServer(reason)),
        Err(_) => {
            link.out = None;
            Err(ConnectError::NotAServer("no welcome in time".into()))
        }
    }
}

struct SocketLink {
    out: Option<tokio_mpsc::UnboundedSender<ClientMsg>>,
    events: mpsc::Receiver<NetEvent>,
    thread: Option<JoinHandle<()>>,
}

impl Link for SocketLink {
    fn send(&mut self, msg: ClientMsg) {
        if let Some(out) = &self.out {
            let _ = out.send(msg);
        }
    }

    fn poll(&mut self) -> Option<NetEvent> {
        self.events.try_recv().ok()
    }
}

impl Drop for SocketLink {
    fn drop(&mut self) {
        // Without a sender, the thread closes the socket and ends.
        self.out = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// How long to wait for the server to answer a close.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(1);

fn serve(
    url: String,
    hello: ClientMsg,
    timeout: Duration,
    mut out: tokio_mpsc::UnboundedReceiver<ClientMsg>,
    events: mpsc::Sender<NetEvent>,
    connected: mpsc::Sender<Result<(), ConnectError>>,
) {
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
        let mut ws = match tokio::time::timeout(timeout, connect).await {
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
        if let Err(e) = ws.send(Message::Text(hello.to_json().into())).await {
            let _ = connected.send(Err(ConnectError::NotAServer(e.to_string())));
            return;
        }
        let _ = connected.send(Ok(()));
        let reason = loop {
            tokio::select! {
                incoming = ws.next() => match incoming {
                    Some(Ok(Message::Text(text))) => match ServerMsg::from_json(&text) {
                        Ok(msg) => {
                            if events.send(NetEvent::Msg(msg)).is_err() {
                                break None;
                            }
                        }
                        Err(e) => {
                            break Some(format!("cannot read a message from the server: {e}"));
                        }
                    },
                    Some(Ok(Message::Close(_))) | None => break Some("closed by the server".into()),
                    Some(Ok(_)) => {}
                    Some(Err(e)) => break Some(e.to_string()),
                },
                msg = out.recv() => match msg {
                    Some(msg) => {
                        if let Err(e) = ws.send(Message::Text(msg.to_json().into())).await {
                            break Some(e.to_string());
                        }
                    }
                    None => {
                        let _ = tokio::time::timeout(CLOSE_TIMEOUT, async {
                            if ws.close(None).await.is_ok() {
                                while let Some(Ok(_)) = ws.next().await {}
                            }
                        })
                        .await;
                        break None;
                    }
                },
            }
        };
        if let Some(reason) = reason {
            let _ = events.send(NetEvent::Closed(reason));
        }
    });
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
