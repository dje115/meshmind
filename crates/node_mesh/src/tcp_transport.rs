//! Real TCP+mTLS transport using tokio-rustls.
//!
//! Wire format: [4-byte LE length][protobuf envelope bytes]
//! For `request()`: client sends envelope, reads one envelope back.
//! For `send()`: client sends envelope and closes (fire-and-forget).

use std::sync::Arc;

use anyhow::{Context, Result};
use node_crypto::{build_client_config, build_server_config, NodeIdentity};
use node_proto::mesh::Envelope;
use prost::Message;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::{TlsAcceptor, TlsConnector};
use tracing::{debug, warn};

use crate::transport::Transport;

/// TCP+mTLS transport that sends/receives length-prefixed protobuf envelopes.
pub struct TcpTransport {
    connector: TlsConnector,
    server_name: rustls::pki_types::ServerName<'static>,
}

impl TcpTransport {
    pub fn new(identity: &NodeIdentity, ca_cert_pem: &str) -> Result<Self> {
        let client_config = build_client_config(&identity.cert_pem, &identity.key_pem, ca_cert_pem)
            .context("build client TLS config")?;

        let server_name = rustls::pki_types::ServerName::try_from("localhost")
            .map_err(|e| anyhow::anyhow!("invalid server name: {e}"))?
            .to_owned();

        Ok(Self {
            connector: TlsConnector::from(Arc::new(client_config)),
            server_name,
        })
    }
}

#[async_trait::async_trait]
impl Transport for TcpTransport {
    async fn send(&self, address: &str, port: u16, envelope: &Envelope) -> Result<()> {
        let tcp = TcpStream::connect((address, port))
            .await
            .with_context(|| format!("connect to {address}:{port}"))?;

        let mut tls = self
            .connector
            .connect(self.server_name.clone(), tcp)
            .await
            .context("TLS handshake")?;

        write_envelope(&mut tls, envelope).await?;
        tls.shutdown().await.ok();
        Ok(())
    }

    async fn request(&self, address: &str, port: u16, envelope: &Envelope) -> Result<Envelope> {
        let tcp = TcpStream::connect((address, port))
            .await
            .with_context(|| format!("connect to {address}:{port}"))?;

        let mut tls = self
            .connector
            .connect(self.server_name.clone(), tcp)
            .await
            .context("TLS handshake")?;

        write_envelope(&mut tls, envelope).await?;
        let response = read_envelope(&mut tls).await?;
        tls.shutdown().await.ok();
        Ok(response)
    }
}

/// A TCP+mTLS listener that accepts connections and dispatches envelopes to a handler.
pub struct TcpServer {
    acceptor: TlsAcceptor,
    listener: TcpListener,
}

pub type EnvelopeHandler = Arc<dyn Fn(Envelope) -> Option<Envelope> + Send + Sync>;

impl TcpServer {
    pub async fn bind(
        identity: &NodeIdentity,
        ca_cert_pem: &str,
        addr: &str,
        port: u16,
    ) -> Result<Self> {
        let server_config = build_server_config(&identity.cert_pem, &identity.key_pem, ca_cert_pem)
            .context("build server TLS config")?;
        let acceptor = TlsAcceptor::from(Arc::new(server_config));
        let listener = TcpListener::bind((addr, port))
            .await
            .with_context(|| format!("bind {addr}:{port}"))?;
        debug!("TcpServer listening on {addr}:{port}");
        Ok(Self { acceptor, listener })
    }

    pub fn local_port(&self) -> Result<u16> {
        Ok(self.listener.local_addr()?.port())
    }

    /// Run the accept loop. Each connection reads one envelope, calls the handler,
    /// and optionally writes back a response envelope.
    pub async fn serve(self, handler: EnvelopeHandler) -> Result<()> {
        loop {
            let (tcp_stream, peer_addr) = self.listener.accept().await?;
            let acceptor = self.acceptor.clone();
            let handler = handler.clone();

            tokio::spawn(async move {
                match acceptor.accept(tcp_stream).await {
                    Ok(mut tls_stream) => match read_envelope(&mut tls_stream).await {
                        Ok(envelope) => {
                            debug!(
                                "received envelope from {peer_addr}: type={}",
                                envelope.r#type
                            );
                            if let Some(response) = handler(envelope) {
                                if let Err(e) = write_envelope(&mut tls_stream, &response).await {
                                    warn!("failed to write response to {peer_addr}: {e}");
                                }
                            }
                            let _ = tls_stream.shutdown().await;
                        }
                        Err(e) => {
                            warn!("failed to read envelope from {peer_addr}: {e}");
                        }
                    },
                    Err(e) => {
                        warn!("TLS accept failed from {peer_addr}: {e}");
                    }
                }
            });
        }
    }
}

