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

pub mod datasets {
    include!(concat!(env!("OUT_DIR"), "/meshmind.datasets.rs"));
}

pub mod federated {
    include!(concat!(env!("OUT_DIR"), "/meshmind.federated.rs"));
}

#[cfg(test)]
mod tests {
    use prost::Message;

    fn roundtrip<M: Message + Default + PartialEq + std::fmt::Debug>(msg: &M) -> M {
        let encoded = msg.encode_to_vec();
        assert!(!encoded.is_empty(), "encoded message should not be empty");
        M::decode(encoded.as_slice()).expect("decode should succeed")
    }

    // ===== Common types =====

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

    // ===== CAS types =====

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

    // ===== Event types =====

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

    // ===== NEW: Data discovery event payloads =====

    #[test]
    fn event_data_source_discovered() {
        use super::common::*;
        use super::events::*;

        let env = EventEnvelope {
            event_id: "evt-dsd-1".into(),
            r#type: EventType::DataSourceDiscovered as i32,
            payload: Some(event_envelope::Payload::DataSourceDiscovered(
                DataSourceDiscovered {
                    source_id: "src-001".into(),
                    connector_type: ConnectorType::SqliteDb as i32,
                    path_or_uri: "/data/inventory.db".into(),
                    display_name: "Inventory Database".into(),
                    estimated_size_bytes: 5_000_000,
                    estimated_tables: 8,
                    discovered_at: Some(Timestamp {
                        unix_ms: 1700000000000,
                    }),
                },
            )),
            ..Default::default()
        };
        let decoded = roundtrip(&env);
        match decoded.payload {
            Some(event_envelope::Payload::DataSourceDiscovered(d)) => {
                assert_eq!(d.source_id, "src-001");
                assert_eq!(d.connector_type, ConnectorType::SqliteDb as i32);
                assert_eq!(d.estimated_tables, 8);
            }
            other => panic!("expected DataSourceDiscovered, got {other:?}"),
        }
    }

    #[test]
    fn event_data_source_classified() {
        use super::common::*;
        use super::events::*;

        let env = EventEnvelope {
            event_id: "evt-dsc-1".into(),
            r#type: EventType::DataSourceClassified as i32,
            payload: Some(event_envelope::Payload::DataSourceClassified(
                DataSourceClassified {
                    source_id: "src-001".into(),
                    schema_snapshot_ref: Some(HashRef {
                        sha256: "schema-hash".into(),
                    }),
                    suggested_sensitivity: Sensitivity::Internal as i32,
                    pii_detected: true,
                    secrets_detected: false,
                    total_columns: 24,
                    restricted_columns: 3,
                    classification_summary: "PII in email, phone, address columns".into(),
                },
            )),
            ..Default::default()
        };
        let decoded = roundtrip(&env);
        match decoded.payload {
            Some(event_envelope::Payload::DataSourceClassified(c)) => {
                assert!(c.pii_detected);
                assert_eq!(c.restricted_columns, 3);
            }
            other => panic!("expected DataSourceClassified, got {other:?}"),
        }
    }

    #[test]
    fn event_data_source_approved() {
        use super::common::*;
        use super::events::*;

        let env = EventEnvelope {
            event_id: "evt-dsa-1".into(),
            r#type: EventType::DataSourceApproved as i32,
            payload: Some(event_envelope::Payload::DataSourceApproved(
                DataSourceApproved {
                    source_id: "src-001".into(),
                    source_profile_ref: Some(HashRef {
                        sha256: "profile-hash".into(),
                    }),
                    approved_by: "admin".into(),
                    approved_at: Some(Timestamp {
                        unix_ms: 1700000100000,
                    }),
                    allowed_tables: vec!["orders".into(), "products".into()],
                    row_limit: 10000,
                },
            )),
            ..Default::default()
        };
        let decoded = roundtrip(&env);
        match decoded.payload {
            Some(event_envelope::Payload::DataSourceApproved(a)) => {
                assert_eq!(a.allowed_tables.len(), 2);
                assert_eq!(a.row_limit, 10000);
            }
            other => panic!("expected DataSourceApproved, got {other:?}"),
        }
    }

