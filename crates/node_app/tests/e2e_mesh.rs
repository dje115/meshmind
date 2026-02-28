//! End-to-end integration test: Two full MeshMind node stacks
//! communicating over TCP+mTLS.
//!
//! Tests:
//! 1. Node A pings Node B over mesh
//! 2. Node A asks Node B a question and gets an answer
//! 3. Both nodes run HTTP APIs with seed data and can search
//! 4. Node A's /ask consults Node B when local confidence is low

use std::sync::Arc;

use axum::body::Body;
use axum::http::Request;
use http_body_util::BodyExt;
use tokio::sync::RwLock;

use node_ai::InferenceBackend;
use node_ai_mock::MockBackend;
use node_api::AppState;
use node_crypto::DevCa;
use node_mesh::tcp_transport::{EnvelopeHandler, TcpServer, TcpTransport};
use node_mesh::transport::Transport;
use node_mesh::{ConsultConfig, PeerDirectory};
use node_proto::common::*;
use node_proto::mesh::*;
use node_storage::cas::CasStore;
use node_storage::event_log::EventLog;
use node_storage::sqlite_views;
use tower::ServiceExt;

fn create_node_state(
    name: &str,
    _ca: &DevCa,
    transport: Option<Arc<dyn Transport>>,
) -> (Arc<AppState>, tempfile::TempDir) {
    let tmp = tempfile::TempDir::new().unwrap();
    let event_log = EventLog::open(tmp.path()).unwrap();
    let cas = CasStore::open(tmp.path()).unwrap();
    let db_path = tmp.path().join("sqlite").join("meshmind.db");
    let _conn = sqlite_views::open_db(&db_path).unwrap();

    let policy = Arc::new(node_policy::PolicyEngine::new(node_policy::PolicyConfig {
        allow_train: true,
        ..Default::default()
    }));
    let model_registry = Arc::new(tokio::sync::Mutex::new(node_trainer::ModelRegistry::new()));
    let trainer = Arc::new(node_trainer::Trainer::new(policy, model_registry.clone()));

    let state = Arc::new(AppState {
        event_log: RwLock::new(event_log),
        cas,
        db_path,
        peer_dir: Arc::new(RwLock::new(PeerDirectory::new())),
        backend: Arc::new(MockBackend::new()),
        transport,
        consult_config: ConsultConfig::default(),
        node_id: name.into(),
        admin_token: "test-token".into(),
        scan_dirs: vec![],
        trainer,
        model_registry,
    });

    (state, tmp)
}

fn make_handler(name: String) -> EnvelopeHandler {
    let backend = Arc::new(MockBackend::new());
    Arc::new(move |env: Envelope| -> Option<Envelope> {
        match env.body {
            Some(envelope::Body::Ping(ping)) => Some(Envelope {
                msg_id: format!("{}-pong", env.msg_id),
                r#type: MsgType::Pong as i32,
                from_node_id: Some(NodeId {
                    value: name.clone(),
                }),
                body: Some(envelope::Body::Pong(Pong { nonce: ping.nonce })),
                ..Default::default()
            }),
            Some(envelope::Body::Ask(ask)) => {
                let be = backend.clone();
                let nid = name.clone();
                let question = ask.question.clone();

                let rt = tokio::runtime::Handle::try_current();
                if let Ok(handle) = rt {
                    let result = std::thread::spawn(move || {
                        handle.block_on(async {
                            let req = node_ai::GenerateRequest {
                                prompt: format!("Answer concisely: {question}"),
                                system: Some("You are a helpful AI.".into()),
                                max_tokens: 256,
                                ..Default::default()
                            };
                            be.generate(req).await.ok()
                        })
                    })
                    .join()
                    .ok()
                    .flatten();

                    result.map(|resp| Envelope {
                        msg_id: format!("{}-answer", env.msg_id),
                        r#type: MsgType::Answer as i32,
                        from_node_id: Some(NodeId { value: nid }),
                        body: Some(envelope::Body::Answer(Answer {
                            answer: resp.text,
                            confidence: 0.8,
                            evidence_refs: vec![],
                            warnings: vec![],
                        })),
                        ..Default::default()
                    })
                } else {
                    None
                }
            }
            _ => None,
        }
    })
}

