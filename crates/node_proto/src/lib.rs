//! Generated protobuf types for MeshMind.

pub mod common {
    include!(concat!(env!("OUT_DIR"), "/meshmind.common.rs"));
}

pub mod cas {
    include!(concat!(env!("OUT_DIR"), "/meshmind.cas.rs"));
}

pub mod events {
    include!(concat!(env!("OUT_DIR"), "/meshmind.events.rs"));
}

pub mod snapshot {
    include!(concat!(env!("OUT_DIR"), "/meshmind.snapshot.rs"));
}

pub mod repl {
    include!(concat!(env!("OUT_DIR"), "/meshmind.repl.rs"));
}

pub mod mesh {
    include!(concat!(env!("OUT_DIR"), "/meshmind.mesh.rs"));
}

pub mod research {
    include!(concat!(env!("OUT_DIR"), "/meshmind.research.rs"));
}

pub mod training {
    include!(concat!(env!("OUT_DIR"), "/meshmind.training.rs"));
}

#[cfg(test)]
mod tests {
    use prost::Message;

    fn roundtrip<M: Message + Default + PartialEq + std::fmt::Debug>(msg: &M) -> M {
        let encoded = msg.encode_to_vec();
        assert!(!encoded.is_empty(), "encoded message should not be empty");
        M::decode(encoded.as_slice()).expect("decode should succeed")
    }

    #[test]
    fn common_types_roundtrip() {
        use super::common::*;

        let ts = Timestamp {
            unix_ms: 1700000000000,
        };
        assert_eq!(roundtrip(&ts), ts);

        let tid = TenantId {
            value: "acme-corp".into(),
        };
        assert_eq!(roundtrip(&tid), tid);

        let nid = NodeId {
            value: "node-abc-123".into(),
        };
        assert_eq!(roundtrip(&nid), nid);

        let href = HashRef {
            sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".into(),
        };
        assert_eq!(roundtrip(&href), href);

        let budget = Budget {
            max_bytes: 1_000_000,
            max_items: 100,
            max_ms: 5000,
        };
        assert_eq!(roundtrip(&budget), budget);

        let kv = KeyValue {
            key: "env".into(),
            value: "prod".into(),
        };
        assert_eq!(roundtrip(&kv), kv);
    }

    #[test]
    fn sensitivity_enum_values() {
        use super::common::Sensitivity;
        assert_eq!(Sensitivity::Unspecified as i32, 0);
        assert_eq!(Sensitivity::Public as i32, 1);
        assert_eq!(Sensitivity::Internal as i32, 2);
        assert_eq!(Sensitivity::Restricted as i32, 3);
    }

    #[test]
    fn cas_manifest_roundtrip() {
        use super::cas::*;
        use super::common::HashRef;

        let manifest = CasManifest {
            objects: vec![
                CasObjectHeader {
                    hash: Some(HashRef {
                        sha256: "aabb".into(),
                    }),
                    size_bytes: 1024,
                    content_type: "application/octet-stream".into(),
                },
                CasObjectHeader {
                    hash: Some(HashRef {
                        sha256: "ccdd".into(),
                    }),
                    size_bytes: 2048,
                    content_type: "text/plain".into(),
                },
            ],
        };
        assert_eq!(roundtrip(&manifest), manifest);
    }

    #[test]
    fn event_envelope_case_created() {
        use super::common::*;
        use super::events::*;

        let env = EventEnvelope {
            event_id: "evt-001".into(),
            r#type: EventType::CaseCreated as i32,
            ts: Some(Timestamp {
                unix_ms: 1700000000000,
            }),
            node_id: Some(NodeId {
                value: "node-1".into(),
            }),
            tenant_id: Some(TenantId {
                value: "public".into(),
            }),
            sensitivity: Sensitivity::Public as i32,
            prev_hash: Some(HashRef {
                sha256: "0000".into(),
            }),
            event_hash: Some(HashRef {
                sha256: "1111".into(),
            }),
            signature: vec![0xDE, 0xAD],
            refs: vec![HashRef {
                sha256: "ref1".into(),
            }],
            payload: Some(event_envelope::Payload::CaseCreated(CaseCreated {
                case_id: "case-1".into(),
                title: "DNS failure".into(),
                summary: "DNS resolution failed for api.example.com".into(),
                content_ref: Some(HashRef {
                    sha256: "content-hash".into(),
                }),
                shareable: false,
            })),
            tags: vec!["dns".into(), "network".into()],
        };
        let decoded = roundtrip(&env);
        assert_eq!(decoded, env);
        assert_eq!(decoded.tags.len(), 2);
        match decoded.payload {
            Some(event_envelope::Payload::CaseCreated(cc)) => {
                assert_eq!(cc.case_id, "case-1");
                assert!(!cc.shareable);
            }
            other => panic!("expected CaseCreated, got {other:?}"),
        }
    }