    #[test]
    fn event_ingest_started_completed() {
        use super::common::*;
        use super::events::*;

        let started = EventEnvelope {
            event_id: "evt-is-1".into(),
            r#type: EventType::IngestStarted as i32,
            payload: Some(event_envelope::Payload::IngestStarted(IngestStarted {
                ingest_id: "ing-001".into(),
                source_id: "src-001".into(),
                connector_type: ConnectorType::SqliteDb as i32,
                source_profile_ref: Some(HashRef {
                    sha256: "profile-hash".into(),
                }),
                started_at: Some(Timestamp {
                    unix_ms: 1700000200000,
                }),
            })),
            ..Default::default()
        };
        assert_eq!(roundtrip(&started), started);

        let completed = EventEnvelope {
            event_id: "evt-ic-1".into(),
            r#type: EventType::IngestCompleted as i32,
            payload: Some(event_envelope::Payload::IngestCompleted(IngestCompleted {
                ingest_id: "ing-001".into(),
                source_id: "src-001".into(),
                success: true,
                rows_ingested: 5000,
                documents_created: 5000,
                facts_created: 150,
                bytes_stored: 2_500_000,
                duration_ms: 12000,
                notes: "all tables ingested".into(),
            })),
            ..Default::default()
        };
        let decoded = roundtrip(&completed);
        match decoded.payload {
            Some(event_envelope::Payload::IngestCompleted(c)) => {
                assert!(c.success);
                assert_eq!(c.rows_ingested, 5000);
                assert_eq!(c.documents_created, 5000);
            }
            other => panic!("expected IngestCompleted, got {other:?}"),
        }
    }

    #[test]
    fn event_dataset_manifest_created() {
        use super::common::*;
        use super::events::*;

        let env = EventEnvelope {
            event_id: "evt-dm-1".into(),
            r#type: EventType::DatasetManifestCreated as i32,
            payload: Some(event_envelope::Payload::DatasetManifestCreated(
                DatasetManifestCreated {
                    manifest_id: "dm-001".into(),
                    manifest_ref: Some(HashRef {
                        sha256: "manifest-hash".into(),
                    }),
                    source_id: "src-001".into(),
                    preset: "public_shareable_only".into(),
                    item_count: 5000,
                    total_bytes: 2_500_000,
                },
            )),
            ..Default::default()
        };
        assert_eq!(roundtrip(&env), env);
    }

    // ===== NEW: Federated learning event payloads =====

    #[test]
    fn event_train_delta_published() {
        use super::common::*;
        use super::events::*;

        let env = EventEnvelope {
            event_id: "evt-tdp-1".into(),
            r#type: EventType::TrainDeltaPublished as i32,
            payload: Some(event_envelope::Payload::TrainDeltaPublished(
                TrainDeltaPublished {
                    delta_id: "delta-001".into(),
                    model_id: "router-v1".into(),
                    base_version: "v2".into(),
                    delta_ref: Some(HashRef {
                        sha256: "delta-hash".into(),
                    }),
                    metrics: vec![TrainMetric {
                        name: "loss".into(),
                        value: 0.04,
                    }],
                    from_node_id: Some(NodeId {
                        value: "node-3".into(),
                    }),
                },
            )),
            ..Default::default()
        };
        let decoded = roundtrip(&env);
        match decoded.payload {
            Some(event_envelope::Payload::TrainDeltaPublished(d)) => {
                assert_eq!(d.delta_id, "delta-001");
                assert_eq!(d.base_version, "v2");
            }
            other => panic!("expected TrainDeltaPublished, got {other:?}"),
        }
    }

    #[test]
    fn event_train_delta_applied() {
        use super::common::*;
        use super::events::*;

        let env = EventEnvelope {
            event_id: "evt-tda-1".into(),
            r#type: EventType::TrainDeltaApplied as i32,
            payload: Some(event_envelope::Payload::TrainDeltaApplied(
                TrainDeltaApplied {
                    delta_id: "delta-001".into(),
                    model_id: "router-v1".into(),
                    resulting_version: "v3".into(),
                    applied_by: Some(NodeId {
                        value: "node-1".into(),
                    }),
                    metrics: vec![TrainMetric {
                        name: "f1".into(),
                        value: 0.96,
                    }],
                },
            )),
            ..Default::default()
        };
        assert_eq!(roundtrip(&env), env);
    }

