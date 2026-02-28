//! Dataset manifest builder: build reproducible datasets for training.
//!
//! Iterates over the event log, filters by preset, and stores a
//! JSON manifest in CAS for deterministic reproducibility.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use node_proto::common::{HashRef, NodeId, Sensitivity};
use node_proto::events::{
    event_envelope::Payload, DatasetManifestCreated, EventEnvelope, EventType,
};
use node_storage::cas::CasStore;
use node_storage::event_log::EventLog;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DatasetPreset {
    PublicShareableOnly,
    ThisTenantConfirmed,
    AllApprovedNoRestricted,
    NumericOnly,
    Custom(String),
}

impl DatasetPreset {
    fn label(&self) -> String {
        match self {
            Self::PublicShareableOnly => "public_shareable_only".into(),
            Self::ThisTenantConfirmed => "this_tenant_confirmed".into(),
            Self::AllApprovedNoRestricted => "all_approved_no_restricted".into(),
            Self::NumericOnly => "numeric_only".into(),
            Self::Custom(s) => format!("custom:{s}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetBuildConfig {
    pub preset: DatasetPreset,
    pub source_id: Option<String>,
    #[serde(default = "default_max_items")]
    pub max_items: u64,
    #[serde(default)]
    pub redact_columns: Vec<String>,
}

fn default_max_items() -> u64 {
    10_000
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetItem {
    pub event_id: String,
    pub cas_hash: String,
    pub item_type: String,
    pub content_preview: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestResult {
    pub manifest_id: String,
    pub cas_hash: String,
    pub items: Vec<DatasetItem>,
    pub total_items: u64,
    pub total_bytes: u64,
}

fn matches_preset(event: &EventEnvelope, preset: &DatasetPreset) -> bool {
    match preset {
        DatasetPreset::PublicShareableOnly => {
            if event.sensitivity != Sensitivity::Public as i32 {
                return false;
            }
            match &event.payload {
                Some(Payload::CaseCreated(cc)) => cc.shareable,
                Some(Payload::ArtifactPublished(ap)) => ap.shareable,
                _ => false,
            }
        }
        DatasetPreset::AllApprovedNoRestricted => {
            event.sensitivity != Sensitivity::Restricted as i32
        }
        DatasetPreset::ThisTenantConfirmed => {
            matches!(&event.payload, Some(Payload::CaseConfirmed(_)))
        }
        DatasetPreset::NumericOnly => false,
        DatasetPreset::Custom(_) => true,
    }
}

fn extract_item(event: &EventEnvelope) -> DatasetItem {
    let (cas_hash, item_type, preview) = match &event.payload {
        Some(Payload::CaseCreated(cc)) => {
            let hash = cc
                .content_ref
                .as_ref()
                .map(|r| r.sha256.clone())
                .unwrap_or_default();
            (hash, "case", truncate_preview(&cc.summary))
        }
        Some(Payload::ArtifactPublished(ap)) => {
            let hash = ap
                .content_ref
                .as_ref()
                .map(|r| r.sha256.clone())
                .unwrap_or_default();
            (hash, "artifact", truncate_preview(&ap.title))
        }
        Some(Payload::CaseConfirmed(cc)) => (String::new(), "case", truncate_preview(&cc.outcome)),
        _ => {
            let hash = event
                .event_hash
                .as_ref()
                .map(|h| h.sha256.clone())
                .unwrap_or_default();
            (hash, "document", String::new())
        }
    };

    DatasetItem {
        event_id: event.event_id.clone(),
        cas_hash,
        item_type: item_type.into(),
        content_preview: preview,
    }
}

fn truncate_preview(s: &str) -> String {
    s.chars().take(200).collect()
}

/// Build a dataset manifest by replaying the event log and filtering by preset.
///
/// The resulting manifest JSON is stored in CAS so it can be referenced
/// deterministically by hash.
pub fn build_dataset(
    config: &DatasetBuildConfig,
    event_log: &EventLog,
    cas: &CasStore,
    node_id: &str,
) -> Result<ManifestResult> {
    let events = event_log.replay().map_err(|e| anyhow::anyhow!("{e}"))?;

    let mut items = Vec::new();
    for event in &events {
        if items.len() as u64 >= config.max_items {
            break;
        }
        if matches_preset(event, &config.preset) {
            items.push(extract_item(event));
        }
    }

    let manifest_id = Uuid::new_v4().to_string();
    let total_items = items.len() as u64;

    let manifest_json = serde_json::to_vec(&serde_json::json!({
        "manifest_id": manifest_id,
        "node_id": node_id,
        "preset": config.preset.label(),
        "source_id": config.source_id,
        "redact_columns": config.redact_columns,
        "items": items,
    }))?;

    let total_bytes = manifest_json.len() as u64;
    let href = cas
        .put_bytes("application/json", &manifest_json)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    tracing::info!(
        manifest_id = %manifest_id,
        cas_hash = %href.sha256,
        total_items,
        total_bytes,
        "dataset manifest built"
    );

    Ok(ManifestResult {
        manifest_id,
        cas_hash: href.sha256,
        items,
        total_items,
        total_bytes,
    })
}

/// Create an `EventEnvelope` recording that a dataset manifest was created.
pub fn build_manifest_event(
    result: &ManifestResult,
    config: &DatasetBuildConfig,
    node_id: &str,
) -> EventEnvelope {
    EventEnvelope {
        event_id: format!("evt-dm-{}", result.manifest_id),
        r#type: EventType::DatasetManifestCreated as i32,
        node_id: Some(NodeId {
            value: node_id.into(),
        }),
        sensitivity: Sensitivity::Internal as i32,
        payload: Some(Payload::DatasetManifestCreated(DatasetManifestCreated {
            manifest_id: result.manifest_id.clone(),
            manifest_ref: Some(HashRef {
                sha256: result.cas_hash.clone(),
            }),
            source_id: config.source_id.clone().unwrap_or_default(),
            preset: config.preset.label(),
            item_count: result.total_items,
            total_bytes: result.total_bytes,
        })),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use node_proto::common::*;
    use node_proto::events::{self, event_envelope, CaseCreated};
    use tempfile::TempDir;

    fn make_case(id: &str, shareable: bool, sensitivity: Sensitivity) -> EventEnvelope {
        EventEnvelope {
            event_id: id.into(),
            r#type: events::EventType::CaseCreated as i32,
            tenant_id: Some(TenantId {
                value: "public".into(),
            }),
            sensitivity: sensitivity as i32,
            payload: Some(event_envelope::Payload::CaseCreated(CaseCreated {
                case_id: id.into(),
                title: format!("Case {id}"),
                summary: format!("Summary {id}"),
                content_ref: Some(HashRef {
                    sha256: format!("hash-{id}"),
                }),
                shareable,
            })),
            ..Default::default()
        }
    }

    fn setup() -> (TempDir, EventLog, CasStore) {
        let tmp = TempDir::new().unwrap();
        let log = EventLog::open(tmp.path()).unwrap();
        let cas = CasStore::open(tmp.path()).unwrap();
        (tmp, log, cas)
    }

    fn default_config(preset: DatasetPreset) -> DatasetBuildConfig {
        DatasetBuildConfig {
            preset,
            source_id: None,
            max_items: 10_000,
            redact_columns: vec![],
        }
    }

    #[test]
    fn test_build_empty_dataset() {
        let (_tmp, log, cas) = setup();
        let config = default_config(DatasetPreset::PublicShareableOnly);
        let result = build_dataset(&config, &log, &cas, "node-1").unwrap();
        assert_eq!(result.total_items, 0);
        assert!(result.items.is_empty());
    }

    #[test]
    fn test_build_public_shareable() {
        let (_tmp, mut log, cas) = setup();
        log.append(make_case("c1", true, Sensitivity::Public))
            .unwrap();
        log.append(make_case("c2", true, Sensitivity::Public))
            .unwrap();
        log.append(make_case("c3", false, Sensitivity::Public))
            .unwrap();

        let config = default_config(DatasetPreset::PublicShareableOnly);
        let result = build_dataset(&config, &log, &cas, "node-1").unwrap();

        assert_eq!(result.total_items, 2);
        assert_eq!(result.items.len(), 2);
        assert_eq!(result.items[0].event_id, "c1");
        assert_eq!(result.items[1].event_id, "c2");
        for item in &result.items {
            assert_eq!(item.item_type, "case");
        }
    }

    #[test]
    fn test_build_no_restricted() {
        let (_tmp, mut log, cas) = setup();
        log.append(make_case("c1", true, Sensitivity::Public))
            .unwrap();
        log.append(make_case("c2", false, Sensitivity::Internal))
            .unwrap();
        log.append(make_case("c3", false, Sensitivity::Restricted))
            .unwrap();

        let config = default_config(DatasetPreset::AllApprovedNoRestricted);
        let result = build_dataset(&config, &log, &cas, "node-1").unwrap();

        assert_eq!(result.total_items, 2);
        assert_eq!(result.items[0].event_id, "c1");
        assert_eq!(result.items[1].event_id, "c2");
    }

    #[test]
    fn test_manifest_stored_in_cas() {
        let (_tmp, mut log, cas) = setup();
        log.append(make_case("c1", true, Sensitivity::Public))
            .unwrap();

        let config = default_config(DatasetPreset::PublicShareableOnly);
        let result = build_dataset(&config, &log, &cas, "node-1").unwrap();

        let data = cas.get_bytes(&result.cas_hash).unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&data).unwrap();
        assert_eq!(parsed["manifest_id"].as_str().unwrap(), result.manifest_id);
        let items = parsed["items"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["event_id"].as_str().unwrap(), "c1");
    }
}
