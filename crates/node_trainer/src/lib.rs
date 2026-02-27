//! Bounded on-device training: job queue, dataset manifests, eval gates, model registry.
//!
//! Design:
//! - CPU-only training jobs with bounded time
//! - Dataset manifests reference CAS objects
//! - Eval gate: new model must beat old by threshold
//! - Model registry with instant rollback

use std::collections::HashMap;
use std::sync::Arc;

use node_policy::{PolicyDecision, PolicyEngine};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrainingJob {
    pub job_id: String,
    pub model_name: String,
    pub dataset_manifest: DatasetManifest,
    pub max_duration_secs: u64,
    pub eval_threshold: f64,
    pub status: JobStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatasetManifest {
    pub name: String,
    pub cas_refs: Vec<String>,
    pub sample_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum JobStatus {
    Queued,
    Running,
    Completed { score: f64 },
    Failed { reason: String },
    Rejected { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelVersion {
    pub version: String,
    pub model_name: String,
    pub score: f64,
    pub cas_ref: String,
    pub created_at_ms: u64,
}

/// Model registry: tracks versioned models with instant rollback.
pub struct ModelRegistry {
    models: HashMap<String, Vec<ModelVersion>>,
    active: HashMap<String, usize>,
}

impl ModelRegistry {
    pub fn new() -> Self {
        Self {
            models: HashMap::new(),
            active: HashMap::new(),
        }
    }

    pub fn register(&mut self, version: ModelVersion) {
        let name = version.model_name.clone();
        let versions = self.models.entry(name.clone()).or_default();
        versions.push(version);
        self.active.insert(name, versions.len() - 1);
    }

    pub fn active_version(&self, model_name: &str) -> Option<&ModelVersion> {
        let idx = self.active.get(model_name)?;
        self.models.get(model_name)?.get(*idx)
    }

    /// Rollback to a previous version. Returns true if successful.
    pub fn rollback(&mut self, model_name: &str) -> bool {
        if let Some(idx) = self.active.get_mut(model_name) {
            if *idx > 0 {
                *idx -= 1;
                return true;
            }
        }
        false
    }

    pub fn versions(&self, model_name: &str) -> Vec<&ModelVersion> {
        self.models
            .get(model_name)
            .map(|v| v.iter().collect())
            .unwrap_or_default()
    }
}

impl Default for ModelRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Training coordinator: queues jobs, checks policy, runs eval gates.
pub struct Trainer {
    policy: Arc<PolicyEngine>,
    registry: Arc<Mutex<ModelRegistry>>,
    max_concurrent: usize,
}

impl Trainer {
    pub fn new(policy: Arc<PolicyEngine>, registry: Arc<Mutex<ModelRegistry>>) -> Self {
        Self {
            policy,
            registry,
            max_concurrent: 1,
        }
    }

    /// Submit a training job. Returns the job with updated status after policy check.
    pub fn submit(&self, mut job: TrainingJob) -> TrainingJob {
        match self.policy.can_train(true) {
            PolicyDecision::Allow => {
                job.status = JobStatus::Queued;
            }
            PolicyDecision::Deny(reason) => {
                job.status = JobStatus::Rejected { reason };
            }
        }
        job
    }

    /// Simulate training and eval gate. In a real implementation this would
    /// spawn a bounded CPU task.
    pub async fn run_job(&self, mut job: TrainingJob) -> TrainingJob {
        if job.status != JobStatus::Queued {
            return job;
        }

        job.status = JobStatus::Running;

        // Simulated training score
        let score = 0.85;

        let registry = self.registry.lock().await;
        let current_score = registry
            .active_version(&job.model_name)
            .map(|v| v.score)
            .unwrap_or(0.0);
        drop(registry);

        if score - current_score >= job.eval_threshold {
            job.status = JobStatus::Completed { score };

            let version = ModelVersion {
                version: format!("v{}", &uuid::Uuid::new_v4().to_string()[..8]),
                model_name: job.model_name.clone(),
                score,
                cas_ref: format!("model-{}", job.job_id),
                created_at_ms: 0,
            };

            let mut registry = self.registry.lock().await;
            registry.register(version);
        } else {
            job.status = JobStatus::Failed {
                reason: format!(
                    "eval gate: score {score:.3} did not beat current {current_score:.3} by threshold {:.3}",
                    job.eval_threshold
                ),
            };
        }

        job
    }

    pub fn max_concurrent(&self) -> usize {
        self.max_concurrent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_manifest() -> DatasetManifest {
        DatasetManifest {
            name: "test-dataset".into(),
            cas_refs: vec!["hash-1".into(), "hash-2".into()],
            sample_count: 100,
        }
    }

    fn make_job(id: &str) -> TrainingJob {
        TrainingJob {
            job_id: id.into(),
            model_name: "test-model".into(),
            dataset_manifest: make_manifest(),
            max_duration_secs: 60,
            eval_threshold: 0.1,
            status: JobStatus::Queued,
        }
    }

    #[test]
    fn model_registry_basics() {
        let mut reg = ModelRegistry::new();

        reg.register(ModelVersion {
            version: "v1".into(),
            model_name: "test".into(),
            score: 0.7,
            cas_ref: "hash-v1".into(),
            created_at_ms: 1000,
        });

        reg.register(ModelVersion {
            version: "v2".into(),
            model_name: "test".into(),
            score: 0.85,
            cas_ref: "hash-v2".into(),
            created_at_ms: 2000,
        });

        let active = reg.active_version("test").unwrap();
        assert_eq!(active.version, "v2");
        assert_eq!(reg.versions("test").len(), 2);

        assert!(reg.rollback("test"));
        let active = reg.active_version("test").unwrap();
        assert_eq!(active.version, "v1");

        assert!(!reg.rollback("nonexistent"));
    }

    fn training_policy() -> Arc<PolicyEngine> {
        use node_policy::PolicyConfig;
        Arc::new(PolicyEngine::new(PolicyConfig {
            allow_train: true,
            ..Default::default()
        }))
    }

    #[test]
    fn policy_gates_training() {
        let policy = training_policy();
        let registry = Arc::new(Mutex::new(ModelRegistry::new()));
        let trainer = Trainer::new(policy, registry);

        let job = trainer.submit(make_job("j1"));
        assert_eq!(job.status, JobStatus::Queued);
    }

    #[test]
    fn default_policy_denies_training() {
        let policy = Arc::new(PolicyEngine::with_defaults());
        let registry = Arc::new(Mutex::new(ModelRegistry::new()));
        let trainer = Trainer::new(policy, registry);

        let job = trainer.submit(make_job("j1"));
        assert!(matches!(job.status, JobStatus::Rejected { .. }));
    }

    #[tokio::test]
    async fn run_job_passes_eval_gate() {
        let policy = training_policy();
        let registry = Arc::new(Mutex::new(ModelRegistry::new()));
        let trainer = Trainer::new(policy, registry.clone());

        let job = trainer.submit(make_job("j1"));
        let result = trainer.run_job(job).await;

        assert!(matches!(result.status, JobStatus::Completed { .. }));

        let reg = registry.lock().await;
        assert!(reg.active_version("test-model").is_some());
    }

    #[tokio::test]
    async fn eval_gate_rejects_low_improvement() {
        let policy = training_policy();
        let registry = Arc::new(Mutex::new(ModelRegistry::new()));

        {
            let mut reg = registry.lock().await;
            reg.register(ModelVersion {
                version: "v1".into(),
                model_name: "test-model".into(),
                score: 0.80,
                cas_ref: "hash-v1".into(),
                created_at_ms: 1000,
            });
        }

        let trainer = Trainer::new(policy, registry);

        let mut job = make_job("j2");
        job.eval_threshold = 0.5; // requires huge improvement
        let job = trainer.submit(job);
        let result = trainer.run_job(job).await;

        assert!(matches!(result.status, JobStatus::Failed { .. }));
    }

    #[tokio::test]
    async fn rollback_after_bad_model() {
        let policy = training_policy();
        let registry = Arc::new(Mutex::new(ModelRegistry::new()));

        {
            let mut reg = registry.lock().await;
            reg.register(ModelVersion {
                version: "v1".into(),
                model_name: "test-model".into(),
                score: 0.7,
                cas_ref: "hash-v1".into(),
                created_at_ms: 1000,
            });
        }

        let trainer = Trainer::new(policy, registry.clone());
        let job = trainer.submit(make_job("j3"));
        let _result = trainer.run_job(job).await;

        {
            let mut reg = registry.lock().await;
            let active = reg.active_version("test-model").unwrap();
            assert!(active.version.starts_with("v"));
            assert_ne!(active.version, "v1");

            assert!(reg.rollback("test-model"));
            let active = reg.active_version("test-model").unwrap();
            assert_eq!(active.version, "v1");
        }
    }
}