    #[test]
    fn event_federated_round_started() {
        use super::common::*;
        use super::events::*;

        let env = EventEnvelope {
            event_id: "evt-frs-1".into(),
            r#type: EventType::FederatedRoundStarted as i32,
            payload: Some(event_envelope::Payload::FederatedRoundStarted(
                FederatedRoundStarted {
                    round_id: "round-001".into(),
                    model_id: "router-v1".into(),
                    round_number: 1,
                    expected_participants: 3,
                    coordinator: Some(NodeId {
                        value: "node-1".into(),
                    }),
                    started_at: Some(Timestamp {
                        unix_ms: 1700000300000,
                    }),
                },
            )),
            ..Default::default()
        };
        assert_eq!(roundtrip(&env), env);
    }

    #[test]
    fn event_federated_round_completed() {
        use super::common::*;
        use super::events::*;

        let env = EventEnvelope {
            event_id: "evt-frc-1".into(),
            r#type: EventType::FederatedRoundCompleted as i32,
            payload: Some(event_envelope::Payload::FederatedRoundCompleted(
                FederatedRoundCompleted {
                    round_id: "round-001".into(),
                    model_id: "router-v1".into(),
                    round_number: 1,
                    actual_participants: 3,
                    success: true,
                    aggregate_metrics: vec![
                        TrainMetric {
                            name: "avg_loss".into(),
                            value: 0.035,
                        },
                        TrainMetric {
                            name: "avg_f1".into(),
                            value: 0.97,
                        },
                    ],
                    resulting_model_ref: Some(HashRef {
                        sha256: "agg-model-hash".into(),
                    }),
                    notes: "all participants contributed".into(),
                },
            )),
            ..Default::default()
        };
        let decoded = roundtrip(&env);
        match decoded.payload {
            Some(event_envelope::Payload::FederatedRoundCompleted(c)) => {
                assert!(c.success);
                assert_eq!(c.actual_participants, 3);
                assert_eq!(c.aggregate_metrics.len(), 2);
            }
            other => panic!("expected FederatedRoundCompleted, got {other:?}"),
        }
    }

    // ===== NEW: datasets.proto types =====

    #[test]
    fn datasets_discovered_source_roundtrip() {
        use super::common::*;
        use super::datasets::*;

        let src = DiscoveredSource {
            source_id: "src-csv-1".into(),
            connector_type: "csv_folder".into(),
            path_or_uri: "/data/exports/sales".into(),
            display_name: "Sales Exports".into(),
            estimated_size_bytes: 10_000_000,
            estimated_tables: 5,
            discovered_at: Some(Timestamp {
                unix_ms: 1700000000000,
            }),
            metadata: [("format".into(), "csv".into())].into(),
        };
        assert_eq!(roundtrip(&src), src);
    }

    #[test]
    fn datasets_column_classification_roundtrip() {
        use super::common::*;
        use super::datasets::*;

        let cc = ColumnClassification {
            table_name: "customers".into(),
            column_name: "email".into(),
            data_type: "TEXT".into(),
            classification: ColumnClass::Pii as i32,
            suggested_sensitivity: Sensitivity::Restricted as i32,
            confidence: 0.95,
            pattern_matched: "email_pattern".into(),
            redact_by_default: true,
        };
        assert_eq!(roundtrip(&cc), cc);
    }

    #[test]
    fn datasets_schema_snapshot_roundtrip() {
        use super::common::*;
        use super::datasets::*;

        let snapshot = SchemaSnapshot {
            source_id: "src-001".into(),
            connector_type: "sqlite".into(),
            tables: vec![TableSchema {
                table_name: "orders".into(),
                columns: vec![
                    ColumnDef {
                        name: "id".into(),
                        data_type: "INTEGER".into(),
                        nullable: false,
                        is_primary_key: true,
                    },
                    ColumnDef {
                        name: "customer_email".into(),
                        data_type: "TEXT".into(),
                        nullable: true,
                        is_primary_key: false,
                    },
                ],
                row_count_estimate: 50000,
                size_bytes_estimate: 2_000_000,
            }],
            column_classifications: vec![ColumnClassification {
                table_name: "orders".into(),
                column_name: "customer_email".into(),
                data_type: "TEXT".into(),
                classification: ColumnClass::Pii as i32,
                suggested_sensitivity: Sensitivity::Restricted as i32,
                confidence: 0.95,
                pattern_matched: "email".into(),
                redact_by_default: true,
            }],
            inspected_at: Some(Timestamp {
                unix_ms: 1700000050000,
            }),
            total_rows_estimate: 50000,
        };
        assert_eq!(roundtrip(&snapshot), snapshot);
    }