    #[test]
    fn event_envelope_artifact_published() {
        use super::common::*;
        use super::events::*;

        let env = EventEnvelope {
            event_id: "evt-002".into(),
            r#type: EventType::ArtifactPublished as i32,
            ts: Some(Timestamp {
                unix_ms: 1700000001000,
            }),
            node_id: Some(NodeId {
                value: "node-1".into(),
            }),
            tenant_id: Some(TenantId {
                value: "public".into(),
            }),
            sensitivity: Sensitivity::Internal as i32,
            payload: Some(event_envelope::Payload::ArtifactPublished(
                ArtifactPublished {
                    artifact_id: "art-1".into(),
                    artifact_type: ArtifactType::Runbook as i32,
                    version: 1,
                    title: "K8s rollback playbook".into(),
                    content_ref: Some(HashRef {
                        sha256: "abcd".into(),
                    }),
                    shareable: true,
                    expires_unix_ms: 0,
                },
            )),
            ..Default::default()
        };
        assert_eq!(roundtrip(&env), env);
    }

    #[test]
    fn event_envelope_web_brief() {
        use super::common::*;
        use super::events::*;

        let env = EventEnvelope {
            event_id: "evt-003".into(),
            r#type: EventType::WebBriefCreated as i32,
            ts: Some(Timestamp {
                unix_ms: 1700000002000,
            }),
            node_id: Some(NodeId {
                value: "node-2".into(),
            }),
            tenant_id: Some(TenantId {
                value: "public".into(),
            }),
            sensitivity: Sensitivity::Public as i32,
            payload: Some(event_envelope::Payload::WebBriefCreated(WebBriefCreated {
                artifact_id: "wb-1".into(),
                question: "What is Rust ownership?".into(),
                summary: "Rust ownership is...".into(),
                sources: vec![WebSource {
                    url: "https://doc.rust-lang.org/book/ch04-01-what-is-ownership.html".into(),
                    retrieved_at: Some(Timestamp {
                        unix_ms: 1700000001500,
                    }),
                    publisher: "Rust Book".into(),
                    snippet: "Each value in Rust has a variable...".into(),
                }],
                confidence: 0.92,
                expires_unix_ms: 1700086400000,
            })),
            ..Default::default()
        };
        let decoded = roundtrip(&env);
        match decoded.payload {
            Some(event_envelope::Payload::WebBriefCreated(wb)) => {
                assert_eq!(wb.sources.len(), 1);
                assert!((wb.confidence - 0.92).abs() < f32::EPSILON);
            }
            other => panic!("expected WebBriefCreated, got {other:?}"),
        }
    }

