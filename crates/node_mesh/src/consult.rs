//! Peer consult: ASK/ANSWER forwarding with budgets.
//!
//! When a local node can't confidently answer a question, it forwards
//! an ASK to reachable peers, respecting ttl_hops, deadline_ms, and
//! max_context_bytes budgets.

use std::sync::Arc;
use std::time::Instant;

use crate::peer_dir::PeerDirectory;
use crate::transport::Transport;
use node_proto::common::*;
use node_proto::mesh::*;
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub struct ConsultConfig {
    pub max_ttl_hops: u32,
    pub default_deadline_ms: u32,
    pub default_max_context_bytes: u32,
    pub max_peers_to_ask: usize,
    pub min_confidence: f32,
}

impl Default for ConsultConfig {
    fn default() -> Self {
        Self {
            max_ttl_hops: 3,
            default_deadline_ms: 5000,
            default_max_context_bytes: 16384,
            max_peers_to_ask: 3,
            min_confidence: 0.5,
        }
    }
}

#[derive(Debug)]
pub struct ConsultResult {
    pub answers: Vec<PeerAnswer>,
    pub refused: Vec<PeerRefusal>,
    pub timed_out: Vec<String>,
    pub best_answer: Option<PeerAnswer>,
}

#[derive(Debug, Clone)]
pub struct PeerAnswer {
    pub peer_id: String,
    pub answer: String,
    pub confidence: f32,
    pub evidence_refs: Vec<String>,
    pub warnings: Vec<String>,
    pub rtt_ms: u32,
}

#[derive(Debug, Clone)]
pub struct PeerRefusal {
    pub peer_id: String,
    pub code: String,
    pub message: String,
}

/// Build an ASK envelope with budget constraints.
pub fn build_ask_envelope(
    from_node_id: &str,
    tenant_id: &str,
    question: &str,
    context_bullets: &[String],
    ttl_hops: u32,
    deadline_ms: u32,
    max_context_bytes: u32,
) -> Envelope {
    let msg_id = uuid::Uuid::new_v4().to_string();

    Envelope {
        msg_id,
        r#type: MsgType::Ask as i32,
        from_node_id: Some(NodeId {
            value: from_node_id.into(),
        }),
        to_node_id: None,
        tenant_id: Some(TenantId {
            value: tenant_id.into(),
        }),
        sensitivity: Sensitivity::Public as i32,
        ttl_hops,
        deadline_ms,
        max_context_bytes,
        max_answer_bytes: 4096,
        policy: None,
        body: Some(envelope::Body::Ask(Ask {
            question: question.into(),
            context_bullets: truncate_context(context_bullets, max_context_bytes),
        })),
    }
}

/// Build an ANSWER envelope in response to an ASK.
pub fn build_answer_envelope(
    from_node_id: &str,
    ask_msg_id: &str,
    answer_text: &str,
    confidence: f32,
    evidence_refs: Vec<String>,
) -> Envelope {
    Envelope {
        msg_id: format!("{ask_msg_id}-answer"),
        r#type: MsgType::Answer as i32,
        from_node_id: Some(NodeId {
            value: from_node_id.into(),
        }),
        body: Some(envelope::Body::Answer(Answer {
            answer: answer_text.into(),
            confidence,
            evidence_refs,
            warnings: vec![],
        })),
        ..Default::default()
    }
}

/// Build a REFUSE envelope.
pub fn build_refuse_envelope(
    from_node_id: &str,
    ask_msg_id: &str,
    code: &str,
    message: &str,
) -> Envelope {
    Envelope {
        msg_id: format!("{ask_msg_id}-refuse"),
        r#type: MsgType::Refuse as i32,
        from_node_id: Some(NodeId {
            value: from_node_id.into(),
        }),
        body: Some(envelope::Body::Refuse(Refuse {
            code: code.into(),
            message: message.into(),
        })),
        ..Default::default()
    }
}

