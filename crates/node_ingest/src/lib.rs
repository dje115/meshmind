//! Ingestion pipelines: convert raw data into normalized Documents and Facts.
//!
//! Runs incremental, resumable ingestion jobs per approved SourceProfile.
//! Stores content in CAS, emits events, projects into SQLite views.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

use anyhow::Context;
use tracing::{debug, info};
use uuid::Uuid;

use node_connectors::Connector;
use node_proto::common::{NodeId, Sensitivity, TenantId, Timestamp};
use node_proto::events::{
    event_envelope, ArtifactPublished, ArtifactType, ConnectorType, EventEnvelope, EventType,
    IngestCompleted, IngestStarted,
};
use node_storage::cas::CasStore;
use node_storage::event_log::EventLog;

// ── Configuration ──────────────────────────────────────────────────────────

pub struct IngestConfig {
    pub batch_size: u64,
    pub max_rows_per_table: u64,
}

impl Default for IngestConfig {
    fn default() -> Self {
        Self {
            batch_size: 100,
            max_rows_per_table: 10_000,
        }
    }
}

// ── Job / Result ───────────────────────────────────────────────────────────

pub struct IngestJob {
    pub ingest_id: String,
    pub source_id: String,
    pub connector_type: String,
}

pub struct IngestResult {
    pub ingest_id: String,
    pub source_id: String,
    pub success: bool,
    pub rows_ingested: u64,
    pub documents_created: u64,
    pub facts_created: u64,
    pub bytes_stored: u64,
    pub duration_ms: u32,
}

// ── Helpers ────────────────────────────────────────────────────────────────

