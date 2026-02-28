//! Data source discovery: scan local directories for databases, CSV, JSON.
//!
//! Discovers potential data sources but does NOT ingest by default.
//! Emits DATA_SOURCE_DISCOVERED events for each found source.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use node_proto::common::{NodeId, Sensitivity, Timestamp};
use node_proto::events::{
    event_envelope::Payload, ConnectorType, DataSourceDiscovered, EventEnvelope, EventType,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryConfig {
    pub scan_dirs: Vec<PathBuf>,
    pub scan_sqlite: bool,
    pub scan_csv: bool,
    pub scan_json: bool,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            scan_dirs: Vec::new(),
            scan_sqlite: true,
            scan_csv: true,
            scan_json: true,
        }
    }
}

/// Stores as i32 for proto/serde compatibility; use `connector_type_enum()` to
/// get the typed `ConnectorType`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredSourceInfo {
    pub source_id: String,
    pub connector_type: i32,
    pub path: PathBuf,
    pub display_name: String,
    pub estimated_size_bytes: u64,
    pub estimated_tables: u32,
}

impl DiscoveredSourceInfo {
    pub fn connector_type_enum(&self) -> ConnectorType {
        ConnectorType::try_from(self.connector_type).unwrap_or(ConnectorType::Unspecified)
    }
}

fn is_sqlite_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("db") || e.eq_ignore_ascii_case("sqlite"))
        .unwrap_or(false)
}

fn count_files_with_ext(dir: &Path, ext: &str) -> (u32, u64) {
    let mut count = 0u32;
    let mut size = 0u64;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file() {
                let matches = p
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.eq_ignore_ascii_case(ext))
                    .unwrap_or(false);
                if matches {
                    count += 1;
                    size += entry.metadata().map(|m| m.len()).unwrap_or(0);
                }
            }
        }
    }
    (count, size)
}

fn display_name_from_path(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string()
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// Scan a single directory (non-recursive, immediate children only) and return
/// discovered data sources according to the config flags.
pub fn scan_directory(dir: &Path, config: &DiscoveryConfig) -> Vec<DiscoveredSourceInfo> {
    let mut results = Vec::new();

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(path = %dir.display(), error = %e, "cannot read directory");
            return results;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();

        if path.is_file() && config.scan_sqlite && is_sqlite_file(&path) {
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            results.push(DiscoveredSourceInfo {
                source_id: Uuid::new_v4().to_string(),
                connector_type: ConnectorType::SqliteDb as i32,
                path: path.clone(),
                display_name: display_name_from_path(&path),
                estimated_size_bytes: size,
                estimated_tables: 0,
            });
        }

        if path.is_dir() {
            if config.scan_csv {
                let (csv_count, csv_size) = count_files_with_ext(&path, "csv");
                if csv_count > 0 {
                    results.push(DiscoveredSourceInfo {
                        source_id: Uuid::new_v4().to_string(),
                        connector_type: ConnectorType::CsvFolder as i32,
                        path: path.clone(),
                        display_name: display_name_from_path(&path),
                        estimated_size_bytes: csv_size,
                        estimated_tables: csv_count,
                    });
                }
            }

            if config.scan_json {
                let (json_count, json_size) = count_files_with_ext(&path, "json");
                if json_count > 0 {
                    results.push(DiscoveredSourceInfo {
                        source_id: Uuid::new_v4().to_string(),
                        connector_type: ConnectorType::JsonFolder as i32,
                        path: path.clone(),
                        display_name: display_name_from_path(&path),
                        estimated_size_bytes: json_size,
                        estimated_tables: json_count,
                    });
                }
            }
        }
    }

    results
}

