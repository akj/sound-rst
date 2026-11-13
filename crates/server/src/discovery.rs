//! Answers clients looking for servers on the local network.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use soundrts_protocol::{DISCOVERY_REQUEST, PROTOCOL_VERSION, ServerAnnouncement, Wire};
use tokio::net::UdpSocket;
use tracing::debug;

/// Answers discovery requests on `socket` for a server on TCP `port`.
pub(crate) fn spawn(socket: UdpSocket, name: String, port: u16, players: Arc<AtomicU32>) {
    tokio::spawn(async move {
        let mut buf = [0u8; 64];
        loop {
            let (n, from) = match socket.recv_from(&mut buf).await {
                Ok(received) => received,
                // Windows reports an earlier answer that did not arrive as
                // an error here; the socket still works.
                Err(e) => {
                    debug!(error = %e, "discovery receive failed");
                    continue;
                }
            };
            if &buf[..n] != DISCOVERY_REQUEST {
                continue;
            }
            let answer = ServerAnnouncement {
                protocol: PROTOCOL_VERSION,
                name: name.clone(),
                port,
                players: players.load(Ordering::Relaxed),
            };
            if let Err(e) = socket.send_to(answer.to_json().as_bytes(), from).await {
                debug!(%from, error = %e, "discovery answer failed");
            }
        }
    });
}