/// Decrement ttl_hops on a forwarded ASK. Returns None if budget exhausted.
pub fn decrement_budget(mut envelope: Envelope) -> Option<Envelope> {
    if envelope.ttl_hops == 0 {
        return None;
    }
    envelope.ttl_hops -= 1;
    Some(envelope)
}

/// Consult reachable peers for answers to a question.
pub async fn consult_peers(
    transport: &Arc<dyn Transport>,
    peer_dir: &RwLock<PeerDirectory>,
    config: &ConsultConfig,
    from_node_id: &str,
    tenant_id: &str,
    question: &str,
    context: &[String],
) -> ConsultResult {
    let peers = {
        let dir = peer_dir.read().await;
        dir.reachable_peers()
            .iter()
            .take(config.max_peers_to_ask)
            .map(|p| (p.node_id.clone(), p.address.clone(), p.port))
            .collect::<Vec<_>>()
    };

    let ask = build_ask_envelope(
        from_node_id,
        tenant_id,
        question,
        context,
        config.max_ttl_hops,
        config.default_deadline_ms,
        config.default_max_context_bytes,
    );

    let mut answers = Vec::new();
    let mut refused = Vec::new();
    let mut timed_out = Vec::new();

    for (peer_id, address, port) in &peers {
        let start = Instant::now();
        match tokio::time::timeout(
            std::time::Duration::from_millis(config.default_deadline_ms as u64),
            transport.request(address, *port, &ask),
        )
        .await
        {
            Ok(Ok(response)) => {
                let rtt_ms = start.elapsed().as_millis() as u32;
                match response.body {
                    Some(envelope::Body::Answer(ans)) => {
                        answers.push(PeerAnswer {
                            peer_id: peer_id.clone(),
                            answer: ans.answer,
                            confidence: ans.confidence,
                            evidence_refs: ans.evidence_refs,
                            warnings: ans.warnings,
                            rtt_ms,
                        });
                    }
                    Some(envelope::Body::Refuse(ref_msg)) => {
                        refused.push(PeerRefusal {
                            peer_id: peer_id.clone(),
                            code: ref_msg.code,
                            message: ref_msg.message,
                        });
                    }
                    _ => {}
                }
            }
            Ok(Err(_)) => {
                timed_out.push(peer_id.clone());
            }
            Err(_) => {
                timed_out.push(peer_id.clone());
            }
        }
    }

    let best_answer = answers
        .iter()
        .filter(|a| a.confidence >= config.min_confidence)
        .max_by(|a, b| a.confidence.partial_cmp(&b.confidence).unwrap())
        .cloned();

    ConsultResult {
        answers,
        refused,
        timed_out,
        best_answer,
    }
}

