//! The connection to a server: a WebSocket served by its own thread, so
//! that the menus and the game only ever poll a channel and never wait on
//! the network.
//!
//! A server's address says whether the connection is encrypted
//! ([`ServerAddress`]). An encrypted one is TLS, with the server's
//! certificate checked against the roots of trust ([`Trust`]) and for the
//! server's name; a certificate that fails either is refused, with the
//! reason ([`BadCertificate`]), and nothing is sent. There is no way to go
//! on regardless, and no plain connection in its place.

use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, OnceLock, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use rustls::pki_types::ServerName;
use soundrts_protocol::game::{SeatFrame, decode_frame};
use soundrts_protocol::{
    ClientMsg, DEFAULT_PORT, Invalid, LobbySnapshot, MAX_MESSAGE_BYTES, Rejection, ServerMsg, Wire,
};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc as tokio_mpsc;
use tokio_tungstenite::WebSocketStream;
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
pub struct Connection {
    link: Box<dyn Link>,
    transport: Transport,
}

impl Connection {
    /// A connection on `link`, which crosses no network or does not say.
    pub fn new(link: Box<dyn Link>) -> Connection {
        Connection::over(link, Transport::Plain)
    }

    /// A connection on `link`, opened to an address of this transport.
    pub fn over(link: Box<dyn Link>, transport: Transport) -> Connection {
        Connection { link, transport }
    }

    /// Whether what it carries is encrypted on the network.
    pub fn transport(&self) -> Transport {
        self.transport
    }

    pub fn send(&mut self, msg: ClientMsg) {
        self.link.send(Outgoing::Msg(msg));
    }

    /// Sends an encoded command of the player.
    pub fn send_command(&mut self, bytes: Vec<u8>) {
        self.link.send(Outgoing::Command(bytes));
    }

    pub fn poll(&mut self) -> Option<NetEvent> {
        self.link.poll()
    }

    /// Closes the connection: nothing more is sent or received.
    pub fn close(&mut self) {
        self.link = Box::new(ClosedLink);
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
    /// Nothing answered, or not in time.
    Unreachable(String),
    /// Something answered, but not as a SoundRTS server does.
    NotAServer(String),
    /// The server's certificate does not prove it is the server asked
    /// for: nothing was sent.
    Certificate(BadCertificate),
    Rejected(Rejection),
}

/// What is wrong with a server's certificate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BadCertificate {
    /// It is for another name than the server's.
    OtherName,
    /// It has expired, or is not valid yet.
    OutOfDate,
    /// No root of trust vouches for it, or it is not valid otherwise.
    NotTrusted,
}

impl BadCertificate {
    fn of(e: &rustls::CertificateError) -> BadCertificate {
        use rustls::CertificateError as E;
        match e {
            E::NotValidForName | E::NotValidForNameContext { .. } => BadCertificate::OtherName,
            E::Expired
            | E::ExpiredContext { .. }
            | E::NotValidYet
            | E::NotValidYetContext { .. } => BadCertificate::OutOfDate,
            _ => BadCertificate::NotTrusted,
        }
    }
}

/// Whether what a connection carries is encrypted on the network.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    Plain,
    Encrypted,
}

/// The port of an encrypted server with none given: a web server's, as a
/// server on the Internet listens behind a proxy that speaks TLS.
pub const ENCRYPTED_PORT: u16 = 443;

/// Where a server is, and whether the connection to it is encrypted: a
/// player types it, a list of servers holds it as text
/// ([`ServerAddress::parse`], [`fmt::Display`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerAddress {
    /// `ws://`, or no scheme: what is sent crosses the network as it is.
    /// The port is [`DEFAULT_PORT`] unless given.
    Plain(Endpoint),
    /// `wss://`: TLS, with the certificate checked against the roots of
    /// trust and for the host's name. The port is [`ENCRYPTED_PORT`]
    /// unless given.
    Encrypted(Endpoint),
}

/// A host, by name or IP address, and a port.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    host: ServerName<'static>,
    port: u16,
}

