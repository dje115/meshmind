//! Ingestion pipelines: convert raw data into normalized Documents and Facts.
//!
//! Runs incremental, resumable ingestion jobs per approved SourceProfile.
//! Stores content in CAS, emits events, projects into SQLite views.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use node_ai::InferenceBackend;

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
use node_extraction::{extract_entities, extract_entities_with_llm, ExtractionConfig};
use node_proto::common::{NodeId, Sensitivity, TenantId, Timestamp};
use node_proto::events::{
    event_envelope, ArtifactPublished, ArtifactType, ConnectorType, EntityRelationshipRecorded,
    EventEnvelope, EventType, ExtractedEntityRecorded, IngestCompleted, IngestStarted,
};
use node_storage::cas::CasStore;
use node_storage::event_log::EventLog;

// ── Configuration ──────────────────────────────────────────────────────────

pub struct IngestConfig {
    pub batch_size: u64,
    pub max_rows_per_table: u64,
    /// Entity extraction config. When backend is provided and enable_llm_entity_extraction,
    /// LLM-assisted extraction may run for long chunks with few rule-based entities.
    pub extraction_config: ExtractionConfig,
}

impl Default for IngestConfig {
    fn default() -> Self {
        Self {
            batch_size: 100,
            max_rows_per_table: 10_000,
            extraction_config: ExtractionConfig::default(),
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

/// Default summary cap: 800–1000 chars to retain more context than the old 500-char limit.
const SUMMARY_CAP: usize = 900;

/// Metadata keys to include as prefix when building document summaries.
const SUMMARY_METADATA_KEYS: &[&str] = &["source_file", "page_number"];

fn build_artifact_summary(columns: &BTreeMap<String, String>, max_len: usize) -> String {
    let cap = max_len.clamp(800, SUMMARY_CAP);
    // Prefer chunk_text for document chunks (canonical full chunk); fallback to content_text
    let main_text = columns
        .get("chunk_text")
        .or_else(|| columns.get("content_text"))
        .filter(|s| !s.is_empty());
    if let Some(t) = main_text {
        let main = truncate_str(t, cap);
        // Include detected metadata when available (e.g. source_file, page_number)
        let meta_parts: Vec<String> = SUMMARY_METADATA_KEYS
            .iter()
            .filter_map(|k| columns.get(*k))
            .filter(|v| !v.is_empty())
            .map(|v| format!("[{}]", truncate_str(v, 80)))
            .collect();
        let meta_prefix = if meta_parts.is_empty() {
            String::new()
        } else {
            format!("{} ", meta_parts.join(" "))
        };
        let combined = format!("{meta_prefix}{main}");
        return truncate_str(&combined, cap);
    }
    // Fallback for entity cards etc.: key-value pairs from other columns
    let mut parts = Vec::new();
    for (k, v) in columns {
        if k == "content_text" || k == "chunk_text" || v.is_empty() {
            continue;
        }
        parts.push(format!("{}: {}", k, truncate_str(v, 200)));
    }
    truncate_str(&parts.join(" | "), cap)
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
    backend: Option<Arc<dyn InferenceBackend>>,
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
                let is_document_chunk = row.columns.contains_key("chunk_index")
                    && row.columns.contains_key("document_id");
                let summary = build_artifact_summary(&row.columns, SUMMARY_CAP);

                let (doc_subtype, entity_type, entity_key) = if is_document_chunk {
                    (
                        "document_chunk".into(),
                        "document".into(),
                        row.columns
                            .get("document_id")
                            .cloned()
                            .unwrap_or_else(|| row.entity_id.clone()),
                    )
                } else {
                    (
                        "entity_card".into(),
                        infer_entity_type(table, mapping_hints.and_then(|m| m.get(table))),
                        infer_entity_key(row, mapping_hints.and_then(|m| m.get(table))),
                    )
                };

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
                            document_subtype: doc_subtype,
                            entity_type,
                            entity_key,
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

                // Extract entities from document chunks and emit ExtractedEntityRecorded
                if is_document_chunk {
                    let chunk_text = row
                        .columns
                        .get("chunk_text")
                        .or_else(|| row.columns.get("content_text"))
                        .map(String::as_str)
                        .unwrap_or("");
                    let document_id = row
                        .columns
                        .get("document_id")
                        .map(String::as_str)
                        .unwrap_or("");
                    let chunk_index: i32 = row
                        .columns
                        .get("chunk_index")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    if !chunk_text.is_empty() && !document_id.is_empty() {
                        let result = if config.extraction_config.enable_llm_entity_extraction
                            && backend.is_some()
                        {
                            let rt = tokio::runtime::Runtime::new()
                                .context("create tokio runtime for LLM extraction")?;
                            rt.block_on(extract_entities_with_llm(
                                backend.as_ref(),
                                chunk_text,
                                &config.extraction_config,
                            ))
                        } else {
                            extract_entities(chunk_text)
                        };
                        for entity in result.entities {
                            let rel_event = EventEnvelope {
                                event_id: Uuid::new_v4().to_string(),
                                r#type: EventType::ExtractedEntityRecorded as i32,
                                node_id: Some(NodeId {
                                    value: node_id.to_string(),
                                }),
                                tenant_id: Some(TenantId {
                                    value: "public".to_string(),
                                }),
                                sensitivity: Sensitivity::Public as i32,
                                payload: Some(event_envelope::Payload::ExtractedEntityRecorded(
                                    ExtractedEntityRecorded {
                                        entity_id: entity.entity_id(),
                                        entity_type: entity.entity_type,
                                        entity_value: entity.entity_value,
                                        normalized_value: entity.normalized_value,
                                        source_document_id: document_id.to_string(),
                                        chunk_index,
                                        confidence: entity.confidence,
                                        extraction_method: entity.extraction_method.to_string(),
                                    },
                                )),
                                ..Default::default()
                            };
                            let rel_event = event_log.append(rel_event)?;
                            node_storage::projector::apply_event(&proj_conn, &rel_event)?;
                        }
                    }
                }

                // Emit relationship events for FK columns (skip for document chunks)
                if !is_document_chunk {
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
                                    payload: Some(
                                        event_envelope::Payload::EntityRelationshipRecorded(
                                            EntityRelationshipRecorded {
                                                from_entity_id: from_entity_id.clone(),
                                                to_entity_id: to_entity_id.clone(),
                                                relationship_type: format!(
                                                    "belongs_to_{target_type}"
                                                ),
                                                source_id: job.source_id.clone(),
                                                table_name: table.to_string(),
                                            },
                                        ),
                                    ),
                                    ..Default::default()
                                };
                                let rel_event = event_log.append(rel_event)?;
                                node_storage::projector::apply_event(&proj_conn, &rel_event)?;
                            }
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
            extraction_config: ExtractionConfig::default(),
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
            extraction_config: ExtractionConfig::default(),
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

    /// Integration test: ingest multi-chunk document, verify chunk creation,
    /// phrase in middle is searchable, and ask pipeline retrieves chunk evidence.
    #[test]
    fn test_document_chunk_ingest_search_and_ask_retrieval() {
        use node_connectors::DocumentConnector;
        use node_storage::search;
        use std::fs;

        let tmp = TempDir::new().unwrap();
        let doc_dir = tmp.path().join("docs");
        fs::create_dir_all(&doc_dir).unwrap();

        // Build a ~3500 char document so we get 3 chunks. Put unique phrase in middle (chunk 1).
        let mut content = "Introduction. ".repeat(100);
        content.push_str(" UNIQUE_MIDPHRASE_Q7R9 ");
        content.push_str(&"Conclusion and details. ".repeat(100));

        fs::write(doc_dir.join("report.txt"), content.as_bytes()).unwrap();

        let (cas, mut event_log, proj_db) = setup_infra(tmp.path());
        let connector = DocumentConnector::new("doc-chunk-test");
        let job = IngestJob {
            ingest_id: "ing-doc".into(),
            source_id: "src-doc".into(),
            connector_type: "document".into(),
        };

        run_ingest(
            &job,
            &connector,
            &doc_dir,
            &["documents".to_string()],
            &IngestConfig::default(),
            &cas,
            &mut event_log,
            &proj_db,
            "node-test",
            None,
            None,
        )
        .unwrap();

        let conn = node_storage::sqlite_views::open_db(&proj_db).unwrap();

        // 1. Verify chunk creation
        let chunk_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM documents_view WHERE document_type = 'document_chunk'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            chunk_count >= 2,
            "expect multiple chunks for large doc, got {chunk_count}"
        );

        // 2. Verify phrase in middle of document is searchable
        let doc_hits = search::search_documents_fts(&conn, "UNIQUE_MIDPHRASE_Q7R9", 10).unwrap();
        assert!(
            !doc_hits.is_empty(),
            "phrase in middle of document should be searchable via documents_fts"
        );
        assert!(doc_hits[0].chunk_text.contains("UNIQUE_MIDPHRASE_Q7R9"));

        // 3. Verify ask pipeline (search_all) retrieves chunk evidence
        let all_hits = search::search_all(&conn, "UNIQUE_MIDPHRASE_Q7R9", 10).unwrap();
        let chunk_evidence: Vec<_> = all_hits
            .iter()
            .filter(|h| h.hit_type == "document_chunk")
            .collect();
        assert!(
            !chunk_evidence.is_empty(),
            "ask pipeline should retrieve document chunk as context candidate"
        );
        assert!(
            chunk_evidence[0].summary.contains("UNIQUE_MIDPHRASE_Q7R9"),
            "chunk_text should be in summary for context bullets"
        );
    }

    /// Integration test: ingest document with person, company, email, money, invoice number;
    /// verify entities extracted and queryable.
    #[test]
    fn test_document_entity_extraction_integration() {
        use node_connectors::DocumentConnector;
        use node_storage::search;
        use std::fs;

        let tmp = TempDir::new().unwrap();
        let doc_dir = tmp.path().join("docs");
        fs::create_dir_all(&doc_dir).unwrap();

        let content = "Invoice from Acme Corporation Ltd.
Contact: Mr. John Smith, john.smith@example.com, +44 20 7123 4567
Total: £1,234.56
Invoice No: INV-2024-001";

        fs::write(doc_dir.join("invoice.txt"), content.as_bytes()).unwrap();

        let (cas, mut event_log, proj_db) = setup_infra(tmp.path());
        let connector = DocumentConnector::new("doc-entity-test");
        let job = IngestJob {
            ingest_id: "ing-entity".into(),
            source_id: "src-entity".into(),
            connector_type: "document".into(),
        };

        run_ingest(
            &job,
            &connector,
            &doc_dir,
            &["documents".to_string()],
            &IngestConfig::default(),
            &cas,
            &mut event_log,
            &proj_db,
            "node-test",
            None,
            None,
        )
        .unwrap();

        let conn = node_storage::sqlite_views::open_db(&proj_db).unwrap();

        let people = search::list_entities_by_type(&conn, "person", 20).unwrap();
        assert!(
            people
                .iter()
                .any(|e| e.normalized_value.contains("john") || e.entity_value.contains("John")),
            "expect at least one person extracted"
        );

        let companies = search::list_entities_by_type(&conn, "company", 20).unwrap();
        assert!(
            companies
                .iter()
                .any(|e| e.normalized_value.contains("acme") || e.entity_value.contains("Acme")),
            "expect at least one company extracted"
        );
        // Use actual normalized_value from DB for list_documents_for_entity
        let company_norm = companies
            .iter()
            .find(|e| e.normalized_value.contains("acme") || e.entity_value.contains("Acme"))
            .map(|e| e.normalized_value.clone())
            .unwrap_or_else(|| "acme corp ltd".into());

        let emails = search::list_entities_by_type(&conn, "email", 20).unwrap();
        assert!(
            emails
                .iter()
                .any(|e| e.entity_value.contains("john.smith@example.com")),
            "expect email extracted"
        );

        let money = search::list_entities_by_type(&conn, "money", 20).unwrap();
        assert!(!money.is_empty(), "expect money amount extracted");

        let inv = search::list_entities_by_type(&conn, "invoice_number", 20).unwrap();
        assert!(
            inv.iter().any(|e| e.entity_value.contains("INV-2024")),
            "expect invoice number extracted"
        );

        // list_documents_for_entity: use normalized_value
        let docs_email =
            search::list_documents_for_entity(&conn, "john.smith@example.com", Some("email"), 10)
                .unwrap();
        assert!(
            !docs_email.is_empty(),
            "documents_for_entity should return invoice.txt for email"
        );
        let docs_company =
            search::list_documents_for_entity(&conn, &company_norm, Some("company"), 10).unwrap();
        assert!(
            !docs_company.is_empty(),
            "documents_for_entity should return invoice.txt for company (normalized: {})",
            company_norm
        );
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
            None,
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
