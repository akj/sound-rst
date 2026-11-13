//! The SoundRTS multiplayer server: a lobby, a chat and a turn clock.
//!
//! The server never runs the simulation. Clients run it from the seed the
//! server gives them, and the server tells them which orders to queue at
//! each turn. See `soundrts-protocol` for the messages.
//!
//! [`start`] runs a server on its own thread with its own runtime, so a
//! program that is not async can embed it. The `soundrts-server` binary is
//! a thin wrapper around it.
//!
//! Inside, each connection has one task, one lobby task owns the players,
//! rooms, chat history and map cache, and each running game has one task
//! that owns its turn clock, orders and digests.

mod connection;
mod discovery;
mod game;
mod lobby;
mod room;
mod upnp;

use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::thread::JoinHandle;
use std::time::Duration;

use soundrts_protocol::DEFAULT_PORT;
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::watch;
use tokio::task::JoinSet;
use tracing::{info, warn};

pub use upnp::UpnpOutcome;

/// How a server runs. [`Default`] gives the values of a server players host.
#[derive(Clone, Debug)]
pub struct ServerConfig {
    /// Said to players who find the server on the local network.
    pub name: String,
    /// The address to listen on: all interfaces by default.
    pub ip: IpAddr,
    /// The TCP port for players, and the UDP port for local discovery.
    /// 0 picks a free port; [`ServerHandle::local_addr`] tells which.
    pub port: u16,
    /// The most players on the server at once.
    pub max_clients: usize,
    /// The most rooms, games included, at once.
    pub max_rooms: usize,
    /// How many turns the clock may run ahead of the slowest player
    /// before the game waits for them.
    pub max_lead: u32,
    /// How long one player may keep a game waiting before they are taken
    /// out of it.
    pub lag_timeout: Duration,
    /// How long a new connection has to say hello.
    pub hello_timeout: Duration,
    /// Ask the router to forward the port (UPnP).
    pub upnp: bool,
}

impl Default for ServerConfig {
    fn default() -> ServerConfig {
        ServerConfig {
            name: "SoundRTS".into(),
            ip: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            port: DEFAULT_PORT,
            max_clients: 40,
            max_rooms: 10,
            max_lead: 10,
            lag_timeout: Duration::from_secs(60),
            hello_timeout: Duration::from_secs(10),
            upnp: false,
        }
    }
}

/// A running server. Dropping it stops the server, as [`shutdown`] does.
///
/// [`shutdown`]: ServerHandle::shutdown
pub struct ServerHandle {
    addr: SocketAddr,
    upnp: UpnpOutcome,
    stop: watch::Sender<bool>,
    thread: Option<JoinHandle<()>>,
}

impl ServerHandle {
    /// The address players connect to on this machine.
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// What the router said when asked to forward the port.
    pub fn upnp(&self) -> &UpnpOutcome {
        &self.upnp
    }

    /// Closes every connection, removes the router's port mapping, and
    /// returns when the server has stopped.
    pub fn shutdown(mut self) {
        self.stop_and_join();
    }

    fn stop_and_join(&mut self) {
        let _ = self.stop.send(true);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.stop_and_join();
    }
}

/// Starts a server on a new thread and returns once it listens. With
/// `config.upnp`, it also returns only after the router answered or the
/// search timed out (a few seconds).
///
/// Callable from any thread, async or not.
pub fn start(config: ServerConfig) -> io::Result<ServerHandle> {
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (stop_tx, stop_rx) = watch::channel(false);
    let thread = std::thread::Builder::new()
        .name("soundrts-server".into())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("soundrts-server-worker")
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            runtime.block_on(run(config, stop_rx, ready_tx));
            runtime.shutdown_timeout(Duration::from_secs(1));
        })?;
    match ready_rx.recv() {
        Ok(Ok((addr, upnp))) => Ok(ServerHandle {
            addr,
            upnp,
            stop: stop_tx,
            thread: Some(thread),
        }),
        Ok(Err(e)) => {
            let _ = thread.join();
            Err(e)
        }
        Err(_) => {
            let _ = thread.join();
            Err(io::Error::other(
                "the server thread stopped before it listened",
            ))
        }
    }
}

type Ready = std::sync::mpsc::Sender<io::Result<(SocketAddr, UpnpOutcome)>>;

/// How long connections get to close cleanly when the server stops.
const CLOSE_GRACE: Duration = Duration::from_secs(2);

/// How many ports to try, when any will do, for one free on both TCP and
/// UDP. On Windows a free TCP port can be reserved for UDP.
const ANY_PORT_TRIES: usize = 20;

/// The ports a server on "any port" picks from.
const DYNAMIC_PORTS: std::ops::RangeInclusive<u16> = 49152..=65535;

/// Listens on TCP, and on UDP at the same port for local discovery. A
/// server without discovery still works, so a UDP failure is a warning.
///
/// For any port, the tries are random rather than the system's choice:
/// the system hands out ports in sequence, and Windows reserves blocks of
/// a hundred ports for one protocol only, so a TCP port at the start of a
/// block reserved for UDP is followed by many more of the same block.
async fn bind(ip: IpAddr, port: u16) -> io::Result<(TcpListener, SocketAddr, Option<UdpSocket>)> {
    if port == 0 {
        for _ in 0..ANY_PORT_TRIES {
            let port = rand::random_range(DYNAMIC_PORTS);
            let Ok(listener) = TcpListener::bind((ip, port)).await else {
                continue;
            };
            let addr = listener.local_addr()?;
            if let Ok(udp) = UdpSocket::bind(addr).await {
                return Ok((listener, addr, Some(udp)));
            }
        }
    }
    let listener = TcpListener::bind((ip, port)).await?;
    let addr = listener.local_addr()?;
    match UdpSocket::bind(addr).await {
        Ok(udp) => Ok((listener, addr, Some(udp))),
        Err(e) => {
            warn!(%addr, error = %e, "no local discovery: cannot listen on UDP");
            Ok((listener, addr, None))
        }
    }
}

async fn run(config: ServerConfig, mut stop: watch::Receiver<bool>, ready: Ready) {
    let (listener, addr, udp) = match bind(config.ip, config.port).await {
        Ok(bound) => bound,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    let players = Arc::new(AtomicU32::new(0));
    if let Some(udp) = udp {
        discovery::spawn(udp, config.name.clone(), addr.port(), players.clone());
    }
    let (upnp, mapping) = if config.upnp {
        upnp::map(addr.port()).await
    } else {
        (UpnpOutcome::Off, None)
    };
    info!(%addr, name = %config.name, "server started");
    let lobby = lobby::spawn(&config, players);
    if ready.send(Ok((addr, upnp))).is_err() {
        return;
    }

    let mut connections = JoinSet::new();
    let mut next_conn: connection::ConnId = 0;
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, peer)) => {
                    next_conn += 1;
                    connections.spawn(connection::serve(
                        stream,
                        peer,
                        next_conn,
                        lobby.clone(),
                        stop.clone(),
                        config.hello_timeout,
                    ));
                }
                Err(e) => {
                    warn!(error = %e, "cannot accept a connection");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            },
            Some(_) = connections.join_next(), if !connections.is_empty() => {}
            _ = stop.changed() => break,
        }
    }
    drop(listener);
    let _ = tokio::time::timeout(CLOSE_GRACE, async {
        while connections.join_next().await.is_some() {}
    })
    .await;
    if let Some(mapping) = mapping {
        mapping.remove().await;
    }
    info!("server stopped");
}