impl Endpoint {
    /// `host`, `host:port`, an IP address, `ip:port` or `[ipv6]:port`.
    fn parse(text: &str, default_port: u16) -> Option<Endpoint> {
        if let Ok(a) = text.parse::<SocketAddr>() {
            return Endpoint::of_ip(a.ip(), a.port());
        }
        let bare = text
            .strip_prefix('[')
            .and_then(|t| t.strip_suffix(']'))
            .unwrap_or(text);
        if let Ok(ip) = bare.parse::<IpAddr>() {
            return Endpoint::of_ip(ip, default_port);
        }
        let (host, port) = match text.rsplit_once(':') {
            Some((h, p)) => (h, p.parse::<u16>().ok()?),
            None => (text, default_port),
        };
        let allowed = |b: u8| b.is_ascii_alphanumeric() || b == b'.' || b == b'-';
        if port == 0 || host.is_empty() || !host.bytes().all(allowed) {
            return None;
        }
        let host = ServerName::try_from(host.to_owned()).ok()?;
        Some(Endpoint { host, port })
    }

    fn of_ip(ip: IpAddr, port: u16) -> Option<Endpoint> {
        (port != 0).then(|| Endpoint {
            host: ServerName::IpAddress(ip.into()),
            port,
        })
    }

    /// The host as a URL holds it: an IPv6 address in brackets.
    fn url_host(&self) -> String {
        let host = self.host.to_str();
        if host.contains(':') {
            format!("[{host}]")
        } else {
            host.into_owned()
        }
    }

    /// As text, the port left out if it is `default_port`.
    fn write(&self, f: &mut fmt::Formatter<'_>, default_port: u16) -> fmt::Result {
        f.write_str(&self.url_host())?;
        if self.port != default_port {
            write!(f, ":{}", self.port)?;
        }
        Ok(())
    }
}

impl ServerAddress {
    /// The address a player typed, or a list holds: `wss://` and an
    /// endpoint for an encrypted connection; an endpoint alone, or after
    /// `ws://`, for a plain one. `None` if it is not one.
    pub fn parse(text: &str) -> Option<ServerAddress> {
        let text = text.trim();
        let (encrypted, rest) = match text.split_once("://") {
            Some((scheme, rest)) if scheme.eq_ignore_ascii_case("wss") => (true, rest),
            Some((scheme, rest)) if scheme.eq_ignore_ascii_case("ws") => (false, rest),
            Some(_) => return None,
            None => (false, text),
        };
        let rest = rest.strip_suffix('/').unwrap_or(rest);
        Some(if encrypted {
            ServerAddress::Encrypted(Endpoint::parse(rest, ENCRYPTED_PORT)?)
        } else {
            ServerAddress::Plain(Endpoint::parse(rest, DEFAULT_PORT)?)
        })
    }

    pub fn transport(&self) -> Transport {
        match self {
            ServerAddress::Plain(_) => Transport::Plain,
            ServerAddress::Encrypted(_) => Transport::Encrypted,
        }
    }
}

impl From<SocketAddr> for ServerAddress {
    /// A server on the local network, or this machine: plain.
    fn from(a: SocketAddr) -> ServerAddress {
        ServerAddress::Plain(Endpoint {
            host: ServerName::IpAddress(a.ip().into()),
            port: a.port(),
        })
    }
}

impl fmt::Display for ServerAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ServerAddress::Plain(e) => e.write(f, DEFAULT_PORT),
            ServerAddress::Encrypted(e) => {
                f.write_str("wss://")?;
                e.write(f, ENCRYPTED_PORT)
            }
        }
    }
}

/// The roots of trust an encrypted connection checks a server's
/// certificate against, with the TLS settings.
#[derive(Clone)]
pub struct Trust(Arc<rustls::ClientConfig>);

impl Trust {
    /// The roots built into the game: Mozilla's, as the `webpki-roots`
    /// crate carries them, the same on every machine.
    pub fn built_in() -> Trust {
        static BUILT_IN: OnceLock<Trust> = OnceLock::new();
        BUILT_IN
            .get_or_init(|| {
                Trust::only(rustls::RootCertStore {
                    roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
                })
            })
            .clone()
    }

