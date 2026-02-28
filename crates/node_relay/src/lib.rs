//! Rendezvous + relay server for WAN connectivity.
//!
//! Provides two core functions:
//! 1. **Rendezvous**: Nodes register and discover each other across the internet.
//! 2. **Relay**: Forwards mesh envelopes between nodes that cannot connect directly.
//!
//! Wire protocol: length-prefixed `RelayWireFrame` over TCP+mTLS.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use node_crypto::{build_server_config, NodeIdentity};
use node_proto::relay::*;
use prost::Message;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, RwLock};
use tokio_rustls::TlsAcceptor;
use tracing::{debug, info, warn};

const MAX_FRAME_SIZE: usize = 4 * 1024 * 1024;
const REGISTRATION_TTL: Duration = Duration::from_secs(120);

/// A registered peer in the rendezvous directory.
#[derive(Debug, Clone)]
pub struct RegisteredPeer {
    pub node_id: String,
    pub public_addr: String,
    pub mesh_port: u32,
    pub capabilities: Vec<String>,
    pub tenant_id: String,
    pub relay_only: bool,
    pub relay_token: String,
    pub last_seen: Instant,
    pub relay_tx: Option<mpsc::Sender<Vec<u8>>>,
}

impl RegisteredPeer {
    fn is_expired(&self) -> bool {
        self.last_seen.elapsed() > REGISTRATION_TTL
    }

    fn to_peer_info(&self) -> PeerInfo {
        PeerInfo {
            node_id: Some(node_proto::common::NodeId {
                value: self.node_id.clone(),
            }),
            public_addr: self.public_addr.clone(),
            mesh_port: self.mesh_port,
            capabilities: self.capabilities.clone(),
            relay_only: self.relay_only,
            last_seen_ms: self.last_seen.elapsed().as_millis() as u64,
        }
    }
}

/// The in-memory peer directory used by the rendezvous server.
#[derive(Default)]
pub struct RelayDirectory {
    peers: HashMap<String, RegisteredPeer>,
}

impl RelayDirectory {
    pub fn new() -> Self {
        Self {
            peers: HashMap::new(),
        }
    }

    pub fn register(&mut self, req: &RegisterRequest, relay_token: String) -> RegisterResponse {
        let node_id = req
            .node_id
            .as_ref()
            .map(|n| n.value.clone())
            .unwrap_or_default();

        if node_id.is_empty() {
            return RegisterResponse {
                success: false,
                error: "node_id required".into(),
                ..Default::default()
            };
        }

        let tenant_id = req
            .tenant_id
            .as_ref()
            .map(|t| t.value.clone())
            .unwrap_or_else(|| "public".into());

        self.peers.insert(
            node_id.clone(),
            RegisteredPeer {
                node_id: node_id.clone(),
                public_addr: req.public_addr.clone(),
                mesh_port: req.mesh_port,
                capabilities: req.capabilities.clone(),
                tenant_id,
                relay_only: req.relay_only,
                relay_token: relay_token.clone(),
                last_seen: Instant::now(),
                relay_tx: None,
            },
        );

        info!("registered peer {node_id} (relay_only={})", req.relay_only);

        RegisterResponse {
            success: true,
            relay_token,
            assigned_relay_id: format!("relay-{node_id}"),
            error: String::new(),
        }
    }

    pub fn heartbeat(&mut self, node_id: &str, token: &str) -> HeartbeatResponse {
        if let Some(peer) = self.peers.get_mut(node_id) {
            if peer.relay_token == token {
                peer.last_seen = Instant::now();
                return HeartbeatResponse {
                    alive: true,
                    connected_peers: self.peers.len() as u32,
                };
            }
        }
        HeartbeatResponse {
            alive: false,
            connected_peers: 0,
        }
    }

    pub fn discover(&self, requester: &str, tenant_id: &str, max_results: u32) -> DiscoverResponse {
        let max = if max_results == 0 { 30 } else { max_results as usize };

        let peers: Vec<PeerInfo> = self
            .peers
            .values()
            .filter(|p| !p.is_expired())
            .filter(|p| p.node_id != requester)
            .filter(|p| tenant_id.is_empty() || p.tenant_id == tenant_id || p.tenant_id == "public")
            .take(max)
            .map(|p| p.to_peer_info())
            .collect();

        DiscoverResponse { peers }
    }

    pub fn get_relay_tx(&self, node_id: &str) -> Option<mpsc::Sender<Vec<u8>>> {
        self.peers.get(node_id).and_then(|p| p.relay_tx.clone())
    }

    pub fn set_relay_tx(&mut self, node_id: &str, tx: mpsc::Sender<Vec<u8>>) {
        if let Some(peer) = self.peers.get_mut(node_id) {
            peer.relay_tx = Some(tx);
        }
    }

