//! Connector trait and implementations for data source introspection and ingestion.
//!
//! Connectors:
//! - SQLiteConnector: discover/inspect/ingest from SQLite databases
//! - CsvFolderConnector: discover/inspect/ingest from CSV directories
//! - JsonFolderConnector: discover/inspect/ingest from JSON directories

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

use anyhow::{bail, Context};
use rusqlite::types::ValueRef;
use rusqlite::Connection;
use tracing::debug;

// ── Data types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SchemaColumn {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub is_primary_key: bool,
}

#[derive(Debug, Clone)]
pub struct TableInfo {
    pub table_name: String,
    pub columns: Vec<SchemaColumn>,
    pub row_count_estimate: u64,
}

#[derive(Debug, Clone)]
pub struct IngestRow {
    pub entity_id: String,
    pub columns: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct IngestBatchResult {
    pub table_name: String,
    pub rows: Vec<IngestRow>,
    pub offset: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnClassResult {
    pub classification: String,
    pub is_pii: bool,
    pub is_secret: bool,
    pub suggested_sensitivity: i32,
}

// ── Connector trait ─────────────────────────────────────────────────────────

pub trait Connector: Send + Sync {
    fn id(&self) -> &str;
    fn inspect_schema(&self, path: &Path) -> anyhow::Result<Vec<TableInfo>>;
    fn ingest_batch(
        &self,
        path: &Path,
        table: &str,
        offset: u64,
        limit: u64,
    ) -> anyhow::Result<IngestBatchResult>;
}

// ── SQLiteConnector ─────────────────────────────────────────────────────────

pub struct SQLiteConnector {
    id: String,
}

impl SQLiteConnector {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }
}

fn sqlite_value_to_string(v: ValueRef<'_>) -> String {
    match v {
        ValueRef::Null => String::new(),
        ValueRef::Integer(i) => i.to_string(),
        ValueRef::Real(f) => f.to_string(),
        ValueRef::Text(b) => String::from_utf8_lossy(b).into_owned(),
        ValueRef::Blob(b) => format!("<{} bytes>", b.len()),
    }
}

impl Connector for SQLiteConnector {
    fn id(&self) -> &str {
        &self.id
    }

    fn inspect_schema(&self, path: &Path) -> anyhow::Result<Vec<TableInfo>> {
        let conn = Connection::open(path).context("open SQLite database")?;
        let mut stmt = conn.prepare(
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
        )?;
        let table_names: Vec<String> = stmt
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;

        let mut tables = Vec::new();
        for table_name in table_names {
            debug!(table = %table_name, "inspecting SQLite table");

            let mut col_stmt = conn.prepare(&format!("PRAGMA table_info(\"{}\")", table_name))?;
            let columns: Vec<SchemaColumn> = col_stmt
                .query_map([], |row| {
                    let notnull: i32 = row.get(3)?;
                    let pk: i32 = row.get(5)?;
                    Ok(SchemaColumn {
                        name: row.get(1)?,
                        data_type: row.get::<_, String>(2).unwrap_or_default(),
                        nullable: notnull == 0,
                        is_primary_key: pk != 0,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;

            let count: i64 = conn.query_row(
                &format!("SELECT COUNT(*) FROM \"{}\"", table_name),
                [],
                |row| row.get(0),
            )?;

            tables.push(TableInfo {
                table_name,
                columns,
                row_count_estimate: count as u64,
            });
        }

        Ok(tables)
    }

    fn ingest_batch(
        &self,
        path: &Path,
        table: &str,
        offset: u64,
        limit: u64,
    ) -> anyhow::Result<IngestBatchResult> {
        let conn = Connection::open(path).context("open SQLite database")?;
        let sql = format!(
            "SELECT * FROM \"{}\" LIMIT {} OFFSET {}",
            table, limit, offset
        );
        let mut stmt = conn.prepare(&sql)?;
        let col_names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();

        let mut rows = Vec::new();
        let mut qr = stmt.query([])?;
        let mut row_idx = 0u64;
        while let Some(row) = qr.next()? {
            let entity_id = format!("{}", offset + row_idx);
            let mut columns = BTreeMap::new();
            for (i, name) in col_names.iter().enumerate() {
                columns.insert(name.clone(), sqlite_value_to_string(row.get_ref(i)?));
            }
            rows.push(IngestRow { entity_id, columns });
            row_idx += 1;
        }

        Ok(IngestBatchResult {
            table_name: table.to_string(),
            rows,
            offset,
        })
    }
}

// ── CsvFolderConnector ──────────────────────────────────────────────────────

pub struct CsvFolderConnector {
    id: String,
}

impl CsvFolderConnector {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }
}

fn parse_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();

    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    current.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            } else {
                current.push(c);
            }
        } else if c == '"' {
            in_quotes = true;
        } else if c == ',' {
            fields.push(std::mem::take(&mut current));
        } else {
            current.push(c);
        }
    }
    fields.push(current);
    fields
}

