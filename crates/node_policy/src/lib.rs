//! Policy evaluation, redaction rules, and permissions.
//!
//! Default policy:
//! - Only tenant_id="public" allowed to replicate
//! - shareable=false blocks replication
//! - RESTRICTED sensitivity blocks replication
//! - Web research requires allow_web + research_web capability + redaction
//! - Training requires allow_train + dataset provenance

use node_proto::common::Sensitivity;
use node_proto::events::EventEnvelope;

#[derive(Debug, Clone)]
pub struct PolicyConfig {
    /// Additional tenant IDs allowed for replication beyond "public".
    pub allowed_tenant_ids: Vec<String>,
    /// Whether web research is allowed on this node.
    pub allow_web: bool,
    /// Whether this node has the research_web capability.
    pub research_web_capable: bool,
    /// Whether training is allowed on this node.
    pub allow_train: bool,
    /// Maximum sensitivity level allowed for replication (0=unspecified, 1=public, 2=internal, 3=restricted).
    pub max_replication_sensitivity: i32,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            allowed_tenant_ids: vec![],
            allow_web: false,
            research_web_capable: false,
            allow_train: false,
            max_replication_sensitivity: Sensitivity::Internal as i32,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow,
    Deny(String),
}

impl PolicyDecision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, PolicyDecision::Allow)
    }
}

pub struct PolicyEngine {
    config: PolicyConfig,
}

impl PolicyEngine {
    pub fn new(config: PolicyConfig) -> Self {
        Self { config }
    }

    pub fn with_defaults() -> Self {
        Self::new(PolicyConfig::default())
    }

    /// Can this event be accepted from a remote node via replication?
    pub fn can_accept_event(&self, event: &EventEnvelope) -> PolicyDecision {
        let tenant = event
            .tenant_id
            .as_ref()
            .map(|t| t.value.as_str())
            .unwrap_or("");

        if !self.is_tenant_allowed(tenant) {
            return PolicyDecision::Deny(format!("tenant '{tenant}' not allowed for replication"));
        }

        if event.sensitivity == Sensitivity::Restricted as i32 {
            return PolicyDecision::Deny("RESTRICTED sensitivity blocks replication".into());
        }

        if event.sensitivity > self.config.max_replication_sensitivity {
            return PolicyDecision::Deny(format!(
                "sensitivity {} exceeds max {}",
                event.sensitivity, self.config.max_replication_sensitivity
            ));
        }

        PolicyDecision::Allow
    }

    /// Can this CAS object be accepted from a remote node?
    pub fn can_accept_object(
        &self,
        _hash: &str,
        tenant_id: &str,
        sensitivity: i32,
    ) -> PolicyDecision {
        if !self.is_tenant_allowed(tenant_id) {
            return PolicyDecision::Deny(format!(
                "tenant '{tenant_id}' not allowed for replication"
            ));
        }

        if sensitivity == Sensitivity::Restricted as i32 {
            return PolicyDecision::Deny("RESTRICTED sensitivity blocks replication".into());
        }

        PolicyDecision::Allow
    }

    /// Can this artifact be shared (replicated outward)?
    pub fn can_share_artifact(
        &self,
        tenant_id: &str,
        sensitivity: i32,
        shareable: bool,
    ) -> PolicyDecision {
        if !shareable {
            return PolicyDecision::Deny("artifact is not shareable".into());
        }

        if sensitivity == Sensitivity::Restricted as i32 {
            return PolicyDecision::Deny("RESTRICTED sensitivity blocks sharing".into());
        }

        if !self.is_tenant_allowed(tenant_id) {
            return PolicyDecision::Deny(format!("tenant '{tenant_id}' not allowed for sharing"));
        }

        PolicyDecision::Allow
    }

    /// Can web research be performed?
    pub fn can_research_web(
        &self,
        allow_web_flag: bool,
        redaction_required: bool,
    ) -> PolicyDecision {
        if !allow_web_flag {
            return PolicyDecision::Deny("policy flag allow_web is false".into());
        }

        if !self.config.allow_web {
            return PolicyDecision::Deny("node policy does not allow web research".into());
        }

        if !self.config.research_web_capable {
            return PolicyDecision::Deny("node lacks research_web capability".into());
        }

        if !redaction_required {
            return PolicyDecision::Deny("redaction_required must be true for web research".into());
        }

        PolicyDecision::Allow
    }

    /// Can a training job be started?
    pub fn can_train(&self, allow_train_flag: bool) -> PolicyDecision {
        if !allow_train_flag {
            return PolicyDecision::Deny("policy flag allow_train is false".into());
        }

        if !self.config.allow_train {
            return PolicyDecision::Deny("node policy does not allow training".into());
        }

        PolicyDecision::Allow
    }