    pub fn validate_token(&self, node_id: &str, token: &str) -> bool {
        self.peers
            .get(node_id)
            .map(|p| p.relay_token == token)
            .unwrap_or(false)
    }

    pub fn remove_expired(&mut self) -> usize {
        let before = self.peers.len();
        self.peers.retain(|_, p| !p.is_expired());
        before - self.peers.len()
    }

    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }
}

fn generate_token() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Handle a single relay wire frame from a connected node.
pub fn handle_frame(
    dir: &mut RelayDirectory,
    wire: &RelayWireFrame,
) -> Result<RelayWireFrame> {
    let msg_type = RelayMsgType::try_from(wire.msg_type).unwrap_or(RelayMsgType::Unspecified);

    match msg_type {
        RelayMsgType::Register => {
            let req = RegisterRequest::decode(wire.payload.as_slice())
                .context("decode RegisterRequest")?;
            let token = generate_token();
            let resp = dir.register(&req, token);
            Ok(RelayWireFrame {
                msg_type: RelayMsgType::RegisterResp as i32,
                payload: resp.encode_to_vec(),
            })
        }
        RelayMsgType::Heartbeat => {
            let req = HeartbeatRequest::decode(wire.payload.as_slice())
                .context("decode HeartbeatRequest")?;
            let node_id = req.node_id.map(|n| n.value).unwrap_or_default();
            let resp = dir.heartbeat(&node_id, &req.relay_token);
            Ok(RelayWireFrame {
                msg_type: RelayMsgType::HeartbeatResp as i32,
                payload: resp.encode_to_vec(),
            })
        }
        RelayMsgType::Discover => {
            let req = DiscoverRequest::decode(wire.payload.as_slice())
                .context("decode DiscoverRequest")?;
            let requester = req.requester.map(|n| n.value).unwrap_or_default();
            let tenant = req.tenant_id.map(|t| t.value).unwrap_or_default();
            let resp = dir.discover(&requester, &tenant, req.max_results);
            Ok(RelayWireFrame {
                msg_type: RelayMsgType::DiscoverResp as i32,
                payload: resp.encode_to_vec(),
            })
        }
        RelayMsgType::Relay => {
            let frame = RelayFrame::decode(wire.payload.as_slice())
                .context("decode RelayFrame")?;
            let from_id = frame.from_node_id.as_ref().map(|n| n.value.as_str()).unwrap_or("");
            let to_id = frame.to_node_id.as_ref().map(|n| n.value.as_str()).unwrap_or("");

            if !dir.validate_token(from_id, &frame.relay_token) {
                return Ok(RelayWireFrame {
                    msg_type: RelayMsgType::RelayAck as i32,
                    payload: RelayAck {
                        sequence: frame.sequence,
                        delivered: false,
                        error: "invalid relay token".into(),
                    }
                    .encode_to_vec(),
                });
            }

            let delivered = if let Some(tx) = dir.get_relay_tx(to_id) {
                let relay_bytes = wire.payload.clone();
                tx.try_send(relay_bytes).is_ok()
            } else {
                false
            };

            Ok(RelayWireFrame {
                msg_type: RelayMsgType::RelayAck as i32,
                payload: RelayAck {
                    sequence: frame.sequence,
                    delivered,
                    error: if delivered { String::new() } else { "peer not connected".into() },
                }
                .encode_to_vec(),
            })
        }
        _ => Ok(RelayWireFrame {
            msg_type: RelayMsgType::Unspecified as i32,
            payload: vec![],
        }),
    }
}

