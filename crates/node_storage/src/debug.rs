//! Debug queries for inspecting documents, chunks, entities, and ask sessions.

use rusqlite::Connection;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct DebugDocumentSummary {
    pub document_id: String,
    pub filename: String,
    pub ocr_used: bool,
    pub chunk_count: u64,
    pub entity_count: u64,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DebugChunkInfo {
    pub artifact_id: String,
    pub document_id: String,
    pub chunk_index: i64,
    pub chunk_text_preview: String,
    pub source_file: String,
    pub page_number: i64,
    pub ocr_used: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DebugEntityInfo {
    pub entity_id: String,
    pub entity_type: String,
    pub entity_value: String,
    pub normalized_value: String,
    pub extraction_method: String,
    pub confidence: f64,
    pub source_document_id: String,
    pub chunk_index: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DebugAskSession {
    pub case_id: String,
    pub question: String,
    pub plan_json: String,
    pub evidence_json: String,
    pub confidence: f64,
    pub source_types: String,
    pub web_fallback_used: bool,
    pub peer_consult_used: bool,
    pub created_at_ms: i64,
}

const CHUNK_PREVIEW_LEN: usize = 200;

/// List ingested documents with OCR status, chunk count, entity count.
pub fn list_debug_documents(
    conn: &Connection,
) -> Result<Vec<DebugDocumentSummary>, rusqlite::Error> {
    // Use document_chunks_view if it exists and has rows; else fall back to documents_fts
    let chunks_table_exists: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='document_chunks_view'",
            [],
            |_| Ok(()),
        )
        .is_ok();
    let use_chunks_view = chunks_table_exists
        && conn
            .query_row("SELECT 1 FROM document_chunks_view LIMIT 1", [], |_| Ok(()))
            .is_ok();

    let sql = if use_chunks_view {
        "SELECT dc.document_id,
                COALESCE(MAX(dc.source_file), dc.document_id),
                MAX(dc.ocr_used),
                COUNT(*),
                (SELECT COUNT(*) FROM entities_view e WHERE e.document_id = dc.document_id),
                MIN(dc.created_at_ms)
         FROM document_chunks_view dc
         GROUP BY dc.document_id
         ORDER BY MIN(dc.created_at_ms) DESC"
    } else {
        "SELECT df.document_id,
                df.document_id,
                0,
                COUNT(*),
                (SELECT COUNT(*) FROM entities_view e WHERE e.document_id = df.document_id),
                0
         FROM documents_fts df
         GROUP BY df.document_id
         ORDER BY df.document_id"
    };

    let mut rows = conn.prepare(sql)?;

    let iter = rows.query_map([], |row| {
        Ok(DebugDocumentSummary {
            document_id: row.get(0)?,
            filename: row
                .get::<_, String>(1)?
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or("")
                .to_string(),
            ocr_used: row.get::<_, i64>(2)? != 0,
            chunk_count: row.get::<_, i64>(3)? as u64,
            entity_count: row.get::<_, i64>(4)? as u64,
            created_at_ms: row.get(5)?,
        })
    })?;

    iter.collect()
}

/// Result of get_debug_document.
pub type DebugDocumentDetail = (
    Option<DebugDocumentSummary>,
    Vec<DebugChunkInfo>,
    Vec<DebugEntityInfo>,
);

/// Get detailed document info including chunks and entities.
pub fn get_debug_document(
    conn: &Connection,
    document_id: &str,
) -> Result<DebugDocumentDetail, rusqlite::Error> {
    let summary = get_document_summary(conn, document_id)?;
    let chunks = list_debug_chunks(conn, document_id)?;
    let entities = list_debug_entities_for_document(conn, document_id)?;
    Ok((summary, chunks, entities))
}

fn get_document_summary(
    conn: &Connection,
    document_id: &str,
) -> Result<Option<DebugDocumentSummary>, rusqlite::Error> {
    let has_chunks: bool = conn
        .query_row(
            "SELECT 1 FROM document_chunks_view WHERE document_id = ?1 LIMIT 1",
            [document_id],
            |_| Ok(()),
        )
        .is_ok();

    if has_chunks {
        let mut stmt = conn.prepare(
            "SELECT document_id,
                    COALESCE(MAX(source_file), document_id),
                    MAX(ocr_used),
                    COUNT(*),
                    (SELECT COUNT(*) FROM entities_view e WHERE e.document_id = dc.document_id),
                    MIN(created_at_ms)
             FROM document_chunks_view dc
             WHERE document_id = ?1
             GROUP BY document_id",
        )?;
        let mut rows = stmt.query([document_id])?;
        if let Ok(Some(r)) = rows.next() {
            return Ok(Some(DebugDocumentSummary {
                document_id: r.get(0)?,
                filename: r
                    .get::<_, String>(1)?
                    .rsplit(['/', '\\'])
                    .next()
                    .unwrap_or("")
                    .to_string(),
                ocr_used: r.get::<_, i64>(2)? != 0,
                chunk_count: r.get::<_, i64>(3)? as u64,
                entity_count: r.get::<_, i64>(4)? as u64,
                created_at_ms: r.get(5)?,
            }));
        }
    }

    // Fallback: from documents_fts
    let chunk_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM documents_fts WHERE document_id = ?1",
        [document_id],
        |r| r.get(0),
    )?;
    let entity_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM entities_view WHERE document_id = ?1",
        [document_id],
        |r| r.get(0),
    )?;

    Ok(Some(DebugDocumentSummary {
        document_id: document_id.to_string(),
        filename: document_id.to_string(),
        ocr_used: false,
        chunk_count: chunk_count as u64,
        entity_count: entity_count as u64,
        created_at_ms: 0,
    }))
}

