//! Snapshot export and restore.
//!
//! Exports the SQLite database to CAS, creating a SnapshotFile.
//! Restore: load snapshot, restore SQLite dump, replay remaining events.

use std::fs;
use std::path::{Path, PathBuf};

use prost::Message;
use sha2::Digest;
use thiserror::Error;

use node_proto::common::{HashRef, KeyValue, Timestamp};
use node_proto::snapshot::{SnapshotFile, SnapshotHeader, SnapshotPayload};

use crate::cas::CasStore;

#[derive(Debug, Error)]
pub enum SnapshotError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("CAS error: {0}")]
    Cas(#[from] crate::cas::CasError),
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("protobuf decode error: {0}")]
    Decode(#[from] prost::DecodeError),
    #[error("snapshot missing header")]
    MissingHeader,
    #[error("snapshot missing payload")]
    MissingPayload,
}

pub type Result<T> = std::result::Result<T, SnapshotError>;

/// Write a snapshot of the current SQLite DB to CAS + snapshots dir.
pub fn create_snapshot(
    db_path: &Path,
    cas: &CasStore,
    snapshots_dir: &Path,
    last_event_hash: &str,
    event_count: u64,
) -> Result<SnapshotFile> {
    fs::create_dir_all(snapshots_dir)?;

    let db_bytes = fs::read(db_path)?;
    let sqlite_ref = cas.put_bytes("application/x-sqlite3", &db_bytes)?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;

    let snap_data_for_hash = format!("{last_event_hash}:{now}:{}", sqlite_ref.sha256);
    let snap_hash = hex::encode(sha2::Sha256::digest(snap_data_for_hash.as_bytes()));

    let snap = SnapshotFile {
        header: Some(SnapshotHeader {
            snapshot_version: 1,
            created_at: Some(Timestamp { unix_ms: now }),
            last_applied_event_hash: Some(HashRef {
                sha256: last_event_hash.to_string(),
            }),
            snapshot_hash: Some(HashRef {
                sha256: snap_hash.clone(),
            }),
        }),
        payload: Some(SnapshotPayload {
            sqlite_dump_ref: Some(sqlite_ref),
            notes: vec![KeyValue {
                key: "event_count".into(),
                value: event_count.to_string(),
            }],
        }),
    };

    let snap_bytes = snap.encode_to_vec();
    let snap_path = snapshots_dir.join(format!("snapshot_{snap_hash}.bin"));
    fs::write(&snap_path, &snap_bytes)?;

    tracing::info!(
        hash = %snap_hash,
        path = %snap_path.display(),
        "snapshot created"
    );

    Ok(snap)
}

/// Restore a snapshot: copy SQLite dump from CAS to the target db_path.
/// Returns the last_applied_event_hash from the snapshot header.
pub fn restore_snapshot(
    snap_path: &Path,
    cas: &CasStore,
    db_path: &Path,
) -> Result<String> {
    let snap_bytes = fs::read(snap_path)?;
    let snap = SnapshotFile::decode(snap_bytes.as_slice())?;

    let header = snap.header.as_ref().ok_or(SnapshotError::MissingHeader)?;
    let payload = snap.payload.as_ref().ok_or(SnapshotError::MissingPayload)?;

    let sqlite_ref = payload
        .sqlite_dump_ref
        .as_ref()
        .ok_or(SnapshotError::MissingPayload)?;

    let db_bytes = cas.get_bytes(&sqlite_ref.sha256)?;

    if let Some(parent) = db_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(db_path, &db_bytes)?;

    let last_hash = header
        .last_applied_event_hash
        .as_ref()
        .map(|h| h.sha256.clone())
        .unwrap_or_default();

    tracing::info!(
        last_event_hash = %last_hash,
        db_path = %db_path.display(),
        "snapshot restored"
    );

    Ok(last_hash)
}