    #[test]
    fn event_envelope_training_events() {
        use super::common::*;
        use super::events::*;

        let started = EventEnvelope {
            event_id: "evt-t1".into(),
            r#type: EventType::TrainJobStarted as i32,
            payload: Some(event_envelope::Payload::TrainJobStarted(TrainJobStarted {
                job_id: "job-1".into(),
                target: "router".into(),
                dataset_manifest_ref: Some(HashRef {
                    sha256: "ds-hash".into(),
                }),
                max_steps: 1000,
                max_minutes: 10,
            })),
            ..Default::default()
        };
        assert_eq!(roundtrip(&started), started);

        let completed = EventEnvelope {
            event_id: "evt-t2".into(),
            r#type: EventType::TrainJobCompleted as i32,
            payload: Some(event_envelope::Payload::TrainJobCompleted(
                TrainJobCompleted {
                    job_id: "job-1".into(),
                    success: true,
                    notes: "converged at step 800".into(),
                    metrics: vec![
                        TrainMetric {
                            name: "loss".into(),
                            value: 0.05,
                        },
                        TrainMetric {
                            name: "accuracy".into(),
                            value: 0.95,
                        },
                    ],
                    model_bundle_ref: Some(HashRef {
                        sha256: "model-hash".into(),
                    }),
                },
            )),
            ..Default::default()
        };
        let decoded = roundtrip(&completed);
        match decoded.payload {
            Some(event_envelope::Payload::TrainJobCompleted(tc)) => {
                assert!(tc.success);
                assert_eq!(tc.metrics.len(), 2);
            }
            other => panic!("expected TrainJobCompleted, got {other:?}"),
        }
    }

    #[test]
    fn event_envelope_model_rollback() {
        use super::events::*;

        let env = EventEnvelope {
            event_id: "evt-r1".into(),
            r#type: EventType::ModelRolledBack as i32,
            payload: Some(event_envelope::Payload::ModelRolledBack(ModelRolledBack {
                model_id: "router-v1".into(),
                from_version: 3,
                to_version: 2,
                reason: "regression detected".into(),
            })),
            ..Default::default()
        };
        assert_eq!(roundtrip(&env), env);
    }

    #[test]
    fn event_envelope_tool_invocation() {
        use super::common::*;
        use super::events::*;

        let env = EventEnvelope {
            event_id: "evt-ti1".into(),
            r#type: EventType::ToolInvocationRecorded as i32,
            payload: Some(event_envelope::Payload::ToolInvocationRecorded(
                ToolInvocationRecorded {
                    invocation_id: "inv-1".into(),
                    tool_name: "web_search".into(),
                    summary: "searched for Rust async patterns".into(),
                    success: true,
                    duration_ms: 1500,
                    output_ref: Some(HashRef {
                        sha256: "out-hash".into(),
                    }),
                },
            )),
            ..Default::default()
        };
        assert_eq!(roundtrip(&env), env);
    }

    #[test]
    fn event_envelope_data_shared() {
        use super::common::*;
        use super::events::*;

        let env = EventEnvelope {
            event_id: "evt-ds1".into(),
            r#type: EventType::DataSharedRecorded as i32,
            payload: Some(event_envelope::Payload::DataSharedRecorded(
                DataSharedRecorded {
                    share_id: "share-1".into(),
                    channel: ShareChannel::Peer as i32,
                    peer_node_id: Some(NodeId {
                        value: "node-2".into(),
                    }),
                    destination: "".into(),
                    redaction_summary: "PII removed".into(),
                    allowed: true,
                    policy_reason: "public tenant".into(),
                },
            )),
            ..Default::default()
        };
        assert_eq!(roundtrip(&env), env);
    }

    #[test]
    fn snapshot_file_roundtrip() {
        use super::common::*;
        use super::snapshot::*;

        let snap = SnapshotFile {
            header: Some(SnapshotHeader {
                snapshot_version: 1,
                created_at: Some(Timestamp {
                    unix_ms: 1700000000000,
                }),
                last_applied_event_hash: Some(HashRef {
                    sha256: "last-evt".into(),
                }),
                snapshot_hash: Some(HashRef {
                    sha256: "snap-hash".into(),
                }),
            }),
            payload: Some(SnapshotPayload {
                sqlite_dump_ref: Some(HashRef {
                    sha256: "sqlite-dump".into(),
                }),
                notes: vec![KeyValue {
                    key: "events_count".into(),
                    value: "1000".into(),
                }],
            }),
        };
        assert_eq!(roundtrip(&snap), snap);
    }