    #[test]
    fn datasets_source_profile_roundtrip() {
        use super::common::*;
        use super::datasets::*;

        let profile = SourceProfile {
            profile_id: "prof-001".into(),
            source_id: "src-001".into(),
            approved_by: "admin".into(),
            approved_at: Some(Timestamp {
                unix_ms: 1700000100000,
            }),
            table_rules: vec![TableRule {
                table_name: "orders".into(),
                allowed: true,
                allowed_columns: vec!["id".into(), "product".into(), "amount".into()],
                redacted_columns: vec!["customer_email".into()],
                row_limit: 10000,
                where_clause: "".into(),
            }],
            redaction_policy: Some(RedactionPolicy {
                redact_pii: true,
                redact_secrets: true,
                additional_redact_columns: vec![],
                redaction_method: "hash".into(),
            }),
            retention_policy: Some(RetentionPolicy {
                max_age_days: 365,
                max_size_bytes: 100_000_000,
                keep_raw: false,
            }),
            row_limit: 10000,
            allow_raw_retention: false,
            allow_training: true,
            max_sensitivity: Sensitivity::Internal as i32,
        };
        assert_eq!(roundtrip(&profile), profile);
    }

    #[test]
    fn datasets_manifest_extended_roundtrip() {
        use super::common::*;
        use super::datasets;

        let manifest = datasets::DatasetManifest {
            manifest_id: "dm-ext-001".into(),
            from_event_hash: Some(HashRef {
                sha256: "from".into(),
            }),
            to_event_hash: Some(HashRef {
                sha256: "to".into(),
            }),
            cas_objects: vec![HashRef {
                sha256: "obj-a".into(),
            }],
            notes: vec!["extended manifest".into()],
            source_id: "src-001".into(),
            connector_type: "sqlite".into(),
            preset: "public_shareable_only".into(),
            schema_snapshot_ref: Some(HashRef {
                sha256: "schema-hash".into(),
            }),
            redaction_rules_applied: vec![datasets::RedactionRule {
                column_name: "email".into(),
                method: "hash".into(),
            }],
            stats: Some(datasets::DatasetStats {
                total_items: 5000,
                total_bytes: 2_500_000,
                tables_included: 2,
                columns_included: 8,
                columns_redacted: 1,
            }),
            created_at: Some(Timestamp {
                unix_ms: 1700000200000,
            }),
        };
        assert_eq!(roundtrip(&manifest), manifest);
    }

    #[test]
    fn datasets_ingest_checkpoint_roundtrip() {
        use super::common::*;
        use super::datasets::*;

        let cp = IngestCheckpoint {
            ingest_id: "ing-001".into(),
            source_id: "src-001".into(),
            table_name: "orders".into(),
            last_row_offset: 5000,
            last_event_hash: Some(HashRef {
                sha256: "evt-hash".into(),
            }),
            updated_at: Some(Timestamp {
                unix_ms: 1700000250000,
            }),
        };
        assert_eq!(roundtrip(&cp), cp);
    }

    #[test]
    fn datasets_ingest_batch_roundtrip() {
        use super::common::*;
        use super::datasets::*;

        let batch = IngestBatch {
            ingest_id: "ing-001".into(),
            table_name: "orders".into(),
            batch_index: 0,
            row_offset: 0,
            row_count: 100,
            items: vec![IngestItem {
                entity_id: "order-1".into(),
                content_ref: Some(HashRef {
                    sha256: "item-hash".into(),
                }),
                item_type: "document".into(),
                attributes: [("title".into(), "Order #1".into())].into(),
            }],
        };
        assert_eq!(roundtrip(&batch), batch);
    }

    // ===== NEW: federated.proto types =====

