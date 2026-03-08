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
    pub summary: String,
    pub content_hash: Option<String>,
    pub rank: f64,
}

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub hit_type: String,
    pub id: String,
    pub title: String,
    pub summary: String,
    pub content_hash: Option<String>,
    pub rank: f64,
}

/// Document chunk hit from documents_fts (full-text search over chunk_text).
#[derive(Debug, Clone)]
pub struct DocumentChunkHit {
    pub artifact_id: String,
    pub document_id: String,
    pub chunk_index: String,
    pub chunk_text: String,
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

/// Search document chunks by FTS5 query over chunk_text.
/// Returns document fragments that match the query; results map back to document_id.
pub fn search_documents_fts(
    conn: &Connection,
    query: &str,
    limit: usize,
) -> Result<Vec<DocumentChunkHit>> {
    let mut stmt = conn.prepare(
        "SELECT artifact_id, document_id, chunk_index, chunk_text, rank
         FROM documents_fts
         WHERE documents_fts MATCH ?1
         ORDER BY rank
         LIMIT ?2",
    )?;

    let hits = stmt
        .query_map(params![query, limit as i64], |row| {
            Ok(DocumentChunkHit {
                artifact_id: row.get(0)?,
                document_id: row.get(1)?,
                chunk_index: row.get(2)?,
                chunk_text: row.get(3)?,
                rank: row.get(4)?,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();

    Ok(hits)
}

/// Search artifacts by FTS5 query.
/// Returns content_hash so callers can fetch full content from CAS.
pub fn search_artifacts(conn: &Connection, query: &str, limit: usize) -> Result<Vec<ArtifactHit>> {
    let mut stmt = conn.prepare(
        "SELECT artifacts_fts.artifact_id, artifacts_fts.title, artifacts_fts.summary,
                (SELECT content_hash FROM artifacts_view av WHERE av.artifact_id = artifacts_fts.artifact_id ORDER BY av.version DESC LIMIT 1) AS content_hash,
                rank
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
                summary: row.get(2)?,
                content_hash: row.get(3).ok().flatten(),
                rank: row.get(4)?,
            })
        })?
        .filter_map(|r| r.ok())
        .collect();

    Ok(hits)
}

/// Entity card hit for business queries.
#[derive(Debug, Clone)]
pub struct EntityCardHit {
    pub entity_id: String,
    pub entity_type: String,
    pub attributes_json: String,
}

/// Query entity cards by entity_type (optional filter).
pub fn search_entity_cards(
    conn: &Connection,
    entity_type: Option<&str>,
    limit: usize,
) -> Result<Vec<EntityCardHit>> {
    let limit_i64 = limit as i64;
    if let Some(et) = entity_type {
        let mut out = Vec::new();
        let mut stmt = conn.prepare(
            "SELECT entity_id, entity_type, attributes_json FROM entity_cards_view
             WHERE entity_type = ?1 ORDER BY created_at_ms DESC LIMIT ?2",
        )?;
        let mut rows = stmt.query(rusqlite::params![et, limit_i64])?;
        while let Some(row) = rows.next()? {
            out.push(EntityCardHit {
                entity_id: row.get(0)?,
                entity_type: row.get(1)?,
                attributes_json: row.get(2)?,
            });
        }
        Ok(out)
    } else {
        let mut out = Vec::new();
        let mut stmt = conn.prepare(
            "SELECT entity_id, entity_type, attributes_json FROM entity_cards_view
             ORDER BY created_at_ms DESC LIMIT ?1",
        )?;
        let mut rows = stmt.query(rusqlite::params![limit_i64])?;
        while let Some(row) = rows.next()? {
            out.push(EntityCardHit {
                entity_id: row.get(0)?,
                entity_type: row.get(1)?,
                attributes_json: row.get(2)?,
            });
        }
        Ok(out)
    }
}

/// Fact record for business queries.
#[derive(Debug, Clone)]
pub struct FactHit {
    pub fact_id: String,
    pub metric: String,
    pub value_json: String,
    pub dimensions_json: String,
}

/// Query facts by metric (optional filter).
pub fn query_facts(
    conn: &Connection,
    metric_filter: Option<&str>,
    limit: usize,
) -> Result<Vec<FactHit>> {
    let limit_i64 = limit as i64;
    if let Some(m) = metric_filter {
        let mut out = Vec::new();
        let mut stmt = conn.prepare(
            "SELECT fact_id, metric, value_json, dimensions_json FROM facts_view
             WHERE metric = ?1 ORDER BY created_at_ms DESC LIMIT ?2",
        )?;
        let mut rows = stmt.query(rusqlite::params![m, limit_i64])?;
        while let Some(row) = rows.next()? {
            out.push(FactHit {
                fact_id: row.get(0)?,
                metric: row.get(1)?,
                value_json: row.get(2)?,
                dimensions_json: row.get(3)?,
            });
        }
        Ok(out)
    } else {
        let mut out = Vec::new();
        let mut stmt = conn.prepare(
            "SELECT fact_id, metric, value_json, dimensions_json FROM facts_view
             ORDER BY created_at_ms DESC LIMIT ?1",
        )?;
        let mut rows = stmt.query(rusqlite::params![limit_i64])?;
        while let Some(row) = rows.next()? {
            out.push(FactHit {
                fact_id: row.get(0)?,
                metric: row.get(1)?,
                value_json: row.get(2)?,
                dimensions_json: row.get(3)?,
            });
        }
        Ok(out)
    }
}

/// Extracted entity from documents (Phase B).
#[derive(Debug, Clone)]
pub struct EntityRecord {
    pub entity_id: String,
    pub entity_type: String,
    pub entity_value: String,
    pub normalized_value: String,
    pub document_id: String,
    pub chunk_index: i32,
    pub confidence: f32,
}

/// List entities by type.
pub fn list_entities_by_type(
    conn: &Connection,
    entity_type: &str,
    limit: usize,
) -> Result<Vec<EntityRecord>> {
    let mut stmt = conn.prepare(
        "SELECT entity_id, entity_type, entity_value, normalized_value, document_id, chunk_index, confidence
         FROM entities_view WHERE entity_type = ?1
         ORDER BY created_at_ms DESC LIMIT ?2",
    )?;
    let mut out = Vec::new();
    let mut rows = stmt.query(params![entity_type, limit as i64])?;
    while let Some(row) = rows.next()? {
        out.push(EntityRecord {
            entity_id: row.get(0)?,
            entity_type: row.get(1)?,
            entity_value: row.get(2)?,
            normalized_value: row.get(3)?,
            document_id: row.get(4)?,
            chunk_index: row.get(5)?,
            confidence: row.get(6)?,
        });
    }
    Ok(out)
}

/// Search entities by value (LIKE on entity_value or normalized_value).
pub fn search_entities_by_value(
    conn: &Connection,
    query: &str,
    entity_type: Option<&str>,
    limit: usize,
) -> Result<Vec<EntityRecord>> {
    let pattern = format!("%{query}%");
    let limit_i64 = limit as i64;
    let mut out = Vec::new();
    if let Some(et) = entity_type {
        let mut stmt = conn.prepare(
            "SELECT entity_id, entity_type, entity_value, normalized_value, document_id, chunk_index, confidence
             FROM entities_view
             WHERE entity_type = ?1 AND (entity_value LIKE ?2 OR normalized_value LIKE ?2)
             ORDER BY created_at_ms DESC LIMIT ?3",
        )?;
        let mut rows = stmt.query(params![et, pattern, limit_i64])?;
        while let Some(row) = rows.next()? {
            out.push(EntityRecord {
                entity_id: row.get(0)?,
                entity_type: row.get(1)?,
                entity_value: row.get(2)?,
                normalized_value: row.get(3)?,
                document_id: row.get(4)?,
                chunk_index: row.get(5)?,
                confidence: row.get(6)?,
            });
        }
    } else {
        let mut stmt = conn.prepare(
            "SELECT entity_id, entity_type, entity_value, normalized_value, document_id, chunk_index, confidence
             FROM entities_view
             WHERE entity_value LIKE ?1 OR normalized_value LIKE ?1
             ORDER BY created_at_ms DESC LIMIT ?2",
        )?;
        let mut rows = stmt.query(params![pattern, limit_i64])?;
        while let Some(row) = rows.next()? {
            out.push(EntityRecord {
                entity_id: row.get(0)?,
                entity_type: row.get(1)?,
                entity_value: row.get(2)?,
                normalized_value: row.get(3)?,
                document_id: row.get(4)?,
                chunk_index: row.get(5)?,
                confidence: row.get(6)?,
            });
        }
    }
    Ok(out)
}

/// List documents that mention an entity (by normalized_value).
pub fn list_documents_for_entity(
    conn: &Connection,
    normalized_value: &str,
    entity_type: Option<&str>,
    limit: usize,
) -> Result<Vec<(String, String, String)>> {
    let limit_i64 = limit as i64;
    let mut out = Vec::new();
    if let Some(et) = entity_type {
        let mut stmt = conn.prepare(
            "SELECT DISTINCT document_id, entity_type, entity_value
             FROM entities_view
             WHERE normalized_value = ?1 AND entity_type = ?2
             LIMIT ?3",
        )?;
        let mut rows = stmt.query(params![normalized_value, et, limit_i64])?;
        while let Some(row) = rows.next()? {
            out.push((row.get(0)?, row.get(1)?, row.get(2)?));
        }
    } else {
        let mut stmt = conn.prepare(
            "SELECT DISTINCT document_id, entity_type, entity_value
             FROM entities_view
             WHERE normalized_value = ?1
             LIMIT ?2",
        )?;
        let mut rows = stmt.query(params![normalized_value, limit_i64])?;
        while let Some(row) = rows.next()? {
            out.push((row.get(0)?, row.get(1)?, row.get(2)?));
        }
    }
    Ok(out)
}

/// Count entity mentions by type.
pub fn count_entity_mentions(conn: &Connection, entity_type: &str) -> Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM entities_view WHERE entity_type = ?1",
        params![entity_type],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

/// Extracted entity relationship record.
#[derive(Debug, Clone)]
pub struct EntityRelationshipRow {
    pub relationship_id: String,
    pub from_entity_id: String,
    pub from_entity_value: String,
    pub relationship_type: String,
    pub to_entity_id: String,
    pub to_entity_value: String,
    pub source_document_id: String,
    pub chunk_index: i32,
    pub confidence: f32,
    pub extraction_method: String,
}

/// List relationships where the given entity_id is either from or to.
pub fn list_relationships_for_entity(
    conn: &Connection,
    entity_id: &str,
    limit: usize,
) -> Result<Vec<EntityRelationshipRow>> {
    let limit_i64 = limit as i64;
    let mut stmt = conn.prepare(
        "SELECT relationship_id, from_entity_id, from_entity_value, relationship_type,
                to_entity_id, to_entity_value, source_document_id, chunk_index,
                confidence, extraction_method
         FROM extracted_entity_relationships_view
         WHERE from_entity_id = ?1 OR to_entity_id = ?1
         ORDER BY created_at_ms DESC
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(
        params![entity_id, entity_id, limit_i64],
        map_relationship_row,
    )?;
    rows.collect::<std::result::Result<Vec<_>, rusqlite::Error>>()
        .map_err(Into::into)
}

fn map_relationship_row(row: &rusqlite::Row) -> rusqlite::Result<EntityRelationshipRow> {
    Ok(EntityRelationshipRow {
        relationship_id: row.get(0)?,
        from_entity_id: row.get(1)?,
        from_entity_value: row.get(2)?,
        relationship_type: row.get(3)?,
        to_entity_id: row.get(4)?,
        to_entity_value: row.get(5)?,
        source_document_id: row.get(6)?,
        chunk_index: row.get(7)?,
        confidence: row.get(8)?,
        extraction_method: row.get(9)?,
    })
}

/// List related entities for a given entity value (exact match on from_entity_value or to_entity_value).
/// Optionally filter by relationship_type.
pub fn list_related_entities(
    conn: &Connection,
    entity_value: &str,
    relationship_type: Option<&str>,
    limit: usize,
) -> Result<Vec<EntityRelationshipRow>> {
    let limit_i64 = limit as i64;
    let rows = if let Some(rt) = relationship_type {
        let mut stmt = conn.prepare(
            "SELECT relationship_id, from_entity_id, from_entity_value, relationship_type,
                    to_entity_id, to_entity_value, source_document_id, chunk_index,
                    confidence, extraction_method
             FROM extracted_entity_relationships_view
             WHERE (from_entity_value = ?1 OR to_entity_value = ?1) AND relationship_type = ?2
             ORDER BY confidence DESC, created_at_ms DESC
             LIMIT ?3",
        )?;
        let iter = stmt.query_map(params![entity_value, rt, limit_i64], map_relationship_row)?;
        iter.collect::<std::result::Result<Vec<_>, rusqlite::Error>>()?
    } else {
        let mut stmt = conn.prepare(
            "SELECT relationship_id, from_entity_id, from_entity_value, relationship_type,
                    to_entity_id, to_entity_value, source_document_id, chunk_index,
                    confidence, extraction_method
             FROM extracted_entity_relationships_view
             WHERE from_entity_value = ?1 OR to_entity_value = ?1
             ORDER BY confidence DESC, created_at_ms DESC
             LIMIT ?2",
        )?;
        let iter = stmt.query_map(params![entity_value, limit_i64], map_relationship_row)?;
        iter.collect::<std::result::Result<Vec<_>, rusqlite::Error>>()?
    };
    Ok(rows)
}

/// List documents that contain relationships involving the given entity value (exact match).
pub fn list_documents_for_related_entities(
    conn: &Connection,
    entity_value: &str,
    limit: usize,
) -> Result<Vec<(String, String)>> {
    let limit_i64 = limit as i64;
    let mut stmt = conn.prepare(
        "SELECT DISTINCT source_document_id, relationship_type
         FROM extracted_entity_relationships_view
         WHERE from_entity_value = ?1 OR to_entity_value = ?1
         ORDER BY source_document_id
         LIMIT ?2",
    )?;
    let mut out = Vec::new();
    let mut rows = stmt.query(params![entity_value, limit_i64])?;
    while let Some(row) = rows.next()? {
        out.push((row.get(0)?, row.get(1)?));
    }
    Ok(out)
}

/// Count relationships by type.
pub fn count_relationships_by_type(conn: &Connection, relationship_type: &str) -> Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM extracted_entity_relationships_view WHERE relationship_type = ?1",
        params![relationship_type],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

/// Unified search across cases, artifacts, and document chunks (documents_fts), merged by rank.
/// Document chunk hits include chunk_text in summary so they can be used as context evidence.
pub fn search_all(conn: &Connection, query: &str, limit: usize) -> Result<Vec<SearchHit>> {
    let cases = search_cases(conn, query, limit)?;
    let artifacts = search_artifacts(conn, query, limit)?;
    let doc_chunks = search_documents_fts(conn, query, limit)?;

    let mut all: Vec<SearchHit> =
        Vec::with_capacity(cases.len() + artifacts.len() + doc_chunks.len());

    for c in cases {
        all.push(SearchHit {
            hit_type: "case".into(),
            id: c.case_id,
            title: c.title,
            summary: c.summary,
            content_hash: None,
            rank: c.rank,
        });
    }
    for a in artifacts {
        all.push(SearchHit {
            hit_type: "artifact".into(),
            id: a.artifact_id,
            title: a.title,
            summary: a.summary,
            content_hash: a.content_hash,
            rank: a.rank,
        });
    }
    for d in doc_chunks {
        all.push(SearchHit {
            hit_type: "document_chunk".into(),
            id: d.artifact_id,
            title: format!("{} (chunk {})", d.document_id, d.chunk_index),
            summary: d.chunk_text,
            content_hash: None,
            rank: d.rank,
        });
    }

    all.sort_by(|a, b| {
        a.rank
            .partial_cmp(&b.rank)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    all.truncate(limit);
    Ok(all)
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
            (
                "c1",
                "DNS resolution failure",
                "The DNS resolver times out when querying api.example.com",
            ),
            (
                "c2",
                "Memory leak in Java service",
                "The Java heap grows unbounded after 24h",
            ),
            (
                "c3",
                "SSL certificate expired",
                "TLS handshake fails due to expired certificate on gateway",
            ),
            (
                "c4",
                "Kubernetes pod crash loop",
                "Pod in CrashBackOff state due to OOM kill",
            ),
            (
                "c5",
                "Database connection pool exhaustion",
                "All connections used, new requests timeout",
            ),
        ];

        for (id, title, summary) in cases {
            let event = EventEnvelope {
                event_id: format!("e-{id}"),
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
                event_hash: Some(HashRef {
                    sha256: format!("h-{id}"),
                }),
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
            (
                "a1",
                "K8s rollback playbook",
                "Step-by-step guide to rolling back Kubernetes deployments safely",
                ArtifactType::Runbook,
            ),
            (
                "a2",
                "SSL renewal template",
                "Template for renewing SSL/TLS certificates using certbot and ACME",
                ArtifactType::Template,
            ),
            (
                "a3",
                "Database failover recipe",
                "Automated PostgreSQL failover with pgbouncer connection pooling",
                ArtifactType::Recipe,
            ),
        ];

        for (id, title, summary, atype) in artifacts {
            let event = EventEnvelope {
                event_id: format!("e-{id}"),
                r#type: EventType::ArtifactPublished as i32,
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
                    sha256: format!("h-{id}"),
                }),
                payload: Some(event_envelope::Payload::ArtifactPublished(
                    ArtifactPublished {
                        artifact_id: id.to_string(),
                        artifact_type: atype as i32,
                        version: 1,
                        title: title.to_string(),
                        summary: summary.to_string(),
                        content_ref: Some(HashRef {
                            sha256: format!("content-{id}"),
                        }),
                        shareable: true,
                        expires_unix_ms: 0,
                        ..Default::default()
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

    #[test]
    fn search_documents_fts_phrase_in_chunk() {
        let conn = Connection::open_in_memory().unwrap();
        sqlite_views::create_schema(&conn).unwrap();

        // Insert document chunks via projector (chunk 0, 1, 2 - phrase only in chunk 1)
        for (i, chunk_text) in [
            "First chunk with introductory content about the project.",
            "Second chunk containing the unique zebra stripes phrase in the middle.",
            "Third chunk with concluding remarks and summary.",
        ]
        .into_iter()
        .enumerate()
        {
            let attrs = serde_json::json!({
                "document_id": "doc-multi",
                "chunk_index": i,
                "chunk_text": chunk_text
            });
            let event = EventEnvelope {
                event_id: format!("e-chunk-{i}"),
                r#type: EventType::ArtifactPublished as i32,
                ts: Some(Timestamp {
                    unix_ms: 1700000000000 + i as i64,
                }),
                node_id: Some(NodeId {
                    value: "node-1".into(),
                }),
                tenant_id: Some(TenantId {
                    value: "public".into(),
                }),
                sensitivity: Sensitivity::Public as i32,
                event_hash: Some(HashRef {
                    sha256: format!("h-chunk-{i}"),
                }),
                payload: Some(event_envelope::Payload::ArtifactPublished(
                    ArtifactPublished {
                        artifact_id: format!("doc-multi::chunk::{i}"),
                        artifact_type: ArtifactType::Document as i32,
                        version: 1,
                        title: format!("Doc chunk {i}"),
                        summary: chunk_text.chars().take(100).collect::<String>(),
                        content_ref: Some(HashRef {
                            sha256: format!("ch-{i}"),
                        }),
                        shareable: false,
                        expires_unix_ms: 0,
                        document_subtype: "document_chunk".into(),
                        entity_attributes_json: attrs.to_string(),
                        ..Default::default()
                    },
                )),
                ..Default::default()
            };
            projector::apply_event(&conn, &event).unwrap();
        }

        // Search for phrase appearing only in middle chunk
        let hits = search_documents_fts(&conn, "zebra stripes", 10).unwrap();
        assert!(!hits.is_empty());
        assert!(hits
            .iter()
            .any(|h| h.chunk_index == "1" && h.document_id == "doc-multi"));
        assert!(hits.iter().any(|h| h.chunk_text.contains("zebra stripes")));
    }

    #[test]
    fn search_all_includes_document_chunks() {
        let conn = Connection::open_in_memory().unwrap();
        sqlite_views::create_schema(&conn).unwrap();
        // Insert a document chunk
        let attrs = serde_json::json!({
            "document_id": "doc-x",
            "chunk_index": 0,
            "chunk_text": "The annual budget allocation for infrastructure was approved."
        });
        let event = EventEnvelope {
            event_id: "e1".into(),
            r#type: EventType::ArtifactPublished as i32,
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
                sha256: "h1".into(),
            }),
            payload: Some(event_envelope::Payload::ArtifactPublished(
                ArtifactPublished {
                    artifact_id: "doc-x::chunk::0".into(),
                    artifact_type: ArtifactType::Document as i32,
                    version: 1,
                    title: "Doc chunk 0".into(),
                    summary: "Budget section".into(),
                    content_ref: Some(HashRef {
                        sha256: "ch0".into(),
                    }),
                    shareable: false,
                    expires_unix_ms: 0,
                    document_subtype: "document_chunk".into(),
                    entity_attributes_json: attrs.to_string(),
                    ..Default::default()
                },
            )),
            ..Default::default()
        };
        projector::apply_event(&conn, &event).unwrap();

        let all = search_all(&conn, "budget allocation", 10).unwrap();
        assert!(!all.is_empty());
        let chunk_hits: Vec<_> = all
            .iter()
            .filter(|h| h.hit_type == "document_chunk")
            .collect();
        assert!(!chunk_hits.is_empty());
        assert!(chunk_hits[0].summary.contains("budget allocation"));
    }

    #[test]
    fn search_documents_fts_no_results() {
        let conn = Connection::open_in_memory().unwrap();
        sqlite_views::create_schema(&conn).unwrap();
        let hits = search_documents_fts(&conn, "nonexistentxyzzz", 10).unwrap();
        assert!(hits.is_empty());
    }

    #[test]
    fn search_entity_cards_and_query_facts() {
        let conn = Connection::open_in_memory().unwrap();
        sqlite_views::create_schema(&conn).unwrap();

        conn.execute(
            "INSERT INTO entity_cards_view (entity_id, entity_type, attributes_json, created_at_ms)
             VALUES ('customer:1', 'customer', '{\"name\":\"Acme\"}', 1000),
                    ('invoice:1', 'invoice', '{\"total\":100}', 2000)",
            [],
        )
        .unwrap();

        conn.execute(
            "INSERT INTO facts_view (fact_id, version, metric, value_json, dimensions_json, created_at_ms)
             VALUES ('f1', 1, 'revenue', '{\"amount\":5000}', '{}', 3000)",
            [],
        )
        .unwrap();

        let customers = search_entity_cards(&conn, Some("customer"), 10).unwrap();
        assert_eq!(customers.len(), 1);
        assert_eq!(customers[0].entity_id, "customer:1");
        assert!(customers[0].attributes_json.contains("Acme"));

        let all = search_entity_cards(&conn, None, 10).unwrap();
        assert_eq!(all.len(), 2);

        let facts = query_facts(&conn, Some("revenue"), 10).unwrap();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].metric, "revenue");
    }
}
