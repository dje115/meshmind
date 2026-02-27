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
            }
            Payload::CaseConfirmed(cf) => {
                conn.execute(
                    "UPDATE cases_view SET outcome = ?1, confidence = ?2, updated_at_ms = ?3
                     WHERE case_id = ?4",
                    params![cf.outcome, cf.confidence, ts, cf.case_id],
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
                     (artifact_id, version, artifact_type, title, content_hash,
                      shareable, tenant_id, sensitivity, node_id, expires_at_ms, created_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    params![
                        ap.artifact_id,
                        ap.version,
                        ap.artifact_type,
                        ap.title,
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
            }
            Payload::ArtifactDeprecated(ad) => {
                conn.execute(
                    "UPDATE artifacts_view SET deprecated = 1, deprecate_reason = ?1
                     WHERE artifact_id = ?2 AND version = ?3",
                    params![ad.reason, ad.artifact_id, ad.version],
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
        None => "unknown event".to_string(),
    }
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
        "INSERT INTO artifacts_fts (artifact_id, title)
         SELECT artifact_id, title FROM artifacts_view
         WHERE artifact_id = ?1
         ORDER BY version DESC
         LIMIT 1",
        params![artifact_id],
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
                    content_ref: Some(HashRef {
                        sha256: "rb-hash".into(),
                    }),
                    shareable: true,
                    expires_unix_ms: 0,
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
}