/// List snapshot files in a directory.
pub fn list_snapshots(snapshots_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    if snapshots_dir.exists() {
        for entry in fs::read_dir(snapshots_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("bin") {
                paths.push(path);
            }
        }
    }
    paths.sort();
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projector;
    use crate::sqlite_views;
    use node_proto::common::*;
    use node_proto::events::*;
    use rusqlite::Connection;
    use tempfile::TempDir;

    fn create_test_db(db_path: &Path, count: usize) -> String {
        let conn = sqlite_views::open_db(db_path).unwrap();
        let mut last_hash = String::new();

        for i in 0..count {
            let hash = format!("hash-{i}");
            let event = EventEnvelope {
                event_id: format!("e{i}"),
                r#type: EventType::CaseCreated as i32,
                ts: Some(Timestamp { unix_ms: 1700000000000 + i as i64 * 1000 }),
                node_id: Some(NodeId { value: "node-1".into() }),
                tenant_id: Some(TenantId { value: "public".into() }),
                sensitivity: Sensitivity::Public as i32,
                event_hash: Some(HashRef { sha256: hash.clone() }),
                payload: Some(event_envelope::Payload::CaseCreated(CaseCreated {
                    case_id: format!("case-{i}"),
                    title: format!("Case {i}"),
                    summary: format!("Summary {i}"),
                    content_ref: None,
                    shareable: false,
                })),
                ..Default::default()
            };
            projector::apply_event(&conn, &event).unwrap();
            last_hash = hash;
        }

        last_hash
    }

    fn count_cases(db_path: &Path) -> i64 {
        let conn = Connection::open(db_path).unwrap();
        conn.query_row("SELECT COUNT(*) FROM cases_view", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn snapshot_and_restore() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("sqlite").join("meshmind.db");
        let snap_dir = tmp.path().join("snapshots");
        let cas = CasStore::open(tmp.path()).unwrap();

        let last_hash = create_test_db(&db_path, 10);
        assert_eq!(count_cases(&db_path), 10);

        let snap = create_snapshot(&db_path, &cas, &snap_dir, &last_hash, 10).unwrap();
        assert!(snap.header.is_some());

        let snaps = list_snapshots(&snap_dir).unwrap();
        assert_eq!(snaps.len(), 1);

        let restored_db = tmp.path().join("sqlite").join("restored.db");
        let restored_hash = restore_snapshot(&snaps[0], &cas, &restored_db).unwrap();
        assert_eq!(restored_hash, last_hash);

        assert_eq!(count_cases(&restored_db), 10);
    }

    #[test]
    fn snapshot_restore_then_replay() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("sqlite").join("meshmind.db");
        let snap_dir = tmp.path().join("snapshots");
        let cas = CasStore::open(tmp.path()).unwrap();

        let last_hash = create_test_db(&db_path, 5);
        create_snapshot(&db_path, &cas, &snap_dir, &last_hash, 5).unwrap();

        let restored_db = tmp.path().join("sqlite").join("restored.db");
        let snaps = list_snapshots(&snap_dir).unwrap();
        restore_snapshot(&snaps[0], &cas, &restored_db).unwrap();

        let conn = Connection::open(&restored_db).unwrap();
        for i in 5..10 {
            let event = EventEnvelope {
                event_id: format!("e{i}"),
                r#type: EventType::CaseCreated as i32,
                ts: Some(Timestamp { unix_ms: 1700000000000 + i * 1000 }),
                node_id: Some(NodeId { value: "node-1".into() }),
                tenant_id: Some(TenantId { value: "public".into() }),
                sensitivity: Sensitivity::Public as i32,
                event_hash: Some(HashRef { sha256: format!("hash-{i}") }),
                payload: Some(event_envelope::Payload::CaseCreated(CaseCreated {
                    case_id: format!("case-{i}"),
                    title: format!("Case {i}"),
                    summary: format!("Summary {i}"),
                    content_ref: None,
                    shareable: false,
                })),
                ..Default::default()
            };
            projector::apply_event(&conn, &event).unwrap();
        }

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM cases_view", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 10);
    }

    #[test]
    fn list_snapshots_empty() {
        let tmp = TempDir::new().unwrap();
        let snaps = list_snapshots(&tmp.path().join("nonexistent")).unwrap();
        assert!(snaps.is_empty());
    }
}