async fn write_envelope<W: AsyncWriteExt + Unpin>(writer: &mut W, env: &Envelope) -> Result<()> {
    let data = env.encode_to_vec();
    let len = (data.len() as u32).to_le_bytes();
    writer.write_all(&len).await.context("write length")?;
    writer.write_all(&data).await.context("write payload")?;
    writer.flush().await.context("flush")?;
    Ok(())
}

async fn read_envelope<R: AsyncReadExt + Unpin>(reader: &mut R) -> Result<Envelope> {
    let mut len_buf = [0u8; 4];
    reader
        .read_exact(&mut len_buf)
        .await
        .context("read length")?;
    let len = u32::from_le_bytes(len_buf) as usize;
    anyhow::ensure!(len <= 4 * 1024 * 1024, "envelope too large: {len} bytes");

    let mut data = vec![0u8; len];
    reader.read_exact(&mut data).await.context("read payload")?;
    Envelope::decode(data.as_slice()).context("decode envelope")
}

#[cfg(test)]
mod tests {
    use super::*;
    use node_crypto::DevCa;
    use node_proto::common::*;
    use node_proto::mesh::*;

    fn make_ping(nonce: u64) -> Envelope {
        Envelope {
            msg_id: format!("ping-{nonce}"),
            r#type: MsgType::Ping as i32,
            from_node_id: Some(NodeId {
                value: "test-client".into(),
            }),
            body: Some(envelope::Body::Ping(Ping { nonce })),
            ..Default::default()
        }
    }

    fn make_pong(nonce: u64) -> Envelope {
        Envelope {
            msg_id: format!("pong-{nonce}"),
            r#type: MsgType::Pong as i32,
            from_node_id: Some(NodeId {
                value: "test-server".into(),
            }),
            body: Some(envelope::Body::Pong(Pong { nonce })),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn tcp_mtls_ping_pong() {
        let ca = DevCa::generate().unwrap();
        let server_id = ca.generate_node_cert("server").unwrap();
        let client_id = ca.generate_node_cert("client").unwrap();

        let server = TcpServer::bind(&server_id, &ca.cert_pem, "127.0.0.1", 0)
            .await
            .unwrap();
        let port = server.local_port().unwrap();

        let handler: EnvelopeHandler = Arc::new(|env| {
            if let Some(envelope::Body::Ping(ping)) = env.body {
                Some(make_pong(ping.nonce))
            } else {
                None
            }
        });

        tokio::spawn(async move {
            server.serve(handler).await.ok();
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let transport = TcpTransport::new(&client_id, &ca.cert_pem).unwrap();
        let response = transport
            .request("127.0.0.1", port, &make_ping(42))
            .await
            .unwrap();

        assert_eq!(response.r#type, MsgType::Pong as i32);
        if let Some(envelope::Body::Pong(pong)) = response.body {
            assert_eq!(pong.nonce, 42);
        } else {
            panic!("expected Pong body");
        }
    }

    #[tokio::test]
    async fn tcp_mtls_fire_and_forget() {
        let ca = DevCa::generate().unwrap();
        let server_id = ca.generate_node_cert("server").unwrap();
        let client_id = ca.generate_node_cert("client").unwrap();

        let (tx, mut rx) = tokio::sync::mpsc::channel::<Envelope>(1);

        let server = TcpServer::bind(&server_id, &ca.cert_pem, "127.0.0.1", 0)
            .await
            .unwrap();
        let port = server.local_port().unwrap();

        let handler: EnvelopeHandler = Arc::new(move |env| {
            tx.try_send(env).ok();
            None
        });

        tokio::spawn(async move {
            server.serve(handler).await.ok();
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let transport = TcpTransport::new(&client_id, &ca.cert_pem).unwrap();
        transport
            .send("127.0.0.1", port, &make_ping(99))
            .await
            .unwrap();

        let received = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(received.msg_id, "ping-99");
    }

    #[tokio::test]
    async fn tcp_mtls_multiple_requests() {
        let ca = DevCa::generate().unwrap();
        let server_id = ca.generate_node_cert("server").unwrap();
        let client_id = ca.generate_node_cert("client").unwrap();

        let server = TcpServer::bind(&server_id, &ca.cert_pem, "127.0.0.1", 0)
            .await
            .unwrap();
        let port = server.local_port().unwrap();

        let handler: EnvelopeHandler = Arc::new(|env| {
            if let Some(envelope::Body::Ping(ping)) = env.body {
                Some(make_pong(ping.nonce))
            } else {
                None
            }
        });

        tokio::spawn(async move {
            server.serve(handler).await.ok();
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let transport = TcpTransport::new(&client_id, &ca.cert_pem).unwrap();

        for i in 0..5 {
            let response = transport
                .request("127.0.0.1", port, &make_ping(i))
                .await
                .unwrap();

            if let Some(envelope::Body::Pong(pong)) = response.body {
                assert_eq!(pong.nonce, i);
            } else {
                panic!("expected Pong for nonce {i}");
            }
        }
    }
}