    /// Only these roots.
    pub fn only(roots: rustls::RootCertStore) -> Trust {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        #[expect(
            clippy::expect_used,
            reason = "ring supports rustls's default protocol versions"
        )]
        let config = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("the default versions")
            .with_root_certificates(roots)
            .with_no_client_auth();
        Trust(Arc::new(config))
    }
}

impl Default for Trust {
    fn default() -> Trust {
        Trust::built_in()
    }
}

impl fmt::Debug for Trust {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Trust")
    }
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
/// it is lost, the same way.
#[derive(Clone, Debug)]
pub struct Dialer {
    pub address: ServerAddress,
    /// What an encrypted connection trusts.
    pub trust: Trust,
    pub hello: ClientMsg,
    /// How long to wait for the server, then as long for its welcome.
    pub timeout: Duration,
    pub liveness: Liveness,
}

impl Dialer {
    /// Starts opening a connection: [`Opening::poll`] says when it is open.
    pub fn start(&self) -> Opening {
        start(self.clone())
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
pub fn open(address: &ServerAddress, hello: ClientMsg, timeout: Duration) -> Opened {
    Dialer {
        address: address.clone(),
        trust: Trust::built_in(),
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

/// Starts the thread that connects as `dialer` says.
fn start(dialer: Dialer) -> Opening {
    let timeout = dialer.timeout;
    let transport = dialer.address.transport();
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
            serve(dialer, channels);
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
            transport,
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
            let transport = link.transport;
            Ok((Connection::over(Box::new(link), transport), lobby))
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
    transport: Transport,
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

fn serve(dialer: Dialer, channels: Channels) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            let _ = channels
                .connected
                .send(Err(ConnectError::Unreachable(e.to_string())));
            return;
        }
    };
    runtime.block_on(async move {
        let config = WebSocketConfig::default()
            .max_message_size(Some(MAX_MESSAGE_BYTES))
            .max_frame_size(Some(MAX_MESSAGE_BYTES));
        match &dialer.address {
            ServerAddress::Plain(at) => {
                let url = format!("ws://{}:{}", at.url_host(), at.port);
                let connect = async move {
                    tokio_tungstenite::connect_async_with_config(url, Some(config), true)
                        .await
                        .map(|(ws, _)| ws)
                        .map_err(handshake_failed)
                };
                session(connect, &dialer, channels).await;
            }
            ServerAddress::Encrypted(at) => {
                let connect = encrypted(at, &dialer.trust, config);
                session(connect, &dialer, channels).await;
            }
        }
    });
    // A host name still being looked up, on the runtime's blocking pool,
    // is not waited for.
    runtime.shutdown_background();
}

/// Why the WebSocket handshake failed.
fn handshake_failed(e: tokio_tungstenite::tungstenite::Error) -> ConnectError {
    match e {
        tokio_tungstenite::tungstenite::Error::Io(e) => ConnectError::Unreachable(e.to_string()),
        e => ConnectError::NotAServer(e.to_string()),
    }
}

/// Connects to `at` with TLS, checks its certificate against `trust` and
/// for its name, then opens the WebSocket.
async fn encrypted(
    at: &Endpoint,
    trust: &Trust,
    config: WebSocketConfig,
) -> Result<WebSocketStream<tokio_rustls::client::TlsStream<tokio::net::TcpStream>>, ConnectError> {
    let tcp = tokio::net::TcpStream::connect((at.host.to_str().as_ref(), at.port))
        .await
        .map_err(|e| ConnectError::Unreachable(e.to_string()))?;
    let _ = tcp.set_nodelay(true);
    let tls = tokio_rustls::TlsConnector::from(Arc::clone(&trust.0))
        .connect(at.host.clone(), tcp)
        .await
        .map_err(tls_failed)?;
    let url = format!("wss://{}:{}", at.url_host(), at.port);
    tokio_tungstenite::client_async_with_config(url, tls, Some(config))
        .await
        .map(|(ws, _)| ws)
        .map_err(handshake_failed)
}

/// Why TLS could not be set up: a certificate refused, a server that does
/// not speak it, or the network.
fn tls_failed(e: std::io::Error) -> ConnectError {
    match e
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>())
    {
        Some(rustls::Error::InvalidCertificate(c)) => {
            ConnectError::Certificate(BadCertificate::of(c))
        }
        Some(other) => ConnectError::NotAServer(other.to_string()),
        None => ConnectError::Unreachable(e.to_string()),
    }
}

/// Opens the WebSocket with `connect`, says the dialer's hello, then
/// carries the connection until it ends.
async fn session<S, C>(connect: C, dialer: &Dialer, channels: Channels)
where
    S: AsyncRead + AsyncWrite + Unpin,
    C: std::future::Future<Output = Result<WebSocketStream<S>, ConnectError>>,
{
    let Channels {
        mut out,
        events,
        connected,
    } = channels;
    let (timeout, liveness) = (dialer.timeout, dialer.liveness);
    // A client that gives the connection up meanwhile waits for none of
    // it.
    let connecting = tokio::select! {
        c = tokio::time::timeout(timeout, connect) => c,
        _ = events.closed() => return,
    };
    let mut ws = match connecting {
        Ok(Ok(ws)) => ws,
        Ok(Err(error)) => {
            let _ = connected.send(Err(error));
            return;
        }
        Err(_) => {
            let _ = connected.send(Err(ConnectError::Unreachable("timed out".into())));
            return;
        }
    };
    let hello = Message::Text(dialer.hello.to_json().into());
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
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// The address as text, once read: `None` if it is not one.
    fn read(text: &str) -> Option<String> {
        ServerAddress::parse(text).map(|a| a.to_string())
    }

    #[test]
    fn typed_addresses_are_plain_unless_they_ask_for_encryption() {
        let plain = |text: &str| ServerAddress::parse(text).map(|a| a.transport());
        assert_eq!(plain("1.2.3.4"), Some(Transport::Plain));
        assert_eq!(plain("game.example.org:2502"), Some(Transport::Plain));
        assert_eq!(plain("ws://game.example.org"), Some(Transport::Plain));
        assert_eq!(plain("wss://game.example.org"), Some(Transport::Encrypted));
        assert_eq!(
            plain("WSS://game.example.org:8443"),
            Some(Transport::Encrypted)
        );
        assert_eq!(read("1.2.3.4").as_deref(), Some("1.2.3.4"));
        assert_eq!(read(" 1.2.3.4:99 ").as_deref(), Some("1.2.3.4:99"));
        assert_eq!(read("1.2.3.4:2501").as_deref(), Some("1.2.3.4"));
        assert_eq!(
            read("game.example.org").as_deref(),
            Some("game.example.org")
        );
        assert_eq!(read("::1").as_deref(), Some("[::1]"));
        assert_eq!(read("[::1]:7").as_deref(), Some("[::1]:7"));
        assert_eq!(
            read("wss://g.example.org/").as_deref(),
            Some("wss://g.example.org")
        );
        assert_eq!(
            read("wss://g.example.org:443").as_deref(),
            Some("wss://g.example.org")
        );
        assert_eq!(
            read("wss://g.example.org:2501").as_deref(),
            Some("wss://g.example.org:2501")
        );
        assert_eq!(read("wss://[::1]").as_deref(), Some("wss://[::1]"));
        for bad in [
            "",
            "a b",
            "host:port",
            "host:0",
            "wss://",
            "https://g.example.org",
            "wss://g.example.org/path",
            "wss://user@g.example.org",
        ] {
            assert_eq!(read(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn an_address_read_back_is_the_same() {
        for text in [
            "1.2.3.4",
            "[::1]:7",
            "g.example.org:9",
            "wss://g.example.org",
            "wss://[::1]:8443",
        ] {
            let a = ServerAddress::parse(text).unwrap();
            assert_eq!(ServerAddress::parse(&a.to_string()), Some(a), "{text}");
        }
    }
}