    #[test]
    fn replication_gossip_roundtrip() {
        use super::common::*;
        use super::repl::*;

        let gossip = GossipMeta {
            from_node_id: Some(NodeId {
                value: "node-1".into(),
            }),
            tenant_id: Some(TenantId {
                value: "public".into(),
            }),
            segments: vec![SegmentMeta {
                segment_id: Some(SegmentId {
                    value: "seg-001".into(),
                }),
                first_event_hash: Some(HashRef {
                    sha256: "first".into(),
                }),
                last_event_hash: Some(HashRef {
                    sha256: "last".into(),
                }),
                size_bytes: 50_000,
            }],
            small_object_hashes: vec![HashRef {
                sha256: "obj1".into(),
            }],
        };
        assert_eq!(roundtrip(&gossip), gossip);
    }

    #[test]
    fn replication_pull_segments_roundtrip() {
        use super::common::*;
        use super::repl::*;

        let req = PullSegmentsRequest {
            requester: Some(NodeId {
                value: "node-2".into(),
            }),
            tenant_id: Some(TenantId {
                value: "public".into(),
            }),
            want_segments: vec![
                SegmentId {
                    value: "seg-001".into(),
                },
                SegmentId {
                    value: "seg-002".into(),
                },
            ],
            budget: Some(Budget {
                max_bytes: 1_000_000,
                max_items: 10,
                max_ms: 5000,
            }),
        };
        assert_eq!(roundtrip(&req), req);

        let resp = PullSegmentsResponse {
            responder: Some(NodeId {
                value: "node-1".into(),
            }),
            chunks: vec![SegmentChunk {
                segment_id: Some(SegmentId {
                    value: "seg-001".into(),
                }),
                chunk_index: 0,
                chunk_count: 1,
                data: vec![0x01, 0x02, 0x03],
            }],
        };
        assert_eq!(roundtrip(&resp), resp);
    }

    #[test]
    fn replication_pull_cas_objects_roundtrip() {
        use super::common::*;
        use super::repl::*;

        let req = PullCasObjectsRequest {
            requester: Some(NodeId {
                value: "node-2".into(),
            }),
            want_hashes: vec![
                HashRef {
                    sha256: "hash-a".into(),
                },
                HashRef {
                    sha256: "hash-b".into(),
                },
            ],
            budget: Some(Budget {
                max_bytes: 500_000,
                max_items: 5,
                max_ms: 3000,
            }),
        };
        assert_eq!(roundtrip(&req), req);

        let resp = PullCasObjectsResponse {
            responder: Some(NodeId {
                value: "node-1".into(),
            }),
            chunks: vec![CasObjectChunk {
                hash: Some(HashRef {
                    sha256: "hash-a".into(),
                }),
                chunk_index: 0,
                chunk_count: 1,
                data: b"hello world".to_vec(),
                content_type: "text/plain".into(),
                size_bytes: 11,
            }],
        };
        assert_eq!(roundtrip(&resp), resp);
    }

    #[test]
    fn mesh_envelope_hello() {
        use super::common::*;
        use super::mesh::*;

        let env = Envelope {
            msg_id: "msg-001".into(),
            r#type: MsgType::Hello as i32,
            from_node_id: Some(NodeId {
                value: "node-1".into(),
            }),
            to_node_id: Some(NodeId {
                value: "node-2".into(),
            }),
            tenant_id: Some(TenantId {
                value: "public".into(),
            }),
            sensitivity: Sensitivity::Public as i32,
            ttl_hops: 3,
            deadline_ms: 5000,
            max_context_bytes: 10_000,
            max_answer_bytes: 5_000,
            policy: Some(PolicyFlags {
                allow_web: false,
                allow_train: false,
                require_citations: true,
                redaction_required: true,
                freshness_required: false,
                allowed_domains: vec!["example.com".into()],
                blocked_domains: vec![],
                web_budget: Some(Budget {
                    max_bytes: 100_000,
                    max_items: 5,
                    max_ms: 10_000,
                }),
            }),
            body: Some(envelope::Body::Hello(Hello {
                capabilities: vec!["inference".into(), "research_web".into()],
                version: "0.1.0".into(),
            })),
        };
        assert_eq!(roundtrip(&env), env);
    }

