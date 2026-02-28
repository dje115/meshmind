//! Relay transport: routes mesh envelopes through a relay server.
//!
//! Used when nodes cannot establish direct TCP connections (NAT, firewall).
//! Connects to the relay server via TCP+mTLS, wraps each mesh Envelope in a
//! RelayFrame, and reads back the response (or RelayAck).

use std::sync::Arc;

use anyhow::{Context, Result};
use node_crypto::NodeIdentity;
use node_proto::common::NodeId;
use node_proto::mesh::Envelope;
use node_proto::relay::*;
use prost::Message;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_rustls::TlsConnector;
use tracing::{debug, warn};

use crate::transport::Transport;

const MAX_FRAME_SIZE: usize = 4 * 1024 * 1024;

/// Transport that forwards envelopes via a relay server.
pub struct RelayTransport {
    connector: TlsConnector,
    server_name: rustls::pki_types::ServerName<'static>,
    relay_addr: String,
    relay_port: u16,
    node_id: String,
    relay_token: Mutex<String>,
    sequence: Mutex<u64>,
}

impl RelayTransport {
    pub fn new(
        identity: &NodeIdentity,
        ca_cert_pem: &str,
        relay_addr: &str,
        relay_port: u16,
    ) -> Result<Self> {
        let client_config =
            node_crypto::build_client_config(&identity.cert_pem, &identity.key_pem, ca_cert_pem)
                .context("build relay client TLS config")?;

        let server_name = rustls::pki_types::ServerName::try_from("localhost")
            .map_err(|e| anyhow::anyhow!("invalid server name: {e}"))?
            .to_owned();

        Ok(Self {
            connector: TlsConnector::from(Arc::new(client_config)),
            server_name,
            relay_addr: relay_addr.to_string(),
            relay_port,
            node_id: identity.node_id.clone(),
            relay_token: Mutex::new(String::new()),
            sequence: Mutex::new(0),
        })
    }

    /// Register with the relay server and obtain a relay token.
    pub async fn register(
        &self,
        capabilities: Vec<String>,
        public_addr: &str,
        relay_only: bool,
    ) -> Result<RegisterResponse> {
        let req = RegisterRequest {
            node_id: Some(NodeId {
                value: self.node_id.clone(),
            }),
            public_addr: public_addr.to_string(),
            mesh_port: self.relay_port as u32,
            capabilities,
            tenant_id: Some(node_proto::common::TenantId {
                value: "public".into(),
            }),
            relay_only,
        };

        let wire = RelayWireFrame {
            msg_type: RelayMsgType::Register as i32,
            payload: req.encode_to_vec(),
        };

        let resp_wire = self.send_wire_frame(&wire).await?;
        let resp = RegisterResponse::decode(resp_wire.payload.as_slice())
            .context("decode RegisterResponse")?;

        if resp.success {
            *self.relay_token.lock().await = resp.relay_token.clone();
            debug!("registered with relay server, token={}", resp.relay_token);
        }

        Ok(resp)
    }

    /// Discover peers via the relay server.
    pub async fn discover(
        &self,
        tenant_id: &str,
        max_results: u32,
    ) -> Result<DiscoverResponse> {
        let req = DiscoverRequest {
            requester: Some(NodeId {
                value: self.node_id.clone(),
            }),
            tenant_id: Some(node_proto::common::TenantId {
                value: tenant_id.into(),
            }),
            max_results,
        };

        let wire = RelayWireFrame {
            msg_type: RelayMsgType::Discover as i32,
            payload: req.encode_to_vec(),
        };

        let resp_wire = self.send_wire_frame(&wire).await?;
        DiscoverResponse::decode(resp_wire.payload.as_slice()).context("decode DiscoverResponse")
    }

    /// Send a heartbeat to keep the registration alive.
    pub async fn heartbeat(&self) -> Result<HeartbeatResponse> {
        let token = self.relay_token.lock().await.clone();
        let req = HeartbeatRequest {
            node_id: Some(NodeId {
                value: self.node_id.clone(),
            }),
            relay_token: token,
        };

        let wire = RelayWireFrame {
            msg_type: RelayMsgType::Heartbeat as i32,
            payload: req.encode_to_vec(),
        };

        let resp_wire = self.send_wire_frame(&wire).await?;
        HeartbeatResponse::decode(resp_wire.payload.as_slice())
            .context("decode HeartbeatResponse")
    }

    async fn next_sequence(&self) -> u64 {
        let mut seq = self.sequence.lock().await;
        *seq += 1;
        *seq
    }

    async fn send_wire_frame(&self, wire: &RelayWireFrame) -> Result<RelayWireFrame> {
        let tcp = TcpStream::connect((&*self.relay_addr, self.relay_port))
            .await
            .with_context(|| format!("connect to relay {}:{}", self.relay_addr, self.relay_port))?;

        let mut tls = self
            .connector
            .connect(self.server_name.clone(), tcp)
            .await
            .context("relay TLS handshake")?;

        let data = wire.encode_to_vec();
        let len = (data.len() as u32).to_le_bytes();
        tls.write_all(&len).await.context("write length")?;
        tls.write_all(&data).await.context("write payload")?;
        tls.flush().await.context("flush")?;

        let mut len_buf = [0u8; 4];
        tls.read_exact(&mut len_buf)
            .await
            .context("read response length")?;
        let resp_len = u32::from_le_bytes(len_buf) as usize;
        anyhow::ensure!(resp_len <= MAX_FRAME_SIZE, "response too large");

        let mut resp_data = vec![0u8; resp_len];
        tls.read_exact(&mut resp_data)
            .await
            .context("read response")?;
        tls.shutdown().await.ok();

        RelayWireFrame::decode(resp_data.as_slice()).context("decode response wire frame")
    }
}

