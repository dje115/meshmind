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
            redaction_policy_json TEXT NOT NULL DEFAULT '{}',
            mapping_rules_json   TEXT NOT NULL DEFAULT '{}'
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

        -- Normalized documents (entity cards, runbooks, etc.) from artifacts
        CREATE TABLE IF NOT EXISTS documents_view (
            document_id     TEXT NOT NULL,
            version         INTEGER NOT NULL,
            document_type   TEXT NOT NULL DEFAULT 'entity_card',
            entity_type     TEXT NOT NULL DEFAULT '',
            entity_key      TEXT NOT NULL DEFAULT '',
            content_hash    TEXT,
            source_id       TEXT NOT NULL DEFAULT '',
            table_name      TEXT NOT NULL DEFAULT '',
            title           TEXT NOT NULL DEFAULT '',
            summary         TEXT NOT NULL DEFAULT '',
            created_at_ms   INTEGER NOT NULL,
            PRIMARY KEY (document_id, version)
        );

        -- Fact aggregates (counts, sums, etc.) from ingest runs
        CREATE TABLE IF NOT EXISTS facts_view (
            fact_id         TEXT NOT NULL,
            version         INTEGER NOT NULL,
            source_id       TEXT NOT NULL DEFAULT '',
            ingest_id       TEXT NOT NULL DEFAULT '',
            metric          TEXT NOT NULL DEFAULT '',
            dimensions_json TEXT NOT NULL DEFAULT '{}',
            value_json      TEXT NOT NULL DEFAULT '{}',
            time_window     TEXT NOT NULL DEFAULT '',
            content_hash    TEXT,
            created_at_ms   INTEGER NOT NULL,
            PRIMARY KEY (fact_id, version)
        );

        -- Entity intelligence graph
        CREATE TABLE IF NOT EXISTS entity_cards_view (
            entity_id       TEXT PRIMARY KEY,
            entity_type     TEXT NOT NULL,
            attributes_json TEXT NOT NULL DEFAULT '{}',
            content_hash    TEXT,
            source_id       TEXT NOT NULL DEFAULT '',
            table_name      TEXT NOT NULL DEFAULT '',
            created_at_ms   INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS entity_relationships_view (
            from_entity_id  TEXT NOT NULL,
            to_entity_id    TEXT NOT NULL,
            relationship_type TEXT NOT NULL DEFAULT '',
            source_id       TEXT NOT NULL DEFAULT '',
            table_name      TEXT NOT NULL DEFAULT '',
            created_at_ms   INTEGER NOT NULL,
            PRIMARY KEY (from_entity_id, to_entity_id, relationship_type)
        );

        -- Document-derived entity-to-entity relationships (extracted from chunks)
        CREATE TABLE IF NOT EXISTS extracted_entity_relationships_view (
            relationship_id     TEXT NOT NULL PRIMARY KEY,
            from_entity_id      TEXT NOT NULL,
            from_entity_value   TEXT NOT NULL DEFAULT '',
            relationship_type  TEXT NOT NULL,
            to_entity_id       TEXT NOT NULL,
            to_entity_value    TEXT NOT NULL DEFAULT '',
            source_document_id TEXT NOT NULL,
            chunk_index        INTEGER NOT NULL,
            confidence         REAL NOT NULL DEFAULT 0.0,
            extraction_method  TEXT NOT NULL DEFAULT 'rule_based',
            created_at_ms      INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_extracted_rel_from ON extracted_entity_relationships_view(from_entity_id);
        CREATE INDEX IF NOT EXISTS idx_extracted_rel_to ON extracted_entity_relationships_view(to_entity_id);
        CREATE INDEX IF NOT EXISTS idx_extracted_rel_type ON extracted_entity_relationships_view(relationship_type);
        CREATE INDEX IF NOT EXISTS idx_extracted_rel_doc ON extracted_entity_relationships_view(source_document_id);

        -- Type-specific views (aliases for entity_cards_view)
        CREATE VIEW IF NOT EXISTS customers_view AS
            SELECT * FROM entity_cards_view WHERE entity_type = 'customer';

        CREATE VIEW IF NOT EXISTS quotes_view AS
            SELECT * FROM entity_cards_view WHERE entity_type = 'quote';

        CREATE VIEW IF NOT EXISTS invoices_view AS
            SELECT * FROM entity_cards_view WHERE entity_type = 'invoice';

        CREATE VIEW IF NOT EXISTS accounts_view AS
            SELECT * FROM entity_cards_view WHERE entity_type = 'account';

        -- Extracted entities from document chunks (Phase B)
        CREATE TABLE IF NOT EXISTS entities_view (
            entity_id              TEXT NOT NULL,
            entity_type            TEXT NOT NULL,
            entity_value           TEXT NOT NULL,
            normalized_value       TEXT NOT NULL,
            document_id            TEXT NOT NULL,
            chunk_index            INTEGER NOT NULL,
            confidence             REAL NOT NULL DEFAULT 0.0,
            extraction_method      TEXT NOT NULL DEFAULT 'rule_based',
            classification_method  TEXT NOT NULL DEFAULT 'rule_based',
            created_at_ms          INTEGER NOT NULL,
            PRIMARY KEY (entity_id, document_id, chunk_index)
        );
        CREATE INDEX IF NOT EXISTS idx_entities_view_type ON entities_view(entity_type);
        CREATE INDEX IF NOT EXISTS idx_entities_view_normalized ON entities_view(normalized_value, entity_type);

        -- Vocabulary: learned phrase -> entity_type for classification reuse
        CREATE TABLE IF NOT EXISTS entity_vocabulary (
            normalized_phrase   TEXT NOT NULL PRIMARY KEY,
            entity_type         TEXT NOT NULL,
            confidence          REAL NOT NULL DEFAULT 0.0,
            first_seen          INTEGER NOT NULL,
            last_seen           INTEGER NOT NULL,
            occurrence_count    INTEGER NOT NULL DEFAULT 1,
            source_method       TEXT NOT NULL DEFAULT 'rule_based'
        );
        CREATE INDEX IF NOT EXISTS idx_entity_vocabulary_type ON entity_vocabulary(entity_type);

        CREATE TABLE IF NOT EXISTS documents_entities_view (
            document_id       TEXT NOT NULL,
            entity_id         TEXT NOT NULL,
            entity_type       TEXT NOT NULL,
            entity_value      TEXT NOT NULL,
            chunk_index       INTEGER NOT NULL,
            created_at_ms     INTEGER NOT NULL,
            PRIMARY KEY (document_id, entity_id, chunk_index)
        );
        CREATE INDEX IF NOT EXISTS idx_doc_entities_entity ON documents_entities_view(entity_id);

        -- Debug: document chunks with OCR/source metadata (populated on ingest)
        CREATE TABLE IF NOT EXISTS document_chunks_view (
            artifact_id       TEXT NOT NULL PRIMARY KEY,
            document_id       TEXT NOT NULL,
            chunk_index       INTEGER NOT NULL,
            chunk_text        TEXT NOT NULL DEFAULT '',
            source_file       TEXT NOT NULL DEFAULT '',
            page_number       INTEGER NOT NULL DEFAULT 0,
            ocr_used          INTEGER NOT NULL DEFAULT 0,
            source_id         TEXT NOT NULL DEFAULT '',
            source_locator    TEXT NOT NULL DEFAULT '',
            source_open_target TEXT NOT NULL DEFAULT '',
            source_origin_label TEXT NOT NULL DEFAULT '',
            created_at_ms     INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_document_chunks_doc ON document_chunks_view(document_id);

        -- Debug: ask sessions for inspection (question, plan, evidence)
        -- Extraction corrections (entity, OCR/chunk, classification) for self-learning
        CREATE TABLE IF NOT EXISTS corrections_view (
            correction_id     TEXT NOT NULL PRIMARY KEY,
            correction_type   TEXT NOT NULL,
            target_document_id TEXT NOT NULL DEFAULT '',
            target_entity_id  TEXT NOT NULL DEFAULT '',
            target_chunk_index INTEGER NOT NULL DEFAULT -1,
            original_value    TEXT NOT NULL DEFAULT '',
            corrected_value   TEXT NOT NULL DEFAULT '',
            corrected_type    TEXT NOT NULL DEFAULT '',
            is_valid          INTEGER NOT NULL DEFAULT 1,
            note              TEXT NOT NULL DEFAULT '',
            source_user       TEXT NOT NULL DEFAULT 'admin',
            created_at_ms     INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_corrections_doc ON corrections_view(target_document_id);
        CREATE INDEX IF NOT EXISTS idx_corrections_entity ON corrections_view(target_entity_id);

        -- Effective entities: prefer corrected values when present (for future training)
        CREATE VIEW IF NOT EXISTS effective_entities_view AS
        SELECT e.entity_id,
            COALESCE(
                (SELECT NULLIF(c.corrected_type, '') FROM corrections_view c
                 WHERE c.target_document_id = e.document_id AND c.target_entity_id = e.entity_id
                   AND c.target_chunk_index = e.chunk_index AND c.correction_type = 'entity'
                   AND c.is_valid = 1
                 ORDER BY c.created_at_ms DESC LIMIT 1),
                e.entity_type
            ) AS entity_type,
            COALESCE(
                (SELECT c.corrected_value FROM corrections_view c
                 WHERE c.target_document_id = e.document_id AND c.target_entity_id = e.entity_id
                   AND c.target_chunk_index = e.chunk_index AND c.correction_type = 'entity'
                   AND c.is_valid = 1
                 ORDER BY c.created_at_ms DESC LIMIT 1),
                e.entity_value
            ) AS entity_value,
            COALESCE(
                (SELECT c.corrected_value FROM corrections_view c
                 WHERE c.target_document_id = e.document_id AND c.target_entity_id = e.entity_id
                   AND c.target_chunk_index = e.chunk_index AND c.correction_type = 'entity'
                   AND c.is_valid = 1
                 ORDER BY c.created_at_ms DESC LIMIT 1),
                e.normalized_value
            ) AS normalized_value,
            e.document_id, e.chunk_index, e.confidence, e.extraction_method,
            CASE WHEN (SELECT 1 FROM corrections_view c WHERE c.target_document_id = e.document_id AND c.target_entity_id = e.entity_id AND c.target_chunk_index = e.chunk_index AND c.correction_type = 'entity' AND c.is_valid = 1 LIMIT 1) IS NOT NULL THEN 'corrected' ELSE e.classification_method END AS classification_method,
            e.created_at_ms
        FROM entities_view e;

        -- Per-file ingest results (document folder ingestion)
        CREATE TABLE IF NOT EXISTS ingest_file_results (
            ingest_id        TEXT NOT NULL,
            source_id        TEXT NOT NULL,
            filename         TEXT NOT NULL,
            file_path        TEXT NOT NULL,
            detected_type    TEXT NOT NULL DEFAULT '',
            status           TEXT NOT NULL DEFAULT '',
            failure_reason   TEXT,
            ocr_attempted    INTEGER NOT NULL DEFAULT 0,
            chunks_created   INTEGER NOT NULL DEFAULT 0,
            created_at_ms    INTEGER NOT NULL,
            PRIMARY KEY (ingest_id, file_path)
        );
        CREATE INDEX IF NOT EXISTS idx_ingest_file_results_source ON ingest_file_results(source_id);

        CREATE TABLE IF NOT EXISTS debug_ask_sessions (
            case_id           TEXT NOT NULL PRIMARY KEY,
            question          TEXT NOT NULL,
            plan_json         TEXT NOT NULL DEFAULT '{}',
            evidence_json     TEXT NOT NULL DEFAULT '[]',
            confidence        REAL NOT NULL DEFAULT 0.0,
            source_types      TEXT NOT NULL DEFAULT '[]',
            web_fallback_used INTEGER NOT NULL DEFAULT 0,
            peer_consult_used INTEGER NOT NULL DEFAULT 0,
            created_at_ms     INTEGER NOT NULL
        );

        CREATE VIEW IF NOT EXISTS people_view AS
            SELECT * FROM entities_view WHERE entity_type = 'person';

        CREATE VIEW IF NOT EXISTS companies_view AS
            SELECT * FROM entities_view WHERE entity_type = 'company';

        -- Effective entity relationships (for future corrections; mirrors extracted for now)
        CREATE VIEW IF NOT EXISTS effective_entity_relationships_view AS
            SELECT relationship_id, from_entity_id, from_entity_value, relationship_type,
                   to_entity_id, to_entity_value, source_document_id, chunk_index,
                   confidence, extraction_method, created_at_ms
            FROM extracted_entity_relationships_view;

        -- Knowledge shards (distributed memory)
        CREATE TABLE IF NOT EXISTS shards_view (
            shard_key        TEXT PRIMARY KEY,
            shard_kind       TEXT NOT NULL DEFAULT 'tenant',
            created_at_ms    INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS shard_membership_view (
            shard_key        TEXT NOT NULL,
            member_type      TEXT NOT NULL,
            member_id        TEXT NOT NULL,
            node_id          TEXT NOT NULL DEFAULT '',
            created_at_ms    INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (shard_key, member_type, member_id)
        );

        CREATE TABLE IF NOT EXISTS shard_subscriptions_view (
            shard_key        TEXT NOT NULL,
            node_id          TEXT NOT NULL,
            capability       TEXT NOT NULL DEFAULT 'query',
            last_seen_ms     INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (shard_key, node_id)
        );

        -- Mergeable state (CRDT-like, Phase 3)
        CREATE TABLE IF NOT EXISTS mergeable_tag_events (
            event_id         TEXT NOT NULL,
            object_type      TEXT NOT NULL,
            object_id        TEXT NOT NULL,
            tag              TEXT NOT NULL,
            op               TEXT NOT NULL,
            node_id          TEXT NOT NULL,
            ts_ms            INTEGER NOT NULL,
            PRIMARY KEY (event_id)
        );
        CREATE INDEX IF NOT EXISTS idx_mergeable_tag_events_obj
            ON mergeable_tag_events(object_type, object_id);

        CREATE TABLE IF NOT EXISTS mergeable_counter_deltas (
            event_id         TEXT NOT NULL,
            object_type      TEXT NOT NULL,
            object_id        TEXT NOT NULL,
            counter_key      TEXT NOT NULL,
            node_id          TEXT NOT NULL,
            delta            INTEGER NOT NULL,
            ts_ms            INTEGER NOT NULL,
            PRIMARY KEY (event_id)
        );
        CREATE INDEX IF NOT EXISTS idx_mergeable_counter_obj
            ON mergeable_counter_deltas(object_type, object_id, counter_key);

        CREATE TABLE IF NOT EXISTS mergeable_annotations_view (
            object_type      TEXT NOT NULL,
            object_id        TEXT NOT NULL,
            annotation_key   TEXT NOT NULL,
            value            TEXT NOT NULL,
            node_id          TEXT NOT NULL,
            ts_ms            INTEGER NOT NULL,
            PRIMARY KEY (object_type, object_id, annotation_key)
        );

        -- Proactive insight engine (Phase 5)
        CREATE TABLE IF NOT EXISTS insights_view (
            insight_id       TEXT PRIMARY KEY,
            insight_type     TEXT NOT NULL,
            title            TEXT NOT NULL,
            summary         TEXT NOT NULL,
            entity_ids_json  TEXT NOT NULL DEFAULT '[]',
            confidence       REAL NOT NULL DEFAULT 0.0,
            schedule         TEXT NOT NULL DEFAULT 'manual',
            created_at_ms    INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS alerts_view (
            alert_id         TEXT PRIMARY KEY,
            alert_type       TEXT NOT NULL,
            severity         TEXT NOT NULL DEFAULT 'info',
            title            TEXT NOT NULL,
            message          TEXT NOT NULL,
            entity_ids_json  TEXT NOT NULL DEFAULT '[]',
            schedule         TEXT NOT NULL DEFAULT 'manual',
            created_at_ms    INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS benchmarks_view (
            benchmark_id     TEXT PRIMARY KEY,
            metric           TEXT NOT NULL,
            dimension        TEXT NOT NULL,
            value            REAL NOT NULL,
            time_window      TEXT NOT NULL,
            schedule         TEXT NOT NULL DEFAULT 'manual',
            created_at_ms    INTEGER NOT NULL
        );
        -- Outcome-driven learning (Phase 6)
        CREATE TABLE IF NOT EXISTS outcomes_view (
            outcome_id        TEXT PRIMARY KEY,
            outcome_type      TEXT NOT NULL,
            case_id           TEXT NOT NULL DEFAULT '',
            quote_id          TEXT NOT NULL DEFAULT '',
            outcome_value     TEXT,
            reason            TEXT,
            confidence        REAL,
            created_at_ms     INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_outcomes_case ON outcomes_view(case_id);
        CREATE INDEX IF NOT EXISTS idx_outcomes_quote ON outcomes_view(quote_id);
        CREATE INDEX IF NOT EXISTS idx_outcomes_type ON outcomes_view(outcome_type);

        CREATE TABLE IF NOT EXISTS anomalies_view (
            anomaly_id       TEXT PRIMARY KEY,
            metric           TEXT NOT NULL,
            dimension        TEXT NOT NULL,
            expected_value   REAL NOT NULL,
            actual_value     REAL NOT NULL,
            deviation_pct    REAL NOT NULL,
            schedule         TEXT NOT NULL DEFAULT 'manual',
            created_at_ms    INTEGER NOT NULL
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

        CREATE VIRTUAL TABLE IF NOT EXISTS documents_fts USING fts5(
            artifact_id UNINDEXED,
            document_id UNINDEXED,
            chunk_index UNINDEXED,
            chunk_text
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
    migrate_source_profiles_mapping(&conn)?;
    migrate_debug_tables(&conn)?;
    migrate_document_chunks_provenance(&conn)?;
    migrate_entities_classification_and_vocabulary(&conn)?;
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

/// Migrate source_profiles_view: add mapping_rules_json if missing.
fn migrate_debug_tables(conn: &Connection) -> Result<()> {
    for (name, sql) in [
        (
            "document_chunks_view",
            "CREATE TABLE IF NOT EXISTS document_chunks_view (
                artifact_id TEXT NOT NULL PRIMARY KEY,
                document_id TEXT NOT NULL,
                chunk_index INTEGER NOT NULL,
                chunk_text TEXT NOT NULL DEFAULT '',
                source_file TEXT NOT NULL DEFAULT '',
                page_number INTEGER NOT NULL DEFAULT 0,
                ocr_used INTEGER NOT NULL DEFAULT 0,
                source_id TEXT NOT NULL DEFAULT '',
                source_locator TEXT NOT NULL DEFAULT '',
                source_open_target TEXT NOT NULL DEFAULT '',
                source_origin_label TEXT NOT NULL DEFAULT '',
                created_at_ms INTEGER NOT NULL
            )",
        ),
        (
            "corrections_view",
            "CREATE TABLE IF NOT EXISTS corrections_view (
                correction_id TEXT NOT NULL PRIMARY KEY,
                correction_type TEXT NOT NULL,
                target_document_id TEXT NOT NULL DEFAULT '',
                target_entity_id TEXT NOT NULL DEFAULT '',
                target_chunk_index INTEGER NOT NULL DEFAULT -1,
                original_value TEXT NOT NULL DEFAULT '',
                corrected_value TEXT NOT NULL DEFAULT '',
                corrected_type TEXT NOT NULL DEFAULT '',
                is_valid INTEGER NOT NULL DEFAULT 1,
                note TEXT NOT NULL DEFAULT '',
                source_user TEXT NOT NULL DEFAULT 'admin',
                created_at_ms INTEGER NOT NULL
            )",
        ),
        (
            "debug_ask_sessions",
            "CREATE TABLE IF NOT EXISTS debug_ask_sessions (
                case_id TEXT NOT NULL PRIMARY KEY,
                question TEXT NOT NULL,
                plan_json TEXT NOT NULL DEFAULT '{}',
                evidence_json TEXT NOT NULL DEFAULT '[]',
                confidence REAL NOT NULL DEFAULT 0.0,
                source_types TEXT NOT NULL DEFAULT '[]',
                web_fallback_used INTEGER NOT NULL DEFAULT 0,
                peer_consult_used INTEGER NOT NULL DEFAULT 0,
                created_at_ms INTEGER NOT NULL
            )",
        ),
    ] {
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
                [name],
                |_| Ok(()),
            )
            .is_ok();
        if !exists {
            conn.execute(sql, [])?;
        }
    }
    Ok(())
}

/// Add source provenance columns to document_chunks_view for "Open original" / "Where did this come from?"
fn migrate_document_chunks_provenance(conn: &Connection) -> Result<()> {
    let exists: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='document_chunks_view'",
            [],
            |_| Ok(()),
        )
        .is_ok();
    if !exists {
        return Ok(());
    }
    let cols: Vec<String> = conn
        .prepare("PRAGMA table_info(document_chunks_view)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .filter_map(|r| r.ok())
        .collect();
    for col in [
        "source_locator",
        "source_open_target",
        "source_origin_label",
    ] {
        if !cols.contains(&col.to_string()) {
            let _ = conn.execute(
                &format!(
                    "ALTER TABLE document_chunks_view ADD COLUMN {} TEXT NOT NULL DEFAULT ''",
                    col
                ),
                [],
            );
        }
    }
    Ok(())
}

fn migrate_entities_classification_and_vocabulary(conn: &Connection) -> Result<()> {
    let entities_exists: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='entities_view'",
            [],
            |row| row.get::<_, i32>(0),
        )
        .map(|_| true)
        .unwrap_or(false);

    if entities_exists {
        let has_classification: bool = conn
            .prepare("PRAGMA table_info(entities_view)")
            .and_then(|mut stmt| {
                let names: Vec<String> = stmt
                    .query_map([], |row| row.get::<_, String>(1))?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok(names.contains(&"classification_method".to_string()))
            })
            .unwrap_or(false);

        if !has_classification {
            conn.execute(
                "ALTER TABLE entities_view ADD COLUMN classification_method TEXT NOT NULL DEFAULT 'rule_based'",
                [],
            )?;
            conn.execute("DROP VIEW IF EXISTS effective_entities_view", [])?;
        }
    }

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS entity_vocabulary (
            normalized_phrase   TEXT NOT NULL PRIMARY KEY,
            entity_type         TEXT NOT NULL,
            confidence          REAL NOT NULL DEFAULT 0.0,
            first_seen          INTEGER NOT NULL,
            last_seen           INTEGER NOT NULL,
            occurrence_count    INTEGER NOT NULL DEFAULT 1,
            source_method       TEXT NOT NULL DEFAULT 'rule_based'
        );
        CREATE INDEX IF NOT EXISTS idx_entity_vocabulary_type ON entity_vocabulary(entity_type);",
    )?;
    Ok(())
}

fn migrate_source_profiles_mapping(conn: &Connection) -> Result<()> {
    let table_exists: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='source_profiles_view'",
            [],
            |row| row.get::<_, i32>(0),
        )
        .map(|_| true)
        .unwrap_or(false);

    if !table_exists {
        return Ok(());
    }

    let has_mapping: bool = conn
        .prepare("PRAGMA table_info(source_profiles_view)")
        .and_then(|mut stmt| {
            let names: Vec<String> = stmt
                .query_map([], |row| row.get::<_, String>(1))?
                .filter_map(|r| r.ok())
                .collect();
            Ok(names.contains(&"mapping_rules_json".to_string()))
        })
        .unwrap_or(false);

    if !has_mapping {
        let _ = conn.execute(
            "ALTER TABLE source_profiles_view ADD COLUMN mapping_rules_json TEXT NOT NULL DEFAULT '{}'",
            [],
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
        assert!(tables.contains(&"documents_view".to_string()));
        assert!(tables.contains(&"facts_view".to_string()));
        assert!(tables.contains(&"entity_cards_view".to_string()));
        assert!(tables.contains(&"entity_relationships_view".to_string()));
        assert!(tables.contains(&"shards_view".to_string()));
        assert!(tables.contains(&"shard_membership_view".to_string()));
        assert!(tables.contains(&"shard_subscriptions_view".to_string()));
        assert!(tables.contains(&"mergeable_tag_events".to_string()));
        assert!(tables.contains(&"mergeable_counter_deltas".to_string()));
        assert!(tables.contains(&"mergeable_annotations_view".to_string()));
        assert!(tables.contains(&"insights_view".to_string()));
        assert!(tables.contains(&"alerts_view".to_string()));
        assert!(tables.contains(&"benchmarks_view".to_string()));
        assert!(tables.contains(&"anomalies_view".to_string()));
        assert!(tables.contains(&"outcomes_view".to_string()));
    }

    #[test]
    fn entity_graph_views_exist() {
        let conn = Connection::open_in_memory().unwrap();
        create_schema(&conn).unwrap();

        let _: i64 = conn
            .query_row("SELECT COUNT(*) FROM entity_cards_view", [], |row| {
                row.get(0)
            })
            .unwrap();

        let _: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM entity_relationships_view",
                [],
                |row| row.get(0),
            )
            .unwrap();

        let _: i64 = conn
            .query_row("SELECT COUNT(*) FROM customers_view", [], |row| row.get(0))
            .unwrap();

        let _: i64 = conn
            .query_row("SELECT COUNT(*) FROM quotes_view", [], |row| row.get(0))
            .unwrap();

        let _: i64 = conn
            .query_row("SELECT COUNT(*) FROM invoices_view", [], |row| row.get(0))
            .unwrap();

        let _: i64 = conn
            .query_row("SELECT COUNT(*) FROM accounts_view", [], |row| row.get(0))
            .unwrap();
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
