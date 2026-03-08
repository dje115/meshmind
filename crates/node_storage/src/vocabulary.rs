//! Entity vocabulary: persistent lookup and learning for classification.

use rusqlite::Connection;

/// Normalize phrase for vocabulary key (lowercase, collapse whitespace).
pub fn normalize_phrase(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Lookup entity type by normalized phrase. Returns (entity_type, confidence) if found and >= threshold.
pub fn vocab_lookup(
    conn: &Connection,
    normalized_phrase: &str,
    confidence_threshold: f32,
) -> Result<Option<(String, f32)>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT entity_type, confidence FROM entity_vocabulary WHERE normalized_phrase = ?1",
    )?;
    let mut rows = stmt.query([normalized_phrase])?;
    if let Some(row) = rows.next()? {
        let entity_type: String = row.get(0)?;
        let confidence: f32 = row.get(1)?;
        if confidence >= confidence_threshold {
            return Ok(Some((entity_type, confidence)));
        }
    }
    Ok(None)
}

/// Learn or update a phrase in the vocabulary. Increments occurrence_count if phrase exists.
pub fn vocab_learn(
    conn: &Connection,
    normalized_phrase: &str,
    entity_type: &str,
    confidence: f32,
    source_method: &str,
) -> Result<(), rusqlite::Error> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;

    conn.execute(
        "INSERT INTO entity_vocabulary
         (normalized_phrase, entity_type, confidence, first_seen, last_seen, occurrence_count, source_method)
         VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6)
         ON CONFLICT(normalized_phrase) DO UPDATE SET
           entity_type = excluded.entity_type,
           confidence = excluded.confidence,
           last_seen = excluded.last_seen,
           occurrence_count = occurrence_count + 1,
           source_method = excluded.source_method",
        rusqlite::params![
            normalized_phrase,
            entity_type,
            confidence,
            now,
            now,
            source_method,
        ],
    )?;
    Ok(())
}

/// Record a user correction: upsert with source_method = 'corrected', confidence = 1.0.
pub fn vocab_learn_correction(
    conn: &Connection,
    normalized_phrase: &str,
    entity_type: &str,
) -> Result<(), rusqlite::Error> {
    vocab_learn(conn, normalized_phrase, entity_type, 1.0, "corrected")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite_views;

    #[test]
    fn vocab_lookup_learn_roundtrip() {
        let conn = Connection::open_in_memory().unwrap();
        sqlite_views::create_schema(&conn).unwrap();

        vocab_learn(&conn, "patch panel", "product", 0.9, "llm_assisted").unwrap();

        let (t, c) = vocab_lookup(&conn, "patch panel", 0.8).unwrap().unwrap();
        assert_eq!(t, "product");
        assert!((c - 0.9).abs() < 0.01);
    }
}