impl Connector for CsvFolderConnector {
    fn id(&self) -> &str {
        &self.id
    }

    fn inspect_schema(&self, path: &Path) -> anyhow::Result<Vec<TableInfo>> {
        let entries = fs::read_dir(path).context("read CSV directory")?;
        let mut tables = Vec::new();

        for entry in entries {
            let entry = entry?;
            let file_path = entry.path();
            if file_path.extension().and_then(|e| e.to_str()) != Some("csv") {
                continue;
            }
            let table_name = file_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();

            debug!(table = %table_name, "inspecting CSV file");

            let file = fs::File::open(&file_path)?;
            let reader = BufReader::new(file);
            let mut lines = reader.lines();

            let headers = match lines.next() {
                Some(Ok(line)) => parse_csv_line(&line),
                _ => continue,
            };

            let row_count = lines
                .filter(|l| l.as_ref().map(|s| !s.trim().is_empty()).unwrap_or(false))
                .count() as u64;

            let columns = headers
                .into_iter()
                .map(|h| SchemaColumn {
                    name: h.trim().to_string(),
                    data_type: "TEXT".to_string(),
                    nullable: true,
                    is_primary_key: false,
                })
                .collect();

            tables.push(TableInfo {
                table_name,
                columns,
                row_count_estimate: row_count,
            });
        }

        Ok(tables)
    }

    fn ingest_batch(
        &self,
        path: &Path,
        table: &str,
        offset: u64,
        limit: u64,
    ) -> anyhow::Result<IngestBatchResult> {
        let file_path = path.join(format!("{}.csv", table));
        if !file_path.exists() {
            bail!("CSV file not found: {}", file_path.display());
        }

        let file = fs::File::open(&file_path)?;
        let reader = BufReader::new(file);
        let mut lines = reader.lines();

        let headers = match lines.next() {
            Some(Ok(line)) => parse_csv_line(&line),
            _ => bail!("CSV file is empty or has no header"),
        };

        let mut rows = Vec::new();
        let mut effective_idx = 0u64;

        for line_result in lines {
            let line = line_result?;
            if line.trim().is_empty() {
                continue;
            }
            if effective_idx < offset {
                effective_idx += 1;
                continue;
            }
            if effective_idx >= offset + limit {
                break;
            }
            let fields = parse_csv_line(&line);
            let mut columns = BTreeMap::new();
            for (i, header) in headers.iter().enumerate() {
                columns.insert(
                    header.trim().to_string(),
                    fields.get(i).cloned().unwrap_or_default(),
                );
            }
            rows.push(IngestRow {
                entity_id: format!("{}", effective_idx),
                columns,
            });
            effective_idx += 1;
        }

        Ok(IngestBatchResult {
            table_name: table.to_string(),
            rows,
            offset,
        })
    }
}

// ── JsonFolderConnector ─────────────────────────────────────────────────────

pub struct JsonFolderConnector {
    id: String,
}

impl JsonFolderConnector {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }
}

fn read_json_objects(path: &Path) -> anyhow::Result<Vec<serde_json::Value>> {
    let content = fs::read_to_string(path).context("read JSON file")?;
    let trimmed = content.trim();

    if trimmed.starts_with('[') {
        let arr: Vec<serde_json::Value> = serde_json::from_str(trimmed)?;
        return Ok(arr);
    }

    let mut objects = Vec::new();
    for line in trimmed.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        objects.push(serde_json::from_str(line)?);
    }
    Ok(objects)
}

impl Connector for JsonFolderConnector {
    fn id(&self) -> &str {
        &self.id
    }

    fn inspect_schema(&self, path: &Path) -> anyhow::Result<Vec<TableInfo>> {
        let entries = fs::read_dir(path).context("read JSON directory")?;
        let mut tables = Vec::new();

        for entry in entries {
            let entry = entry?;
            let file_path = entry.path();
            if file_path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let table_name = file_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();

            debug!(table = %table_name, "inspecting JSON file");

            let objects = read_json_objects(&file_path)?;
            let row_count = objects.len() as u64;

            let columns = if let Some(first) = objects.first() {
                if let Some(obj) = first.as_object() {
                    obj.keys()
                        .map(|k| SchemaColumn {
                            name: k.clone(),
                            data_type: "JSON".to_string(),
                            nullable: true,
                            is_primary_key: false,
                        })
                        .collect()
                } else {
                    vec![]
                }
            } else {
                vec![]
            };

            tables.push(TableInfo {
                table_name,
                columns,
                row_count_estimate: row_count,
            });
        }

        Ok(tables)
    }