/// List chunks for a document with previews.
pub fn list_debug_chunks(
    conn: &Connection,
    document_id: &str,
) -> Result<Vec<DebugChunkInfo>, rusqlite::Error> {
    let has_chunks: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='document_chunks_view'",
            [],
            |_| Ok(()),
        )
        .is_ok();

    if has_chunks {
        let mut stmt = conn.prepare(
            "SELECT artifact_id, document_id, chunk_index, chunk_text, source_file, page_number, ocr_used
             FROM document_chunks_view
             WHERE document_id = ?1
             ORDER BY chunk_index",
        )?;
        let rows = stmt.query_map([document_id], |row| {
            let chunk_text: String = row.get(3)?;
            let preview = if chunk_text.len() > CHUNK_PREVIEW_LEN {
                format!("{}...", &chunk_text[..CHUNK_PREVIEW_LEN])
            } else {
                chunk_text
            };
            Ok(DebugChunkInfo {
                artifact_id: row.get(0)?,
                document_id: row.get(1)?,
                chunk_index: row.get(2)?,
                chunk_text_preview: preview,
                source_file: row.get(4)?,
                page_number: row.get(5)?,
                ocr_used: row.get::<_, i64>(6)? != 0,
            })
        })?;
        return rows.collect();
    }

    // Fallback: from documents_fts (no source_file, page_number, ocr_used)
    let mut stmt = conn.prepare(
        "SELECT artifact_id, document_id, chunk_index, chunk_text
         FROM documents_fts
         WHERE document_id = ?1
         ORDER BY chunk_index",
    )?;
    let rows = stmt.query_map([document_id], |row| {
        let chunk_text: String = row.get(3)?;
        let preview = if chunk_text.len() > CHUNK_PREVIEW_LEN {
            format!("{}...", &chunk_text[..CHUNK_PREVIEW_LEN])
        } else {
            chunk_text
        };
        Ok(DebugChunkInfo {
            artifact_id: row.get(0)?,
            document_id: row.get(1)?,
            chunk_index: row.get::<_, String>(2)?.parse().unwrap_or(0),
            chunk_text_preview: preview,
            source_file: String::new(),
            page_number: 0,
            ocr_used: false,
        })
    })?;
    rows.collect()
}

/// List entities for a document.
pub fn list_debug_entities_for_document(
    conn: &Connection,
    document_id: &str,
) -> Result<Vec<DebugEntityInfo>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT entity_id, entity_type, entity_value, normalized_value, extraction_method, confidence, document_id, chunk_index
         FROM entities_view
         WHERE document_id = ?1
         ORDER BY chunk_index, entity_type",
    )?;
    let rows = stmt.query_map([document_id], |row| {
        Ok(DebugEntityInfo {
            entity_id: row.get(0)?,
            entity_type: row.get(1)?,
            entity_value: row.get(2)?,
            normalized_value: row.get(3)?,
            extraction_method: row.get(4)?,
            confidence: row.get(5)?,
            source_document_id: row.get(6)?,
            chunk_index: row.get(7)?,
        })
    })?;
    rows.collect()
}