    fn is_tenant_allowed(&self, tenant_id: &str) -> bool {
        if tenant_id == "public" {
            return true;
        }
        self.config
            .allowed_tenant_ids
            .iter()
            .any(|t| t == tenant_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use node_proto::common::*;

    fn make_event(tenant: &str, sensitivity: Sensitivity) -> EventEnvelope {
        EventEnvelope {
            event_id: "e1".into(),
            tenant_id: Some(TenantId {
                value: tenant.into(),
            }),
            sensitivity: sensitivity as i32,
            ..Default::default()
        }
    }

    #[test]
    fn default_allows_public_tenant() {
        let engine = PolicyEngine::with_defaults();
        let event = make_event("public", Sensitivity::Public);
        assert!(engine.can_accept_event(&event).is_allowed());
    }

    #[test]
    fn default_denies_non_public_tenant() {
        let engine = PolicyEngine::with_defaults();
        let event = make_event("acme-corp", Sensitivity::Public);
        assert!(!engine.can_accept_event(&event).is_allowed());
    }

    #[test]
    fn allows_configured_tenant() {
        let engine = PolicyEngine::new(PolicyConfig {
            allowed_tenant_ids: vec!["acme-corp".into()],
            ..Default::default()
        });
        let event = make_event("acme-corp", Sensitivity::Public);
        assert!(engine.can_accept_event(&event).is_allowed());
    }

    #[test]
    fn denies_restricted_sensitivity() {
        let engine = PolicyEngine::with_defaults();
        let event = make_event("public", Sensitivity::Restricted);
        let decision = engine.can_accept_event(&event);
        assert!(!decision.is_allowed());
        if let PolicyDecision::Deny(reason) = decision {
            assert!(reason.contains("RESTRICTED"));
        }
    }

    #[test]
    fn allows_internal_sensitivity_by_default() {
        let engine = PolicyEngine::with_defaults();
        let event = make_event("public", Sensitivity::Internal);
        assert!(engine.can_accept_event(&event).is_allowed());
    }

    #[test]
    fn can_accept_object_public() {
        let engine = PolicyEngine::with_defaults();
        assert!(engine
            .can_accept_object("hash", "public", Sensitivity::Public as i32)
            .is_allowed());
    }

    #[test]
    fn cannot_accept_object_restricted() {
        let engine = PolicyEngine::with_defaults();
        assert!(!engine
            .can_accept_object("hash", "public", Sensitivity::Restricted as i32)
            .is_allowed());
    }

    #[test]
    fn cannot_accept_object_wrong_tenant() {
        let engine = PolicyEngine::with_defaults();
        assert!(!engine
            .can_accept_object("hash", "secret-corp", Sensitivity::Public as i32)
            .is_allowed());
    }

    #[test]
    fn share_artifact_shareable_public() {
        let engine = PolicyEngine::with_defaults();
        assert!(engine
            .can_share_artifact("public", Sensitivity::Public as i32, true)
            .is_allowed());
    }

    #[test]
    fn share_artifact_not_shareable() {
        let engine = PolicyEngine::with_defaults();
        let decision = engine.can_share_artifact("public", Sensitivity::Public as i32, false);
        assert!(!decision.is_allowed());
        if let PolicyDecision::Deny(reason) = decision {
            assert!(reason.contains("not shareable"));
        }
    }

    #[test]
    fn share_artifact_restricted_denied() {
        let engine = PolicyEngine::with_defaults();
        assert!(!engine
            .can_share_artifact("public", Sensitivity::Restricted as i32, true)
            .is_allowed());
    }

    #[test]
    fn share_artifact_wrong_tenant() {
        let engine = PolicyEngine::with_defaults();
        assert!(!engine
            .can_share_artifact("secret", Sensitivity::Public as i32, true)
            .is_allowed());
    }

    #[test]
    fn web_research_all_gates_pass() {
        let engine = PolicyEngine::new(PolicyConfig {
            allow_web: true,
            research_web_capable: true,
            ..Default::default()
        });
        assert!(engine.can_research_web(true, true).is_allowed());
    }

    #[test]
    fn web_research_policy_flag_false() {
        let engine = PolicyEngine::new(PolicyConfig {
            allow_web: true,
            research_web_capable: true,
            ..Default::default()
        });
        assert!(!engine.can_research_web(false, true).is_allowed());
    }

    #[test]
    fn web_research_node_not_allowed() {
        let engine = PolicyEngine::new(PolicyConfig {
            allow_web: false,
            research_web_capable: true,
            ..Default::default()
        });
        assert!(!engine.can_research_web(true, true).is_allowed());
    }

    #[test]
    fn web_research_not_capable() {
        let engine = PolicyEngine::new(PolicyConfig {
            allow_web: true,
            research_web_capable: false,
            ..Default::default()
        });
        assert!(!engine.can_research_web(true, true).is_allowed());
    }

    #[test]
    fn web_research_no_redaction() {
        let engine = PolicyEngine::new(PolicyConfig {
            allow_web: true,
            research_web_capable: true,
            ..Default::default()
        });
        let decision = engine.can_research_web(true, false);
        assert!(!decision.is_allowed());
        if let PolicyDecision::Deny(reason) = decision {
            assert!(reason.contains("redaction"));
        }
    }

    #[test]
    fn train_all_gates_pass() {
        let engine = PolicyEngine::new(PolicyConfig {
            allow_train: true,
            ..Default::default()
        });
        assert!(engine.can_train(true).is_allowed());
    }

    #[test]
    fn train_policy_flag_false() {
        let engine = PolicyEngine::new(PolicyConfig {
            allow_train: true,
            ..Default::default()
        });
        assert!(!engine.can_train(false).is_allowed());
    }

    #[test]
    fn train_node_not_allowed() {
        let engine = PolicyEngine::new(PolicyConfig {
            allow_train: false,
            ..Default::default()
        });
        assert!(!engine.can_train(true).is_allowed());
    }

    #[test]
    fn default_policy_denies_web() {
        let engine = PolicyEngine::with_defaults();
        assert!(!engine.can_research_web(true, true).is_allowed());
    }

    #[test]
    fn default_policy_denies_train() {
        let engine = PolicyEngine::with_defaults();
        assert!(!engine.can_train(true).is_allowed());
    }
}