    fn ingest_batch(
        &self,
        path: &Path,
        table: &str,
        offset: u64,
        limit: u64,
    ) -> anyhow::Result<IngestBatchResult> {
        let file_path = path.join(format!("{}.json", table));
        if !file_path.exists() {
            bail!("JSON file not found: {}", file_path.display());
        }

        let objects = read_json_objects(&file_path)?;
        let start = offset as usize;
        let end = std::cmp::min(start + limit as usize, objects.len());
        let mut rows = Vec::new();

        for (i, obj) in objects[start..end].iter().enumerate() {
            let mut columns = BTreeMap::new();
            if let Some(map) = obj.as_object() {
                for (k, v) in map {
                    let val = match v {
                        serde_json::Value::String(s) => s.clone(),
                        serde_json::Value::Null => String::new(),
                        other => other.to_string(),
                    };
                    columns.insert(k.clone(), val);
                }
            }
            rows.push(IngestRow {
                entity_id: format!("{}", start + i),
                columns,
            });
        }

        Ok(IngestBatchResult {
            table_name: table.to_string(),
            rows,
            offset,
        })
    }
}

// ── Column classifier ───────────────────────────────────────────────────────

const PII_PATTERNS: &[&str] = &[
    "email",
    "phone",
    "address",
    "name",
    "dob",
    "date_of_birth",
    "ssn",
    "social_security",
    "iban",
    "sort_code",
    "card_number",
];

const SECRET_PATTERNS: &[&str] = &["api_key", "token", "password", "secret", "credential"];

