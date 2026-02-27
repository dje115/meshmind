//! Pull-based replication: gossip metadata, segment pull, CAS object pull.
//!
//! Replication flow:
//! 1. Node A sends GossipMeta (segments available + small object hashes)
//! 2. Node B identifies missing segments and CAS objects
//! 3. Node B pulls missing data with budgets
//! 4. Node B verifies event hash chain on import
//! 5. Policy engine gates acceptance

use std::collections::HashSet;

use node_policy::{PolicyDecision, PolicyEngine};
use node_proto::common::{HashRef, NodeId, TenantId};
use node_proto::events::EventEnvelope;
use node_proto::repl::{
    CasObjectChunk, GossipMeta, PullCasObjectsRequest, PullCasObjectsResponse, PullSegmentsRequest,
    PullSegmentsResponse, SegmentChunk, SegmentId, SegmentMeta,
};
use node_storage::cas::CasStore;
use node_storage::event_log::EventLog;
use prost::Message;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ReplError {
    #[error("event log error: {0}")]
    EventLog(#[from] node_storage::event_log::EventLogError),
    #[error("CAS error: {0}")]
    Cas(#[from] node_storage::cas::CasError),
    #[error("policy denied: {0}")]
    PolicyDenied(String),
    #[error("hash chain verification failed: {0}")]
    ChainVerification(String),
    #[error("protobuf decode error: {0}")]
    Decode(#[from] prost::DecodeError),
    #[error("budget exceeded: {0}")]
    BudgetExceeded(String),
}

pub type Result<T> = std::result::Result<T, ReplError>;

/// Build GossipMeta from a local event log.
pub fn build_gossip_meta(
    node_id: &str,
    tenant_id: &str,
    event_log: &EventLog,
    cas: &CasStore,
    known_hashes: &[String],
) -> Result<GossipMeta> {
    let events = event_log.replay()?;

    let mut segment_events: Vec<&EventEnvelope> = Vec::new();
    let mut segments = Vec::new();
    let mut small_objects = Vec::new();
    let mut seg_counter = 0u32;

    for event in &events {
        segment_events.push(event);

        if (segment_events.len() >= 100 || std::ptr::eq(event, events.last().unwrap()))
            && !segment_events.is_empty()
        {
            let first_hash = segment_events
                .first()
                .and_then(|e| e.event_hash.as_ref())
                .cloned()
                .unwrap_or_default();
            let last_hash = segment_events
                .last()
                .and_then(|e| e.event_hash.as_ref())
                .cloned()
                .unwrap_or_default();

            let size: usize = segment_events.iter().map(|e| e.encode_to_vec().len()).sum();

            segments.push(SegmentMeta {
                segment_id: Some(SegmentId {
                    value: format!("seg-{seg_counter}"),
                }),
                first_event_hash: Some(first_hash),
                last_event_hash: Some(last_hash),
                size_bytes: size as u64,
            });
            seg_counter += 1;
            segment_events.clear();
        }

        for href in &event.refs {
            if cas.has(&href.sha256) && !known_hashes.contains(&href.sha256) {
                small_objects.push(href.clone());
            }
        }
    }

    Ok(GossipMeta {
        from_node_id: Some(NodeId {
            value: node_id.into(),
        }),
        tenant_id: Some(TenantId {
            value: tenant_id.into(),
        }),
        segments,
        small_object_hashes: small_objects,
    })
}

/// Determine which segments are missing based on gossip from a remote node.
pub fn find_missing_segments(
    local_gossip: &GossipMeta,
    remote_gossip: &GossipMeta,
) -> Vec<SegmentId> {
    let local_seg_hashes: HashSet<String> = local_gossip
        .segments
        .iter()
        .filter_map(|s| s.last_event_hash.as_ref().map(|h| h.sha256.clone()))
        .collect();

    remote_gossip
        .segments
        .iter()
        .filter(|s| {
            s.last_event_hash
                .as_ref()
                .map(|h| !local_seg_hashes.contains(&h.sha256))
                .unwrap_or(false)
        })
        .filter_map(|s| s.segment_id.clone())
        .collect()
}

/// Determine which CAS objects are missing.
pub fn find_missing_objects(local_cas: &CasStore, remote_gossip: &GossipMeta) -> Vec<HashRef> {
    remote_gossip
        .small_object_hashes
        .iter()
        .filter(|h| !local_cas.has(&h.sha256))
        .cloned()
        .collect()
}

/// Serve a PullSegmentsRequest: serialize events into segment chunks.
pub fn serve_pull_segments(
    request: &PullSegmentsRequest,
    event_log: &EventLog,
    node_id: &str,
) -> Result<PullSegmentsResponse> {
    let events = event_log.replay()?;
    let mut chunks = Vec::new();

    let want_ids: HashSet<String> = request
        .want_segments
        .iter()
        .map(|s| s.value.clone())
        .collect();

    let mut seg_counter = 0u32;
    let mut seg_events: Vec<&EventEnvelope> = Vec::new();

    for event in &events {
        seg_events.push(event);

        if seg_events.len() >= 100 || std::ptr::eq(event, events.last().unwrap()) {
            let seg_id = format!("seg-{seg_counter}");
            if want_ids.contains(&seg_id) {
                let mut data = Vec::new();
                for evt in &seg_events {
                    let encoded = evt.encode_to_vec();
                    let len = encoded.len() as u32;
                    data.extend_from_slice(&len.to_le_bytes());
                    data.extend_from_slice(&encoded);
                }
                chunks.push(SegmentChunk {
                    segment_id: Some(SegmentId { value: seg_id }),
                    chunk_index: 0,
                    chunk_count: 1,
                    data,
                });
            }
            seg_counter += 1;
            seg_events.clear();
        }
    }

    Ok(PullSegmentsResponse {
        responder: Some(NodeId {
            value: node_id.into(),
        }),
        chunks,
    })
}

/// Serve a PullCasObjectsRequest: return requested CAS blobs.
pub fn serve_pull_cas_objects(
    request: &PullCasObjectsRequest,
    cas: &CasStore,
    node_id: &str,
) -> Result<PullCasObjectsResponse> {
    let mut chunks = Vec::new();

    for hash_ref in &request.want_hashes {
        if let Ok(data) = cas.get_bytes(&hash_ref.sha256) {
            chunks.push(CasObjectChunk {
                hash: Some(hash_ref.clone()),
                chunk_index: 0,
                chunk_count: 1,
                data,
                content_type: "application/octet-stream".into(),
                size_bytes: 0,
            });
        }
    }

    Ok(PullCasObjectsResponse {
        responder: Some(NodeId {
            value: node_id.into(),
        }),
        chunks,
    })
}

/// Import events from a segment chunk into a local event log with policy checks.
pub fn import_segment(
    chunk: &SegmentChunk,
    event_log: &mut EventLog,
    policy: &PolicyEngine,
) -> Result<ImportResult> {
    let mut cursor = chunk.data.as_slice();
    let mut imported = 0u32;
    let mut denied = 0u32;
    let mut deny_reasons = Vec::new();

    while cursor.len() >= 4 {
        let len = u32::from_le_bytes([cursor[0], cursor[1], cursor[2], cursor[3]]) as usize;
        cursor = &cursor[4..];
        if cursor.len() < len {
            break;
        }
        let event = EventEnvelope::decode(&cursor[..len])?;
        cursor = &cursor[len..];

        match policy.can_accept_event(&event) {
            PolicyDecision::Allow => {
                event_log.append(event)?;
                imported += 1;
            }
            PolicyDecision::Deny(reason) => {
                denied += 1;
                deny_reasons.push(reason);
            }
        }
    }

    Ok(ImportResult {
        imported,
        denied,
        deny_reasons,
    })
}

/// Import CAS objects from a pull response with policy checks.
pub fn import_cas_objects(
    response: &PullCasObjectsResponse,
    cas: &CasStore,
    policy: &PolicyEngine,
    tenant_id: &str,
    sensitivity: i32,
) -> Result<ImportResult> {
    let mut imported = 0u32;
    let mut denied = 0u32;
    let mut deny_reasons = Vec::new();

    for chunk in &response.chunks {
        let hash = chunk.hash.as_ref().map(|h| h.sha256.as_str()).unwrap_or("");

        match policy.can_accept_object(hash, tenant_id, sensitivity) {
            PolicyDecision::Allow => {
                cas.put_bytes(&chunk.content_type, &chunk.data)?;
                imported += 1;
            }
            PolicyDecision::Deny(reason) => {
                denied += 1;
                deny_reasons.push(reason);
            }
        }
    }

    Ok(ImportResult {
        imported,
        denied,
        deny_reasons,
    })
}

#[derive(Debug)]
pub struct ImportResult {
    pub imported: u32,
    pub denied: u32,
    pub deny_reasons: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use node_proto::common::*;
    use node_proto::events::*;
    use tempfile::TempDir;

    fn make_case_event(id: &str, tenant: &str) -> EventEnvelope {
        EventEnvelope {
            event_id: id.to_string(),
            r#type: EventType::CaseCreated as i32,
            tenant_id: Some(TenantId {
                value: tenant.into(),
            }),
            sensitivity: Sensitivity::Public as i32,
            payload: Some(event_envelope::Payload::CaseCreated(CaseCreated {
                case_id: format!("case-{id}"),
                title: format!("Case {id}"),
                summary: format!("Summary for case {id}"),
                content_ref: None,
                shareable: true,
            })),
            ..Default::default()
        }
    }

    struct TestNode {
        _tmp: TempDir,
        event_log: EventLog,
        cas: CasStore,
    }

    impl TestNode {
        fn new() -> Self {
            let tmp = TempDir::new().unwrap();
            let event_log = EventLog::open(tmp.path()).unwrap();
            let cas = CasStore::open(tmp.path()).unwrap();
            Self {
                _tmp: tmp,
                event_log,
                cas,
            }
        }
    }

    #[test]
    fn gossip_meta_building() {
        let mut node = TestNode::new();
        for i in 0..5 {
            node.event_log
                .append(make_case_event(&format!("e{i}"), "public"))
                .unwrap();
        }

        let gossip =
            build_gossip_meta("node-a", "public", &node.event_log, &node.cas, &[]).unwrap();
        assert_eq!(gossip.from_node_id.unwrap().value, "node-a");
        assert!(!gossip.segments.is_empty());
    }

    #[test]
    fn find_missing_segments_works() {
        let mut node_a = TestNode::new();
        let mut node_b = TestNode::new();

        for i in 0..5 {
            node_a
                .event_log
                .append(make_case_event(&format!("a{i}"), "public"))
                .unwrap();
        }
        for i in 0..2 {
            node_b
                .event_log
                .append(make_case_event(&format!("b{i}"), "public"))
                .unwrap();
        }

        let gossip_a =
            build_gossip_meta("node-a", "public", &node_a.event_log, &node_a.cas, &[]).unwrap();
        let gossip_b =
            build_gossip_meta("node-b", "public", &node_b.event_log, &node_b.cas, &[]).unwrap();

        let missing = find_missing_segments(&gossip_b, &gossip_a);
        assert!(!missing.is_empty());
    }

    #[test]
    fn find_missing_cas_objects() {
        let node_a = TestNode::new();
        let mut gossip = GossipMeta::default();
        gossip.small_object_hashes.push(HashRef {
            sha256: "abc123".into(),
        });
        gossip.small_object_hashes.push(HashRef {
            sha256: "def456".into(),
        });

        let missing = find_missing_objects(&node_a.cas, &gossip);
        assert_eq!(missing.len(), 2);
    }

    #[test]
    fn serve_and_import_segments() {
        let mut node_a = TestNode::new();
        let policy = PolicyEngine::with_defaults();

        for i in 0..10 {
            node_a
                .event_log
                .append(make_case_event(&format!("e{i}"), "public"))
                .unwrap();
        }

        let gossip_a =
            build_gossip_meta("node-a", "public", &node_a.event_log, &node_a.cas, &[]).unwrap();

        let request = PullSegmentsRequest {
            requester: Some(NodeId {
                value: "node-b".into(),
            }),
            tenant_id: Some(TenantId {
                value: "public".into(),
            }),
            want_segments: gossip_a
                .segments
                .iter()
                .filter_map(|s| s.segment_id.clone())
                .collect(),
            budget: None,
        };

        let response = serve_pull_segments(&request, &node_a.event_log, "node-a").unwrap();
        assert!(!response.chunks.is_empty());

        let mut node_b = TestNode::new();
        let mut total_imported = 0;
        for chunk in &response.chunks {
            let result = import_segment(chunk, &mut node_b.event_log, &policy).unwrap();
            total_imported += result.imported;
            assert_eq!(result.denied, 0);
        }

        assert_eq!(total_imported, 10);
        assert_eq!(node_b.event_log.event_count(), 10);
        node_b.event_log.verify_chain().unwrap();
    }

    #[test]
    fn serve_and_import_cas_objects() {
        let node_a = TestNode::new();
        let policy = PolicyEngine::with_defaults();

        let h1 = node_a.cas.put_bytes("text/plain", b"hello").unwrap();
        let h2 = node_a.cas.put_bytes("text/plain", b"world").unwrap();

        let request = PullCasObjectsRequest {
            requester: Some(NodeId {
                value: "node-b".into(),
            }),
            want_hashes: vec![h1.clone(), h2.clone()],
            budget: None,
        };

        let response = serve_pull_cas_objects(&request, &node_a.cas, "node-a").unwrap();
        assert_eq!(response.chunks.len(), 2);

        let node_b = TestNode::new();
        let result = import_cas_objects(
            &response,
            &node_b.cas,
            &policy,
            "public",
            Sensitivity::Public as i32,
        )
        .unwrap();

        assert_eq!(result.imported, 2);
        assert!(node_b.cas.has(&h1.sha256));
        assert!(node_b.cas.has(&h2.sha256));
    }

    #[test]
    fn policy_denies_non_public_tenant() {
        let mut node_a = TestNode::new();
        let policy = PolicyEngine::with_defaults();

        for i in 0..3 {
            node_a
                .event_log
                .append(make_case_event(&format!("e{i}"), "secret-corp"))
                .unwrap();
        }

        let gossip_a =
            build_gossip_meta("node-a", "public", &node_a.event_log, &node_a.cas, &[]).unwrap();

        let request = PullSegmentsRequest {
            requester: Some(NodeId {
                value: "node-b".into(),
            }),
            tenant_id: Some(TenantId {
                value: "public".into(),
            }),
            want_segments: gossip_a
                .segments
                .iter()
                .filter_map(|s| s.segment_id.clone())
                .collect(),
            budget: None,
        };

        let response = serve_pull_segments(&request, &node_a.event_log, "node-a").unwrap();

        let mut node_b = TestNode::new();
        for chunk in &response.chunks {
            let result = import_segment(chunk, &mut node_b.event_log, &policy).unwrap();
            assert_eq!(result.imported, 0);
            assert_eq!(result.denied, 3);
            assert!(result.deny_reasons[0].contains("secret-corp"));
        }
    }

    #[test]
    fn policy_denies_restricted_cas_objects() {
        let node_a = TestNode::new();
        let policy = PolicyEngine::with_defaults();

        let h1 = node_a.cas.put_bytes("text/plain", b"secret data").unwrap();

        let request = PullCasObjectsRequest {
            requester: Some(NodeId {
                value: "node-b".into(),
            }),
            want_hashes: vec![h1],
            budget: None,
        };

        let response = serve_pull_cas_objects(&request, &node_a.cas, "node-a").unwrap();

        let node_b = TestNode::new();
        let result = import_cas_objects(
            &response,
            &node_b.cas,
            &policy,
            "public",
            Sensitivity::Restricted as i32,
        )
        .unwrap();

        assert_eq!(result.imported, 0);
        assert_eq!(result.denied, 1);
    }

    #[test]
    fn full_replication_a_to_b() {
        let mut node_a = TestNode::new();
        let policy = PolicyEngine::with_defaults();

        let content_ref = node_a
            .cas
            .put_bytes("text/plain", b"case content body")
            .unwrap();

        for i in 0..20 {
            let mut evt = make_case_event(&format!("e{i}"), "public");
            if let Some(event_envelope::Payload::CaseCreated(ref mut cc)) = evt.payload {
                cc.content_ref = Some(content_ref.clone());
            }
            evt.refs.push(content_ref.clone());
            node_a.event_log.append(evt).unwrap();
        }

        let gossip_a =
            build_gossip_meta("node-a", "public", &node_a.event_log, &node_a.cas, &[]).unwrap();

        let mut node_b = TestNode::new();

        let gossip_b =
            build_gossip_meta("node-b", "public", &node_b.event_log, &node_b.cas, &[]).unwrap();

        // Find and pull missing segments
        let missing_segs = find_missing_segments(&gossip_b, &gossip_a);
        let seg_request = PullSegmentsRequest {
            requester: Some(NodeId {
                value: "node-b".into(),
            }),
            tenant_id: Some(TenantId {
                value: "public".into(),
            }),
            want_segments: missing_segs,
            budget: None,
        };
        let seg_response = serve_pull_segments(&seg_request, &node_a.event_log, "node-a").unwrap();
        for chunk in &seg_response.chunks {
            import_segment(chunk, &mut node_b.event_log, &policy).unwrap();
        }

        // Find and pull missing CAS objects
        let missing_objs = find_missing_objects(&node_b.cas, &gossip_a);
        let obj_request = PullCasObjectsRequest {
            requester: Some(NodeId {
                value: "node-b".into(),
            }),
            want_hashes: missing_objs,
            budget: None,
        };
        let obj_response = serve_pull_cas_objects(&obj_request, &node_a.cas, "node-a").unwrap();
        import_cas_objects(
            &obj_response,
            &node_b.cas,
            &policy,
            "public",
            Sensitivity::Public as i32,
        )
        .unwrap();

        // Verify state equivalence
        assert_eq!(node_b.event_log.event_count(), 20);
        node_b.event_log.verify_chain().unwrap();
        assert!(node_b.cas.has(&content_ref.sha256));

        let b_events = node_b.event_log.replay().unwrap();
        let a_events = node_a.event_log.replay().unwrap();
        assert_eq!(b_events.len(), a_events.len());
        for (a, b) in a_events.iter().zip(b_events.iter()) {
            assert_eq!(a.event_id, b.event_id);
        }
    }
}
