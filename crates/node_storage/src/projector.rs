//! Projects events from the event log into SQLite materialized views.

use rusqlite::{params, Connection};
use thiserror::Error;

use node_proto::events::{event_envelope::Payload, EventEnvelope};

#[derive(Debug, Error)]
pub enum ProjectorError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("unknown event payload")]
    UnknownPayload,
}

pub type Result<T> = std::result::Result<T, ProjectorError>;

/// Apply a single event to the materialized views.
pub fn apply_event(conn: &Connection, event: &EventEnvelope) -> Result<()> {
    let ts = event.ts.as_ref().map(|t| t.unix_ms).unwrap_or(0);
    let node_id = event
        .node_id
        .as_ref()
        .map(|n| n.value.as_str())
        .unwrap_or("");
    let tenant_id = event
        .tenant_id
        .as_ref()
        .map(|t| t.value.as_str())
        .unwrap_or("");
    let event_hash = event
        .event_hash
        .as_ref()
        .map(|h| h.sha256.as_str())
        .unwrap_or("");

    apply_audit(conn, event, ts, node_id, tenant_id, event_hash)?;

    if let Some(payload) = &event.payload {
        match payload {
            Payload::CaseCreated(cc) => {
                let tags_json = serde_json::to_string(&event.tags).unwrap_or_else(|_| "[]".into());
                conn.execute(
                    "INSERT OR REPLACE INTO cases_view
                     (case_id, title, summary, content_hash, shareable, tenant_id,
                      sensitivity, node_id, tags, created_at_ms, updated_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
                    params![
                        cc.case_id,
                        cc.title,
                        cc.summary,
                        cc.content_ref.as_ref().map(|h| &h.sha256),
                        cc.shareable as i32,
                        tenant_id,
                        event.sensitivity,
                        node_id,
                        tags_json,
                        ts,
                    ],
                )?;
                update_cases_fts(conn, &cc.case_id)?;
                // Shard membership: tenant, public if shareable
                let mut shards = vec![format!("tenant:{}", tenant_id)];
                if cc.shareable {
                    shards.push("public".into());
                }
                apply_shard_membership(conn, &shards, "case", &cc.case_id, node_id, ts)?;
            }
            Payload::CaseConfirmed(cf) => {
                conn.execute(
                    "UPDATE cases_view SET outcome = ?1, confidence = ?2, updated_at_ms = ?3
                     WHERE case_id = ?4",
                    params![cf.outcome, cf.confidence, ts, cf.case_id],
                )?;
                let outcome_id = format!("out-cc-{}", event.event_id);
                conn.execute(
                    "INSERT OR REPLACE INTO outcomes_view
                     (outcome_id, outcome_type, case_id, quote_id, outcome_value, reason, confidence, created_at_ms)
                     VALUES (?1, 'case_confirmed', ?2, '', ?3, '', ?4, ?5)",
                    params![outcome_id, cf.case_id, cf.outcome, cf.confidence, ts],
                )?;
            }
            Payload::CaseFailed(cf) => {
                let outcome_id = format!("out-cf-{}", event.event_id);
                conn.execute(
                    "INSERT OR REPLACE INTO outcomes_view
                     (outcome_id, outcome_type, case_id, quote_id, outcome_value, reason, confidence, created_at_ms)
                     VALUES (?1, 'case_failed', ?2, '', '', ?3, 0, ?4)",
                    params![outcome_id, cf.case_id, cf.reason, ts],
                )?;
            }
            Payload::QuoteAccepted(qa) => {
                let outcome_id = format!("out-qa-{}", event.event_id);
                conn.execute(
                    "INSERT OR REPLACE INTO outcomes_view
                     (outcome_id, outcome_type, case_id, quote_id, outcome_value, reason, confidence, created_at_ms)
                     VALUES (?1, 'quote_accepted', ?2, ?3, ?4, '', ?5, ?6)",
                    params![
                        outcome_id,
                        qa.case_id,
                        qa.quote_id,
                        qa.value_summary,
                        qa.confidence,
                        ts,
                    ],
                )?;
            }
            Payload::QuoteLost(ql) => {
                let outcome_id = format!("out-ql-{}", event.event_id);
                conn.execute(
                    "INSERT OR REPLACE INTO outcomes_view
                     (outcome_id, outcome_type, case_id, quote_id, outcome_value, reason, confidence, created_at_ms)
                     VALUES (?1, 'quote_lost', ?2, ?3, '', ?4, 0, ?5)",
                    params![outcome_id, ql.case_id, ql.quote_id, ql.reason, ts],
                )?;
            }
            Payload::QuoteRevised(qr) => {
                let outcome_id = format!("out-qr-{}", event.event_id);
                conn.execute(
                    "INSERT OR REPLACE INTO outcomes_view
                     (outcome_id, outcome_type, case_id, quote_id, outcome_value, reason, confidence, created_at_ms)
                     VALUES (?1, 'quote_revised', ?2, ?3, '', ?4, 0, ?5)",
                    params![outcome_id, qr.case_id, qr.quote_id, qr.revision_reason, ts],
                )?;
            }
            Payload::CaseTagged(ct) => {
                let existing: String = conn
                    .query_row(
                        "SELECT tags FROM cases_view WHERE case_id = ?1",
                        params![ct.case_id],
                        |row| row.get(0),
                    )
                    .unwrap_or_else(|_| "[]".into());

                let mut tags: Vec<String> = serde_json::from_str(&existing).unwrap_or_default();
                for add in &ct.add_tags {
                    if !tags.contains(add) {
                        tags.push(add.clone());
                    }
                }
                tags.retain(|t| !ct.remove_tags.contains(t));
                let tags_json = serde_json::to_string(&tags).unwrap_or_else(|_| "[]".into());

                conn.execute(
                    "UPDATE cases_view SET tags = ?1, updated_at_ms = ?2 WHERE case_id = ?3",
                    params![tags_json, ts, ct.case_id],
                )?;
                update_cases_fts(conn, &ct.case_id)?;
            }
            Payload::ArtifactPublished(ap) => {
                conn.execute(
                    "INSERT OR REPLACE INTO artifacts_view
                     (artifact_id, version, artifact_type, title, summary, content_hash,
                      shareable, tenant_id, sensitivity, node_id, expires_at_ms, created_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                    params![
                        ap.artifact_id,
                        ap.version,
                        ap.artifact_type,
                        ap.title,
                        ap.summary,
                        ap.content_ref.as_ref().map(|h| &h.sha256),
                        ap.shareable as i32,
                        tenant_id,
                        event.sensitivity,
                        node_id,
                        if ap.expires_unix_ms > 0 {
                            Some(ap.expires_unix_ms)
                        } else {
                            None
                        },
                        ts,
                    ],
                )?;
                update_artifacts_fts(conn, &ap.artifact_id, ap.version)?;
                // Populate documents_view for DOCUMENT type; facts_view for FACT type
                const ARTIFACT_TYPE_DOCUMENT: i32 = 6;
                const ARTIFACT_TYPE_FACT: i32 = 7;
                if ap.artifact_type == ARTIFACT_TYPE_DOCUMENT {
                    let doc_type = if ap.document_subtype.is_empty() {
                        "entity_card"
                    } else {
                        &ap.document_subtype
                    };
                    conn.execute(
                        "INSERT OR REPLACE INTO documents_view
                         (document_id, version, document_type, entity_type, entity_key,
                          content_hash, source_id, table_name, title, summary, created_at_ms)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                        params![
                            ap.artifact_id,
                            ap.version,
                            doc_type,
                            ap.entity_type,
                            ap.entity_key,
                            ap.content_ref.as_ref().map(|h| &h.sha256),
                            ap.source_ref,
                            ap.table_name,
                            ap.title,
                            ap.summary,
                            ts,
                        ],
                    )?;
                    // Populate documents_fts for document_chunk (full chunk text indexing)
                    if doc_type == "document_chunk" && !ap.entity_attributes_json.is_empty() {
                        if let Ok(attrs) = serde_json::from_str::<
                            serde_json::Map<String, serde_json::Value>,
                        >(&ap.entity_attributes_json)
                        {
                            let doc_id = attrs
                                .get("document_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let chunk_idx = attrs
                                .get("chunk_index")
                                .map(|v| {
                                    v.as_str()
                                        .map(String::from)
                                        .or_else(|| v.as_u64().map(|n| n.to_string()))
                                        .unwrap_or_default()
                                })
                                .unwrap_or_default();
                            let chunk_text = attrs
                                .get("chunk_text")
                                .or(attrs.get("content_text"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            if !doc_id.is_empty() && !chunk_text.is_empty() {
                                update_documents_fts(
                                    conn,
                                    &ap.artifact_id,
                                    doc_id,
                                    &chunk_idx,
                                    chunk_text,
                                )?;
                                // Populate document_chunks_view for debug (OCR, source_file, provenance)
                                let source_file = attrs
                                    .get("source_file")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                let source_locator = attrs
                                    .get("source_locator")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or(source_file);
                                let source_open_target = attrs
                                    .get("source_open_target")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                let source_origin_label = attrs
                                    .get("source_origin_label")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or(source_file);
                                let page_number: i64 = attrs
                                    .get("page_number")
                                    .and_then(|v| v.as_str().and_then(|s| s.parse().ok()))
                                    .or_else(|| attrs.get("page_number").and_then(|v| v.as_i64()))
                                    .unwrap_or(0);
                                let ocr_used: i32 = attrs
                                    .get("ocr_used")
                                    .and_then(|v| {
                                        v.as_str()
                                            .map(|s| if s == "1" { 1 } else { 0 })
                                            .or_else(|| v.as_i64().map(|n| n as i32))
                                    })
                                    .unwrap_or(0);
                                let chunk_idx_int: i64 = chunk_idx.parse().unwrap_or(0);
                                let _ = conn.execute(
                                    "INSERT OR REPLACE INTO document_chunks_view
                                     (artifact_id, document_id, chunk_index, chunk_text, source_file, page_number, ocr_used, source_id, source_locator, source_open_target, source_origin_label, created_at_ms)
                                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                                    params![
                                        ap.artifact_id,
                                        doc_id,
                                        chunk_idx_int,
                                        chunk_text,
                                        source_file,
                                        page_number,
                                        ocr_used,
                                        ap.source_ref,
                                        source_locator,
                                        source_open_target,
                                        source_origin_label,
                                        ts,
                                    ],
                                );
                            }
                        }
                    }
                    // Populate entity_cards_view for entity_card documents
                    if doc_type == "entity_card" && !ap.entity_type.is_empty() {
                        let entity_id = format!("{}:{}", ap.entity_type, ap.entity_key);
                        let attrs_json = if ap.entity_attributes_json.is_empty() {
                            "{}"
                        } else {
                            &ap.entity_attributes_json
                        };
                        conn.execute(
                            "INSERT OR REPLACE INTO entity_cards_view
                             (entity_id, entity_type, attributes_json, content_hash, source_id, table_name, created_at_ms)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                            params![
                                entity_id,
                                ap.entity_type,
                                attrs_json,
                                ap.content_ref.as_ref().map(|h| &h.sha256),
                                ap.source_ref,
                                ap.table_name,
                                ts,
                            ],
                        )?;
                    }
                }
                // Shard membership: tenant, entity_type, artifact_class, public if shareable
                let mut shards = vec![format!("tenant:{}", tenant_id)];
                if ap.shareable {
                    shards.push("public".into());
                }
                if !ap.entity_type.is_empty() {
                    shards.push(format!("entity_type:{}", ap.entity_type));
                }
                let artifact_class = match ap.artifact_type {
                    1 => "runbook",
                    2 => "template",
                    3 => "recipe",
                    4 => "web_brief",
                    5 => "model_bundle",
                    6 => "document",
                    7 => "fact",
                    _ => "artifact",
                };
                shards.push(format!("artifact_class:{}", artifact_class));
                apply_shard_membership(conn, &shards, "artifact", &ap.artifact_id, node_id, ts)?;

                if ap.artifact_type == ARTIFACT_TYPE_FACT && !ap.metric.is_empty() {
                    conn.execute(
                        "INSERT OR REPLACE INTO facts_view
                         (fact_id, version, source_id, ingest_id, metric,
                          dimensions_json, value_json, time_window, content_hash, created_at_ms)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                        params![
                            ap.artifact_id,
                            ap.version,
                            ap.source_ref,
                            ap.ingest_id,
                            ap.metric,
                            if ap.dimensions_json.is_empty() {
                                "{}"
                            } else {
                                &ap.dimensions_json
                            },
                            if ap.value_json.is_empty() {
                                "{}"
                            } else {
                                &ap.value_json
                            },
                            ap.time_window,
                            ap.content_ref.as_ref().map(|h| &h.sha256),
                            ts,
                        ],
                    )?;
                }
            }
            Payload::ArtifactDeprecated(ad) => {
                conn.execute(
                    "UPDATE artifacts_view SET deprecated = 1, deprecate_reason = ?1
                     WHERE artifact_id = ?2 AND version = ?3",
                    params![ad.reason, ad.artifact_id, ad.version],
                )?;
                // Remove deprecated document chunks from documents_fts
                conn.execute(
                    "DELETE FROM documents_fts WHERE artifact_id = ?1",
                    params![ad.artifact_id],
                )?;
            }
            Payload::WebBriefCreated(wb) => {
                let sources_json = serde_json::to_string(
                    &wb.sources
                        .iter()
                        .map(|s| {
                            serde_json::json!({
                                "url": s.url,
                                "publisher": s.publisher,
                                "snippet": s.snippet,
                            })
                        })
                        .collect::<Vec<_>>(),
                )
                .unwrap_or_else(|_| "[]".into());

                conn.execute(
                    "INSERT OR REPLACE INTO web_briefs_view
                     (artifact_id, question, summary, sources_json, confidence,
                      expires_at_ms, tenant_id, node_id, created_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        wb.artifact_id,
                        wb.question,
                        wb.summary,
                        sources_json,
                        wb.confidence,
                        if wb.expires_unix_ms > 0 {
                            Some(wb.expires_unix_ms)
                        } else {
                            None
                        },
                        tenant_id,
                        node_id,
                        ts,
                    ],
                )?;
            }
            Payload::WebBriefExpired(we) => {
                conn.execute(
                    "UPDATE web_briefs_view SET expired = 1 WHERE artifact_id = ?1",
                    params![we.artifact_id],
                )?;
            }
            Payload::PeerSeen(ps) => {
                let caps_json =
                    serde_json::to_string(&ps.capabilities).unwrap_or_else(|_| "[]".into());
                let peer_id = ps
                    .peer_node_id
                    .as_ref()
                    .map(|n| n.value.as_str())
                    .unwrap_or("");
                conn.execute(
                    "INSERT INTO peers_view (peer_node_id, rtt_ms, capabilities, last_seen_ms)
                     VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(peer_node_id) DO UPDATE SET
                       rtt_ms = excluded.rtt_ms,
                       capabilities = excluded.capabilities,
                       last_seen_ms = excluded.last_seen_ms",
                    params![peer_id, ps.rtt_ms, caps_json, ts],
                )?;
            }
            Payload::PeerTrustUpdated(pt) => {
                let peer_id = pt
                    .peer_node_id
                    .as_ref()
                    .map(|n| n.value.as_str())
                    .unwrap_or("");
                conn.execute(
                    "UPDATE peers_view SET trust_score = ?1, trust_reason = ?2
                     WHERE peer_node_id = ?3",
                    params![pt.trust_score, pt.reason, peer_id],
                )?;
            }
            Payload::TrainJobStarted(_) | Payload::TrainJobCompleted(_) => {
                // Audit entry already written above
            }
            Payload::ModelPromoted(mp) => {
                conn.execute(
                    "INSERT OR REPLACE INTO models_view
                     (model_id, version, model_bundle_hash, promoted, node_id, created_at_ms)
                     VALUES (?1, ?2, ?3, 1, ?4, ?5)",
                    params![
                        mp.model_id,
                        mp.version,
                        mp.model_bundle_ref.as_ref().map(|h| &h.sha256),
                        node_id,
                        ts,
                    ],
                )?;
            }
            Payload::ModelRolledBack(mr) => {
                conn.execute(
                    "UPDATE models_view SET rolled_back = 1, rollback_reason = ?1
                     WHERE model_id = ?2 AND version = ?3",
                    params![mr.reason, mr.model_id, mr.from_version],
                )?;
            }
            Payload::PolicyUpdated(_)
            | Payload::ToolInvocationRecorded(_)
            | Payload::DataSharedRecorded(_) => {
                // Audit entry already written above
            }
            Payload::DataSourceDiscovered(d) => {
                conn.execute(
                    "INSERT OR REPLACE INTO sources_view
                     (source_id, connector_type, path_or_uri, display_name,
                      estimated_size_bytes, estimated_tables, status, discovered_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'discovered', ?7)",
                    params![
                        d.source_id,
                        d.connector_type,
                        d.path_or_uri,
                        d.display_name,
                        d.estimated_size_bytes as i64,
                        d.estimated_tables,
                        d.discovered_at.as_ref().map(|t| t.unix_ms).unwrap_or(ts),
                    ],
                )?;
            }
            Payload::DataSourceClassified(c) => {
                conn.execute(
                    "UPDATE sources_view SET status = 'classified', sensitivity = ?1,
                     pii_detected = ?2, secrets_detected = ?3,
                     schema_snapshot_hash = ?4, classified_at_ms = ?5
                     WHERE source_id = ?6",
                    params![
                        c.suggested_sensitivity,
                        c.pii_detected as i32,
                        c.secrets_detected as i32,
                        c.schema_snapshot_ref.as_ref().map(|h| &h.sha256),
                        ts,
                        c.source_id,
                    ],
                )?;
            }
            Payload::DataSourceApproved(a) => {
                let tables_json =
                    serde_json::to_string(&a.allowed_tables).unwrap_or_else(|_| "[]".into());
                conn.execute(
                    "UPDATE sources_view SET status = 'approved', approved_at_ms = ?1
                     WHERE source_id = ?2",
                    params![
                        a.approved_at.as_ref().map(|t| t.unix_ms).unwrap_or(ts),
                        a.source_id
                    ],
                )?;
                conn.execute(
                    "INSERT OR REPLACE INTO source_profiles_view
                     (profile_id, source_id, approved_by, approved_at_ms,
                      profile_hash, allowed_tables_json, row_limit, mapping_rules_json)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, '{}')",
                    params![
                        format!("prof-{}", a.source_id),
                        a.source_id,
                        a.approved_by,
                        a.approved_at.as_ref().map(|t| t.unix_ms).unwrap_or(ts),
                        a.source_profile_ref.as_ref().map(|h| &h.sha256),
                        tables_json,
                        a.row_limit,
                    ],
                )?;
            }
            Payload::DataSourceRemoved(r) => {
                conn.execute(
                    "UPDATE sources_view SET status = 'removed' WHERE source_id = ?1",
                    params![r.source_id],
                )?;
            }
            Payload::IngestStarted(i) => {
                conn.execute(
                    "INSERT OR REPLACE INTO ingests_view
                     (ingest_id, source_id, connector_type, status, started_at_ms)
                     VALUES (?1, ?2, ?3, 'started', ?4)",
                    params![
                        i.ingest_id,
                        i.source_id,
                        i.connector_type,
                        i.started_at.as_ref().map(|t| t.unix_ms).unwrap_or(ts),
                    ],
                )?;
            }
            Payload::IngestCompleted(i) => {
                conn.execute(
                    "UPDATE ingests_view SET status = ?1, rows_ingested = ?2,
                     documents_created = ?3, facts_created = ?4,
                     bytes_stored = ?5, duration_ms = ?6, notes = ?7, completed_at_ms = ?8
                     WHERE ingest_id = ?9",
                    params![
                        if i.success { "completed" } else { "failed" },
                        i.rows_ingested as i64,
                        i.documents_created as i64,
                        i.facts_created as i64,
                        i.bytes_stored as i64,
                        i.duration_ms,
                        i.notes,
                        ts,
                        i.ingest_id,
                    ],
                )?;
            }
            Payload::ShardSubscriptionAdded(ss) => {
                let sub_node = ss.node_id.as_ref().map(|n| n.value.as_str()).unwrap_or("");
                let capability = if ss.capability.is_empty() {
                    "query"
                } else {
                    &ss.capability
                };
                let last_seen = if ss.last_seen_ms > 0 {
                    ss.last_seen_ms
                } else {
                    ts
                };
                conn.execute(
                    "INSERT OR REPLACE INTO shard_subscriptions_view
                     (shard_key, node_id, capability, last_seen_ms)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![ss.shard_key, sub_node, capability, last_seen],
                )?;
            }
            Payload::MergeableTagUpdated(mt) => {
                let event_id = event.event_id.as_str();
                let object_type = if mt.object_type.is_empty() {
                    "case"
                } else {
                    &mt.object_type
                };
                let op = if mt.op.is_empty() { "add" } else { &mt.op };
                conn.execute(
                    "INSERT OR REPLACE INTO mergeable_tag_events
                     (event_id, object_type, object_id, tag, op, node_id, ts_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![event_id, object_type, mt.object_id, mt.tag, op, node_id, ts],
                )?;
            }
            Payload::MergeableCounterUpdated(mc) => {
                let event_id = event.event_id.as_str();
                let object_type = if mc.object_type.is_empty() {
                    "case"
                } else {
                    &mc.object_type
                };
                conn.execute(
                    "INSERT OR REPLACE INTO mergeable_counter_deltas
                     (event_id, object_type, object_id, counter_key, node_id, delta, ts_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        event_id,
                        object_type,
                        mc.object_id,
                        mc.counter_key,
                        node_id,
                        mc.delta,
                        ts,
                    ],
                )?;
            }
            Payload::MergeableAnnotationUpdated(ma) => {
                let object_type = if ma.object_type.is_empty() {
                    "case"
                } else {
                    &ma.object_type
                };
                conn.execute(
                    "INSERT OR IGNORE INTO mergeable_annotations_view
                     (object_type, object_id, annotation_key, value, node_id, ts_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        object_type,
                        ma.object_id,
                        ma.annotation_key,
                        ma.value,
                        node_id,
                        ts,
                    ],
                )?;
                if conn.changes() == 0 {
                    conn.execute(
                        "UPDATE mergeable_annotations_view
                         SET value = ?1, node_id = ?2, ts_ms = ?3
                         WHERE object_type = ?4 AND object_id = ?5 AND annotation_key = ?6
                           AND ?3 > ts_ms",
                        params![
                            ma.value,
                            node_id,
                            ts,
                            object_type,
                            ma.object_id,
                            ma.annotation_key,
                        ],
                    )?;
                }
            }
            Payload::EntityRelationshipRecorded(err) => {
                conn.execute(
                    "INSERT OR REPLACE INTO entity_relationships_view
                     (from_entity_id, to_entity_id, relationship_type, source_id, table_name, created_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        err.from_entity_id,
                        err.to_entity_id,
                        err.relationship_type,
                        err.source_id,
                        err.table_name,
                        ts,
                    ],
                )?;
            }
            Payload::ExtractedRelationshipRecorded(er) => {
                let extraction_method = if er.extraction_method.is_empty() {
                    "rule_based"
                } else {
                    er.extraction_method.as_str()
                };
                conn.execute(
                    "INSERT OR REPLACE INTO extracted_entity_relationships_view
                     (relationship_id, from_entity_id, from_entity_value, relationship_type,
                      to_entity_id, to_entity_value, source_document_id, chunk_index,
                      confidence, extraction_method, created_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    params![
                        er.relationship_id,
                        er.from_entity_id,
                        er.from_entity_value,
                        er.relationship_type,
                        er.to_entity_id,
                        er.to_entity_value,
                        er.source_document_id,
                        er.chunk_index,
                        er.confidence,
                        extraction_method,
                        ts,
                    ],
                )?;
            }
            Payload::ExtractedEntityRecorded(ee) => {
                let doc_entity_id = format!("document:{}", ee.source_document_id);
                let classification_method = if ee.classification_method.is_empty() {
                    "rule_based"
                } else {
                    ee.classification_method.as_str()
                };
                conn.execute(
                    "INSERT OR REPLACE INTO entities_view
                     (entity_id, entity_type, entity_value, normalized_value, document_id, chunk_index,
                      confidence, extraction_method, classification_method, created_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        ee.entity_id,
                        ee.entity_type,
                        ee.entity_value,
                        ee.normalized_value,
                        ee.source_document_id,
                        ee.chunk_index,
                        ee.confidence,
                        ee.extraction_method,
                        classification_method,
                        ts,
                    ],
                )?;
                conn.execute(
                    "INSERT OR REPLACE INTO documents_entities_view
                     (document_id, entity_id, entity_type, entity_value, chunk_index, created_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        ee.source_document_id,
                        ee.entity_id,
                        ee.entity_type,
                        ee.entity_value,
                        ee.chunk_index,
                        ts,
                    ],
                )?;
                conn.execute(
                    "INSERT OR REPLACE INTO entity_relationships_view
                     (from_entity_id, to_entity_id, relationship_type, source_id, table_name, created_at_ms)
                     VALUES (?1, ?2, 'mentions', '', 'document_extraction', ?3)",
                    params![doc_entity_id, ee.entity_id, ts],
                )?;
            }
            Payload::DatasetManifestCreated(dm) => {
                conn.execute(
                    "INSERT OR REPLACE INTO datasets_view
                     (manifest_id, source_id, preset, manifest_hash, item_count, total_bytes, created_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        dm.manifest_id,
                        dm.source_id,
                        dm.preset,
                        dm.manifest_ref.as_ref().map(|h| &h.sha256),
                        dm.item_count as i64,
                        dm.total_bytes as i64,
                        ts,
                    ],
                )?;
            }
            Payload::FederatedRoundStarted(fr) => {
                conn.execute(
                    "INSERT OR REPLACE INTO federated_view
                     (round_id, model_id, round_number, status, expected_participants,
                      coordinator, started_at_ms)
                     VALUES (?1, ?2, ?3, 'started', ?4, ?5, ?6)",
                    params![
                        fr.round_id,
                        fr.model_id,
                        fr.round_number,
                        fr.expected_participants,
                        fr.coordinator
                            .as_ref()
                            .map(|n| &n.value)
                            .unwrap_or(&String::new()),
                        fr.started_at.as_ref().map(|t| t.unix_ms).unwrap_or(ts),
                    ],
                )?;
            }
            Payload::FederatedRoundCompleted(fr) => {
                conn.execute(
                    "UPDATE federated_view SET status = ?1, actual_participants = ?2,
                     success = ?3, resulting_model_hash = ?4, notes = ?5, completed_at_ms = ?6
                     WHERE round_id = ?7",
                    params![
                        if fr.success { "completed" } else { "failed" },
                        fr.actual_participants,
                        fr.success as i32,
                        fr.resulting_model_ref.as_ref().map(|h| &h.sha256),
                        fr.notes,
                        ts,
                        fr.round_id,
                    ],
                )?;
            }
            Payload::InsightGenerated(ig) => {
                let entity_ids_json =
                    serde_json::to_string(&ig.entity_ids).unwrap_or_else(|_| "[]".into());
                let schedule = if ig.schedule.is_empty() {
                    "manual"
                } else {
                    &ig.schedule
                };
                conn.execute(
                    "INSERT OR REPLACE INTO insights_view
                     (insight_id, insight_type, title, summary, entity_ids_json, confidence, schedule, created_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        ig.insight_id,
                        ig.insight_type,
                        ig.title,
                        ig.summary,
                        entity_ids_json,
                        ig.confidence,
                        schedule,
                        ts,
                    ],
                )?;
            }
            Payload::AnomalyDetected(ad) => {
                let schedule = if ad.schedule.is_empty() {
                    "manual"
                } else {
                    &ad.schedule
                };
                conn.execute(
                    "INSERT OR REPLACE INTO anomalies_view
                     (anomaly_id, metric, dimension, expected_value, actual_value, deviation_pct, schedule, created_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        ad.anomaly_id,
                        ad.metric,
                        ad.dimension,
                        ad.expected_value,
                        ad.actual_value,
                        ad.deviation_pct,
                        schedule,
                        ts,
                    ],
                )?;
            }
            Payload::AlertRaised(ar) => {
                let entity_ids_json =
                    serde_json::to_string(&ar.entity_ids).unwrap_or_else(|_| "[]".into());
                let schedule = if ar.schedule.is_empty() {
                    "manual"
                } else {
                    &ar.schedule
                };
                conn.execute(
                    "INSERT OR REPLACE INTO alerts_view
                     (alert_id, alert_type, severity, title, message, entity_ids_json, schedule, created_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        ar.alert_id,
                        ar.alert_type,
                        if ar.severity.is_empty() { "info" } else { &ar.severity },
                        ar.title,
                        ar.message,
                        entity_ids_json,
                        schedule,
                        ts,
                    ],
                )?;
            }
            Payload::BenchmarkUpdated(bu) => {
                let schedule = if bu.schedule.is_empty() {
                    "manual"
                } else {
                    &bu.schedule
                };
                conn.execute(
                    "INSERT OR REPLACE INTO benchmarks_view
                     (benchmark_id, metric, dimension, value, time_window, schedule, created_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        bu.benchmark_id,
                        bu.metric,
                        bu.dimension,
                        bu.value,
                        bu.time_window,
                        schedule,
                        ts,
                    ],
                )?;
            }
            Payload::TrainDeltaPublished(_) | Payload::TrainDeltaApplied(_) => {
                // Audit entry already written above
            }
        }
    }

    update_checkpoint(conn, event_hash, 1)?;
    Ok(())
}

fn apply_audit(
    conn: &Connection,
    event: &EventEnvelope,
    ts: i64,
    node_id: &str,
    tenant_id: &str,
    event_hash: &str,
) -> Result<()> {
    let summary = event_summary(event);
    conn.execute(
        "INSERT OR IGNORE INTO audit_view
         (event_id, event_type, node_id, tenant_id, sensitivity, summary, event_hash, created_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            event.event_id,
            event.r#type,
            node_id,
            tenant_id,
            event.sensitivity,
            summary,
            event_hash,
            ts,
        ],
    )?;
    Ok(())
}

fn event_summary(event: &EventEnvelope) -> String {
    match &event.payload {
        Some(Payload::CaseCreated(cc)) => format!("case created: {}", cc.title),
        Some(Payload::CaseConfirmed(cf)) => {
            format!("case confirmed: {} ({})", cf.case_id, cf.outcome)
        }
        Some(Payload::CaseFailed(cf)) => {
            format!("case failed: {} {}", cf.case_id, cf.reason)
        }
        Some(Payload::QuoteAccepted(qa)) => {
            format!("quote accepted: {} case {}", qa.quote_id, qa.case_id)
        }
        Some(Payload::QuoteLost(ql)) => {
            format!("quote lost: {} case {}", ql.quote_id, ql.case_id)
        }
        Some(Payload::QuoteRevised(qr)) => {
            format!("quote revised: {} case {}", qr.quote_id, qr.case_id)
        }
        Some(Payload::CaseTagged(ct)) => format!("case tagged: {}", ct.case_id),
        Some(Payload::ArtifactPublished(ap)) => format!("artifact published: {}", ap.title),
        Some(Payload::ArtifactDeprecated(ad)) => format!("artifact deprecated: {}", ad.artifact_id),
        Some(Payload::WebBriefCreated(wb)) => format!("web brief: {}", wb.question),
        Some(Payload::WebBriefExpired(we)) => format!("web brief expired: {}", we.artifact_id),
        Some(Payload::PeerSeen(ps)) => format!("peer seen: {:?}", ps.peer_node_id),
        Some(Payload::PeerTrustUpdated(pt)) => {
            format!("trust updated: {:?} -> {}", pt.peer_node_id, pt.trust_score)
        }
        Some(Payload::PolicyUpdated(pu)) => format!("policy updated: {}", pu.policy_id),
        Some(Payload::TrainJobStarted(tj)) => format!("training started: {}", tj.job_id),
        Some(Payload::TrainJobCompleted(tc)) => {
            format!("training completed: {} success={}", tc.job_id, tc.success)
        }
        Some(Payload::ModelPromoted(mp)) => {
            format!("model promoted: {} v{}", mp.model_id, mp.version)
        }
        Some(Payload::ModelRolledBack(mr)) => {
            format!(
                "model rolled back: {} v{} -> v{}",
                mr.model_id, mr.from_version, mr.to_version
            )
        }
        Some(Payload::ToolInvocationRecorded(ti)) => format!("tool invoked: {}", ti.tool_name),
        Some(Payload::DataSharedRecorded(ds)) => format!("data shared: {}", ds.share_id),
        Some(Payload::DataSourceDiscovered(d)) => {
            format!("data source discovered: {}", d.display_name)
        }
        Some(Payload::DataSourceClassified(c)) => {
            format!("data source classified: {}", c.source_id)
        }
        Some(Payload::DataSourceRemoved(r)) => format!("data source removed: {}", r.source_id),
        Some(Payload::DataSourceApproved(a)) => {
            format!("data source approved: {}", a.source_id)
        }
        Some(Payload::MergeableTagUpdated(mt)) => {
            format!(
                "mergeable tag: {} {} {}",
                mt.object_type, mt.object_id, mt.tag
            )
        }
        Some(Payload::MergeableCounterUpdated(mc)) => {
            format!(
                "mergeable counter: {} {} {}",
                mc.object_type, mc.object_id, mc.counter_key
            )
        }
        Some(Payload::MergeableAnnotationUpdated(ma)) => {
            format!(
                "mergeable annotation: {} {} {}",
                ma.object_type, ma.object_id, ma.annotation_key
            )
        }
        Some(Payload::InsightGenerated(ig)) => {
            format!("insight: {} {}", ig.insight_type, ig.title)
        }
        Some(Payload::AnomalyDetected(ad)) => {
            format!(
                "anomaly: {} {} (expected {}, actual {})",
                ad.metric, ad.dimension, ad.expected_value, ad.actual_value
            )
        }
        Some(Payload::AlertRaised(ar)) => {
            format!("alert: {} {}", ar.alert_type, ar.title)
        }
        Some(Payload::BenchmarkUpdated(bu)) => {
            format!("benchmark: {} {} = {}", bu.metric, bu.dimension, bu.value)
        }
        Some(Payload::ShardSubscriptionAdded(ss)) => {
            format!(
                "shard subscription: {} <- {} ({})",
                ss.shard_key,
                ss.node_id.as_ref().map(|n| n.value.as_str()).unwrap_or(""),
                if ss.capability.is_empty() {
                    "query"
                } else {
                    &ss.capability
                }
            )
        }
        Some(Payload::EntityRelationshipRecorded(err)) => {
            format!(
                "entity relationship: {} -[{}]-> {}",
                err.from_entity_id, err.relationship_type, err.to_entity_id
            )
        }
        Some(Payload::ExtractedRelationshipRecorded(er)) => {
            format!(
                "extracted relationship: {} -[{}]-> {}",
                er.from_entity_id, er.relationship_type, er.to_entity_id
            )
        }
        Some(Payload::ExtractedEntityRecorded(ee)) => {
            format!(
                "extracted entity: {} {} from doc {}",
                ee.entity_type, ee.entity_id, ee.source_document_id
            )
        }
        Some(Payload::IngestStarted(i)) => format!("ingest started: {}", i.ingest_id),
        Some(Payload::IngestCompleted(i)) => {
            format!("ingest completed: {} rows={}", i.ingest_id, i.rows_ingested)
        }
        Some(Payload::DatasetManifestCreated(dm)) => {
            format!("dataset manifest: {}", dm.manifest_id)
        }
        Some(Payload::TrainDeltaPublished(td)) => {
            format!("train delta published: {}", td.delta_id)
        }
        Some(Payload::TrainDeltaApplied(td)) => {
            format!("train delta applied: {}", td.delta_id)
        }
        Some(Payload::FederatedRoundStarted(fr)) => {
            format!("federated round started: {}", fr.round_id)
        }
        Some(Payload::FederatedRoundCompleted(fr)) => {
            format!("federated round completed: {}", fr.round_id)
        }
        None => "unknown event".to_string(),
    }
}

/// Insert shard metadata and membership. Upserts shards_view, inserts shard_membership_view.
fn apply_shard_membership(
    conn: &Connection,
    shard_keys: &[String],
    member_type: &str,
    member_id: &str,
    node_id: &str,
    ts: i64,
) -> Result<()> {
    for key in shard_keys {
        if key.is_empty() {
            continue;
        }
        let kind = if key == "public" {
            "public"
        } else if key.starts_with("tenant:") {
            "tenant"
        } else if key.starts_with("entity_type:") {
            "entity_type"
        } else if key.starts_with("artifact_class:") {
            "artifact_class"
        } else if key.starts_with("site:") {
            "site"
        } else {
            "tenant"
        };
        conn.execute(
            "INSERT OR IGNORE INTO shards_view (shard_key, shard_kind, created_at_ms)
             VALUES (?1, ?2, ?3)",
            params![key, kind, ts],
        )?;
        conn.execute(
            "INSERT OR REPLACE INTO shard_membership_view
             (shard_key, member_type, member_id, node_id, created_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![key, member_type, member_id, node_id, ts],
        )?;
    }
    Ok(())
}

fn update_cases_fts(conn: &Connection, case_id: &str) -> Result<()> {
    conn.execute("DELETE FROM cases_fts WHERE case_id = ?1", params![case_id])?;
    conn.execute(
        "INSERT INTO cases_fts (case_id, title, summary, tags)
         SELECT case_id, title, summary, tags FROM cases_view
         WHERE case_id = ?1",
        params![case_id],
    )?;
    Ok(())
}

fn update_artifacts_fts(conn: &Connection, artifact_id: &str, _version: u32) -> Result<()> {
    conn.execute(
        "DELETE FROM artifacts_fts WHERE artifact_id = ?1",
        params![artifact_id],
    )?;
    conn.execute(
        "INSERT INTO artifacts_fts (artifact_id, title, summary)
         SELECT artifact_id, title, summary FROM artifacts_view
         WHERE artifact_id = ?1
         ORDER BY version DESC
         LIMIT 1",
        params![artifact_id],
    )?;
    Ok(())
}

fn update_documents_fts(
    conn: &Connection,
    artifact_id: &str,
    document_id: &str,
    chunk_index: &str,
    chunk_text: &str,
) -> Result<()> {
    conn.execute(
        "DELETE FROM documents_fts WHERE artifact_id = ?1",
        params![artifact_id],
    )?;
    conn.execute(
        "INSERT INTO documents_fts (artifact_id, document_id, chunk_index, chunk_text)
         VALUES (?1, ?2, ?3, ?4)",
        params![artifact_id, document_id, chunk_index, chunk_text],
    )?;
    Ok(())
}

fn update_checkpoint(conn: &Connection, event_hash: &str, increment: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO projector_checkpoint (id, last_event_hash, event_count)
         VALUES (1, ?1, ?2)
         ON CONFLICT(id) DO UPDATE SET
           last_event_hash = excluded.last_event_hash,
           event_count = projector_checkpoint.event_count + excluded.event_count",
        params![event_hash, increment],
    )?;
    Ok(())
}

/// Get the checkpoint (last applied event hash, count).
pub fn get_checkpoint(conn: &Connection) -> Result<Option<(String, i64)>> {
    let result = conn.query_row(
        "SELECT last_event_hash, event_count FROM projector_checkpoint WHERE id = 1",
        [],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
    );
    match result {
        Ok(pair) => Ok(Some(pair)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Replay a list of events into a fresh or existing DB.
pub fn replay_events(conn: &Connection, events: &[EventEnvelope]) -> Result<()> {
    for event in events {
        apply_event(conn, event)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite_views;
    use node_proto::common::*;
    use node_proto::events::*;
    use rusqlite::Connection;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        sqlite_views::create_schema(&conn).unwrap();
        conn
    }

    fn make_event(id: &str, payload: Payload) -> EventEnvelope {
        EventEnvelope {
            event_id: id.to_string(),
            r#type: 0,
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
            event_hash: Some(HashRef {
                sha256: format!("hash-{id}"),
            }),
            payload: Some(payload),
            tags: vec!["test".into()],
            ..Default::default()
        }
    }

    #[test]
    fn project_case_created() {
        let conn = setup();
        let event = make_event(
            "e1",
            Payload::CaseCreated(CaseCreated {
                case_id: "case-1".into(),
                title: "DNS failure".into(),
                summary: "DNS resolution failed".into(),
                content_ref: Some(HashRef {
                    sha256: "content-h".into(),
                }),
                shareable: false,
            }),
        );
        apply_event(&conn, &event).unwrap();

        let title: String = conn
            .query_row(
                "SELECT title FROM cases_view WHERE case_id = 'case-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(title, "DNS failure");
    }

    #[test]
    fn project_case_confirmed() {
        let conn = setup();
        apply_event(
            &conn,
            &make_event(
                "e1",
                Payload::CaseCreated(CaseCreated {
                    case_id: "c1".into(),
                    title: "Test".into(),
                    summary: "Summ".into(),
                    content_ref: None,
                    shareable: false,
                }),
            ),
        )
        .unwrap();

        apply_event(
            &conn,
            &make_event(
                "e2",
                Payload::CaseConfirmed(CaseConfirmed {
                    case_id: "c1".into(),
                    outcome: "resolved".into(),
                    confidence: 0.95,
                }),
            ),
        )
        .unwrap();

        let outcome: String = conn
            .query_row(
                "SELECT outcome FROM cases_view WHERE case_id = 'c1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(outcome, "resolved");
    }

    #[test]
    fn project_artifact_lifecycle() {
        let conn = setup();
        apply_event(
            &conn,
            &make_event(
                "e1",
                Payload::ArtifactPublished(ArtifactPublished {
                    artifact_id: "art-1".into(),
                    artifact_type: ArtifactType::Runbook as i32,
                    version: 1,
                    title: "K8s runbook".into(),
                    summary: "Step-by-step guide to K8s operations".into(),
                    content_ref: Some(HashRef {
                        sha256: "rb-hash".into(),
                    }),
                    shareable: true,
                    expires_unix_ms: 0,
                    ..Default::default()
                }),
            ),
        )
        .unwrap();

        apply_event(
            &conn,
            &make_event(
                "e2",
                Payload::ArtifactDeprecated(ArtifactDeprecated {
                    artifact_id: "art-1".into(),
                    version: 1,
                    reason: "outdated".into(),
                }),
            ),
        )
        .unwrap();

        let deprecated: i32 = conn
            .query_row(
                "SELECT deprecated FROM artifacts_view WHERE artifact_id = 'art-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(deprecated, 1);
    }

    #[test]
    fn project_web_brief_lifecycle() {
        let conn = setup();
        apply_event(
            &conn,
            &make_event(
                "e1",
                Payload::WebBriefCreated(WebBriefCreated {
                    artifact_id: "wb-1".into(),
                    question: "What is Rust?".into(),
                    summary: "Rust is a systems language".into(),
                    sources: vec![WebSource {
                        url: "https://rust-lang.org".into(),
                        retrieved_at: Some(Timestamp {
                            unix_ms: 1700000000000,
                        }),
                        publisher: "Rust Foundation".into(),
                        snippet: "A language empowering everyone".into(),
                    }],
                    confidence: 0.9,
                    expires_unix_ms: 1700100000000,
                }),
            ),
        )
        .unwrap();

        apply_event(
            &conn,
            &make_event(
                "e2",
                Payload::WebBriefExpired(WebBriefExpired {
                    artifact_id: "wb-1".into(),
                    version: 1,
                }),
            ),
        )
        .unwrap();

        let expired: i32 = conn
            .query_row(
                "SELECT expired FROM web_briefs_view WHERE artifact_id = 'wb-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(expired, 1);
    }

    #[test]
    fn project_peers() {
        let conn = setup();
        apply_event(
            &conn,
            &make_event(
                "e1",
                Payload::PeerSeen(PeerSeen {
                    peer_node_id: Some(NodeId {
                        value: "peer-1".into(),
                    }),
                    rtt_ms: 15,
                    capabilities: vec!["inference".into()],
                }),
            ),
        )
        .unwrap();

        apply_event(
            &conn,
            &make_event(
                "e2",
                Payload::PeerTrustUpdated(PeerTrustUpdated {
                    peer_node_id: Some(NodeId {
                        value: "peer-1".into(),
                    }),
                    trust_score: 0.8,
                    reason: "good responses".into(),
                }),
            ),
        )
        .unwrap();

        let trust: f64 = conn
            .query_row(
                "SELECT trust_score FROM peers_view WHERE peer_node_id = 'peer-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!((trust - 0.8).abs() < 0.001);
    }

    #[test]
    fn project_model_lifecycle() {
        let conn = setup();
        apply_event(
            &conn,
            &make_event(
                "e1",
                Payload::ModelPromoted(ModelPromoted {
                    model_id: "router".into(),
                    version: 1,
                    model_bundle_ref: Some(HashRef {
                        sha256: "model-h".into(),
                    }),
                }),
            ),
        )
        .unwrap();

        apply_event(
            &conn,
            &make_event(
                "e2",
                Payload::ModelRolledBack(ModelRolledBack {
                    model_id: "router".into(),
                    from_version: 1,
                    to_version: 0,
                    reason: "regression".into(),
                }),
            ),
        )
        .unwrap();

        let rolled_back: i32 = conn
            .query_row(
                "SELECT rolled_back FROM models_view WHERE model_id = 'router' AND version = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(rolled_back, 1);
    }

    #[test]
    fn audit_trail() {
        let conn = setup();
        for i in 0..5 {
            apply_event(
                &conn,
                &make_event(
                    &format!("e{i}"),
                    Payload::CaseCreated(CaseCreated {
                        case_id: format!("c{i}"),
                        title: format!("Case {i}"),
                        summary: "s".into(),
                        content_ref: None,
                        shareable: false,
                    }),
                ),
            )
            .unwrap();
        }

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM audit_view", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 5);
    }

    #[test]
    fn checkpoint_tracking() {
        let conn = setup();
        assert!(get_checkpoint(&conn).unwrap().is_none());

        apply_event(
            &conn,
            &make_event(
                "e1",
                Payload::CaseCreated(CaseCreated {
                    case_id: "c1".into(),
                    title: "T".into(),
                    summary: "S".into(),
                    content_ref: None,
                    shareable: false,
                }),
            ),
        )
        .unwrap();

        let (hash, count) = get_checkpoint(&conn).unwrap().unwrap();
        assert_eq!(hash, "hash-e1");
        assert_eq!(count, 1);
    }

    #[test]
    fn project_entity_card() {
        let conn = setup();
        apply_event(
            &conn,
            &make_event(
                "e1",
                Payload::ArtifactPublished(ArtifactPublished {
                    artifact_id: "ing-1-invoices-inv001".into(),
                    artifact_type: ArtifactType::Document as i32,
                    version: 1,
                    title: "Invoice inv001".into(),
                    summary: "Invoice for customer".into(),
                    content_ref: Some(HashRef {
                        sha256: "ch-inv".into(),
                    }),
                    shareable: false,
                    expires_unix_ms: 0,
                    document_subtype: "entity_card".into(),
                    entity_type: "invoice".into(),
                    entity_key: "inv001".into(),
                    source_ref: "src-1".into(),
                    table_name: "invoices".into(),
                    entity_attributes_json: r#"{"amount":1500,"customer_id":"cust-1"}"#.into(),
                    ..Default::default()
                }),
            ),
        )
        .unwrap();

        let (entity_id, entity_type, attrs): (String, String, String) = conn
            .query_row(
                "SELECT entity_id, entity_type, attributes_json FROM entity_cards_view WHERE entity_id = 'invoice:inv001'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(entity_id, "invoice:inv001");
        assert_eq!(entity_type, "invoice");
        assert!(attrs.contains("1500"));
        assert!(attrs.contains("cust-1"));
    }

    #[test]
    fn project_entity_relationship() {
        let conn = setup();
        apply_event(
            &conn,
            &make_event(
                "e1",
                Payload::EntityRelationshipRecorded(EntityRelationshipRecorded {
                    from_entity_id: "invoice:inv001".into(),
                    to_entity_id: "customer:cust-1".into(),
                    relationship_type: "belongs_to_customer".into(),
                    source_id: "src-1".into(),
                    table_name: "invoices".into(),
                }),
            ),
        )
        .unwrap();

        let (from_id, to_id, rel_type): (String, String, String) = conn
            .query_row(
                "SELECT from_entity_id, to_entity_id, relationship_type FROM entity_relationships_view WHERE from_entity_id = 'invoice:inv001'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(from_id, "invoice:inv001");
        assert_eq!(to_id, "customer:cust-1");
        assert_eq!(rel_type, "belongs_to_customer");
    }

    #[test]
    fn project_extracted_relationship_recorded() {
        let conn = setup();
        apply_event(
            &conn,
            &make_event(
                "e1",
                Payload::ExtractedRelationshipRecorded(ExtractedRelationshipRecorded {
                    relationship_id: "rel-abc123".into(),
                    from_entity_id: "person:gavin-anthony".into(),
                    from_entity_value: "Gavin Anthony".into(),
                    relationship_type: "works_for".into(),
                    to_entity_id: "company:complete-cabling-systems-ltd".into(),
                    to_entity_value: "Complete Cabling Systems Ltd".into(),
                    source_document_id: "doc1.pdf".into(),
                    chunk_index: 0,
                    confidence: 0.85,
                    extraction_method: "rule_based".into(),
                }),
            ),
        )
        .unwrap();

        let (rel_id, from_id, from_val, rel_type, to_id, to_val, doc_id, method): (
            String,
            String,
            String,
            String,
            String,
            String,
            String,
            String,
        ) = conn
            .query_row(
                "SELECT relationship_id, from_entity_id, from_entity_value, relationship_type,
                         to_entity_id, to_entity_value, source_document_id, extraction_method
                 FROM extracted_entity_relationships_view WHERE relationship_id = 'rel-abc123'",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(rel_id, "rel-abc123");
        assert_eq!(from_id, "person:gavin-anthony");
        assert_eq!(from_val, "Gavin Anthony");
        assert_eq!(rel_type, "works_for");
        assert_eq!(to_id, "company:complete-cabling-systems-ltd");
        assert_eq!(to_val, "Complete Cabling Systems Ltd");
        assert_eq!(doc_id, "doc1.pdf");
        assert_eq!(method, "rule_based");
    }

    #[test]
    fn project_entity_graph_rebuild() {
        let conn = setup();
        let events = [
            make_event(
                "e1",
                Payload::ArtifactPublished(ArtifactPublished {
                    artifact_id: "art-cust-1".into(),
                    artifact_type: ArtifactType::Document as i32,
                    version: 1,
                    title: "Customer ABC".into(),
                    summary: "ABC Ltd".into(),
                    content_ref: Some(HashRef {
                        sha256: "ch-cust".into(),
                    }),
                    shareable: false,
                    expires_unix_ms: 0,
                    document_subtype: "entity_card".into(),
                    entity_type: "customer".into(),
                    entity_key: "cust-1".into(),
                    source_ref: "src-1".into(),
                    table_name: "customers".into(),
                    entity_attributes_json: r#"{"revenue_total":140000,"jobs_completed":9}"#.into(),
                    ..Default::default()
                }),
            ),
            make_event(
                "e2",
                Payload::EntityRelationshipRecorded(EntityRelationshipRecorded {
                    from_entity_id: "quote:q-001".into(),
                    to_entity_id: "customer:cust-1".into(),
                    relationship_type: "belongs_to_customer".into(),
                    source_id: "src-1".into(),
                    table_name: "quotes".into(),
                }),
            ),
        ];
        replay_events(&conn, &events).unwrap();

        let card_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM entity_cards_view", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(card_count, 1);

        let rel_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM entity_relationships_view",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(rel_count, 1);

        let customer_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM customers_view", [], |row| row.get(0))
            .unwrap();
        assert_eq!(customer_count, 1);

        let attrs: String = conn
            .query_row(
                "SELECT attributes_json FROM entity_cards_view WHERE entity_type = 'customer'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(attrs.contains("140000"));
        assert!(attrs.contains("9"));
    }

    #[test]
    fn replay_events_bulk() {
        let conn = setup();
        let events: Vec<EventEnvelope> = (0..20)
            .map(|i| {
                make_event(
                    &format!("e{i}"),
                    Payload::CaseCreated(CaseCreated {
                        case_id: format!("c{i}"),
                        title: format!("Case {i}"),
                        summary: format!("Summary {i}"),
                        content_ref: None,
                        shareable: i % 2 == 0,
                    }),
                )
            })
            .collect();

        replay_events(&conn, &events).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM cases_view", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 20);

        let (_, evt_count) = get_checkpoint(&conn).unwrap().unwrap();
        assert_eq!(evt_count, 20);
    }

    #[test]
    fn project_outcomes() {
        let conn = setup();
        apply_event(
            &conn,
            &make_event(
                "o1",
                Payload::CaseFailed(CaseFailed {
                    case_id: "case-1".into(),
                    reason: "Customer cancelled".into(),
                }),
            ),
        )
        .unwrap();
        apply_event(
            &conn,
            &make_event(
                "o2",
                Payload::QuoteAccepted(QuoteAccepted {
                    quote_id: "q-1".into(),
                    case_id: "case-1".into(),
                    value_summary: "Accepted at list price".into(),
                    confidence: 0.95,
                }),
            ),
        )
        .unwrap();

        let failed: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM outcomes_view WHERE outcome_type = 'case_failed'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(failed, 1);

        let accepted: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM outcomes_view WHERE outcome_type = 'quote_accepted'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(accepted, 1);
    }

    #[test]
    fn project_shard_assignment() {
        let conn = setup();
        // CaseCreated -> tenant:public shard
        apply_event(
            &conn,
            &make_event(
                "e1",
                Payload::CaseCreated(CaseCreated {
                    case_id: "case-1".into(),
                    title: "Test".into(),
                    summary: "Summary".into(),
                    content_ref: None,
                    shareable: true,
                }),
            ),
        )
        .unwrap();

        let case_members: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM shard_membership_view WHERE member_type = 'case' AND member_id = 'case-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(case_members >= 1, "case should have shard membership");
        let has_public: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM shard_membership_view WHERE shard_key = 'public' AND member_id = 'case-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(has_public, 1);

        // ArtifactPublished with entity_type -> entity_type:customer, artifact_class:document
        apply_event(
            &conn,
            &make_event(
                "e2",
                Payload::ArtifactPublished(ArtifactPublished {
                    artifact_id: "art-1".into(),
                    artifact_type: ArtifactType::Document as i32,
                    version: 1,
                    title: "Doc".into(),
                    summary: "".into(),
                    content_ref: None,
                    shareable: false,
                    expires_unix_ms: 0,
                    document_subtype: "entity_card".into(),
                    entity_type: "customer".into(),
                    entity_key: "c1".into(),
                    source_ref: "src-1".into(),
                    table_name: "customers".into(),
                    ..Default::default()
                }),
            ),
        )
        .unwrap();

        let art_members: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM shard_membership_view WHERE member_type = 'artifact' AND member_id = 'art-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            art_members >= 2,
            "artifact should have entity_type and artifact_class shards"
        );
    }

    #[test]
    fn project_mergeable_state() {
        let conn = setup();

        apply_event(
            &conn,
            &make_event(
                "m1",
                Payload::MergeableTagUpdated(MergeableTagUpdated {
                    object_type: "case".into(),
                    object_id: "case-1".into(),
                    tag: "urgent".into(),
                    op: "add".into(),
                }),
            ),
        )
        .unwrap();
        apply_event(
            &conn,
            &make_event(
                "m2",
                Payload::MergeableTagUpdated(MergeableTagUpdated {
                    object_type: "case".into(),
                    object_id: "case-1".into(),
                    tag: "reviewed".into(),
                    op: "add".into(),
                }),
            ),
        )
        .unwrap();

        let tag_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM mergeable_tag_events WHERE object_id = 'case-1' AND op = 'add'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tag_count, 2);

        apply_event(
            &conn,
            &make_event(
                "m3",
                Payload::MergeableCounterUpdated(MergeableCounterUpdated {
                    object_type: "artifact".into(),
                    object_id: "art-1".into(),
                    counter_key: "view_count".into(),
                    delta: 5,
                }),
            ),
        )
        .unwrap();
        apply_event(
            &conn,
            &make_event(
                "m4",
                Payload::MergeableCounterUpdated(MergeableCounterUpdated {
                    object_type: "artifact".into(),
                    object_id: "art-1".into(),
                    counter_key: "view_count".into(),
                    delta: 3,
                }),
            ),
        )
        .unwrap();

        let total: i64 = conn
            .query_row(
                "SELECT COALESCE(SUM(delta), 0) FROM mergeable_counter_deltas
                 WHERE object_type = 'artifact' AND object_id = 'art-1' AND counter_key = 'view_count'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(total, 8);

        apply_event(
            &conn,
            &make_event(
                "m5",
                Payload::MergeableAnnotationUpdated(MergeableAnnotationUpdated {
                    object_type: "entity".into(),
                    object_id: "customer:c1".into(),
                    annotation_key: "notes".into(),
                    value: "VIP customer".into(),
                }),
            ),
        )
        .unwrap();

        let value: String = conn
            .query_row(
                "SELECT value FROM mergeable_annotations_view
                 WHERE object_type = 'entity' AND object_id = 'customer:c1' AND annotation_key = 'notes'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(value, "VIP customer");
    }

    #[test]
    fn project_shard_subscription() {
        let conn = setup();
        apply_event(
            &conn,
            &make_event(
                "e1",
                Payload::ShardSubscriptionAdded(ShardSubscriptionAdded {
                    shard_key: "entity_type:customer".into(),
                    node_id: Some(NodeId {
                        value: "node-abc".into(),
                    }),
                    capability: "host".into(),
                    last_seen_ms: 1700000000000,
                }),
            ),
        )
        .unwrap();

        let (node_id, cap): (String, String) = conn
            .query_row(
                "SELECT node_id, capability FROM shard_subscriptions_view WHERE shard_key = 'entity_type:customer'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(node_id, "node-abc");
        assert_eq!(cap, "host");
    }

    #[test]
    fn project_document_chunks_rebuild_from_events() {
        let conn = setup();
        let events: Vec<EventEnvelope> = vec![
            make_event(
                "e1",
                Payload::ArtifactPublished(ArtifactPublished {
                    artifact_id: "doc-abc::chunk::0".into(),
                    artifact_type: ArtifactType::Document as i32,
                    version: 1,
                    title: "Report chunk 0".into(),
                    summary: "First part of report".into(),
                    content_ref: Some(HashRef {
                        sha256: "ch0".into(),
                    }),
                    shareable: false,
                    expires_unix_ms: 0,
                    document_subtype: "document_chunk".into(),
                    entity_type: "document".into(),
                    entity_key: "doc-abc".into(),
                    entity_attributes_json: serde_json::json!({
                        "document_id": "doc-abc",
                        "chunk_index": 0,
                        "chunk_text": "Introduction to the quarterly report. Revenue grew.",
                        "source_file": "q3.pdf",
                        "page_number": 1
                    })
                    .to_string(),
                    ..Default::default()
                }),
            ),
            make_event(
                "e2",
                Payload::ArtifactPublished(ArtifactPublished {
                    artifact_id: "doc-abc::chunk::1".into(),
                    artifact_type: ArtifactType::Document as i32,
                    version: 1,
                    title: "Report chunk 1".into(),
                    summary: "Middle section".into(),
                    content_ref: Some(HashRef {
                        sha256: "ch1".into(),
                    }),
                    shareable: false,
                    expires_unix_ms: 0,
                    document_subtype: "document_chunk".into(),
                    entity_type: "document".into(),
                    entity_key: "doc-abc".into(),
                    entity_attributes_json: serde_json::json!({
                        "document_id": "doc-abc",
                        "chunk_index": 1,
                        "chunk_text": "Key metrics and operational highlights. Costs decreased.",
                        "source_file": "q3.pdf",
                        "page_number": 2
                    })
                    .to_string(),
                    ..Default::default()
                }),
            ),
        ];
        replay_events(&conn, &events).unwrap();

        let doc_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM documents_view WHERE document_type = 'document_chunk'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(doc_count, 2);

        let fts_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM documents_fts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(fts_count, 2);

        let hits =
            crate::search::search_documents_fts(&conn, "operational highlights", 10).unwrap();
        assert!(!hits.is_empty());
        assert!(hits
            .iter()
            .any(|h| h.document_id == "doc-abc" && h.chunk_index == "1"));
    }

    #[test]
    fn project_document_chunk_populates_documents_fts() {
        let conn = setup();
        let attrs = serde_json::json!({
            "document_id": "doc-123",
            "chunk_index": 1,
            "chunk_text": "The critical phrase mid-document appears only in chunk 1.",
            "source_file": "report.pdf",
            "page_number": 2
        });
        apply_event(
            &conn,
            &make_event(
                "e1",
                Payload::ArtifactPublished(ArtifactPublished {
                    artifact_id: "doc-123::chunk::1".into(),
                    artifact_type: ArtifactType::Document as i32,
                    version: 1,
                    title: "Report (chunk 1)".into(),
                    summary: "Chunk 1 of report".into(),
                    content_ref: Some(HashRef {
                        sha256: "ch-h1".into(),
                    }),
                    shareable: false,
                    expires_unix_ms: 0,
                    document_subtype: "document_chunk".into(),
                    entity_attributes_json: attrs.to_string(),
                    ..Default::default()
                }),
            ),
        )
        .unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM documents_fts", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);

        let (doc_id, chunk_idx, chunk_text): (String, String, String) = conn
            .query_row(
                "SELECT document_id, chunk_index, chunk_text FROM documents_fts WHERE artifact_id = 'doc-123::chunk::1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(doc_id, "doc-123");
        assert_eq!(chunk_idx, "1");
        assert!(chunk_text.contains("critical phrase mid-document"));
    }

    #[test]
    fn project_shard_rebuild() {
        let conn = setup();
        let events = [
            make_event(
                "e1",
                Payload::CaseCreated(CaseCreated {
                    case_id: "c1".into(),
                    title: "T".into(),
                    summary: "S".into(),
                    content_ref: None,
                    shareable: false,
                }),
            ),
            make_event(
                "e2",
                Payload::ShardSubscriptionAdded(ShardSubscriptionAdded {
                    shard_key: "tenant:public".into(),
                    node_id: Some(NodeId {
                        value: "node-x".into(),
                    }),
                    capability: "query".into(),
                    last_seen_ms: 1700000001000,
                }),
            ),
        ];
        replay_events(&conn, &events).unwrap();

        let shard_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM shards_view", [], |row| row.get(0))
            .unwrap();
        assert!(shard_count >= 1);

        let sub_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM shard_subscriptions_view", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(sub_count, 1);
    }
}