pub fn classify_column(column_name: &str) -> ColumnClassResult {
    let lower = column_name.to_lowercase();

    for pattern in PII_PATTERNS {
        if lower.contains(pattern) {
            return ColumnClassResult {
                classification: "pii".to_string(),
                is_pii: true,
                is_secret: false,
                suggested_sensitivity: 3,
            };
        }
    }

    for pattern in SECRET_PATTERNS {
        if lower.contains(pattern) {
            return ColumnClassResult {
                classification: "secret".to_string(),
                is_pii: false,
                is_secret: true,
                suggested_sensitivity: 3,
            };
        }
    }

    ColumnClassResult {
        classification: "normal".to_string(),
        is_pii: false,
        is_secret: false,
        suggested_sensitivity: 1,
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::NamedTempFile;

    fn create_test_sqlite() -> NamedTempFile {
        let tmp = NamedTempFile::new().unwrap();
        let conn = Connection::open(tmp.path()).unwrap();
        conn.execute_batch(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL, email TEXT);
             INSERT INTO users VALUES (1, 'Alice', 'alice@example.com');
             INSERT INTO users VALUES (2, 'Bob', 'bob@example.com');
             INSERT INTO users VALUES (3, 'Charlie', 'charlie@example.com');",
        )
        .unwrap();
        tmp
    }

    #[test]
    fn test_sqlite_inspect() {
        let tmp = create_test_sqlite();
        let c = SQLiteConnector::new("sqlite-test");
        let tables = c.inspect_schema(tmp.path()).unwrap();

        assert_eq!(tables.len(), 1);
        let t = &tables[0];
        assert_eq!(t.table_name, "users");
        assert_eq!(t.row_count_estimate, 3);
        assert_eq!(t.columns.len(), 3);

        assert_eq!(t.columns[0].name, "id");
        assert!(t.columns[0].is_primary_key);
        assert_eq!(t.columns[0].data_type, "INTEGER");

        assert_eq!(t.columns[1].name, "name");
        assert!(!t.columns[1].nullable);

        assert_eq!(t.columns[2].name, "email");
        assert!(t.columns[2].nullable);
    }

    #[test]
    fn test_sqlite_ingest() {
        let tmp = create_test_sqlite();
        let c = SQLiteConnector::new("sqlite-test");

        let batch = c.ingest_batch(tmp.path(), "users", 0, 2).unwrap();
        assert_eq!(batch.table_name, "users");
        assert_eq!(batch.offset, 0);
        assert_eq!(batch.rows.len(), 2);
        assert_eq!(batch.rows[0].columns["name"], "Alice");
        assert_eq!(batch.rows[1].columns["name"], "Bob");

        let batch2 = c.ingest_batch(tmp.path(), "users", 2, 10).unwrap();
        assert_eq!(batch2.rows.len(), 1);
        assert_eq!(batch2.rows[0].columns["name"], "Charlie");
    }

    #[test]
    fn test_csv_inspect() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("sales.csv"),
            "id,product,amount\n1,Widget,9.99\n2,Gadget,19.99\n",
        )
        .unwrap();

        let c = CsvFolderConnector::new("csv-test");
        let tables = c.inspect_schema(dir.path()).unwrap();

        assert_eq!(tables.len(), 1);
        let t = &tables[0];
        assert_eq!(t.table_name, "sales");
        assert_eq!(t.row_count_estimate, 2);
        assert_eq!(t.columns.len(), 3);
        assert_eq!(t.columns[0].name, "id");
        assert_eq!(t.columns[1].name, "product");
        assert_eq!(t.columns[2].name, "amount");
    }

    #[test]
    fn test_csv_ingest() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("sales.csv"),
            "id,product,amount\n1,Widget,9.99\n2,Gadget,19.99\n3,Doohickey,4.99\n",
        )
        .unwrap();

        let c = CsvFolderConnector::new("csv-test");
        let batch = c.ingest_batch(dir.path(), "sales", 0, 2).unwrap();
        assert_eq!(batch.rows.len(), 2);
        assert_eq!(batch.rows[0].columns["product"], "Widget");
        assert_eq!(batch.rows[1].columns["product"], "Gadget");

        let batch2 = c.ingest_batch(dir.path(), "sales", 2, 10).unwrap();
        assert_eq!(batch2.rows.len(), 1);
        assert_eq!(batch2.rows[0].columns["product"], "Doohickey");
    }

    #[test]
    fn test_json_inspect() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("events.json"),
            "{\"id\":1,\"type\":\"click\",\"ts\":1000}\n{\"id\":2,\"type\":\"view\",\"ts\":2000}\n",
        )
        .unwrap();

        let c = JsonFolderConnector::new("json-test");
        let tables = c.inspect_schema(dir.path()).unwrap();

        assert_eq!(tables.len(), 1);
        let t = &tables[0];
        assert_eq!(t.table_name, "events");
        assert_eq!(t.row_count_estimate, 2);
        assert!(t.columns.len() >= 3);
    }

    #[test]
    fn test_json_ingest() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("events.json"),
            "{\"id\":1,\"kind\":\"click\"}\n{\"id\":2,\"kind\":\"view\"}\n{\"id\":3,\"kind\":\"scroll\"}\n",
        )
        .unwrap();

        let c = JsonFolderConnector::new("json-test");
        let batch = c.ingest_batch(dir.path(), "events", 0, 2).unwrap();
        assert_eq!(batch.rows.len(), 2);
        assert_eq!(batch.rows[0].columns["kind"], "click");
        assert_eq!(batch.rows[1].columns["kind"], "view");

        let batch2 = c.ingest_batch(dir.path(), "events", 2, 10).unwrap();
        assert_eq!(batch2.rows.len(), 1);
        assert_eq!(batch2.rows[0].columns["kind"], "scroll");
    }

    #[test]
    fn test_classify_pii() {
        let r = classify_column("email");
        assert!(r.is_pii);
        assert!(!r.is_secret);
        assert_eq!(r.classification, "pii");
        assert_eq!(r.suggested_sensitivity, 3);

        let r = classify_column("phone_number");
        assert!(r.is_pii);

        let r = classify_column("user_ssn");
        assert!(r.is_pii);

        let r = classify_column("home_address");
        assert!(r.is_pii);

        let r = classify_column("DATE_OF_BIRTH");
        assert!(r.is_pii);
    }

    #[test]
    fn test_classify_secrets() {
        let r = classify_column("api_key");
        assert!(r.is_secret);
        assert!(!r.is_pii);
        assert_eq!(r.classification, "secret");
        assert_eq!(r.suggested_sensitivity, 3);

        let r = classify_column("user_password");
        assert!(r.is_secret);

        let r = classify_column("auth_token");
        assert!(r.is_secret);
    }

    #[test]
    fn test_classify_normal() {
        let r = classify_column("amount");
        assert!(!r.is_pii);
        assert!(!r.is_secret);
        assert_eq!(r.classification, "normal");
        assert_eq!(r.suggested_sensitivity, 1);

        let r = classify_column("status");
        assert!(!r.is_pii);
        assert!(!r.is_secret);
    }
}
