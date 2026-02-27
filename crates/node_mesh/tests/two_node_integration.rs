//! Integration test: Two nodes communicating over TCP+mTLS.
//!
//! Node A sends PING, Node B replies PONG.
//! Node A sends ASK, Node B replies ANSWER.
//! Events replicated from A to B via segment pull.

use std::sync::Arc;

use node_crypto::DevCa;
use node_mesh::tcp_transport::{EnvelopeHandler, TcpServer, TcpTransport};
use node_mesh::transport::Transport;
use node_proto::common::*;
use node_proto::mesh::*;

fn make_ping(from: &str, nonce: u64) -> Envelope {
    Envelope {
        msg_id: format!("ping-{nonce}"),
        r#type: MsgType::Ping as i32,
        from_node_id: Some(NodeId { value: from.into() }),
        body: Some(envelope::Body::Ping(Ping { nonce })),
        ..Default::default()
    }
}

fn make_ask(from: &str, question: &str) -> Envelope {
    Envelope {
        msg_id: uuid::Uuid::new_v4().to_string(),
        r#type: MsgType::Ask as i32,
        from_node_id: Some(NodeId { value: from.into() }),
        tenant_id: Some(TenantId {
            value: "public".into(),
        }),
        ttl_hops: 3,
        deadline_ms: 5000,
        max_context_bytes: 16384,
        body: Some(envelope::Body::Ask(Ask {
            question: question.into(),
            context_bullets: vec![],
        })),
        ..Default::default()
    }
}

