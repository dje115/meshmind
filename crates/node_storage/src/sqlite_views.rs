//! SQLite materialized views schema and management.
//!
//! Tables: cases_view, artifacts_view, web_briefs_view, peers_view,
//!         models_view, audit_view, projector_checkpoint
//! FTS5 virtual tables for search.

use rusqlite::Connection;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SchemaError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

pub type Result<T> = std::result::Result<T, SchemaError>;

pub fn create_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS cases_view (
            case_id         TEXT PRIMARY KEY,
            title           TEXT NOT NULL,
            summary         TEXT NOT NULL,
            content_hash    TEXT,
            shareable       INTEGER NOT NULL DEFAULT 0,
            tenant_id       TEXT NOT NULL,
            sensitivity     INTEGER NOT NULL DEFAULT 0,
            node_id         TEXT NOT NULL,
            outcome         TEXT,
            confidence      REAL,
            tags            TEXT NOT NULL DEFAULT '[]',
            created_at_ms   INTEGER NOT NULL,
            updated_at_ms   INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS artifacts_view (
            artifact_id     TEXT NOT NULL,
            version         INTEGER NOT NULL,
            artifact_type   INTEGER NOT NULL,
            title           TEXT NOT NULL,
            summary         TEXT NOT NULL DEFAULT '',
            content_hash    TEXT,
            shareable       INTEGER NOT NULL DEFAULT 0,
            tenant_id       TEXT NOT NULL,
            sensitivity     INTEGER NOT NULL DEFAULT 0,
            node_id         TEXT NOT NULL,
            expires_at_ms   INTEGER,
            deprecated      INTEGER NOT NULL DEFAULT 0,
            deprecate_reason TEXT,
            created_at_ms   INTEGER NOT NULL,
            PRIMARY KEY (artifact_id, version)
        );

        CREATE TABLE IF NOT EXISTS web_briefs_view (
            artifact_id     TEXT PRIMARY KEY,
            question        TEXT NOT NULL,
            summary         TEXT NOT NULL,
            sources_json    TEXT NOT NULL DEFAULT '[]',
            confidence      REAL NOT NULL DEFAULT 0.0,
            expires_at_ms   INTEGER,
            tenant_id       TEXT NOT NULL,
            node_id         TEXT NOT NULL,
            created_at_ms   INTEGER NOT NULL,
            expired         INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS peers_view (
            peer_node_id    TEXT PRIMARY KEY,
            rtt_ms          INTEGER,
            capabilities    TEXT NOT NULL DEFAULT '[]',
            trust_score     REAL NOT NULL DEFAULT 0.5,
            trust_reason    TEXT,
            last_seen_ms    INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS models_view (
            model_id        TEXT NOT NULL,
            version         INTEGER NOT NULL,
            model_bundle_hash TEXT,
            promoted        INTEGER NOT NULL DEFAULT 0,
            rolled_back     INTEGER NOT NULL DEFAULT 0,
            rollback_reason TEXT,
            node_id         TEXT NOT NULL,
            created_at_ms   INTEGER NOT NULL,
            PRIMARY KEY (model_id, version)
        );

        CREATE TABLE IF NOT EXISTS audit_view (
            event_id        TEXT PRIMARY KEY,
            event_type      INTEGER NOT NULL,
            node_id         TEXT NOT NULL,
            tenant_id       TEXT NOT NULL,
            sensitivity     INTEGER NOT NULL DEFAULT 0,
            summary         TEXT NOT NULL DEFAULT '',
            event_hash      TEXT NOT NULL DEFAULT '',
            created_at_ms   INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS projector_checkpoint (
            id              INTEGER PRIMARY KEY CHECK (id = 1),
            last_event_hash TEXT NOT NULL,
            event_count     INTEGER NOT NULL DEFAULT 0
        );

        -- Data discovery views
        CREATE TABLE IF NOT EXISTS sources_view (
            source_id       TEXT PRIMARY KEY,
            connector_type  INTEGER NOT NULL DEFAULT 0,
            path_or_uri     TEXT NOT NULL,
            display_name    TEXT NOT NULL DEFAULT '',
            estimated_size_bytes INTEGER NOT NULL DEFAULT 0,
            estimated_tables INTEGER NOT NULL DEFAULT 0,
            status          TEXT NOT NULL DEFAULT 'discovered',
            sensitivity     INTEGER NOT NULL DEFAULT 0,
            pii_detected    INTEGER NOT NULL DEFAULT 0,
            secrets_detected INTEGER NOT NULL DEFAULT 0,
            schema_snapshot_hash TEXT,
            discovered_at_ms INTEGER NOT NULL DEFAULT 0,
            classified_at_ms INTEGER,
            approved_at_ms  INTEGER
        );

        CREATE TABLE IF NOT EXISTS source_profiles_view (
            profile_id      TEXT PRIMARY KEY,
            source_id       TEXT NOT NULL,
            approved_by     TEXT NOT NULL DEFAULT '',
            approved_at_ms  INTEGER NOT NULL DEFAULT 0,
            profile_hash    TEXT,
            allowed_tables_json TEXT NOT NULL DEFAULT '[]',
            row_limit       INTEGER NOT NULL DEFAULT 0,
            allow_raw_retention INTEGER NOT NULL DEFAULT 0,
            allow_training  INTEGER NOT NULL DEFAULT 0,
            max_sensitivity INTEGER NOT NULL DEFAULT 2,
            redaction_policy_json TEXT NOT NULL DEFAULT '{}'
        );

        CREATE TABLE IF NOT EXISTS ingests_view (
            ingest_id       TEXT PRIMARY KEY,
            source_id       TEXT NOT NULL,
            connector_type  INTEGER NOT NULL DEFAULT 0,
            status          TEXT NOT NULL DEFAULT 'started',
            rows_ingested   INTEGER NOT NULL DEFAULT 0,
            documents_created INTEGER NOT NULL DEFAULT 0,
            facts_created   INTEGER NOT NULL DEFAULT 0,
            bytes_stored    INTEGER NOT NULL DEFAULT 0,
            duration_ms     INTEGER NOT NULL DEFAULT 0,
            notes           TEXT NOT NULL DEFAULT '',
            started_at_ms   INTEGER NOT NULL DEFAULT 0,
            completed_at_ms INTEGER
        );

        CREATE TABLE IF NOT EXISTS datasets_view (
            manifest_id     TEXT PRIMARY KEY,
            source_id       TEXT NOT NULL DEFAULT '',
            preset          TEXT NOT NULL DEFAULT '',
            manifest_hash   TEXT,
            item_count      INTEGER NOT NULL DEFAULT 0,
            total_bytes     INTEGER NOT NULL DEFAULT 0,
            created_at_ms   INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS federated_view (
            round_id        TEXT PRIMARY KEY,
            model_id        TEXT NOT NULL DEFAULT '',
            round_number    INTEGER NOT NULL DEFAULT 0,
            status          TEXT NOT NULL DEFAULT 'started',
            expected_participants INTEGER NOT NULL DEFAULT 0,
            actual_participants INTEGER NOT NULL DEFAULT 0,
            coordinator     TEXT NOT NULL DEFAULT '',
            success         INTEGER NOT NULL DEFAULT 0,
            resulting_model_hash TEXT,
            notes           TEXT NOT NULL DEFAULT '',
            started_at_ms   INTEGER NOT NULL DEFAULT 0,
            completed_at_ms INTEGER
        );

        -- Chat conversations
        CREATE TABLE IF NOT EXISTS conversations_view (
            conversation_id TEXT PRIMARY KEY,
            title           TEXT NOT NULL DEFAULT 'New conversation',
            created_at_ms   INTEGER NOT NULL,
            updated_at_ms   INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS messages_view (
            message_id      TEXT PRIMARY KEY,
            conversation_id TEXT NOT NULL,
            role            TEXT NOT NULL,
            content         TEXT NOT NULL,
            context_used    TEXT NOT NULL DEFAULT '[]',
            model           TEXT NOT NULL DEFAULT '',
            confidence      REAL NOT NULL DEFAULT 0.0,
            created_at_ms   INTEGER NOT NULL
        );
        ",
    )?;

    create_fts_tables(conn)?;

    Ok(())
}

fn create_fts_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE VIRTUAL TABLE IF NOT EXISTS cases_fts USING fts5(
            case_id, title, summary, tags
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS artifacts_fts USING fts5(
            artifact_id, title, summary
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
            message_id, content
        );
        ",
    )?;
    Ok(())
}

pub fn open_db(path: &std::path::Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            SchemaError::Sqlite(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(1),
                Some(format!("failed to create db dir: {e}")),
            ))
        })?;
    }
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
    migrate_artifacts_summary(&conn)?;
    create_schema(&conn)?;
    Ok(conn)
}