    #[test]
    fn mesh_envelope_ping_pong() {
        use super::mesh::*;

        let ping = Envelope {
            msg_id: "msg-002".into(),
            r#type: MsgType::Ping as i32,
            body: Some(envelope::Body::Ping(Ping { nonce: 42 })),
            ..Default::default()
        };
        let decoded_ping = roundtrip(&ping);
        match decoded_ping.body {
            Some(envelope::Body::Ping(p)) => assert_eq!(p.nonce, 42),
            other => panic!("expected Ping, got {other:?}"),
        }

        let pong = Envelope {
            msg_id: "msg-003".into(),
            r#type: MsgType::Pong as i32,
            body: Some(envelope::Body::Pong(Pong { nonce: 42 })),
            ..Default::default()
        };
        assert_eq!(roundtrip(&pong), pong);
    }

    #[test]
    fn mesh_envelope_ask_answer() {
        use super::mesh::*;

        let ask = Envelope {
            msg_id: "msg-004".into(),
            r#type: MsgType::Ask as i32,
            ttl_hops: 2,
            deadline_ms: 3000,
            body: Some(envelope::Body::Ask(Ask {
                question: "How to fix OOM in Java?".into(),
                context_bullets: vec!["Running OpenJDK 17".into(), "Container limit 512MB".into()],
            })),
            ..Default::default()
        };
        assert_eq!(roundtrip(&ask), ask);

        let answer = Envelope {
            msg_id: "msg-005".into(),
            r#type: MsgType::Answer as i32,
            body: Some(envelope::Body::Answer(Answer {
                answer: "Increase -Xmx or reduce heap usage.".into(),
                confidence: 0.85,
                evidence_refs: vec!["case-42".into()],
                warnings: vec!["based on limited context".into()],
            })),
            ..Default::default()
        };
        let decoded = roundtrip(&answer);
        match decoded.body {
            Some(envelope::Body::Answer(a)) => {
                assert!((a.confidence - 0.85).abs() < f32::EPSILON);
                assert_eq!(a.evidence_refs.len(), 1);
            }
            other => panic!("expected Answer, got {other:?}"),
        }
    }

    #[test]
    fn mesh_envelope_refuse() {
        use super::mesh::*;

        let refuse = Envelope {
            msg_id: "msg-006".into(),
            r#type: MsgType::Refuse as i32,
            body: Some(envelope::Body::Refuse(Refuse {
                code: "BUDGET_EXCEEDED".into(),
                message: "context exceeds max_context_bytes".into(),
            })),
            ..Default::default()
        };
        assert_eq!(roundtrip(&refuse), refuse);
    }

    #[test]
    fn research_request_response_roundtrip() {
        use super::common::*;
        use super::events::*;
        use super::research::*;

        let req = ResearchRequest {
            request_id: "rr-001".into(),
            query: "latest Rust async patterns".into(),
            allowed_domains: vec!["rust-lang.org".into(), "docs.rs".into()],
            recency_days: 30,
            max_pages: 5,
            max_bytes: 50_000,
            require_citations: true,
        };
        assert_eq!(roundtrip(&req), req);

        let resp = ResearchResponse {
            request_id: "rr-001".into(),
            success: true,
            notes: "found 3 relevant sources".into(),
            web_brief: Some(WebBriefCreated {
                artifact_id: "wb-rr1".into(),
                question: "latest Rust async patterns".into(),
                summary: "Key patterns include...".into(),
                sources: vec![],
                confidence: 0.88,
                expires_unix_ms: 1700086400000,
            }),
            content_ref: Some(HashRef {
                sha256: "brief-content".into(),
            }),
        };
        assert_eq!(roundtrip(&resp), resp);
    }

    #[test]
    fn training_types_roundtrip() {
        use super::common::*;
        use super::training::*;

        let manifest = DatasetManifest {
            manifest_id: "dm-001".into(),
            from_event_hash: Some(HashRef {
                sha256: "from".into(),
            }),
            to_event_hash: Some(HashRef {
                sha256: "to".into(),
            }),
            cas_objects: vec![
                HashRef {
                    sha256: "obj-a".into(),
                },
                HashRef {
                    sha256: "obj-b".into(),
                },
            ],
            notes: vec!["cases only".into()],
        };
        assert_eq!(roundtrip(&manifest), manifest);

        let config = TrainConfig {
            job_id: "job-001".into(),
            target: "router".into(),
            max_steps: 500,
            max_minutes: 5,
            max_dataset_items: 1000,
            max_threads: 4,
        };
        assert_eq!(roundtrip(&config), config);

        let result = TrainResult {
            job_id: "job-001".into(),
            success: true,
            metrics: vec![
                Metric {
                    name: "loss".into(),
                    value: 0.03,
                },
                Metric {
                    name: "f1".into(),
                    value: 0.97,
                },
            ],
            model_bundle_ref: Some(HashRef {
                sha256: "model-bundle".into(),
            }),
            notes: "converged early".into(),
        };
        assert_eq!(roundtrip(&result), result);
    }