fn map_row_to_entity_info(row: &rusqlite::Row<'_>) -> Result<DebugEntityInfo, rusqlite::Error> {
    Ok(DebugEntityInfo {
        entity_id: row.get(0)?,
        entity_type: row.get(1)?,
        entity_value: row.get(2)?,
        normalized_value: row.get(3)?,
        extraction_method: row.get(4)?,
        confidence: row.get(5)?,
        source_document_id: row.get(6)?,
        chunk_index: row.get(7)?,
    })
}

/// List entities, optionally filtered by type.
pub fn list_debug_entities(
    conn: &Connection,
    entity_type: Option<&str>,
    limit: usize,
) -> Result<Vec<DebugEntityInfo>, rusqlite::Error> {
    let limit = limit.min(500) as i64;
    if let Some(et) = entity_type {
        let mut stmt = conn.prepare(
            "SELECT entity_id, entity_type, entity_value, normalized_value, extraction_method, confidence, document_id, chunk_index
             FROM entities_view
             WHERE entity_type = ?1
             ORDER BY created_at_ms DESC
             LIMIT ?2",
        )?;
        let rows: Vec<DebugEntityInfo> = stmt
            .query_map(rusqlite::params![et, limit], map_row_to_entity_info)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    } else {
        let mut stmt = conn.prepare(
            "SELECT entity_id, entity_type, entity_value, normalized_value, extraction_method, confidence, document_id, chunk_index
             FROM entities_view
             ORDER BY created_at_ms DESC
             LIMIT ?1",
        )?;
        let rows: Vec<DebugEntityInfo> = stmt
            .query_map([limit], map_row_to_entity_info)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}

/// Get ask session by case_id.
pub fn get_debug_ask_session(
    conn: &Connection,
    case_id: &str,
) -> Result<Option<DebugAskSession>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT case_id, question, plan_json, evidence_json, confidence, source_types, web_fallback_used, peer_consult_used, created_at_ms
         FROM debug_ask_sessions
         WHERE case_id = ?1",
    )?;
    let mut rows = stmt.query([case_id])?;
    if let Ok(Some(r)) = rows.next() {
        return Ok(Some(DebugAskSession {
            case_id: r.get(0)?,
            question: r.get(1)?,
            plan_json: r.get(2)?,
            evidence_json: r.get(3)?,
            confidence: r.get(4)?,
            source_types: r.get(5)?,
            web_fallback_used: r.get::<_, i64>(6)? != 0,
            peer_consult_used: r.get::<_, i64>(7)? != 0,
            created_at_ms: r.get(8)?,
        }));
    }
    Ok(None)
}

/// Store ask session for debug inspection.
#[allow(clippy::too_many_arguments)]
pub fn store_debug_ask_session(
    conn: &Connection,
    case_id: &str,
    question: &str,
    plan_json: &str,
    evidence_json: &str,
    confidence: f64,
    source_types: &str,
    web_fallback_used: bool,
    peer_consult_used: bool,
) -> Result<(), rusqlite::Error> {
    let created_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;

    conn.execute(
        "INSERT OR REPLACE INTO debug_ask_sessions
         (case_id, question, plan_json, evidence_json, confidence, source_types, web_fallback_used, peer_consult_used, created_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            case_id,
            question,
            plan_json,
            evidence_json,
            confidence,
            source_types,
            if web_fallback_used { 1i64 } else { 0 },
            if peer_consult_used { 1i64 } else { 0 },
            created_at_ms,
        ],
    )?;
    Ok(())
}