fn truncate_context(bullets: &[String], max_bytes: u32) -> Vec<String> {
    let mut result = Vec::new();
    let mut total = 0usize;
    for bullet in bullets {
        let new_total = total + bullet.len();
        if new_total > max_bytes as usize {
            break;
        }
        result.push(bullet.clone());
        total = new_total;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::MockTransport;

    #[test]
    fn build_ask_envelope_has_correct_fields() {
        let ask = build_ask_envelope("node-a", "public", "What is DNS?", &[], 3, 5000, 16384);
        assert_eq!(ask.r#type, MsgType::Ask as i32);
        assert_eq!(ask.ttl_hops, 3);
        assert_eq!(ask.deadline_ms, 5000);
        assert_eq!(ask.max_context_bytes, 16384);
        assert!(matches!(ask.body, Some(envelope::Body::Ask(_))));
    }

    #[test]
    fn build_answer_envelope_has_correct_fields() {
        let ans = build_answer_envelope("node-b", "msg-1", "DNS resolves names", 0.9, vec![]);
        assert_eq!(ans.r#type, MsgType::Answer as i32);
        if let Some(envelope::Body::Answer(a)) = &ans.body {
            assert_eq!(a.answer, "DNS resolves names");
            assert!((a.confidence - 0.9).abs() < 0.001);
        } else {
            panic!("expected Answer body");
        }
    }

    #[test]
    fn build_refuse_envelope_has_correct_fields() {
        let refuse = build_refuse_envelope("node-b", "msg-1", "NO_KNOWLEDGE", "I don't know");
        assert_eq!(refuse.r#type, MsgType::Refuse as i32);
        if let Some(envelope::Body::Refuse(r)) = &refuse.body {
            assert_eq!(r.code, "NO_KNOWLEDGE");
        } else {
            panic!("expected Refuse body");
        }
    }

    #[test]
    fn decrement_budget_works() {
        let ask = build_ask_envelope("node-a", "public", "test?", &[], 2, 5000, 16384);
        let decremented = decrement_budget(ask).unwrap();
        assert_eq!(decremented.ttl_hops, 1);

        let decremented2 = decrement_budget(decremented).unwrap();
        assert_eq!(decremented2.ttl_hops, 0);

        assert!(decrement_budget(decremented2).is_none());
    }

    #[test]
    fn truncate_context_respects_budget() {
        let bullets: Vec<String> = (0..100)
            .map(|i| format!("Bullet {i} with some text"))
            .collect();
        let result = truncate_context(&bullets, 100);
        let total_bytes: usize = result.iter().map(|s| s.len()).sum();
        assert!(total_bytes <= 100);
        assert!(!result.is_empty());
    }

    #[tokio::test]
    async fn consult_peers_with_answers() {
        let transport = Arc::new(MockTransport::new());
        let peer_dir = RwLock::new(PeerDirectory::new());
        {
            let mut dir = peer_dir.write().await;
            dir.upsert("peer-1", "192.168.1.10", 9000);
            dir.upsert("peer-2", "192.168.1.11", 9000);
        }

        transport.push_response(build_answer_envelope(
            "peer-2",
            "msg-2",
            "Answer from peer 2",
            0.9,
            vec![],
        ));
        transport.push_response(build_answer_envelope(
            "peer-1",
            "msg-1",
            "Answer from peer 1",
            0.7,
            vec![],
        ));

        let config = ConsultConfig::default();
        let result = consult_peers(
            &(transport.clone() as Arc<dyn Transport>),
            &peer_dir,
            &config,
            "node-a",
            "public",
            "What is DNS?",
            &[],
        )
        .await;

        assert_eq!(result.answers.len(), 2);
        assert!(result.best_answer.is_some());
        let best = result.best_answer.unwrap();
        assert!((best.confidence - 0.9).abs() < 0.001);
    }

    #[tokio::test]
    async fn consult_peers_with_refuse() {
        let transport = Arc::new(MockTransport::new());
        let peer_dir = RwLock::new(PeerDirectory::new());
        {
            let mut dir = peer_dir.write().await;
            dir.upsert("peer-1", "192.168.1.10", 9000);
        }

        transport.push_response(build_refuse_envelope(
            "peer-1",
            "msg-1",
            "NO_KNOWLEDGE",
            "I don't know about this",
        ));

        let config = ConsultConfig::default();
        let result = consult_peers(
            &(transport.clone() as Arc<dyn Transport>),
            &peer_dir,
            &config,
            "node-a",
            "public",
            "What is quantum computing?",
            &[],
        )
        .await;

        assert!(result.answers.is_empty());
        assert_eq!(result.refused.len(), 1);
        assert!(result.best_answer.is_none());
    }

    #[tokio::test]
    async fn consult_no_peers() {
        let transport = Arc::new(MockTransport::new());
        let peer_dir = RwLock::new(PeerDirectory::new());

        let config = ConsultConfig::default();
        let result = consult_peers(
            &(transport as Arc<dyn Transport>),
            &peer_dir,
            &config,
            "node-a",
            "public",
            "Hello?",
            &[],
        )
        .await;

        assert!(result.answers.is_empty());
        assert!(result.refused.is_empty());
        assert!(result.timed_out.is_empty());
    }
}
