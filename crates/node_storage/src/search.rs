//! Full-text search over cases and artifacts using FTS5.

use rusqlite::{params, Connection};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SearchError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

pub type Result<T> = std::result::Result<T, SearchError>;

#[derive(Debug, Clone)]
pub struct CaseHit {
    pub case_id: String,
    pub title: String,
    pub summary: String,
    pub rank: f64,
}

#[derive(Debug, Clone)]
pub struct ArtifactHit {
    pub artifact_id: String,
    pub title: String,
    pub rank: f64,
}

/// Search cases by FTS5 query.
pub fn search_cases(conn: &Connection, query: &str, limit: usize) -> Result<Vec<CaseHit>> {
    let mut stmt = conn.prepare(
        "SELECT case_id, title, summary, rank
         FROM cases_fts
         WHERE cases_fts MATCH ?1
         ORDER BY rank
         LIMIT ?2",
    )?;

    let hits = stmt
        .query_map(params![query, limit as i64], |row| {
            Ok(CaseHit {
                case_id: row.get(0)?,
                title: row.get(1)?,
                summary: row.get(2)?,
                rank: row.get(3)?,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();

    Ok(hits)
}

/// Search artifacts by FTS5 query.
pub fn search_artifacts(
    conn: &Connection,
    query: &str,
    limit: usize,
) -> Result<Vec<ArtifactHit>> {
    let mut stmt = conn.prepare(
        "SELECT artifact_id, title, rank
         FROM artifacts_fts
         WHERE artifacts_fts MATCH ?1
         ORDER BY rank
         LIMIT ?2",
    )?;

    let hits = stmt
        .query_map(params![query, limit as i64], |row| {
            Ok(ArtifactHit {
                artifact_id: row.get(0)?,
                title: row.get(1)?,
                rank: row.get(2)?,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();

    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projector;
    use crate::sqlite_views;
    use node_proto::common::*;
    use node_proto::events::*;

    fn setup_with_data() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        sqlite_views::create_schema(&conn).unwrap();

        let cases = vec![
            ("c1", "DNS resolution failure", "The DNS resolver times out when querying api.example.com"),
            ("c2", "Memory leak in Java service", "The Java heap grows unbounded after 24h"),
            ("c3", "SSL certificate expired", "TLS handshake fails due to expired certificate on gateway"),
            ("c4", "Kubernetes pod crash loop", "Pod in CrashBackOff state due to OOM kill"),
            ("c5", "Database connection pool exhaustion", "All connections used, new requests timeout"),
        ];

        for (id, title, summary) in cases {
            let event = EventEnvelope {
                event_id: format!("e-{id}"),
                r#type: EventType::CaseCreated as i32,
                ts: Some(Timestamp { unix_ms: 1700000000000 }),
                node_id: Some(NodeId { value: "node-1".into() }),
                tenant_id: Some(TenantId { value: "public".into() }),
                sensitivity: Sensitivity::Public as i32,
                event_hash: Some(HashRef { sha256: format!("h-{id}") }),
                payload: Some(event_envelope::Payload::CaseCreated(CaseCreated {
                    case_id: id.to_string(),
                    title: title.to_string(),
                    summary: summary.to_string(),
                    content_ref: None,
                    shareable: false,
                })),
                tags: vec!["ops".into()],
                ..Default::default()
            };
            projector::apply_event(&conn, &event).unwrap();
        }

        let artifacts = vec![
            ("a1", "K8s rollback playbook", ArtifactType::Runbook),
            ("a2", "SSL renewal template", ArtifactType::Template),
            ("a3", "Database failover recipe", ArtifactType::Recipe),
        ];

        for (id, title, atype) in artifacts {
            let event = EventEnvelope {
                event_id: format!("e-{id}"),
                r#type: EventType::ArtifactPublished as i32,
                ts: Some(Timestamp { unix_ms: 1700000000000 }),
                node_id: Some(NodeId { value: "node-1".into() }),
                tenant_id: Some(TenantId { value: "public".into() }),
                sensitivity: Sensitivity::Public as i32,
                event_hash: Some(HashRef { sha256: format!("h-{id}") }),
                payload: Some(event_envelope::Payload::ArtifactPublished(
                    ArtifactPublished {
                        artifact_id: id.to_string(),
                        artifact_type: atype as i32,
                        version: 1,
                        title: title.to_string(),
                        content_ref: Some(HashRef { sha256: format!("content-{id}") }),
                        shareable: true,
                        expires_unix_ms: 0,
                    },
                )),
                ..Default::default()
            };
            projector::apply_event(&conn, &event).unwrap();
        }

        conn
    }

    #[test]
    fn search_cases_by_dns() {
        let conn = setup_with_data();
        let hits = search_cases(&conn, "DNS", 10).unwrap();
        assert!(!hits.is_empty());
        assert!(hits.iter().any(|h| h.case_id == "c1"));
    }

    #[test]
    fn search_cases_by_memory() {
        let conn = setup_with_data();
        let hits = search_cases(&conn, "memory leak Java", 10).unwrap();
        assert!(!hits.is_empty());
        assert!(hits.iter().any(|h| h.case_id == "c2"));
    }

    #[test]
    fn search_cases_by_certificate() {
        let conn = setup_with_data();
        let hits = search_cases(&conn, "certificate expired", 10).unwrap();
        assert!(!hits.is_empty());
        assert!(hits.iter().any(|h| h.case_id == "c3"));
    }

    #[test]
    fn search_cases_limit() {
        let conn = setup_with_data();
        let hits = search_cases(&conn, "the", 2).unwrap();
        assert!(hits.len() <= 2);
    }

    #[test]
    fn search_cases_no_results() {
        let conn = setup_with_data();
        let hits = search_cases(&conn, "xyznonexistent", 10).unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn search_artifacts_by_k8s() {
        let conn = setup_with_data();
        let hits = search_artifacts(&conn, "K8s rollback", 10).unwrap();
        assert!(!hits.is_empty());
        assert!(hits.iter().any(|h| h.artifact_id == "a1"));
    }

    #[test]
    fn search_artifacts_by_ssl() {
        let conn = setup_with_data();
        let hits = search_artifacts(&conn, "SSL renewal", 10).unwrap();
        assert!(!hits.is_empty());
        assert!(hits.iter().any(|h| h.artifact_id == "a2"));
    }

    #[test]
    fn search_artifacts_by_database() {
        let conn = setup_with_data();
        let hits = search_artifacts(&conn, "database failover", 10).unwrap();
        assert!(!hits.is_empty());
        assert!(hits.iter().any(|h| h.artifact_id == "a3"));
    }
}
