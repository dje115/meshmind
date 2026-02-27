//! Node wiring: configuration, startup sequences, seed loader.

use std::path::Path;

use anyhow::{Context, Result};
use node_proto::common::*;
use node_proto::events::*;
use node_storage::cas::CasStore;
use node_storage::event_log::EventLog;
use node_storage::projector;
use serde::Deserialize;
use tracing::info;

#[derive(Debug, Deserialize)]
struct SeedCase {
    event_id: String,
    title: String,
    summary: String,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct SeedRunbook {
    artifact_id: String,
    title: String,
    content: String,
    #[serde(default)]
    tags: Vec<String>,
}

/// Load seed data from JSON files in a directory into the event log + projector.
pub fn load_seed_data(
    seed_dir: &Path,
    event_log: &mut EventLog,
    cas: &CasStore,
    db_path: &Path,
    node_id: &str,
) -> Result<u32> {
    let conn = rusqlite::Connection::open(db_path).context("open db for seeding")?;
    let mut loaded = 0u32;

    let cases_path = seed_dir.join("cases").join("seed_cases.json");
    if cases_path.exists() {
        let text = std::fs::read_to_string(&cases_path).context("read seed cases")?;
        let cases: Vec<SeedCase> = serde_json::from_str(&text).context("parse seed cases")?;

        for case in cases {
            if event_log
                .replay()
                .ok()
                .map(|events| events.iter().any(|e| e.event_id == case.event_id))
                .unwrap_or(false)
            {
                continue;
            }

            let content_ref = cas
                .put_bytes("text/plain", case.summary.as_bytes())
                .context("store case content")?;

            let event = EventEnvelope {
                event_id: case.event_id,
                r#type: EventType::CaseCreated as i32,
                node_id: Some(NodeId {
                    value: node_id.into(),
                }),
                tenant_id: Some(TenantId {
                    value: "public".into(),
                }),
                sensitivity: Sensitivity::Public as i32,
                payload: Some(event_envelope::Payload::CaseCreated(CaseCreated {
                    case_id: format!("case-{loaded}"),
                    title: case.title,
                    summary: case.summary,
                    content_ref: Some(content_ref),
                    shareable: true,
                })),
                tags: case.tags,
                ..Default::default()
            };

            let stored = event_log.append(event).context("append seed case")?;
            projector::apply_event(&conn, &stored).context("project seed case")?;
            loaded += 1;
        }
    }

    let runbooks_path = seed_dir.join("runbooks").join("seed_runbooks.json");
    if runbooks_path.exists() {
        let text = std::fs::read_to_string(&runbooks_path).context("read seed runbooks")?;
        let runbooks: Vec<SeedRunbook> =
            serde_json::from_str(&text).context("parse seed runbooks")?;

        for runbook in runbooks {
            if event_log
                .replay()
                .ok()
                .map(|events| events.iter().any(|e| e.event_id == runbook.artifact_id))
                .unwrap_or(false)
            {
                continue;
            }

            let content_ref = cas
                .put_bytes("text/plain", runbook.content.as_bytes())
                .context("store runbook content")?;

            let event = EventEnvelope {
                event_id: runbook.artifact_id.clone(),
                r#type: EventType::ArtifactPublished as i32,
                node_id: Some(NodeId {
                    value: node_id.into(),
                }),
                tenant_id: Some(TenantId {
                    value: "public".into(),
                }),
                sensitivity: Sensitivity::Public as i32,
                payload: Some(event_envelope::Payload::ArtifactPublished(
                    ArtifactPublished {
                        artifact_id: runbook.artifact_id,
                        artifact_type: ArtifactType::Runbook as i32,
                        version: 1,
                        title: runbook.title,
                        content_ref: Some(content_ref),
                        shareable: true,
                        expires_unix_ms: 0,
                    },
                )),
                tags: runbook.tags,
                ..Default::default()
            };

            let stored = event_log.append(event).context("append seed runbook")?;
            projector::apply_event(&conn, &stored).context("project seed runbook")?;
            loaded += 1;
        }
    }

    if loaded > 0 {
        info!("Loaded {loaded} seed items");
    }

    Ok(loaded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use node_storage::sqlite_views;
    use tempfile::TempDir;

    #[test]
    fn load_seed_data_from_files() {
        let tmp = TempDir::new().unwrap();
        let mut event_log = EventLog::open(tmp.path()).unwrap();
        let cas = CasStore::open(tmp.path()).unwrap();
        let db_path = tmp.path().join("sqlite").join("meshmind.db");
        let _conn = sqlite_views::open_db(&db_path).unwrap();

        let seed_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("seed")
            .join("public");

        let loaded =
            load_seed_data(&seed_dir, &mut event_log, &cas, &db_path, "test-node").unwrap();

        assert!(
            loaded >= 12,
            "expected at least 12 seed items, got {loaded}"
        );
        assert_eq!(event_log.event_count(), loaded as u64);

        // Verify FTS works on seeded data
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let hits = node_storage::search::search_cases(&conn, "DNS", 10).unwrap();
        assert!(!hits.is_empty(), "should find DNS case via FTS");

        let hits = node_storage::search::search_artifacts(&conn, "runbook", 10).unwrap();
        assert!(!hits.is_empty(), "should find runbook artifacts via FTS");
    }

    #[test]
    fn seed_data_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let mut event_log = EventLog::open(tmp.path()).unwrap();
        let cas = CasStore::open(tmp.path()).unwrap();
        let db_path = tmp.path().join("sqlite").join("meshmind.db");
        let _conn = sqlite_views::open_db(&db_path).unwrap();

        let seed_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("seed")
            .join("public");

        let first = load_seed_data(&seed_dir, &mut event_log, &cas, &db_path, "test-node").unwrap();
        let second =
            load_seed_data(&seed_dir, &mut event_log, &cas, &db_path, "test-node").unwrap();

        assert!(first > 0);
        assert_eq!(second, 0, "second load should be idempotent");
    }
}