/// Build a DATA_SOURCE_DISCOVERED event envelope from a discovered source.
pub fn build_discovered_event(source: &DiscoveredSourceInfo, node_id: &str) -> EventEnvelope {
    let now_ms = now_unix_ms();

    EventEnvelope {
        event_id: Uuid::new_v4().to_string(),
        r#type: EventType::DataSourceDiscovered as i32,
        ts: Some(Timestamp { unix_ms: now_ms }),
        node_id: Some(NodeId {
            value: node_id.to_string(),
        }),
        sensitivity: Sensitivity::Internal as i32,
        payload: Some(Payload::DataSourceDiscovered(DataSourceDiscovered {
            source_id: source.source_id.clone(),
            connector_type: source.connector_type,
            path_or_uri: source.path.to_string_lossy().into_owned(),
            display_name: source.display_name.clone(),
            estimated_size_bytes: source.estimated_size_bytes,
            estimated_tables: source.estimated_tables,
            discovered_at: Some(Timestamp { unix_ms: now_ms }),
        })),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_scan_finds_sqlite() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("inventory.db"), b"fake-sqlite-data").unwrap();

        let config = DiscoveryConfig {
            scan_sqlite: true,
            ..Default::default()
        };
        let found = scan_directory(tmp.path(), &config);

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].connector_type, ConnectorType::SqliteDb as i32);
        assert_eq!(found[0].display_name, "inventory.db");
        assert!(found[0].estimated_size_bytes > 0);
        assert_eq!(found[0].estimated_tables, 0);
    }

    #[test]
    fn test_scan_finds_csv_folder() {
        let tmp = TempDir::new().unwrap();
        let sub = tmp.path().join("sales_data");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("orders.csv"), b"id,amount\n1,100").unwrap();
        fs::write(sub.join("products.csv"), b"id,name\n1,Widget").unwrap();

        let config = DiscoveryConfig {
            scan_csv: true,
            ..Default::default()
        };
        let found = scan_directory(tmp.path(), &config);

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].connector_type, ConnectorType::CsvFolder as i32);
        assert_eq!(found[0].display_name, "sales_data");
        assert_eq!(found[0].estimated_tables, 2);
        assert!(found[0].estimated_size_bytes > 0);
    }

    #[test]
    fn test_scan_finds_json_folder() {
        let tmp = TempDir::new().unwrap();
        let sub = tmp.path().join("logs");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("events.json"), b"[{\"a\":1}]").unwrap();
        fs::write(sub.join("users.json"), b"[{\"id\":1}]").unwrap();
        fs::write(sub.join("metrics.json"), b"[{\"cpu\":0.5}]").unwrap();

        let config = DiscoveryConfig {
            scan_json: true,
            ..Default::default()
        };
        let found = scan_directory(tmp.path(), &config);

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].connector_type, ConnectorType::JsonFolder as i32);
        assert_eq!(found[0].display_name, "logs");
        assert_eq!(found[0].estimated_tables, 3);
        assert!(found[0].estimated_size_bytes > 0);
    }

    #[test]
    fn test_scan_ignores_when_disabled() {
        let tmp = TempDir::new().unwrap();

        fs::write(tmp.path().join("data.db"), b"sqlite").unwrap();

        let csv_dir = tmp.path().join("csv_stuff");
        fs::create_dir(&csv_dir).unwrap();
        fs::write(csv_dir.join("table.csv"), b"a,b\n1,2").unwrap();

        let json_dir = tmp.path().join("json_stuff");
        fs::create_dir(&json_dir).unwrap();
        fs::write(json_dir.join("doc.json"), b"{}").unwrap();

        let config = DiscoveryConfig {
            scan_sqlite: true,
            scan_csv: false,
            scan_json: false,
            ..Default::default()
        };
        let found = scan_directory(tmp.path(), &config);

        assert_eq!(found.len(), 1, "only SQLite should be found");
        assert_eq!(found[0].connector_type, ConnectorType::SqliteDb as i32);
    }

    #[test]
    fn test_build_event() {
        let source = DiscoveredSourceInfo {
            source_id: "src-test-001".into(),
            connector_type: ConnectorType::CsvFolder as i32,
            path: PathBuf::from("/data/exports"),
            display_name: "exports".into(),
            estimated_size_bytes: 42_000,
            estimated_tables: 3,
        };

        let envelope = build_discovered_event(&source, "node-abc");

        assert_eq!(envelope.r#type, EventType::DataSourceDiscovered as i32);
        assert!(!envelope.event_id.is_empty());
        assert_eq!(envelope.node_id.as_ref().unwrap().value, "node-abc");
        assert!(envelope.ts.is_some());

        match envelope.payload {
            Some(Payload::DataSourceDiscovered(d)) => {
                assert_eq!(d.source_id, "src-test-001");
                assert_eq!(d.connector_type, ConnectorType::CsvFolder as i32);
                assert_eq!(d.display_name, "exports");
                assert_eq!(d.estimated_size_bytes, 42_000);
                assert_eq!(d.estimated_tables, 3);
                assert!(d.discovered_at.is_some());
            }
            other => panic!("expected DataSourceDiscovered payload, got {other:?}"),
        }
    }
}