    #[test]
    fn event_type_enum_coverage() {
        use super::events::EventType;

        let expected = [
            (EventType::Unspecified, 0),
            (EventType::CaseCreated, 10),
            (EventType::CaseConfirmed, 11),
            (EventType::CaseTagged, 12),
            (EventType::ArtifactPublished, 20),
            (EventType::ArtifactDeprecated, 21),
            (EventType::WebBriefCreated, 30),
            (EventType::WebBriefExpired, 31),
            (EventType::PeerSeen, 40),
            (EventType::PeerTrustUpdated, 41),
            (EventType::PolicyUpdated, 42),
            (EventType::TrainJobStarted, 50),
            (EventType::TrainJobCompleted, 51),
            (EventType::ModelPromoted, 52),
            (EventType::ModelRolledBack, 53),
            (EventType::ToolInvocationRecorded, 60),
            (EventType::DataSharedRecorded, 61),
        ];
        for (variant, val) in expected {
            assert_eq!(
                variant as i32, val,
                "EventType::{variant:?} should be {val}"
            );
        }
    }

    #[test]
    fn msg_type_enum_coverage() {
        use super::mesh::MsgType;

        assert_eq!(MsgType::Unspecified as i32, 0);
        assert_eq!(MsgType::Hello as i32, 1);
        assert_eq!(MsgType::Ping as i32, 2);
        assert_eq!(MsgType::Pong as i32, 3);
        assert_eq!(MsgType::Ask as i32, 4);
        assert_eq!(MsgType::Answer as i32, 5);
        assert_eq!(MsgType::Refuse as i32, 6);
    }

    #[test]
    fn empty_message_decodes_to_default() {
        use super::events::EventEnvelope;
        let decoded = EventEnvelope::decode(&[] as &[u8]).expect("empty bytes should decode");
        assert_eq!(decoded, EventEnvelope::default());
    }

    #[test]
    fn length_prefixed_stream() {
        use super::common::*;
        use super::events::*;

        let events: Vec<EventEnvelope> = (0..10)
            .map(|i| EventEnvelope {
                event_id: format!("evt-{i}"),
                r#type: EventType::CaseCreated as i32,
                ts: Some(Timestamp {
                    unix_ms: 1700000000000 + i * 1000,
                }),
                payload: Some(event_envelope::Payload::CaseCreated(CaseCreated {
                    case_id: format!("case-{i}"),
                    title: format!("Case {i}"),
                    summary: format!("Summary for case {i}"),
                    content_ref: Some(HashRef {
                        sha256: format!("hash-{i}"),
                    }),
                    shareable: i % 2 == 0,
                })),
                ..Default::default()
            })
            .collect();

        let mut buf = Vec::new();
        for event in &events {
            let encoded = event.encode_to_vec();
            let len = encoded.len() as u32;
            buf.extend_from_slice(&len.to_le_bytes());
            buf.extend_from_slice(&encoded);
        }

        let mut cursor = &buf[..];
        let mut decoded_events = Vec::new();
        while cursor.len() >= 4 {
            let len = u32::from_le_bytes([cursor[0], cursor[1], cursor[2], cursor[3]]) as usize;
            cursor = &cursor[4..];
            let msg = EventEnvelope::decode(&cursor[..len]).expect("decode event");
            decoded_events.push(msg);
            cursor = &cursor[len..];
        }

        assert_eq!(decoded_events.len(), 10);
        for (i, evt) in decoded_events.iter().enumerate() {
            assert_eq!(evt.event_id, format!("evt-{i}"));
        }
    }
}
