//! Ingestion pipelines: convert raw data into normalized Documents and Facts.
//!
//! Runs incremental, resumable ingestion jobs per approved SourceProfile.
//! Stores content in CAS, emits events, projects into SQLite views.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

/// Accumulates numeric stats for fact extraction.
#[derive(Default)]
struct NumericAccum {
    sum: f64,
    count: u64,
    min: Option<f64>,
    max: Option<f64>,
}

use anyhow::Context;
use tracing::{debug, info};
use uuid::Uuid;

use node_connectors::Connector;
use node_proto::common::{NodeId, Sensitivity, TenantId, Timestamp};
use node_proto::events::{
    event_envelope, ArtifactPublished, ArtifactType, ConnectorType, EntityRelationshipRecorded,
    EventEnvelope, EventType, IngestCompleted, IngestStarted,
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

/// Per-table mapping hints from SourceProfile. Used when present; otherwise infer.
#[derive(Debug, Clone, Default)]
pub struct TableMapping {
    pub entity_type: Option<String>,
    pub entity_key_col: Option<String>,
    pub timestamp_col: Option<String>,
    pub include_cols: Option<Vec<String>>,
    pub exclude_cols: Option<Vec<String>>,
}

/// Mapping rules per table. Key = table name.
pub type MappingHints = BTreeMap<String, TableMapping>;

/// Parse mapping rules from SourceProfile's mapping_rules_json.
pub fn parse_mapping_hints(json: &str) -> MappingHints {
    #[derive(serde::Deserialize)]
    struct TablesWrapper {
        tables: Option<BTreeMap<String, TableMappingSerde>>,
    }
    #[derive(serde::Deserialize)]
    struct TableMappingSerde {
        entity_type: Option<String>,
        entity_key_col: Option<String>,
        timestamp_col: Option<String>,
        include_cols: Option<Vec<String>>,
        exclude_cols: Option<Vec<String>>,
    }
    impl From<TableMappingSerde> for TableMapping {
        fn from(s: TableMappingSerde) -> Self {
            Self {
                entity_type: s.entity_type,
                entity_key_col: s.entity_key_col,
                timestamp_col: s.timestamp_col,
                include_cols: s.include_cols,
                exclude_cols: s.exclude_cols,
            }
        }
    }
    let w: TablesWrapper = serde_json::from_str(json).unwrap_or(TablesWrapper { tables: None });
    w.tables
        .unwrap_or_default()
        .into_iter()
        .map(|(k, v)| (k, v.into()))
        .collect()
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
        "onedrive" => ConnectorType::Onedrive as i32,
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

/// Infer entity_type from table name (best-effort). Use hint if provided.
fn infer_entity_type(table: &str, hint: Option<&TableMapping>) -> String {
    if let Some(h) = hint {
        if let Some(ref et) = h.entity_type {
            return et.clone();
        }
    }
    let t = table.trim().to_lowercase();
    if t.is_empty() {
        return "record".to_string();
    }
    if t.ends_with("ies") {
        format!("{}y", t.trim_end_matches("ies"))
    } else if t.ends_with('s') && !t.ends_with("ss") {
        t.trim_end_matches('s').to_string()
    } else {
        t
    }
}

fn try_parse_f64(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok()
}

/// Emit fact artifacts for table aggregates (row_count, sum/avg/min/max per numeric column).
#[allow(clippy::too_many_arguments)]
fn emit_table_facts(
    job: &IngestJob,
    table: &str,
    row_count: u64,
    numeric_cols: &BTreeMap<String, NumericAccum>,
    cas: &CasStore,
    event_log: &mut EventLog,
    proj_conn: &rusqlite::Connection, // Connection from sqlite_views::open_db
    node_id: &str,
) -> anyhow::Result<u64> {
    use node_storage::projector;
    let mut facts_created = 0u64;

    let dims = serde_json::json!({ "table": table }).to_string();
    let fact_id_base = format!("fact-{}-{}-", job.ingest_id, table);

    // row_count fact
    let value = serde_json::json!({ "count": row_count }).to_string();
    let content = format!(r#"{{"metric":"row_count","dimensions":{dims},"value":{value}}}"#);
    let content_bytes = content.as_bytes();
    let hash_ref = cas.put_bytes("application/json", content_bytes)?;
    let fid = format!("{fact_id_base}row_count");
    let evt = EventEnvelope {
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
                artifact_id: fid,
                artifact_type: ArtifactType::Fact as i32,
                version: 1,
                title: format!("{table} row_count"),
                content_ref: Some(hash_ref),
                shareable: false,
                expires_unix_ms: 0,
                summary: format!("{row_count} rows"),
                source_ref: job.source_id.clone(),
                table_name: table.to_string(),
                metric: "row_count".into(),
                value_json: value,
                dimensions_json: dims,
                ingest_id: job.ingest_id.clone(),
                ..Default::default()
            },
        )),
        ..Default::default()
    };
    let evt = event_log.append(evt)?;
    projector::apply_event(proj_conn, &evt)?;
    facts_created += 1;

    for (col, acc) in numeric_cols {
        if acc.count == 0 {
            continue;
        }
        let dims_col = serde_json::json!({ "table": table, "column": col }).to_string();
        let avg = if acc.count > 0 {
            acc.sum / acc.count as f64
        } else {
            0.0
        };
        let value = serde_json::json!({
            "sum": acc.sum,
            "count": acc.count,
            "avg": avg,
            "min": acc.min,
            "max": acc.max
        })
        .to_string();
        let metric_escaped = col.replace('\\', "\\\\").replace('"', "\\\"");
        let content =
            format!(r#"{{"metric":"{metric_escaped}","dimensions":{dims_col},"value":{value}}}"#);
        let content_bytes = content.as_bytes();
        let hash_ref = cas.put_bytes("application/json", content_bytes)?;
        let col_safe: String = col
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let fid = format!("{fact_id_base}{col_safe}");
        let evt = EventEnvelope {
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
                    artifact_id: fid,
                    artifact_type: ArtifactType::Fact as i32,
                    version: 1,
                    title: format!("{table}.{col}"),
                    content_ref: Some(hash_ref),
                    shareable: false,
                    expires_unix_ms: 0,
                    summary: format!("sum={}, avg={:.2}", acc.sum, avg),
                    source_ref: job.source_id.clone(),
                    table_name: table.to_string(),
                    metric: col.clone(),
                    value_json: value,
                    dimensions_json: dims_col,
                    ingest_id: job.ingest_id.clone(),
                    ..Default::default()
                },
            )),
            ..Default::default()
        };
        let evt = event_log.append(evt)?;
        projector::apply_event(proj_conn, &evt)?;
        facts_created += 1;
    }

    Ok(facts_created)
}