/// Run the relay server on the given address with mTLS.
pub async fn run_relay_server(
    identity: &NodeIdentity,
    ca_cert_pem: &str,
    addr: &str,
    port: u16,
) -> Result<()> {
    let server_config = build_server_config(&identity.cert_pem, &identity.key_pem, ca_cert_pem)
        .context("build server TLS config")?;
    let acceptor = TlsAcceptor::from(Arc::new(server_config));
    let listener = TcpListener::bind((addr, port))
        .await
        .with_context(|| format!("bind relay server to {addr}:{port}"))?;

    info!("relay server listening on {addr}:{port}");

    let directory = Arc::new(RwLock::new(RelayDirectory::new()));

    // Expiry cleanup task
    let dir_cleanup = directory.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        loop {
            interval.tick().await;
            let removed = dir_cleanup.write().await.remove_expired();
            if removed > 0 {
                debug!("removed {removed} expired peer registrations");
            }
        }
    });

    loop {
        let (tcp_stream, peer_addr) = listener.accept().await?;
        let acceptor = acceptor.clone();
        let directory = directory.clone();

        tokio::spawn(async move {
            match acceptor.accept(tcp_stream).await {
                Ok(mut tls_stream) => {
                    debug!("relay connection from {peer_addr}");
                    loop {
                        match read_wire_frame(&mut tls_stream).await {
                            Ok(wire) => {
                                let response = {
                                    let mut dir = directory.write().await;
                                    handle_frame(&mut dir, &wire)
                                };
                                match response {
                                    Ok(resp) => {
                                        if let Err(e) = write_wire_frame(&mut tls_stream, &resp).await {
                                            warn!("failed to write response to {peer_addr}: {e}");
                                            break;
                                        }
                                    }
                                    Err(e) => {
                                        warn!("error handling frame from {peer_addr}: {e}");
                                        break;
                                    }
                                }
                            }
                            Err(_) => {
                                debug!("connection from {peer_addr} closed");
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("TLS accept failed from {peer_addr}: {e}");
                }
            }
        });
    }
}

async fn write_wire_frame<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    frame: &RelayWireFrame,
) -> Result<()> {
    let data = frame.encode_to_vec();
    let len = (data.len() as u32).to_le_bytes();
    writer.write_all(&len).await.context("write length")?;
    writer.write_all(&data).await.context("write payload")?;
    writer.flush().await.context("flush")?;
    Ok(())
}

async fn read_wire_frame<R: AsyncReadExt + Unpin>(reader: &mut R) -> Result<RelayWireFrame> {
    let mut len_buf = [0u8; 4];
    reader
        .read_exact(&mut len_buf)
        .await
        .context("read length")?;
    let len = u32::from_le_bytes(len_buf) as usize;
    anyhow::ensure!(len <= MAX_FRAME_SIZE, "frame too large: {len} bytes");

    let mut data = vec![0u8; len];
    reader.read_exact(&mut data).await.context("read payload")?;
    RelayWireFrame::decode(data.as_slice()).context("decode wire frame")
}

/// Client-side helper: connect to a relay, send a wire frame, read a response.
pub async fn relay_request(
    connector: &tokio_rustls::TlsConnector,
    server_name: &rustls::pki_types::ServerName<'static>,
    addr: &str,
    port: u16,
    frame: &RelayWireFrame,
) -> Result<RelayWireFrame> {
    let tcp = tokio::net::TcpStream::connect((addr, port))
        .await
        .with_context(|| format!("connect to relay {addr}:{port}"))?;

    let mut tls = connector
        .connect(server_name.clone(), tcp)
        .await
        .context("relay TLS handshake")?;

    write_wire_frame(&mut tls, frame).await?;
    let response = read_wire_frame(&mut tls).await?;
    tls.shutdown().await.ok();
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use node_proto::common::*;

    fn make_register_req(node_id: &str, relay_only: bool) -> RegisterRequest {
        RegisterRequest {
            node_id: Some(NodeId {
                value: node_id.into(),
            }),
            public_addr: if relay_only {
                String::new()
            } else {
                "10.0.0.1:9901".to_string()
            },
            mesh_port: 9901,
            capabilities: vec!["inference".into()],
            tenant_id: Some(TenantId {
                value: "public".into(),
            }),
            relay_only,
        }
    }

    fn make_wire_frame(msg_type: RelayMsgType, payload: &impl Message) -> RelayWireFrame {
        RelayWireFrame {
            msg_type: msg_type as i32,
            payload: payload.encode_to_vec(),
        }
    }

    #[test]
    fn register_and_discover() {
        let mut dir = RelayDirectory::new();
        let req = make_register_req("node-a", false);
        let resp = dir.register(&req, "token-a".into());
        assert!(resp.success);
        assert_eq!(dir.peer_count(), 1);

        let req2 = make_register_req("node-b", true);
        dir.register(&req2, "token-b".into());
        assert_eq!(dir.peer_count(), 2);

        let discovered = dir.discover("node-a", "public", 10);
        assert_eq!(discovered.peers.len(), 1);
        assert_eq!(
            discovered.peers[0].node_id.as_ref().unwrap().value,
            "node-b"
        );
        assert!(discovered.peers[0].relay_only);
    }

    #[test]
    fn register_empty_node_id_fails() {
        let mut dir = RelayDirectory::new();
        let req = RegisterRequest::default();
        let resp = dir.register(&req, "token".into());
        assert!(!resp.success);
    }

    #[test]
    fn heartbeat_refreshes_peer() {
        let mut dir = RelayDirectory::new();
        let req = make_register_req("node-x", false);
        dir.register(&req, "tok-x".into());

        let resp = dir.heartbeat("node-x", "tok-x");
        assert!(resp.alive);
        assert_eq!(resp.connected_peers, 1);
    }

    #[test]
    fn heartbeat_wrong_token_rejected() {
        let mut dir = RelayDirectory::new();
        let req = make_register_req("node-x", false);
        dir.register(&req, "tok-x".into());

        let resp = dir.heartbeat("node-x", "wrong-token");
        assert!(!resp.alive);
    }

    #[test]
    fn discover_excludes_self() {
        let mut dir = RelayDirectory::new();
        dir.register(&make_register_req("node-a", false), "tok-a".into());
        dir.register(&make_register_req("node-b", false), "tok-b".into());

        let result = dir.discover("node-a", "public", 10);
        assert_eq!(result.peers.len(), 1);
        assert_eq!(result.peers[0].node_id.as_ref().unwrap().value, "node-b");
    }

    #[test]
    fn token_validation() {
        let mut dir = RelayDirectory::new();
        dir.register(&make_register_req("node-a", false), "tok-a".into());
        assert!(dir.validate_token("node-a", "tok-a"));
        assert!(!dir.validate_token("node-a", "wrong"));
        assert!(!dir.validate_token("nonexistent", "tok-a"));
    }

    #[test]
    fn handle_register_frame() {
        let mut dir = RelayDirectory::new();
        let req = make_register_req("node-frame", false);
        let wire = make_wire_frame(RelayMsgType::Register, &req);
        let resp = handle_frame(&mut dir, &wire).unwrap();
        assert_eq!(resp.msg_type, RelayMsgType::RegisterResp as i32);

        let decoded = RegisterResponse::decode(resp.payload.as_slice()).unwrap();
        assert!(decoded.success);
        assert_eq!(dir.peer_count(), 1);
    }

    #[test]
    fn handle_discover_frame() {
        let mut dir = RelayDirectory::new();
        dir.register(&make_register_req("node-a", false), "tok-a".into());
        dir.register(&make_register_req("node-b", false), "tok-b".into());

        let discover = DiscoverRequest {
            requester: Some(NodeId {
                value: "node-a".into(),
            }),
            tenant_id: Some(TenantId {
                value: "public".into(),
            }),
            max_results: 10,
        };
        let wire = make_wire_frame(RelayMsgType::Discover, &discover);
        let resp = handle_frame(&mut dir, &wire).unwrap();

        let decoded = DiscoverResponse::decode(resp.payload.as_slice()).unwrap();
        assert_eq!(decoded.peers.len(), 1);
    }

    #[test]
    fn handle_relay_invalid_token() {
        let mut dir = RelayDirectory::new();
        dir.register(&make_register_req("node-a", false), "tok-a".into());
        dir.register(&make_register_req("node-b", false), "tok-b".into());

        let frame = RelayFrame {
            from_node_id: Some(NodeId {
                value: "node-a".into(),
            }),
            to_node_id: Some(NodeId {
                value: "node-b".into(),
            }),
            relay_token: "wrong-token".into(),
            envelope_bytes: vec![1, 2, 3],
            sequence: 1,
        };
        let wire = make_wire_frame(RelayMsgType::Relay, &frame);
        let resp = handle_frame(&mut dir, &wire).unwrap();

        let ack = RelayAck::decode(resp.payload.as_slice()).unwrap();
        assert!(!ack.delivered);
        assert!(ack.error.contains("invalid relay token"));
    }

    #[tokio::test]
    async fn relay_server_accepts_connections() {
        use node_crypto::DevCa;

        let ca = DevCa::generate().unwrap();
        let server_id = ca.generate_node_cert("relay-server").unwrap();
        let client_id = ca.generate_node_cert("test-client").unwrap();

        let server_config =
            build_server_config(&server_id.cert_pem, &server_id.key_pem, &ca.cert_pem).unwrap();
        let acceptor = TlsAcceptor::from(Arc::new(server_config));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let directory = Arc::new(RwLock::new(RelayDirectory::new()));
        let dir_clone = directory.clone();

        tokio::spawn(async move {
            if let Ok((tcp, _)) = listener.accept().await {
                if let Ok(mut tls) = acceptor.accept(tcp).await {
                    if let Ok(wire) = read_wire_frame(&mut tls).await {
                        let resp = {
                            let mut dir = dir_clone.write().await;
                            handle_frame(&mut dir, &wire).unwrap()
                        };
                        write_wire_frame(&mut tls, &resp).await.ok();
                    }
                }
            }
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let client_config =
            node_crypto::build_client_config(&client_id.cert_pem, &client_id.key_pem, &ca.cert_pem)
                .unwrap();
        let connector = tokio_rustls::TlsConnector::from(Arc::new(client_config));
        let server_name = rustls::pki_types::ServerName::try_from("localhost")
            .unwrap()
            .to_owned();

        let req = make_register_req("test-client-node", false);
        let wire = make_wire_frame(RelayMsgType::Register, &req);

        let resp = relay_request(&connector, &server_name, "127.0.0.1", port, &wire)
            .await
            .unwrap();

        assert_eq!(resp.msg_type, RelayMsgType::RegisterResp as i32);
        let decoded = RegisterResponse::decode(resp.payload.as_slice()).unwrap();
        assert!(decoded.success);
    }
}
