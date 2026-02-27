//! Transport abstraction for peer-to-peer communication.
//!
//! Phase 1: TCP + mTLS
//! Phase 2: QUIC + mTLS (designed from the start via this trait)

use node_proto::mesh::Envelope;

/// Transport abstraction trait. Implementations handle the wire protocol.
#[async_trait::async_trait]
pub trait Transport: Send + Sync {
    /// Send an envelope to a peer at the given address.
    async fn send(&self, address: &str, port: u16, envelope: &Envelope) -> anyhow::Result<()>;

    /// Send an envelope and wait for a response.
    async fn request(
        &self,
        address: &str,
        port: u16,
        envelope: &Envelope,
    ) -> anyhow::Result<Envelope>;
}

/// Mock transport for testing: records sent messages and returns canned responses.
pub struct MockTransport {
    responses: std::sync::Mutex<Vec<Envelope>>,
    sent: std::sync::Mutex<Vec<(String, u16, Envelope)>>,
}

impl MockTransport {
    pub fn new() -> Self {
        Self {
            responses: std::sync::Mutex::new(Vec::new()),
            sent: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub fn push_response(&self, env: Envelope) {
        self.responses.lock().unwrap().push(env);
    }

    pub fn take_sent(&self) -> Vec<(String, u16, Envelope)> {
        std::mem::take(&mut *self.sent.lock().unwrap())
    }
}

impl Default for MockTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Transport for MockTransport {
    async fn send(&self, address: &str, port: u16, envelope: &Envelope) -> anyhow::Result<()> {
        self.sent
            .lock()
            .unwrap()
            .push((address.to_string(), port, envelope.clone()));
        Ok(())
    }

    async fn request(
        &self,
        address: &str,
        port: u16,
        envelope: &Envelope,
    ) -> anyhow::Result<Envelope> {
        self.sent
            .lock()
            .unwrap()
            .push((address.to_string(), port, envelope.clone()));
        self.responses
            .lock()
            .unwrap()
            .pop()
            .ok_or_else(|| anyhow::anyhow!("no canned response"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use node_proto::common::*;
    use node_proto::mesh::*;

    fn make_hello() -> Envelope {
        Envelope {
            msg_id: "msg-1".into(),
            r#type: MsgType::Hello as i32,
            from_node_id: Some(NodeId {
                value: "node-1".into(),
            }),
            body: Some(envelope::Body::Hello(Hello {
                capabilities: vec!["inference".into()],
                version: "0.1.0".into(),
            })),
            ..Default::default()
        }
    }

    fn make_pong() -> Envelope {
        Envelope {
            msg_id: "msg-2".into(),
            r#type: MsgType::Pong as i32,
            body: Some(envelope::Body::Pong(Pong { nonce: 42 })),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn mock_send_records() {
        let transport = MockTransport::new();
        let hello = make_hello();
        transport.send("192.168.1.10", 9000, &hello).await.unwrap();

        let sent = transport.take_sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, "192.168.1.10");
        assert_eq!(sent[0].1, 9000);
    }

    #[tokio::test]
    async fn mock_request_returns_response() {
        let transport = MockTransport::new();
        transport.push_response(make_pong());

        let hello = make_hello();
        let resp = transport
            .request("192.168.1.10", 9000, &hello)
            .await
            .unwrap();

        assert_eq!(resp.r#type, MsgType::Pong as i32);
    }

    #[tokio::test]
    async fn mock_no_response_errors() {
        let transport = MockTransport::new();
        let hello = make_hello();
        let result = transport.request("192.168.1.10", 9000, &hello).await;
        assert!(result.is_err());
    }
}