/// Prefer business key from columns. Use hint.entity_key_col if provided, else id/customer_id/etc, else entity_id.
fn infer_entity_key(row: &node_connectors::IngestRow, hint: Option<&TableMapping>) -> String {
    if let Some(h) = hint {
        if let Some(ref col) = h.entity_key_col {
            if let Some(v) = row.columns.get(col) {
                if !v.is_empty() {
                    return v.clone();
                }
            }
        }
    }
    const KEY_COLS: &[&str] = &["id", "invoice_id", "customer_id", "order_id", "product_id"];
    for col in KEY_COLS {
        if let Some(v) = row.columns.get(*col) {
            if !v.is_empty() {
                return v.clone();
            }
        }
    }
    row.entity_id.clone()
}

fn build_artifact_title(
    table: &str,
    entity_id: &str,
    columns: &BTreeMap<String, String>,
) -> String {
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

/// FK column -> target entity type for relationship inference
const FK_RELATIONSHIPS: &[(&str, &str)] = &[
    ("customer_id", "customer"),
    ("quote_id", "quote"),
    ("invoice_id", "invoice"),
    ("account_id", "account"),
    ("supplier_id", "supplier"),
    ("product_id", "product"),
    ("job_id", "job"),
    ("project_id", "project"),
];

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
    mapping_hints: Option<&MappingHints>,
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
    let mut total_facts: u64 = 0;
    let mut total_bytes: u64 = 0;

    for table in tables {
        let mut offset = 0u64;
        let mut table_rows = 0u64;
        let mut numeric_cols: BTreeMap<String, NumericAccum> = BTreeMap::new();

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
                for (col, val) in &row.columns {
                    if let Some(n) = try_parse_f64(val) {
                        let acc = numeric_cols.entry(col.clone()).or_default();
                        acc.sum += n;
                        acc.count += 1;
                        acc.min = Some(acc.min.map_or(n, |m| m.min(n)));
                        acc.max = Some(acc.max.map_or(n, |m| m.max(n)));
                    }
                }
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
                            document_subtype: "entity_card".into(),
                            entity_type: infer_entity_type(
                                table,
                                mapping_hints.and_then(|m| m.get(table)),
                            ),
                            entity_key: infer_entity_key(
                                row,
                                mapping_hints.and_then(|m| m.get(table)),
                            ),
                            source_ref: job.source_id.clone(),
                            table_name: table.to_string(),
                            entity_attributes_json: serde_json::to_string(&row.columns)
                                .unwrap_or_else(|_| "{}".into()),
                            ..Default::default()
                        },
                    )),
                    ..Default::default()
                };

                let artifact_event = event_log.append(artifact_event)?;
                node_storage::projector::apply_event(&proj_conn, &artifact_event)?;

                total_bytes += json_len;
                total_docs += 1;

                // Emit relationship events for FK columns
                let from_entity_id = format!(
                    "{}:{}",
                    infer_entity_type(table, mapping_hints.and_then(|m| m.get(table)),),
                    infer_entity_key(row, mapping_hints.and_then(|m| m.get(table)),),
                );
                for (fk_col, target_type) in FK_RELATIONSHIPS {
                    if let Some(ref to_key) = row.columns.get(*fk_col) {
                        if !to_key.is_empty() {
                            let to_entity_id = format!("{target_type}:{to_key}");
                            let rel_event = EventEnvelope {
                                event_id: Uuid::new_v4().to_string(),
                                r#type: EventType::EntityRelationshipRecorded as i32,
                                node_id: Some(NodeId {
                                    value: node_id.to_string(),
                                }),
                                tenant_id: Some(TenantId {
                                    value: "public".to_string(),
                                }),
                                sensitivity: Sensitivity::Public as i32,
                                payload: Some(event_envelope::Payload::EntityRelationshipRecorded(
                                    EntityRelationshipRecorded {
                                        from_entity_id: from_entity_id.clone(),
                                        to_entity_id: to_entity_id.clone(),
                                        relationship_type: format!("belongs_to_{target_type}"),
                                        source_id: job.source_id.clone(),
                                        table_name: table.to_string(),
                                    },
                                )),
                                ..Default::default()
                            };
                            let rel_event = event_log.append(rel_event)?;
                            node_storage::projector::apply_event(&proj_conn, &rel_event)?;
                        }
                    }
                }
            }

            let batch_len = batch.rows.len() as u64;
            offset += batch_len;
            table_rows += batch_len;

            debug!(table = %table, offset, table_rows, "batch ingested");
        }

        let facts = emit_table_facts(
            job,
            table,
            table_rows,
            &numeric_cols,
            cas,
            event_log,
            &proj_conn,
            node_id,
        )?;
        total_facts += facts;

        total_rows += table_rows;
    }

    let duration = start.elapsed();

    let result = IngestResult {
        ingest_id: job.ingest_id.clone(),
        source_id: job.source_id.clone(),
        success: true,
        rows_ingested: total_rows,
        documents_created: total_docs,
        facts_created: total_facts,
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
    use node_storage::sqlite_views;
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
            None,
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
            None,
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
            None,
        )
        .unwrap();

        let events = event_log.replay().unwrap();

        // IngestStarted + 3 ArtifactPublished (docs) + 3 ArtifactPublished (facts) + IngestCompleted = 8
        assert_eq!(events.len(), 8);

        assert_eq!(events[0].r#type, EventType::IngestStarted as i32);
        match &events[0].payload {
            Some(Payload::IngestStarted(is)) => {
                assert_eq!(is.ingest_id, "ing-003");
                assert_eq!(is.source_id, "src-003");
                assert_eq!(is.connector_type, ConnectorType::SqliteDb as i32);
            }
            other => panic!("expected IngestStarted, got {other:?}"),
        }

        let (doc_count, fact_count): (usize, usize) = events
            .iter()
            .filter_map(|e| {
                if e.r#type != EventType::ArtifactPublished as i32 {
                    return None;
                }
                match &e.payload {
                    Some(Payload::ArtifactPublished(ap)) => {
                        if ap.artifact_type == ArtifactType::Document as i32 {
                            Some((1, 0))
                        } else if ap.artifact_type == ArtifactType::Fact as i32 {
                            Some((0, 1))
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            })
            .fold((0, 0), |(d, f), (dd, ff)| (d + dd, f + ff));
        assert_eq!(doc_count, 3);
        assert_eq!(fact_count, 3);

        assert_eq!(events[7].r#type, EventType::IngestCompleted as i32);
        match &events[7].payload {
            Some(Payload::IngestCompleted(ic)) => {
                assert!(ic.success);
                assert_eq!(ic.rows_ingested, 3);
                assert_eq!(ic.documents_created, 3);
                assert_eq!(ic.facts_created, 3);
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

    #[test]
    fn test_entity_cards_and_facts_in_views() {
        let tmp = TempDir::new().unwrap();
        let fixture_db = create_test_fixture(tmp.path());
        let (cas, mut event_log, proj_db) = setup_infra(tmp.path());

        let connector = SQLiteConnector::new("sqlite-test");
        let job = IngestJob {
            ingest_id: "ing-ecf".into(),
            source_id: "src-ecf".into(),
            connector_type: "sqlite".into(),
        };

        run_ingest(
            &job,
            &connector,
            &fixture_db,
            &["test_items".to_string()],
            &IngestConfig::default(),
            &cas,
            &mut event_log,
            &proj_db,
            "node-test",
            None,
        )
        .unwrap();

        let conn = sqlite_views::open_db(&proj_db).unwrap();

        let doc_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM documents_view WHERE document_type = 'entity_card'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(doc_count, 5, "expect 5 entity cards for 5 rows");

        let entity_types: Vec<String> = conn
            .prepare(
                "SELECT DISTINCT entity_type FROM documents_view WHERE document_type = 'entity_card'",
            )
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert!(
            entity_types.iter().any(|t| t == "test_item"),
            "expect entity_type test_item (from table test_items)"
        );

        let fact_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM facts_view", [], |row| row.get(0))
            .unwrap();
        assert!(
            fact_count >= 2,
            "expect at least row_count + 1 numeric fact"
        );

        let row_count_facts: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM facts_view WHERE metric = 'row_count'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(row_count_facts, 1);
    }

    #[test]
    fn test_entity_graph_from_ingest() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        let db_path = dir.join("invoices.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE invoices (id TEXT PRIMARY KEY, customer_id TEXT, amount REAL);
             INSERT INTO invoices VALUES ('inv-1', 'cust-a', 1200.50);
             INSERT INTO invoices VALUES ('inv-2', 'cust-a', 800.00);
             INSERT INTO invoices VALUES ('inv-3', 'cust-b', 2500.00);",
        )
        .unwrap();
        drop(conn);

        let (cas, mut event_log, proj_db) = setup_infra(dir);
        let connector = SQLiteConnector::new("sqlite-entity");
        let job = IngestJob {
            ingest_id: "ing-entity".into(),
            source_id: "src-entity".into(),
            connector_type: "sqlite".into(),
        };

        let mut hints = MappingHints::new();
        hints.insert(
            "invoices".to_string(),
            TableMapping {
                entity_type: Some("invoice".to_string()),
                entity_key_col: Some("id".to_string()),
                ..Default::default()
            },
        );

        run_ingest(
            &job,
            &connector,
            &db_path,
            &["invoices".to_string()],
            &IngestConfig::default(),
            &cas,
            &mut event_log,
            &proj_db,
            "node-test",
            Some(&hints),
        )
        .unwrap();

        let conn = sqlite_views::open_db(&proj_db).unwrap();

        let entity_card_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM entity_cards_view", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(
            entity_card_count, 3,
            "expect 3 entity cards for 3 invoice rows"
        );

        let rel_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM entity_relationships_view",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            rel_count >= 3,
            "expect at least 3 relationships (invoice->customer for each row)"
        );

        let invoice_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM invoices_view", [], |row| row.get(0))
            .unwrap();
        assert_eq!(invoice_count, 3);

        let attrs: String = conn
            .query_row(
                "SELECT attributes_json FROM entity_cards_view WHERE entity_id = 'invoice:inv-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            attrs.contains("1200") || attrs.contains("1200.5"),
            "attributes should contain amount"
        );
        assert!(attrs.contains("cust-a"));

        let (from_id, to_id): (String, String) = conn
            .query_row(
                "SELECT from_entity_id, to_entity_id FROM entity_relationships_view WHERE from_entity_id = 'invoice:inv-1' LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(from_id, "invoice:inv-1");
        assert_eq!(to_id, "customer:cust-a");
    }
}