/// Store an entity correction.
#[allow(clippy::too_many_arguments)]
pub fn store_entity_correction(
    conn: &Connection,
    document_id: &str,
    entity_id: &str,
    chunk_index: i64,
    original_value: &str,
    corrected_type: Option<&str>,
    corrected_value: Option<&str>,
    is_valid: bool,
    source_user: &str,
) -> Result<String, rusqlite::Error> {
    let correction_id = format!("corr-entity-{}", uuid::Uuid::new_v4());
    let created_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let corrected_type = corrected_type.unwrap_or("");
    let corrected_value = corrected_value.unwrap_or(original_value);
    let is_valid_int = if is_valid { 1 } else { 0 };

    conn.execute(
        "INSERT INTO corrections_view
         (correction_id, correction_type, target_document_id, target_entity_id, target_chunk_index,
          original_value, corrected_value, corrected_type, is_valid, source_user, created_at_ms)
         VALUES (?1, 'entity', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        rusqlite::params![
            correction_id,
            document_id,
            entity_id,
            chunk_index,
            original_value,
            corrected_value,
            corrected_type,
            is_valid_int,
            source_user,
            created_at_ms,
        ],
    )?;
    Ok(correction_id)
}

/// Store a chunk (OCR) correction.
pub fn store_chunk_correction(
    conn: &Connection,
    document_id: &str,
    chunk_index: i64,
    original_text: &str,
    corrected_text: &str,
    note: &str,
    source_user: &str,
) -> Result<String, rusqlite::Error> {
    let correction_id = format!("corr-chunk-{}", uuid::Uuid::new_v4());
    let created_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;

    conn.execute(
        "INSERT INTO corrections_view
         (correction_id, correction_type, target_document_id, target_entity_id, target_chunk_index,
          original_value, corrected_value, note, source_user, created_at_ms)
         VALUES (?1, 'chunk', ?2, '', ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            correction_id,
            document_id,
            chunk_index,
            original_text,
            corrected_text,
            note,
            source_user,
            created_at_ms,
        ],
    )?;
    Ok(correction_id)
}

/// Store a document classification correction.
pub fn store_classification_correction(
    conn: &Connection,
    document_id: &str,
    corrected_document_type: &str,
    corrected_entity_type: &str,
    source_user: &str,
) -> Result<String, rusqlite::Error> {
    let correction_id = format!("corr-class-{}", uuid::Uuid::new_v4());
    let created_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;

    conn.execute(
        "INSERT INTO corrections_view
         (correction_id, correction_type, target_document_id, original_value, corrected_value, corrected_type, source_user, created_at_ms)
         VALUES (?1, 'classification', ?2, '', ?3, ?4, ?5, ?6)",
        rusqlite::params![
            correction_id,
            document_id,
            corrected_document_type,
            corrected_entity_type,
            source_user,
            created_at_ms,
        ],
    )?;
    Ok(correction_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite_views;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        sqlite_views::create_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn test_list_debug_documents_empty() {
        let conn = setup();
        let docs = list_debug_documents(&conn).unwrap();
        assert!(docs.is_empty());
    }

    #[test]
    fn test_store_and_effective_entity_correction() {
        let conn = setup();
        conn.execute(
            "INSERT INTO entities_view (entity_id, entity_type, entity_value, normalized_value, document_id, chunk_index, confidence, extraction_method, created_at_ms)
             VALUES ('person:jane', 'person', 'Jane Doe', 'jane doe', 'doc1.pdf', 0, 0.9, 'rule_based', 1000)",
            [],
        )
        .unwrap();

        let id = store_entity_correction(
            &conn,
            "doc1.pdf",
            "person:jane",
            0,
            "Jane Doe",
            None,
            Some("Jane Smith"),
            true,
            "admin",
        )
        .unwrap();
        assert!(id.starts_with("corr-entity-"));

        let effective: (String, String) = conn
            .query_row(
                "SELECT entity_value, normalized_value FROM effective_entities_view WHERE entity_id = 'person:jane'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(effective.0, "Jane Smith");
        assert_eq!(effective.1, "Jane Smith");
    }

    #[test]
    fn test_store_ask_session_and_retrieve() {
        let conn = setup();
        store_debug_ask_session(
            &conn,
            "ask-123",
            "What is the total?",
            r#"{"intent":"unknown"}"#,
            "[]",
            0.7,
            r#"["local"]"#,
            false,
            false,
        )
        .unwrap();
        let s = get_debug_ask_session(&conn, "ask-123").unwrap().unwrap();
        assert_eq!(s.case_id, "ask-123");
        assert_eq!(s.question, "What is the total?");
        assert_eq!(s.confidence, 0.7);
        assert!(!s.web_fallback_used);
    }
}