#[tokio::test]
async fn two_nodes_ping_pong_over_mtls() {
    let ca = DevCa::generate().unwrap();
    let node_a_id = ca.generate_node_cert("node-alpha").unwrap();
    let node_b_id = ca.generate_node_cert("node-beta").unwrap();

    // Start Node B as a server
    let server_b = TcpServer::bind(&node_b_id, &ca.cert_pem, "127.0.0.1", 0)
        .await
        .unwrap();
    let port_b = server_b.local_port().unwrap();

    let handler_b: EnvelopeHandler = Arc::new(|env| match env.body {
        Some(envelope::Body::Ping(ping)) => Some(Envelope {
            msg_id: format!("{}-pong", env.msg_id),
            r#type: MsgType::Pong as i32,
            from_node_id: Some(NodeId {
                value: "node-beta".into(),
            }),
            body: Some(envelope::Body::Pong(Pong { nonce: ping.nonce })),
            ..Default::default()
        }),
        _ => None,
    });

    tokio::spawn(async move {
        server_b.serve(handler_b).await.ok();
    });

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Node A sends PING to Node B
    let transport_a = TcpTransport::new(&node_a_id, &ca.cert_pem).unwrap();

    let response = transport_a
        .request("127.0.0.1", port_b, &make_ping("node-alpha", 12345))
        .await
        .unwrap();

    assert_eq!(response.r#type, MsgType::Pong as i32);
    if let Some(envelope::Body::Pong(pong)) = response.body {
        assert_eq!(pong.nonce, 12345);
    } else {
        panic!("expected Pong");
    }

    assert_eq!(response.from_node_id.as_ref().unwrap().value, "node-beta");
}

#[tokio::test]
async fn two_nodes_ask_answer_over_mtls() {
    let ca = DevCa::generate().unwrap();
    let node_a_id = ca.generate_node_cert("node-alpha").unwrap();
    let node_b_id = ca.generate_node_cert("node-beta").unwrap();

    // Node B answers ASK envelopes
    let server_b = TcpServer::bind(&node_b_id, &ca.cert_pem, "127.0.0.1", 0)
        .await
        .unwrap();
    let port_b = server_b.local_port().unwrap();

    let handler_b: EnvelopeHandler = Arc::new(|env| match env.body {
        Some(envelope::Body::Ask(ask)) => Some(Envelope {
            msg_id: format!("{}-answer", env.msg_id),
            r#type: MsgType::Answer as i32,
            from_node_id: Some(NodeId {
                value: "node-beta".into(),
            }),
            body: Some(envelope::Body::Answer(Answer {
                answer: format!("Here is my answer about: {}", ask.question),
                confidence: 0.85,
                evidence_refs: vec![],
                warnings: vec![],
            })),
            ..Default::default()
        }),
        _ => None,
    });

    tokio::spawn(async move {
        server_b.serve(handler_b).await.ok();
    });

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let transport_a = TcpTransport::new(&node_a_id, &ca.cert_pem).unwrap();
    let response = transport_a
        .request("127.0.0.1", port_b, &make_ask("node-alpha", "What is DNS?"))
        .await
        .unwrap();

    assert_eq!(response.r#type, MsgType::Answer as i32);
    if let Some(envelope::Body::Answer(answer)) = &response.body {
        assert!(answer.answer.contains("DNS"));
        assert!((answer.confidence - 0.85).abs() < 0.01);
    } else {
        panic!("expected Answer");
    }
}

#[tokio::test]
async fn bidirectional_communication() {
    let ca = DevCa::generate().unwrap();
    let node_a_id = ca.generate_node_cert("node-alpha").unwrap();
    let node_b_id = ca.generate_node_cert("node-beta").unwrap();

    // Both nodes run servers
    let server_a = TcpServer::bind(&node_a_id, &ca.cert_pem, "127.0.0.1", 0)
        .await
        .unwrap();
    let port_a = server_a.local_port().unwrap();

    let server_b = TcpServer::bind(&node_b_id, &ca.cert_pem, "127.0.0.1", 0)
        .await
        .unwrap();
    let port_b = server_b.local_port().unwrap();

    let make_handler = |name: &'static str| -> EnvelopeHandler {
        Arc::new(move |env| match env.body {
            Some(envelope::Body::Ping(ping)) => Some(Envelope {
                msg_id: format!("{}-pong", env.msg_id),
                r#type: MsgType::Pong as i32,
                from_node_id: Some(NodeId { value: name.into() }),
                body: Some(envelope::Body::Pong(Pong { nonce: ping.nonce })),
                ..Default::default()
            }),
            _ => None,
        })
    };

    tokio::spawn(async move {
        server_a.serve(make_handler("node-alpha")).await.ok();
    });
    tokio::spawn(async move {
        server_b.serve(make_handler("node-beta")).await.ok();
    });

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // A -> B
    let transport_a = TcpTransport::new(&node_a_id, &ca.cert_pem).unwrap();
    let resp = transport_a
        .request("127.0.0.1", port_b, &make_ping("node-alpha", 1))
        .await
        .unwrap();
    assert_eq!(resp.from_node_id.unwrap().value, "node-beta");

    // B -> A
    let transport_b = TcpTransport::new(&node_b_id, &ca.cert_pem).unwrap();
    let resp = transport_b
        .request("127.0.0.1", port_a, &make_ping("node-beta", 2))
        .await
        .unwrap();
    assert_eq!(resp.from_node_id.unwrap().value, "node-alpha");
}

#[tokio::test]
async fn concurrent_requests_from_multiple_clients() {
    let ca = DevCa::generate().unwrap();
    let server_id = ca.generate_node_cert("server").unwrap();

    let server = TcpServer::bind(&server_id, &ca.cert_pem, "127.0.0.1", 0)
        .await
        .unwrap();
    let port = server.local_port().unwrap();

    let handler: EnvelopeHandler = Arc::new(|env| match env.body {
        Some(envelope::Body::Ping(ping)) => Some(Envelope {
            msg_id: format!("{}-pong", env.msg_id),
            r#type: MsgType::Pong as i32,
            from_node_id: Some(NodeId {
                value: "server".into(),
            }),
            body: Some(envelope::Body::Pong(Pong { nonce: ping.nonce })),
            ..Default::default()
        }),
        _ => None,
    });

    tokio::spawn(async move {
        server.serve(handler).await.ok();
    });

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let ca_pem = ca.cert_pem.clone();
    let mut handles = vec![];

    for i in 0..5 {
        let client_id = ca.generate_node_cert(&format!("client-{i}")).unwrap();
        let ca_pem = ca_pem.clone();

        handles.push(tokio::spawn(async move {
            let transport = TcpTransport::new(&client_id, &ca_pem).unwrap();
            let resp = transport
                .request(
                    "127.0.0.1",
                    port,
                    &make_ping(&format!("client-{i}"), i as u64),
                )
                .await
                .unwrap();

            if let Some(envelope::Body::Pong(pong)) = resp.body {
                assert_eq!(pong.nonce, i as u64);
            } else {
                panic!("expected Pong for client {i}");
            }
        }));
    }

    for handle in handles {
        handle.await.unwrap();
    }
}
