//! Federated learning: distributed training without sharing raw data.
//!
//! Modes:
//! - Default: share model deltas only (policy-gated)
//! - Optional: share aggregate stats only
//! - Optional: share curated anonymized synthetic examples

use node_proto::common::{HashRef, NodeId, Sensitivity, Timestamp};
use node_proto::events::{
    event_envelope, EventEnvelope, EventType, FederatedRoundCompleted, FederatedRoundStarted,
    TrainDeltaPublished, TrainMetric,
};
use node_storage::cas::CasStore;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederatedConfig {
    pub model_id: String,
    pub min_participants: u32,
    pub max_participants: u32,
    pub deadline_seconds: u32,
    pub aggregation_strategy: String,
}

impl FederatedConfig {
    pub fn new(model_id: &str) -> Self {
        Self {
            model_id: model_id.to_string(),
            min_participants: 2,
            max_participants: 10,
            deadline_seconds: 300,
            aggregation_strategy: "fedavg".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeltaInfo {
    pub delta_id: String,
    pub model_id: String,
    pub base_version: String,
    pub from_node: String,
    pub cas_hash: String,
    pub metrics: Vec<(String, f64)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoundState {
    pub round_id: String,
    pub model_id: String,
    pub round_number: u32,
    pub status: String,
    pub deltas: Vec<DeltaInfo>,
    pub result_model_hash: Option<String>,
}

pub struct FederatedCoordinator {
    config: FederatedConfig,
}

impl FederatedCoordinator {
    pub fn new(config: FederatedConfig) -> Self {
        Self { config }
    }

    pub fn start_round(&self, model_id: &str, round_number: u32) -> RoundState {
        let round_id = format!("round-{}-{}", model_id, round_number);
        tracing::info!(round_id = %round_id, model_id = %model_id, round_number, "federated round started");
        RoundState {
            round_id,
            model_id: model_id.to_string(),
            round_number,
            status: "collecting".to_string(),
            deltas: Vec::new(),
            result_model_hash: None,
        }
    }

    pub fn submit_delta(&self, state: &mut RoundState, delta: DeltaInfo) -> bool {
        if state.deltas.len() as u32 >= self.config.max_participants {
            tracing::warn!(round_id = %state.round_id, "max participants reached, rejecting delta");
            return false;
        }
        tracing::debug!(
            round_id = %state.round_id,
            delta_id = %delta.delta_id,
            from_node = %delta.from_node,
            "delta submitted"
        );
        state.deltas.push(delta);
        state.deltas.len() as u32 >= self.config.min_participants
    }

    pub fn aggregate(&self, state: &mut RoundState, cas: &CasStore) -> anyhow::Result<String> {
        if state.deltas.is_empty() {
            anyhow::bail!("no deltas to aggregate");
        }

        state.status = "aggregating".to_string();

        let mut metric_sums: std::collections::HashMap<String, (f64, u32)> =
            std::collections::HashMap::new();
        for delta in &state.deltas {
            for (name, value) in &delta.metrics {
                let entry = metric_sums.entry(name.clone()).or_insert((0.0, 0));
                entry.0 += value;
                entry.1 += 1;
            }
        }

        let mut averaged: Vec<(String, f64)> = metric_sums
            .into_iter()
            .map(|(name, (sum, count))| (name, sum / count as f64))
            .collect();
        averaged.sort_by(|a, b| a.0.cmp(&b.0));

        let merged_payload = serde_json::json!({
            "type": "merged_model",
            "model_id": state.model_id,
            "round_id": state.round_id,
            "round_number": state.round_number,
            "strategy": self.config.aggregation_strategy,
            "participants": state.deltas.len(),
            "averaged_metrics": averaged,
        });
        let merged_bytes = serde_json::to_vec(&merged_payload)?;
        let hash_ref = cas.put_bytes("application/json", &merged_bytes)?;

        state.status = "completed".to_string();
        state.result_model_hash = Some(hash_ref.sha256.clone());
        tracing::info!(
            round_id = %state.round_id,
            hash = %hash_ref.sha256,
            participants = state.deltas.len(),
            "round aggregated"
        );

        Ok(hash_ref.sha256)
    }

    pub fn is_round_complete(&self, state: &RoundState) -> bool {
        state.status == "completed"
    }
}

fn now_ts() -> Timestamp {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    Timestamp { unix_ms: ms }
}

pub fn build_round_started_event(state: &RoundState, coordinator_node_id: &str) -> EventEnvelope {
    EventEnvelope {
        event_id: format!("evt-frs-{}", state.round_id),
        r#type: EventType::FederatedRoundStarted as i32,
        node_id: Some(NodeId {
            value: coordinator_node_id.to_string(),
        }),
        sensitivity: Sensitivity::Internal as i32,
        ts: Some(now_ts()),
        payload: Some(event_envelope::Payload::FederatedRoundStarted(
            FederatedRoundStarted {
                round_id: state.round_id.clone(),
                model_id: state.model_id.clone(),
                round_number: state.round_number,
                expected_participants: state.deltas.len() as u32,
                coordinator: Some(NodeId {
                    value: coordinator_node_id.to_string(),
                }),
                started_at: Some(now_ts()),
            },
        )),
        ..Default::default()
    }
}

pub fn build_round_completed_event(state: &RoundState, coordinator_node_id: &str) -> EventEnvelope {
    let aggregate_metrics: Vec<TrainMetric> = if let Some(ref _hash) = state.result_model_hash {
        let mut metric_sums: std::collections::HashMap<String, (f64, u32)> =
            std::collections::HashMap::new();
        for delta in &state.deltas {
            for (name, value) in &delta.metrics {
                let entry = metric_sums.entry(name.clone()).or_insert((0.0, 0));
                entry.0 += value;
                entry.1 += 1;
            }
        }
        let mut metrics: Vec<TrainMetric> = metric_sums
            .into_iter()
            .map(|(name, (sum, count))| TrainMetric {
                name,
                value: sum / count as f64,
            })
            .collect();
        metrics.sort_by(|a, b| a.name.cmp(&b.name));
        metrics
    } else {
        Vec::new()
    };

    let success = state.status == "completed";

    EventEnvelope {
        event_id: format!("evt-frc-{}", state.round_id),
        r#type: EventType::FederatedRoundCompleted as i32,
        node_id: Some(NodeId {
            value: coordinator_node_id.to_string(),
        }),
        sensitivity: Sensitivity::Internal as i32,
        ts: Some(now_ts()),
        payload: Some(event_envelope::Payload::FederatedRoundCompleted(
            FederatedRoundCompleted {
                round_id: state.round_id.clone(),
                model_id: state.model_id.clone(),
                round_number: state.round_number,
                actual_participants: state.deltas.len() as u32,
                success,
                aggregate_metrics,
                resulting_model_ref: state
                    .result_model_hash
                    .as_ref()
                    .map(|h| HashRef { sha256: h.clone() }),
                notes: String::new(),
            },
        )),
        ..Default::default()
    }
}

pub fn build_delta_published_event(delta: &DeltaInfo, node_id: &str) -> EventEnvelope {
    let metrics: Vec<TrainMetric> = delta
        .metrics
        .iter()
        .map(|(name, value)| TrainMetric {
            name: name.clone(),
            value: *value,
        })
        .collect();

    EventEnvelope {
        event_id: format!("evt-tdp-{}", delta.delta_id),
        r#type: EventType::TrainDeltaPublished as i32,
        node_id: Some(NodeId {
            value: node_id.to_string(),
        }),
        sensitivity: Sensitivity::Internal as i32,
        ts: Some(now_ts()),
        payload: Some(event_envelope::Payload::TrainDeltaPublished(
            TrainDeltaPublished {
                delta_id: delta.delta_id.clone(),
                model_id: delta.model_id.clone(),
                base_version: delta.base_version.clone(),
                delta_ref: Some(HashRef {
                    sha256: delta.cas_hash.clone(),
                }),
                metrics,
                from_node_id: Some(NodeId {
                    value: delta.from_node.clone(),
                }),
            },
        )),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use node_policy::{PolicyConfig, PolicyEngine};
    use node_proto::events::event_envelope;
    use tempfile::TempDir;

    fn make_delta(id: &str, node: &str, metrics: Vec<(String, f64)>) -> DeltaInfo {
        DeltaInfo {
            delta_id: id.to_string(),
            model_id: "router-v1".to_string(),
            base_version: "v2".to_string(),
            from_node: node.to_string(),
            cas_hash: format!("hash-{id}"),
            metrics,
        }
    }

    #[test]
    fn test_start_round() {
        let config = FederatedConfig::new("router-v1");
        let coordinator = FederatedCoordinator::new(config);
        let state = coordinator.start_round("router-v1", 1);

        assert_eq!(state.round_id, "round-router-v1-1");
        assert_eq!(state.model_id, "router-v1");
        assert_eq!(state.round_number, 1);
        assert_eq!(state.status, "collecting");
        assert!(state.deltas.is_empty());
        assert!(state.result_model_hash.is_none());
    }

    #[test]
    fn test_submit_deltas() {
        let config = FederatedConfig {
            min_participants: 3,
            ..FederatedConfig::new("router-v1")
        };
        let coordinator = FederatedCoordinator::new(config);
        let mut state = coordinator.start_round("router-v1", 1);

        let d1 = make_delta("d1", "node-a", vec![("loss".into(), 0.05)]);
        let d2 = make_delta("d2", "node-b", vec![("loss".into(), 0.03)]);
        let d3 = make_delta("d3", "node-c", vec![("loss".into(), 0.04)]);

        assert!(!coordinator.submit_delta(&mut state, d1));
        assert!(!coordinator.submit_delta(&mut state, d2));
        assert!(coordinator.submit_delta(&mut state, d3));

        assert_eq!(state.deltas.len(), 3);
        assert_eq!(state.deltas[0].from_node, "node-a");
        assert_eq!(state.deltas[1].from_node, "node-b");
        assert_eq!(state.deltas[2].from_node, "node-c");
    }

    #[test]
    fn test_aggregate_metrics() {
        let tmp = TempDir::new().unwrap();
        let cas = CasStore::open(tmp.path()).unwrap();
        let config = FederatedConfig::new("router-v1");
        let coordinator = FederatedCoordinator::new(config);
        let mut state = coordinator.start_round("router-v1", 1);

        let d1 = make_delta(
            "d1",
            "node-a",
            vec![("loss".into(), 0.06), ("f1".into(), 0.90)],
        );
        let d2 = make_delta(
            "d2",
            "node-b",
            vec![("loss".into(), 0.04), ("f1".into(), 0.94)],
        );

        coordinator.submit_delta(&mut state, d1);
        coordinator.submit_delta(&mut state, d2);

        let hash = coordinator.aggregate(&mut state, &cas).unwrap();
        assert!(!hash.is_empty());
        assert!(cas.has(&hash));
        assert_eq!(state.status, "completed");
        assert_eq!(state.result_model_hash.as_deref(), Some(hash.as_str()));

        let stored = cas.get_bytes(&hash).unwrap();
        let doc: serde_json::Value = serde_json::from_slice(&stored).unwrap();
        let avg_metrics = doc["averaged_metrics"].as_array().unwrap();
        assert_eq!(avg_metrics.len(), 2);

        for m in avg_metrics {
            let name = m[0].as_str().unwrap();
            let val = m[1].as_f64().unwrap();
            match name {
                "f1" => assert!((val - 0.92).abs() < 1e-9),
                "loss" => assert!((val - 0.05).abs() < 1e-9),
                other => panic!("unexpected metric: {other}"),
            }
        }
    }

    #[test]
    fn test_round_lifecycle() {
        let tmp = TempDir::new().unwrap();
        let cas = CasStore::open(tmp.path()).unwrap();
        let config = FederatedConfig::new("router-v1");
        let coordinator = FederatedCoordinator::new(config);

        let mut state = coordinator.start_round("router-v1", 1);
        assert!(!coordinator.is_round_complete(&state));

        let started_evt = build_round_started_event(&state, "coord-node");
        assert_eq!(started_evt.r#type, EventType::FederatedRoundStarted as i32);

        let d1 = make_delta("d1", "node-a", vec![("loss".into(), 0.05)]);
        let d2 = make_delta("d2", "node-b", vec![("loss".into(), 0.03)]);
        coordinator.submit_delta(&mut state, d1.clone());
        coordinator.submit_delta(&mut state, d2);

        let delta_evt = build_delta_published_event(&d1, "node-a");
        assert_eq!(delta_evt.r#type, EventType::TrainDeltaPublished as i32);
        match &delta_evt.payload {
            Some(event_envelope::Payload::TrainDeltaPublished(p)) => {
                assert_eq!(p.delta_id, "d1");
            }
            other => panic!("unexpected payload: {other:?}"),
        }

        let hash = coordinator.aggregate(&mut state, &cas).unwrap();
        assert!(coordinator.is_round_complete(&state));
        assert!(!hash.is_empty());

        let completed_evt = build_round_completed_event(&state, "coord-node");
        assert_eq!(
            completed_evt.r#type,
            EventType::FederatedRoundCompleted as i32
        );
        match &completed_evt.payload {
            Some(event_envelope::Payload::FederatedRoundCompleted(c)) => {
                assert!(c.success);
                assert_eq!(c.actual_participants, 2);
                assert!(c.resulting_model_ref.is_some());
            }
            other => panic!("unexpected payload: {other:?}"),
        }
    }

    #[test]
    fn test_policy_blocks_deltas() {
        let engine = PolicyEngine::new(PolicyConfig {
            allow_train: false,
            ..Default::default()
        });

        let decision = engine.can_share_deltas();
        assert!(!decision.is_allowed());

        let engine_ok = PolicyEngine::new(PolicyConfig {
            allow_train: true,
            ..Default::default()
        });
        assert!(engine_ok.can_share_deltas().is_allowed());
    }
}
