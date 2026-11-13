//! Asks the router to forward the server's TCP port (UPnP), keeps the
//! mapping alive, and removes it when the server stops.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use igd_next::aio::Gateway;
use igd_next::aio::tokio::{Tokio, search_gateway};
use igd_next::{AddPortError, PortMappingProtocol, SearchOptions};
use tokio::task::JoinHandle;
use tracing::{info, warn};

/// What came of asking the router to forward the port.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpnpOutcome {
    /// The server did not ask.
    Off,
    /// Players on the internet reach the server at this address.
    Mapped { external: SocketAddr },
    /// No mapping; players on the internet may not reach the server.
    Failed { reason: String },
}

const SEARCH_TIMEOUT: Duration = Duration::from_secs(5);
const LEASE_SECONDS: u32 = 3600;
const DESCRIPTION: &str = "SoundRTS server";

pub(crate) struct Mapping {
    gateway: Gateway<Tokio>,
    port: u16,
    renewal: Option<JoinHandle<()>>,
}

impl Mapping {
    pub(crate) async fn remove(self) {
        if let Some(renewal) = self.renewal {
            renewal.abort();
        }
        match self
            .gateway
            .remove_port(PortMappingProtocol::TCP, self.port)
            .await
        {
            Ok(()) => info!(port = self.port, "UPnP mapping removed"),
            Err(e) => warn!(port = self.port, error = %e, "cannot remove the UPnP mapping"),
        }
    }
}

/// Maps `port` on the router to the same port on this machine.
pub(crate) async fn map(port: u16) -> (UpnpOutcome, Option<Mapping>) {
    match try_map(port).await {
        Ok((external, mapping)) => {
            info!(%external, "UPnP mapping added");
            (UpnpOutcome::Mapped { external }, Some(mapping))
        }
        Err(reason) => {
            warn!(%reason, "no UPnP mapping");
            (UpnpOutcome::Failed { reason }, None)
        }
    }
}

async fn try_map(port: u16) -> Result<(SocketAddr, Mapping), String> {
    let mut options = SearchOptions::default();
    options.timeout = Some(SEARCH_TIMEOUT);
    let gateway = search_gateway(options)
        .await
        .map_err(|e| format!("no router found: {e}"))?;
    let local = SocketAddr::new(local_ip_towards(gateway.addr)?, port);
    let lease = match add(&gateway, port, local, LEASE_SECONDS).await {
        Ok(()) => LEASE_SECONDS,
        Err(AddPortError::OnlyPermanentLeasesSupported) => {
            add(&gateway, port, local, 0)
                .await
                .map_err(|e| format!("the router refused the mapping: {e}"))?;
            0
        }
        Err(e) => return Err(format!("the router refused the mapping: {e}")),
    };
    let external_ip = match gateway.get_external_ip().await {
        Ok(ip) => ip,
        Err(e) => {
            let _ = gateway.remove_port(PortMappingProtocol::TCP, port).await;
            return Err(format!("the router did not give its address: {e}"));
        }
    };
    let renewal = (lease > 0).then(|| {
        let gateway = gateway.clone();
        tokio::spawn(async move {
            let every = Duration::from_secs(u64::from(lease / 2));
            loop {
                tokio::time::sleep(every).await;
                if let Err(e) = add(&gateway, port, local, lease).await {
                    warn!(error = %e, "cannot renew the UPnP mapping");
                }
            }
        })
    });
    let mapping = Mapping {
        gateway,
        port,
        renewal,
    };
    Ok((SocketAddr::new(external_ip, port), mapping))
}

async fn add(
    gateway: &Gateway<Tokio>,
    port: u16,
    local: SocketAddr,
    lease: u32,
) -> Result<(), AddPortError> {
    gateway
        .add_port(PortMappingProtocol::TCP, port, local, lease, DESCRIPTION)
        .await
}

/// This machine's address on the network that reaches `gateway`.
fn local_ip_towards(gateway: SocketAddr) -> Result<IpAddr, String> {
    let socket = std::net::UdpSocket::bind(match gateway {
        SocketAddr::V4(_) => "0.0.0.0:0",
        SocketAddr::V6(_) => "[::]:0",
    })
    .map_err(|e| format!("cannot find this machine's address: {e}"))?;
    socket
        .connect(gateway)
        .and_then(|()| socket.local_addr())
        .map(|a| a.ip())
        .map_err(|e| format!("cannot find this machine's address: {e}"))
}