    #[test]
    fn federated_round_config_roundtrip() {
        use super::common::*;
        use super::federated::*;

        let config = FederatedRoundConfig {
            round_id: "round-001".into(),
            model_id: "router-v1".into(),
            round_number: 1,
            min_participants: 2,
            max_participants: 5,
            deadline_seconds: 300,
            aggregation_strategy: "fedavg".into(),
            base_model_ref: Some(HashRef {
                sha256: "base-model".into(),
            }),
            policy: Some(FederatedPolicy {
                share_deltas: true,
                share_aggregate_stats: true,
                share_synthetic_examples: false,
                max_delta_size_bytes: 10_000_000,
                max_sensitivity: Sensitivity::Internal as i32,
            }),
        };
        assert_eq!(roundtrip(&config), config);
    }

    #[test]
    fn federated_model_delta_roundtrip() {
        use super::common::*;
        use super::federated::*;

        let delta = ModelDelta {
            delta_id: "delta-001".into(),
            round_id: "round-001".into(),
            model_id: "router-v1".into(),
            base_version: "v2".into(),
            delta_ref: Some(HashRef {
                sha256: "delta-hash".into(),
            }),
            from_node: Some(NodeId {
                value: "node-3".into(),
            }),
            training_samples: 500,
            training_steps: 100,
            metrics: vec![DeltaMetric {
                name: "loss".into(),
                value: 0.04,
            }],
            created_at: Some(Timestamp {
                unix_ms: 1700000350000,
            }),
        };
        assert_eq!(roundtrip(&delta), delta);
    }

    #[test]
    fn federated_aggregate_result_roundtrip() {
        use super::common::*;
        use super::federated::*;

        let result = AggregateResult {
            round_id: "round-001".into(),
            model_id: "router-v1".into(),
            round_number: 1,
            participants_count: 3,
            deltas_applied: 3,
            resulting_model_ref: Some(HashRef {
                sha256: "agg-model".into(),
            }),
            aggregate_metrics: vec![
                DeltaMetric {
                    name: "avg_loss".into(),
                    value: 0.035,
                },
                DeltaMetric {
                    name: "avg_f1".into(),
                    value: 0.97,
                },
            ],
            promoted: true,
            promotion_reason: "all gates passed".into(),
            completed_at: Some(Timestamp {
                unix_ms: 1700000400000,
            }),
        };
        assert_eq!(roundtrip(&result), result);
    }

    #[test]
    fn federated_round_summary_roundtrip() {
        use super::common::*;
        use super::federated::*;

        let summary = FederatedRoundSummary {
            round_id: "round-001".into(),
            model_id: "router-v1".into(),
            round_number: 1,
            participants: vec![
                RoundParticipant {
                    node_id: Some(NodeId {
                        value: "node-2".into(),
                    }),
                    status: ParticipantStatus::DeltaSubmitted as i32,
                    delta_ref: Some(HashRef {
                        sha256: "delta-2".into(),
                    }),
                    training_samples: 300,
                    last_update: Some(Timestamp {
                        unix_ms: 1700000360000,
                    }),
                },
                RoundParticipant {
                    node_id: Some(NodeId {
                        value: "node-3".into(),
                    }),
                    status: ParticipantStatus::TimedOut as i32,
                    delta_ref: None,
                    training_samples: 0,
                    last_update: Some(Timestamp {
                        unix_ms: 1700000370000,
                    }),
                },
            ],
            result: None,
        };
        assert_eq!(roundtrip(&summary), summary);
    }