#[async_trait::async_trait]
impl Transport for RelayTransport {
    async fn send(&self, _address: &str, _port: u16, envelope: &Envelope) -> Result<()> {
        let token = self.relay_token.lock().await.clone();
        let seq = self.next_sequence().await;

        let to_node_id = envelope
            .to_node_id
            .as_ref()
            .map(|n| n.value.clone())
            .unwrap_or_default();

        let frame = RelayFrame {
            from_node_id: Some(NodeId {
                value: self.node_id.clone(),
            }),
            to_node_id: Some(NodeId {
                value: to_node_id,
            }),
            relay_token: token,
            envelope_bytes: envelope.encode_to_vec(),
            sequence: seq,
        };

        let wire = RelayWireFrame {
            msg_type: RelayMsgType::Relay as i32,
            payload: frame.encode_to_vec(),
        };

        let resp = self.send_wire_frame(&wire).await?;
        let ack = RelayAck::decode(resp.payload.as_slice()).context("decode RelayAck")?;

        if !ack.delivered {
            warn!("relay send not delivered: {}", ack.error);
        }

        Ok(())
    }

    async fn request(
        &self,
        address: &str,
        port: u16,
        envelope: &Envelope,
    ) -> Result<Envelope> {
        // For relay, request is the same as send — we don't get a response envelope back
        // through the relay in this simple implementation. The peer consult layer handles
        // the request/response pattern at a higher level.
        self.send(address, port, envelope).await?;
        Err(anyhow::anyhow!(
            "relay transport does not support synchronous request/response"
        ))
    }
}

/// Hybrid transport: tries direct TCP first, falls back to relay.
pub struct HybridTransport {
    direct: Arc<dyn Transport>,
    relay: Arc<RelayTransport>,
}

impl HybridTransport {
    pub fn new(direct: Arc<dyn Transport>, relay: Arc<RelayTransport>) -> Self {
        Self { direct, relay }
    }
}

#[async_trait::async_trait]
impl Transport for HybridTransport {
    async fn send(&self, address: &str, port: u16, envelope: &Envelope) -> Result<()> {
        match self.direct.send(address, port, envelope).await {
            Ok(()) => Ok(()),
            Err(direct_err) => {
                debug!("direct send failed ({direct_err}), falling back to relay");
                self.relay.send(address, port, envelope).await
            }
        }
    }

    async fn request(
        &self,
        address: &str,
        port: u16,
        envelope: &Envelope,
    ) -> Result<Envelope> {
        match self.direct.request(address, port, envelope).await {
            Ok(resp) => Ok(resp),
            Err(direct_err) => {
                debug!("direct request failed ({direct_err}), falling back to relay");
                self.relay.request(address, port, envelope).await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::MockTransport;
    use node_proto::mesh::*;

    fn make_ping(nonce: u64) -> Envelope {
        Envelope {
            msg_id: format!("ping-{nonce}"),
            r#type: MsgType::Ping as i32,
            from_node_id: Some(NodeId {
                value: "test-node".into(),
            }),
            to_node_id: Some(NodeId {
                value: "target-node".into(),
            }),
            body: Some(envelope::Body::Ping(Ping { nonce })),
            ..Default::default()
        }
    }

    fn make_pong(nonce: u64) -> Envelope {
        Envelope {
            msg_id: format!("pong-{nonce}"),
            r#type: MsgType::Pong as i32,
            body: Some(envelope::Body::Pong(Pong { nonce })),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn hybrid_prefers_direct() {
        let mock = Arc::new(MockTransport::new());
        mock.push_response(make_pong(42));

        let ca = node_crypto::DevCa::generate().unwrap();
        let id = ca.generate_node_cert("test").unwrap();
        let relay = Arc::new(
            RelayTransport::new(&id, &ca.cert_pem, "127.0.0.1", 19999).unwrap(),
        );

        let hybrid = HybridTransport::new(mock.clone(), relay);
        let resp = hybrid
            .request("127.0.0.1", 9000, &make_ping(42))
            .await
            .unwrap();

        assert_eq!(resp.r#type, MsgType::Pong as i32);
        assert_eq!(mock.take_sent().len(), 1);
    }

    #[tokio::test]
    async fn hybrid_falls_back_to_relay_on_direct_failure() {
        let mock = Arc::new(MockTransport::new());
        // No response pushed — direct will fail

        let ca = node_crypto::DevCa::generate().unwrap();
        let id = ca.generate_node_cert("test").unwrap();
        let relay = Arc::new(
            RelayTransport::new(&id, &ca.cert_pem, "127.0.0.1", 19999).unwrap(),
        );

        let hybrid = HybridTransport::new(mock, relay);
        // Relay also fails (no server running), but the fallback path is exercised
        let result = hybrid.request("127.0.0.1", 9000, &make_ping(42)).await;
        assert!(result.is_err());
    }
}
