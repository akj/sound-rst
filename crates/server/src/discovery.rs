//! Answers clients looking for servers on the local network.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use soundrts_protocol::{
    DISCOVERY_REQUEST, PROTOCOL_VERSION, ServerAnnouncement, ServerName, Wire,
};
use tokio::net::UdpSocket;
use tokio::time::Instant;
use tracing::debug;

/// The answers the server gives at once, then how many a second. A client
/// asks once each time its player opens the list of servers, so even a
/// room full of players opening it together stays under the burst. A
/// request with a forged sender gets no answer beyond these.
const ANSWER_BURST: f64 = 20.0;
const ANSWERS_PER_SECOND: f64 = 5.0;

/// Answers discovery requests on `socket` for a server on TCP `port`.
pub(crate) fn spawn(socket: UdpSocket, name: ServerName, port: u16, players: Arc<AtomicU32>) {
    tokio::spawn(async move {
        let mut buf = [0u8; 64];
        let mut tokens = ANSWER_BURST;
        let mut refilled = Instant::now();
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
            let now = Instant::now();
            let elapsed = now.duration_since(refilled).as_secs_f64();
            tokens = (tokens + elapsed * ANSWERS_PER_SECOND).min(ANSWER_BURST);
            refilled = now;
            if tokens < 1.0 {
                debug!(%from, "too many discovery requests; not answered");
                continue;
            }
            tokens -= 1.0;
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
