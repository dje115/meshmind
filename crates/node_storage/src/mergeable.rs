//! Mergeable state queries (CRDT-like tags, counters, annotations).

use rusqlite::Connection;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MergeableError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

pub type Result<T> = std::result::Result<T, MergeableError>;

/// Current tags for an object (add set minus remove set).
pub fn tags_for_object(
    conn: &Connection,
    object_type: &str,
    object_id: &str,
) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT tag FROM mergeable_tag_events
         WHERE object_type = ?1 AND object_id = ?2 AND op = 'add'
         AND (object_type, object_id, tag) NOT IN (
           SELECT object_type, object_id, tag FROM mergeable_tag_events WHERE op = 'remove'
         )",
    )?;
    let rows = stmt.query_map([object_type, object_id], |row| row.get(0))?;
    let mut tags: Vec<String> = rows.filter_map(|r| r.ok()).collect();
    tags.sort();
    tags.dedup();
    Ok(tags)
}

/// All counters for an object (counter_key -> total).
pub fn counters_for_object(
    conn: &Connection,
    object_type: &str,
    object_id: &str,
) -> Result<Vec<(String, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT counter_key, SUM(delta) as total FROM mergeable_counter_deltas
         WHERE object_type = ?1 AND object_id = ?2
         GROUP BY counter_key ORDER BY counter_key",
    )?;
    let rows = stmt.query_map([object_type, object_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// Total for a counter (sum of deltas).
pub fn counter_total(
    conn: &Connection,
    object_type: &str,
    object_id: &str,
    counter_key: &str,
) -> Result<i64> {
    let total: i64 = conn.query_row(
        "SELECT COALESCE(SUM(delta), 0) FROM mergeable_counter_deltas
         WHERE object_type = ?1 AND object_id = ?2 AND counter_key = ?3",
        [object_type, object_id, counter_key],
        |row| row.get(0),
    )?;
    Ok(total)
}

/// Annotation value (LWW).
pub fn annotation_value(
    conn: &Connection,
    object_type: &str,
    object_id: &str,
    annotation_key: &str,
) -> Result<Option<String>> {
    let result = conn.query_row(
        "SELECT value FROM mergeable_annotations_view
         WHERE object_type = ?1 AND object_id = ?2 AND annotation_key = ?3",
        [object_type, object_id, annotation_key],
        |row| row.get(0),
    );
    match result {
        Ok(v) => Ok(Some(v)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// All annotations for an object.
pub fn annotations_for_object(
    conn: &Connection,
    object_type: &str,
    object_id: &str,
) -> Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare(
        "SELECT annotation_key, value FROM mergeable_annotations_view
         WHERE object_type = ?1 AND object_id = ?2 ORDER BY annotation_key",
    )?;
    let rows = stmt.query_map([object_type, object_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}