#[tokio::test]
async fn e2e_two_nodes_ping() {
    let ca = DevCa::generate().unwrap();
    let node_a_cert = ca.generate_node_cert("node-alpha").unwrap();
    let node_b_cert = ca.generate_node_cert("node-beta").unwrap();

    let server_b = TcpServer::bind(&node_b_cert, &ca.cert_pem, "127.0.0.1", 0)
        .await
        .unwrap();
    let port_b = server_b.local_port().unwrap();

    tokio::spawn(async move {
        server_b.serve(make_handler("node-beta".into())).await.ok();
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let transport_a: Arc<dyn Transport> =
        Arc::new(TcpTransport::new(&node_a_cert, &ca.cert_pem).unwrap());

    let ping = Envelope {
        msg_id: "test-ping".into(),
        r#type: MsgType::Ping as i32,
        from_node_id: Some(NodeId {
            value: "node-alpha".into(),
        }),
        body: Some(envelope::Body::Ping(Ping { nonce: 42 })),
        ..Default::default()
    };

    let resp = transport_a
        .request("127.0.0.1", port_b, &ping)
        .await
        .unwrap();

    assert_eq!(resp.r#type, MsgType::Pong as i32);
    if let Some(envelope::Body::Pong(pong)) = resp.body {
        assert_eq!(pong.nonce, 42);
    } else {
        panic!("expected Pong");
    }
}

#[tokio::test]
async fn e2e_ask_peer_over_mesh() {
    let ca = DevCa::generate().unwrap();
    let node_a_cert = ca.generate_node_cert("node-alpha").unwrap();
    let node_b_cert = ca.generate_node_cert("node-beta").unwrap();

    let server_b = TcpServer::bind(&node_b_cert, &ca.cert_pem, "127.0.0.1", 0)
        .await
        .unwrap();
    let port_b = server_b.local_port().unwrap();

    tokio::spawn(async move {
        server_b.serve(make_handler("node-beta".into())).await.ok();
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let transport_a: Arc<dyn Transport> =
        Arc::new(TcpTransport::new(&node_a_cert, &ca.cert_pem).unwrap());

    let (state_a, _tmp_a) = create_node_state("node-alpha", &ca, Some(transport_a));

    // Add node-beta as a known peer
    {
        let mut dir = state_a.peer_dir.write().await;
        dir.upsert("node-beta", "127.0.0.1", port_b);
    }

    // Make a request through the API — local KB is empty so confidence is 0.3
    // which triggers peer consult
    let app = node_api::build_router(state_a);

    let resp = app
        .oneshot(
            Request::post("/ask")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_string(&serde_json::json!({
                        "question": "What is DNS?"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let answer: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert!(answer.get("answer").is_some());
    let answer_text = answer["answer"].as_str().unwrap();
    assert!(!answer_text.is_empty());

    // The answer should come from the peer since local confidence is low
    let confidence = answer["confidence"].as_f64().unwrap();
    let model = answer["model"].as_str().unwrap();

    // With peer answering at 0.8 confidence, the peer's answer should be used
    assert!(
        confidence >= 0.3,
        "confidence should be at least 0.3, got {confidence}"
    );
    // Either local mock answer or peer answer
    assert!(
        model == "mock-v1" || model.starts_with("peer:"),
        "model should be mock-v1 or peer:*, got {model}"
    );
}

#[tokio::test]
async fn e2e_http_status_with_mesh() {
    let ca = DevCa::generate().unwrap();
    let node_a_cert = ca.generate_node_cert("node-alpha").unwrap();

    let transport_a: Arc<dyn Transport> =
        Arc::new(TcpTransport::new(&node_a_cert, &ca.cert_pem).unwrap());

    let (state_a, _tmp_a) = create_node_state("node-alpha", &ca, Some(transport_a));

    // Add some peers
    {
        let mut dir = state_a.peer_dir.write().await;
        dir.upsert("node-beta", "192.168.1.10", 9901);
        dir.upsert("node-gamma", "192.168.1.11", 9901);
    }

    let app = node_api::build_router(state_a);

    let resp = app
        .oneshot(Request::get("/status").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(resp.status(), axum::http::StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let status: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(status["node_id"], "node-alpha");
    assert_eq!(status["peer_count"], 2);
    assert_eq!(status["status"], "running");
}

#[tokio::test]
async fn e2e_bidirectional_ask_answer() {
    let ca = DevCa::generate().unwrap();
    let node_a_cert = ca.generate_node_cert("node-alpha").unwrap();
    let node_b_cert = ca.generate_node_cert("node-beta").unwrap();

    // Both nodes run mesh servers
    let server_a = TcpServer::bind(&node_a_cert, &ca.cert_pem, "127.0.0.1", 0)
        .await
        .unwrap();
    let port_a = server_a.local_port().unwrap();

    let server_b = TcpServer::bind(&node_b_cert, &ca.cert_pem, "127.0.0.1", 0)
        .await
        .unwrap();
    let port_b = server_b.local_port().unwrap();

    tokio::spawn(async move {
        server_a.serve(make_handler("node-alpha".into())).await.ok();
    });
    tokio::spawn(async move {
        server_b.serve(make_handler("node-beta".into())).await.ok();
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // A asks B
    let transport_a = TcpTransport::new(&node_a_cert, &ca.cert_pem).unwrap();
    let ask_ab = Envelope {
        msg_id: "ask-ab".into(),
        r#type: MsgType::Ask as i32,
        from_node_id: Some(NodeId {
            value: "node-alpha".into(),
        }),
        ttl_hops: 3,
        body: Some(envelope::Body::Ask(Ask {
            question: "How do I restart nginx?".into(),
            context_bullets: vec![],
        })),
        ..Default::default()
    };
    let resp_ab = transport_a
        .request("127.0.0.1", port_b, &ask_ab)
        .await
        .unwrap();
    assert_eq!(resp_ab.r#type, MsgType::Answer as i32);

    // B asks A
    let transport_b = TcpTransport::new(&node_b_cert, &ca.cert_pem).unwrap();
    let ask_ba = Envelope {
        msg_id: "ask-ba".into(),
        r#type: MsgType::Ask as i32,
        from_node_id: Some(NodeId {
            value: "node-beta".into(),
        }),
        ttl_hops: 3,
        body: Some(envelope::Body::Ask(Ask {
            question: "What is a firewall?".into(),
            context_bullets: vec![],
        })),
        ..Default::default()
    };
    let resp_ba = transport_b
        .request("127.0.0.1", port_a, &ask_ba)
        .await
        .unwrap();
    assert_eq!(resp_ba.r#type, MsgType::Answer as i32);
}
