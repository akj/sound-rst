//! Finds the servers of the local network: a UDP broadcast that every
//! server answers.

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use soundrts_protocol::{
    DEFAULT_PORT, DISCOVERY_REQUEST, PROTOCOL_VERSION, ServerAnnouncement, Wire,
};

/// A server that answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoundServer {
    /// Where players connect: the address that answered, with the port
    /// the server announced.
    pub addr: SocketAddr,
    pub name: String,
    pub players: u32,
}

/// Where the request goes by default: every machine of the local network.
pub fn broadcast_target() -> SocketAddr {
    SocketAddr::from((Ipv4Addr::BROADCAST, DEFAULT_PORT))
}

/// Sends the request to each target, then collects every answer for
/// `wait`. Servers of another protocol version are left out; a server
/// that answers twice is listed once.
pub fn discover(targets: &[SocketAddr], wait: Duration) -> Vec<FoundServer> {
    if targets.is_empty() {
        return Vec::new();
    }
    // Only on this machine when only this machine is asked.
    let local = targets.iter().all(|t| t.ip().is_loopback());
    let ip = if local {
        Ipv4Addr::LOCALHOST
    } else {
        Ipv4Addr::UNSPECIFIED
    };
    let Ok(socket) = UdpSocket::bind((ip, 0)) else {
        return Vec::new();
    };
    let _ = socket.set_broadcast(true);
    for t in targets {
        let _ = socket.send_to(DISCOVERY_REQUEST, t);
    }
    let end = Instant::now() + wait;
    let mut found: Vec<FoundServer> = Vec::new();
    let mut buf = [0u8; 2048];
    loop {
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() || socket.set_read_timeout(Some(left)).is_err() {
            break;
        }
        let (n, from) = match socket.recv_from(&mut buf) {
            Ok(received) => received,
            // A timeout, or on Windows a request that reached no one.
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                break;
            }
            Err(_) => continue,
        };
        let Ok(text) = std::str::from_utf8(&buf[..n]) else {
            continue;
        };
        let Ok(a) = ServerAnnouncement::from_json(text) else {
            continue;
        };
        if a.protocol != PROTOCOL_VERSION {
            continue;
        }
        let server = FoundServer {
            addr: SocketAddr::new(from.ip(), a.port),
            name: a.name,
            players: a.players,
        };
        if !found.iter().any(|f| f.addr == server.addr) {
            found.push(server);
        }
    }
    found
}
