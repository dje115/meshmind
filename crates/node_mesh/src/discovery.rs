//! mDNS-based LAN discovery using mdns-sd.
//!
//! Each node registers a service `_meshmind._tcp.local.` with its node_id and port.
//! A background task browses for peers and auto-populates the PeerDirectory.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use crate::peer_dir::PeerDirectory;

const SERVICE_TYPE: &str = "_meshmind._tcp.local.";
const PROP_NODE_ID: &str = "node_id";
const PROP_VERSION: &str = "version";

/// Register this node on mDNS so other LAN peers can discover it.
pub fn register_service(daemon: &ServiceDaemon, node_id: &str, port: u16) -> Result<()> {
    let host = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "meshmind-node".into());

    let instance_name = format!("{node_id}.{host}");

    let properties = [
        (PROP_NODE_ID.to_string(), node_id.to_string()),
        (
            PROP_VERSION.to_string(),
            env!("CARGO_PKG_VERSION").to_string(),
        ),
    ];

    let service = ServiceInfo::new(
        SERVICE_TYPE,
        &instance_name,
        &format!("{host}.local."),
        "",
        port,
        &properties[..],
    )
    .context("create mDNS service info")?;

    daemon.register(service).context("register mDNS service")?;

    info!("mDNS: registered {node_id} on port {port}");
    Ok(())
}

/// Browse for peers on the LAN and update the PeerDirectory.
/// This spawns a background task that runs until the returned handle is dropped.
pub fn start_discovery(
    daemon: &ServiceDaemon,
    peer_dir: Arc<RwLock<PeerDirectory>>,
    own_node_id: String,
) -> Result<tokio::task::JoinHandle<()>> {
    let receiver = daemon.browse(SERVICE_TYPE).context("start mDNS browse")?;

    let handle = tokio::spawn(async move {
        loop {
            match tokio::time::timeout(Duration::from_secs(5), async { receiver.recv().ok() }).await
            {
                Ok(Some(event)) => {
                    handle_event(&peer_dir, &own_node_id, event).await;
                }
                Ok(None) => {
                    debug!("mDNS browse channel closed");
                    break;
                }
                Err(_) => {
                    // Timeout, just loop again
                }
            }
        }
    });

    Ok(handle)
}

async fn handle_event(peer_dir: &RwLock<PeerDirectory>, own_node_id: &str, event: ServiceEvent) {
    match event {
        ServiceEvent::ServiceResolved(info) => {
            let node_id = info
                .get_properties()
                .get(PROP_NODE_ID)
                .map(|v| v.val_str().to_string())
                .unwrap_or_default();

            if node_id.is_empty() || node_id == own_node_id {
                return;
            }

            let port = info.get_port();
            let addresses = info.get_addresses();
            if let Some(addr) = addresses.iter().next() {
                let addr_str = addr.to_string();
                let mut dir = peer_dir.write().await;
                let is_new = dir.upsert(&node_id, &addr_str, port);
                if is_new {
                    info!("mDNS: discovered new peer {node_id} at {addr_str}:{port}");
                } else {
                    debug!("mDNS: refreshed peer {node_id} at {addr_str}:{port}");
                }
            }
        }
        ServiceEvent::ServiceRemoved(_type, fullname) => {
            debug!("mDNS: service removed: {fullname}");
        }
        ServiceEvent::SearchStarted(_) => {
            debug!("mDNS: browse started");
        }
        _ => {}
    }
}

/// Unregister from mDNS when shutting down.
pub fn unregister_service(daemon: &ServiceDaemon, node_id: &str) {
    let host = hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "meshmind-node".into());

    let fullname = format!("{node_id}.{host}.{SERVICE_TYPE}");
    if let Err(e) = daemon.unregister(&fullname) {
        warn!("mDNS: failed to unregister: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_type_is_valid() {
        assert!(SERVICE_TYPE.ends_with(".local."));
        assert!(SERVICE_TYPE.starts_with('_'));
    }

    #[tokio::test]
    async fn register_and_unregister() {
        let daemon = ServiceDaemon::new().expect("create mdns daemon");
        let result = register_service(&daemon, "test-node-001", 9900);
        assert!(result.is_ok());
        unregister_service(&daemon, "test-node-001");
        daemon.shutdown().ok();
    }
}