/// Migrate existing databases: add `summary` column to artifacts_view
/// and recreate artifacts_fts with the new schema.
fn migrate_artifacts_summary(conn: &Connection) -> Result<()> {
    let has_summary: bool = conn
        .prepare("PRAGMA table_info(artifacts_view)")
        .and_then(|mut stmt| {
            let names: Vec<String> = stmt
                .query_map([], |row| row.get::<_, String>(1))?
                .filter_map(|r| r.ok())
                .collect();
            Ok(names.contains(&"summary".to_string()))
        })
        .unwrap_or(false);

    if !has_summary {
        let _ = conn.execute_batch(
            "ALTER TABLE artifacts_view ADD COLUMN summary TEXT NOT NULL DEFAULT '';
             DROP TABLE IF EXISTS artifacts_fts;",
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_creation() {
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();

        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        assert!(tables.contains(&"cases_view".to_string()));
        assert!(tables.contains(&"artifacts_view".to_string()));
        assert!(tables.contains(&"web_briefs_view".to_string()));
        assert!(tables.contains(&"peers_view".to_string()));
        assert!(tables.contains(&"models_view".to_string()));
        assert!(tables.contains(&"audit_view".to_string()));
        assert!(tables.contains(&"projector_checkpoint".to_string()));
        assert!(tables.contains(&"sources_view".to_string()));
        assert!(tables.contains(&"source_profiles_view".to_string()));
        assert!(tables.contains(&"ingests_view".to_string()));
        assert!(tables.contains(&"datasets_view".to_string()));
        assert!(tables.contains(&"federated_view".to_string()));
        assert!(tables.contains(&"conversations_view".to_string()));
        assert!(tables.contains(&"messages_view".to_string()));
    }

    #[test]
    fn schema_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();
        create_schema(&conn).unwrap();
    }

    #[test]
    fn open_db_creates_dirs() {
        let tmp = tempfile::TempDir::new().unwrap();
        let db_path = tmp.path().join("sqlite").join("meshmind.db");
        let conn = open_db(&db_path).unwrap();
        assert!(db_path.exists());
        drop(conn);
    }
}