    // ===== Training proto extended =====

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
            source_id: "src-001".into(),
            connector_type: "sqlite".into(),
            preset: "public_shareable_only".into(),
            schema_snapshot_ref: Some(HashRef {
                sha256: "schema".into(),
            }),
            redaction_rules: vec!["email:hash".into()],
            item_count: 5000,
            total_bytes: 2_500_000,
        };
        assert_eq!(roundtrip(&manifest), manifest);

        let config = TrainConfig {
            job_id: "job-001".into(),
            target: "router".into(),
            max_steps: 500,
            max_minutes: 5,
            max_dataset_items: 1000,
            max_threads: 4,
            dataset_preset: "public_shareable_only".into(),
            dataset_manifest_ref: Some(HashRef {
                sha256: "manifest-hash".into(),
            }),
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
    fn training_eval_gate_roundtrip() {
        use super::training::*;

        let gate = EvalGate {
            metric_name: "f1".into(),
            min_threshold: 0.9,
            max_threshold: 1.0,
            must_improve_over_baseline: true,
        };
        assert_eq!(roundtrip(&gate), gate);

        let result = EvalResult {
            model_id: "router".into(),
            version: "v3".into(),
            metrics: vec![Metric {
                name: "f1".into(),
                value: 0.95,
            }],
            gate_results: vec![EvalGateResult {
                metric_name: "f1".into(),
                actual_value: 0.95,
                threshold: 0.9,
                passed: true,
            }],
            all_gates_passed: true,
        };
        assert_eq!(roundtrip(&result), result);
    }

    // ===== Snapshot =====

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

    // ===== Replication =====

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

    // ===== Mesh =====

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

    // ===== Research =====

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

    // ===== Enum coverage =====

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
            (EventType::DataSourceDiscovered, 70),
            (EventType::DataSourceClassified, 71),
            (EventType::DataSourceApproved, 72),
            (EventType::IngestStarted, 80),
            (EventType::IngestCompleted, 81),
            (EventType::DatasetManifestCreated, 82),
            (EventType::TrainDeltaPublished, 90),
            (EventType::TrainDeltaApplied, 91),
            (EventType::FederatedRoundStarted, 92),
            (EventType::FederatedRoundCompleted, 93),
        ];
        for (variant, val) in expected {
            assert_eq!(
                variant as i32, val,
                "EventType::{variant:?} should be {val}"
            );
        }
    }

    #[test]
    fn connector_type_enum_coverage() {
        use super::events::ConnectorType;

        assert_eq!(ConnectorType::Unspecified as i32, 0);
        assert_eq!(ConnectorType::SqliteDb as i32, 1);
        assert_eq!(ConnectorType::CsvFolder as i32, 2);
        assert_eq!(ConnectorType::JsonFolder as i32, 3);
        assert_eq!(ConnectorType::Postgres as i32, 4);
        assert_eq!(ConnectorType::Mysql as i32, 5);
        assert_eq!(ConnectorType::Odbc as i32, 6);
    }

    #[test]
    fn artifact_type_enum_coverage() {
        use super::events::ArtifactType;

        assert_eq!(ArtifactType::Unspecified as i32, 0);
        assert_eq!(ArtifactType::Runbook as i32, 1);
        assert_eq!(ArtifactType::Template as i32, 2);
        assert_eq!(ArtifactType::Recipe as i32, 3);
        assert_eq!(ArtifactType::WebBrief as i32, 4);
        assert_eq!(ArtifactType::ModelBundle as i32, 5);
        assert_eq!(ArtifactType::Document as i32, 6);
        assert_eq!(ArtifactType::Fact as i32, 7);
    }

    #[test]
    fn column_class_enum_coverage() {
        use super::datasets::ColumnClass;

        assert_eq!(ColumnClass::Unspecified as i32, 0);
        assert_eq!(ColumnClass::Identifier as i32, 1);
        assert_eq!(ColumnClass::Pii as i32, 2);
        assert_eq!(ColumnClass::Financial as i32, 3);
        assert_eq!(ColumnClass::Operational as i32, 4);
        assert_eq!(ColumnClass::FreeText as i32, 5);
        assert_eq!(ColumnClass::TimestampCol as i32, 6);
        assert_eq!(ColumnClass::Numeric as i32, 7);
        assert_eq!(ColumnClass::Secret as i32, 8);
        assert_eq!(ColumnClass::Unknown as i32, 9);
    }

    #[test]
    fn participant_status_enum_coverage() {
        use super::federated::ParticipantStatus;

        assert_eq!(ParticipantStatus::Unspecified as i32, 0);
        assert_eq!(ParticipantStatus::Enrolled as i32, 1);
        assert_eq!(ParticipantStatus::Training as i32, 2);
        assert_eq!(ParticipantStatus::DeltaSubmitted as i32, 3);
        assert_eq!(ParticipantStatus::TimedOut as i32, 4);
        assert_eq!(ParticipantStatus::Failed as i32, 5);
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