fn connector_type_from_str(s: &str) -> i32 {
    match s {
        "sqlite" => ConnectorType::SqliteDb as i32,
        "csv" => ConnectorType::CsvFolder as i32,
        "json" => ConnectorType::JsonFolder as i32,
        "postgres" => ConnectorType::Postgres as i32,
        "mysql" => ConnectorType::Mysql as i32,
        "odbc" => ConnectorType::Odbc as i32,
        "image" => ConnectorType::ImageFolder as i32,
        "document" => ConnectorType::DocumentFolder as i32,
        _ => ConnectorType::Unspecified as i32,
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

const TITLE_KEYS: &[&str] = &["filename", "file_name", "name", "title", "file_path"];

fn build_artifact_title(table: &str, entity_id: &str, columns: &BTreeMap<String, String>) -> String {
    for key in TITLE_KEYS {
        if let Some(val) = columns.get(*key) {
            if !val.is_empty() {
                let name: String = val.chars().take(120).collect();
                return name;
            }
        }
    }
    format!("{}/{}", table, entity_id)
}

fn build_artifact_summary(columns: &BTreeMap<String, String>, max_len: usize) -> String {
    if let Some(text) = columns.get("content_text") {
        if !text.is_empty() {
            return truncate_str(text, max_len);
        }
    }

    let mut parts = Vec::new();
    for (k, v) in columns {
        if k == "content_text" || v.is_empty() {
            continue;
        }
        parts.push(format!("{}: {}", k, truncate_str(v, 200)));
    }
    truncate_str(&parts.join(" | "), max_len)
}

fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let mut end = max_len;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

// ── Event builders ─────────────────────────────────────────────────────────

pub fn build_ingest_started_event(job: &IngestJob, node_id: &str) -> EventEnvelope {
    EventEnvelope {
        event_id: Uuid::new_v4().to_string(),
        r#type: EventType::IngestStarted as i32,
        node_id: Some(NodeId {
            value: node_id.to_string(),
        }),
        tenant_id: Some(TenantId {
            value: "public".to_string(),
        }),
        sensitivity: Sensitivity::Public as i32,
        payload: Some(event_envelope::Payload::IngestStarted(IngestStarted {
            ingest_id: job.ingest_id.clone(),
            source_id: job.source_id.clone(),
            connector_type: connector_type_from_str(&job.connector_type),
            source_profile_ref: None,
            started_at: Some(Timestamp { unix_ms: now_ms() }),
        })),
        ..Default::default()
    }
}

pub fn build_ingest_completed_event(
    job: &IngestJob,
    result: &IngestResult,
    node_id: &str,
) -> EventEnvelope {
    EventEnvelope {
        event_id: Uuid::new_v4().to_string(),
        r#type: EventType::IngestCompleted as i32,
        node_id: Some(NodeId {
            value: node_id.to_string(),
        }),
        tenant_id: Some(TenantId {
            value: "public".to_string(),
        }),
        sensitivity: Sensitivity::Public as i32,
        payload: Some(event_envelope::Payload::IngestCompleted(IngestCompleted {
            ingest_id: job.ingest_id.clone(),
            source_id: job.source_id.clone(),
            success: result.success,
            rows_ingested: result.rows_ingested,
            documents_created: result.documents_created,
            facts_created: result.facts_created,
            bytes_stored: result.bytes_stored,
            duration_ms: result.duration_ms,
            notes: String::new(),
        })),
        ..Default::default()
    }
}

// ── Main entry point ───────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub fn run_ingest(
    job: &IngestJob,
    connector: &dyn Connector,
    source_path: &Path,
    tables: &[String],
    config: &IngestConfig,
    cas: &CasStore,
    event_log: &mut EventLog,
    db_path: &Path,
    node_id: &str,
) -> anyhow::Result<IngestResult> {
    let start = Instant::now();

    let proj_conn =
        node_storage::sqlite_views::open_db(db_path).context("open projector database")?;

    let started_evt = build_ingest_started_event(job, node_id);
    let started_evt = event_log.append(started_evt)?;
    node_storage::projector::apply_event(&proj_conn, &started_evt)?;
    info!(ingest_id = %job.ingest_id, tables = ?tables, "ingest started");

    let mut total_rows: u64 = 0;
    let mut total_docs: u64 = 0;
    let mut total_bytes: u64 = 0;

    for table in tables {
        let mut offset = 0u64;
        let mut table_rows = 0u64;

        loop {
            if table_rows >= config.max_rows_per_table {
                break;
            }

            let remaining = config.max_rows_per_table - table_rows;
            let limit = config.batch_size.min(remaining);

            let batch = connector
                .ingest_batch(source_path, table, offset, limit)
                .with_context(|| format!("ingest_batch for table {table} at offset {offset}"))?;

            if batch.rows.is_empty() {
                break;
            }

            for row in &batch.rows {
                let json = serde_json::to_vec(&row.columns)?;
                let json_len = json.len() as u64;
                let hash_ref = cas.put_bytes("application/json", &json)?;

                let title = build_artifact_title(table, &row.entity_id, &row.columns);
                let summary = build_artifact_summary(&row.columns, 500);

                let artifact_id = format!("{}-{}-{}", job.ingest_id, table, row.entity_id);
                let artifact_event = EventEnvelope {
                    event_id: Uuid::new_v4().to_string(),
                    r#type: EventType::ArtifactPublished as i32,
                    node_id: Some(NodeId {
                        value: node_id.to_string(),
                    }),
                    tenant_id: Some(TenantId {
                        value: "public".to_string(),
                    }),
                    sensitivity: Sensitivity::Public as i32,
                    payload: Some(event_envelope::Payload::ArtifactPublished(
                        ArtifactPublished {
                            artifact_id,
                            artifact_type: ArtifactType::Document as i32,
                            version: 1,
                            title,
                            content_ref: Some(hash_ref),
                            shareable: false,
                            expires_unix_ms: 0,
                            summary,
                        },
                    )),
                    ..Default::default()
                };

                let artifact_event = event_log.append(artifact_event)?;
                node_storage::projector::apply_event(&proj_conn, &artifact_event)?;

                total_bytes += json_len;
                total_docs += 1;
            }

            let batch_len = batch.rows.len() as u64;
            offset += batch_len;
            table_rows += batch_len;

            debug!(table = %table, offset, table_rows, "batch ingested");
        }

        total_rows += table_rows;
    }

    let duration = start.elapsed();

    let result = IngestResult {
        ingest_id: job.ingest_id.clone(),
        source_id: job.source_id.clone(),
        success: true,
        rows_ingested: total_rows,
        documents_created: total_docs,
        facts_created: 0,
        bytes_stored: total_bytes,
        duration_ms: duration.as_millis() as u32,
    };

    let completed_evt = build_ingest_completed_event(job, &result, node_id);
    let completed_evt = event_log.append(completed_evt)?;
    node_storage::projector::apply_event(&proj_conn, &completed_evt)?;

    info!(
        ingest_id = %job.ingest_id,
        rows = total_rows,
        docs = total_docs,
        bytes = total_bytes,
        duration_ms = %result.duration_ms,
        "ingest completed"
    );

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use node_connectors::SQLiteConnector;
    use node_proto::events::event_envelope::Payload;
    use rusqlite::Connection;
    use tempfile::TempDir;

    fn create_test_fixture(dir: &Path) -> std::path::PathBuf {
        let db_path = dir.join("fixture.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "CREATE TABLE test_items (id INTEGER PRIMARY KEY, name TEXT, value REAL)",
            [],
        )
        .unwrap();
        for i in 0..5 {
            conn.execute(
                "INSERT INTO test_items VALUES (?1, ?2, ?3)",
                rusqlite::params![i, format!("item-{i}"), i as f64 * 1.5],
            )
            .unwrap();
        }
        db_path
    }

    fn setup_infra(dir: &Path) -> (CasStore, EventLog, std::path::PathBuf) {
        let cas_dir = dir.join("cas");
        let events_dir = dir.join("data");
        let proj_db = dir.join("projector.db");

        let cas = CasStore::open(&cas_dir).unwrap();
        let event_log = EventLog::open(&events_dir).unwrap();

        (cas, event_log, proj_db)
    }

    #[test]
    fn test_ingest_sqlite_table() {
        let tmp = TempDir::new().unwrap();
        let fixture_db = create_test_fixture(tmp.path());
        let (cas, mut event_log, proj_db) = setup_infra(tmp.path());

        let connector = SQLiteConnector::new("sqlite-test");
        let job = IngestJob {
            ingest_id: "ing-001".into(),
            source_id: "src-001".into(),
            connector_type: "sqlite".into(),
        };

        let result = run_ingest(
            &job,
            &connector,
            &fixture_db,
            &["test_items".to_string()],
            &IngestConfig::default(),
            &cas,
            &mut event_log,
            &proj_db,
            "node-test",
        )
        .unwrap();

        assert!(result.success);
        assert_eq!(result.rows_ingested, 5);
        assert_eq!(result.documents_created, 5);
        assert!(result.bytes_stored > 0);
    }

    #[test]
    fn test_ingest_respects_max_rows() {
        let tmp = TempDir::new().unwrap();
        let fixture_db = create_test_fixture(tmp.path());
        let (cas, mut event_log, proj_db) = setup_infra(tmp.path());

        let connector = SQLiteConnector::new("sqlite-test");
        let job = IngestJob {
            ingest_id: "ing-002".into(),
            source_id: "src-002".into(),
            connector_type: "sqlite".into(),
        };

        let config = IngestConfig {
            batch_size: 100,
            max_rows_per_table: 2,
        };

        let result = run_ingest(
            &job,
            &connector,
            &fixture_db,
            &["test_items".to_string()],
            &config,
            &cas,
            &mut event_log,
            &proj_db,
            "node-test",
        )
        .unwrap();

        assert!(result.success);
        assert_eq!(result.rows_ingested, 2);
        assert_eq!(result.documents_created, 2);
    }

    #[test]
    fn test_ingest_events_created() {
        let tmp = TempDir::new().unwrap();
        let fixture_db = create_test_fixture(tmp.path());
        let (cas, mut event_log, proj_db) = setup_infra(tmp.path());

        let connector = SQLiteConnector::new("sqlite-test");
        let job = IngestJob {
            ingest_id: "ing-003".into(),
            source_id: "src-003".into(),
            connector_type: "sqlite".into(),
        };

        let config = IngestConfig {
            batch_size: 100,
            max_rows_per_table: 3,
        };

        run_ingest(
            &job,
            &connector,
            &fixture_db,
            &["test_items".to_string()],
            &config,
            &cas,
            &mut event_log,
            &proj_db,
            "node-test",
        )
        .unwrap();

        let events = event_log.replay().unwrap();

        // IngestStarted + 3 ArtifactPublished + IngestCompleted = 5
        assert_eq!(events.len(), 5);

        assert_eq!(events[0].r#type, EventType::IngestStarted as i32);
        match &events[0].payload {
            Some(Payload::IngestStarted(is)) => {
                assert_eq!(is.ingest_id, "ing-003");
                assert_eq!(is.source_id, "src-003");
                assert_eq!(is.connector_type, ConnectorType::SqliteDb as i32);
            }
            other => panic!("expected IngestStarted, got {other:?}"),
        }

        for evt in events.iter().skip(1).take(3) {
            assert_eq!(evt.r#type, EventType::ArtifactPublished as i32);
            match &evt.payload {
                Some(Payload::ArtifactPublished(ap)) => {
                    assert_eq!(ap.artifact_type, ArtifactType::Document as i32);
                    assert!(ap.content_ref.is_some());
                }
                other => panic!("expected ArtifactPublished, got {other:?}"),
            }
        }

        assert_eq!(events[4].r#type, EventType::IngestCompleted as i32);
        match &events[4].payload {
            Some(Payload::IngestCompleted(ic)) => {
                assert!(ic.success);
                assert_eq!(ic.rows_ingested, 3);
                assert_eq!(ic.documents_created, 3);
            }
            other => panic!("expected IngestCompleted, got {other:?}"),
        }

        for event in &events {
            assert_eq!(
                event.node_id.as_ref().map(|n| n.value.as_str()),
                Some("node-test")
            );
            assert_eq!(
                event.tenant_id.as_ref().map(|t| t.value.as_str()),
                Some("public")
            );
            assert_eq!(event.sensitivity, Sensitivity::Public as i32);
        }
    }
}
