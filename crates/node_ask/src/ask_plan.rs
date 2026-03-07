//! AskPlan: structured representation of the retrieval plan for a question.

use serde::{Deserialize, Serialize};

/// Intent classification for the question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AskIntent {
    ListEntities,
    CountEntities,
    DocumentLookup,
    CustomerHistory,
    QuoteHistory,
    PricingHistory,
    InvoiceStatus,
    AccountingSummary,
    ProfitLossQuery,
    TrendChange,
    AnomalyQuestion,
    GeneralDocumentSummary,
    WebFreshnessNeeded,
    Unknown,
}

/// Source type for a retrieval step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalSource {
    EntityQuery,
    FactQuery,
    EntityCards,
    FtsSearch,
    DocumentChunk,
    DocumentReference,
    PeerConsult,
    WebFallback,
}

/// A single retrieval step in the plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalStep {
    pub source_type: RetrievalSource,
    /// Query string or action (e.g. "person", "revenue", FTS query).
    pub query: String,
    /// Optional filters (e.g. entity_type, metric).
    pub filters: Vec<(String, String)>,
    pub limit: usize,
    pub required: bool,
}

/// Retrieval budget limits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalBudget {
    pub max_steps: usize,
    pub max_hits: usize,
    pub max_chunk_chars: usize,
    pub max_entity_results: usize,
    pub max_fact_results: usize,
}

impl Default for RetrievalBudget {
    fn default() -> Self {
        Self {
            max_steps: 5,
            max_hits: 100,
            max_chunk_chars: 95_000,
            max_entity_results: 50,
            max_fact_results: 20,
        }
    }
}

/// The complete retrieval plan for a question.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AskPlan {
    pub intent: AskIntent,
    pub retrieval_steps: Vec<RetrievalStep>,
    pub requires_peer_consult: bool,
    pub allows_web_fallback: bool,
    /// How the LLM should explain: "concise" | "detailed" | "cite_sources"
    pub explanation_mode: String,
    pub retrieval_budget: RetrievalBudget,
    /// Source priority for merging: e.g. ["entity", "fact", "document_chunk", "fts"]
    pub source_priority: Vec<String>,
}

impl AskPlan {
    pub fn new(intent: AskIntent) -> Self {
        Self {
            intent,
            retrieval_steps: Vec::new(),
            requires_peer_consult: false,
            allows_web_fallback: false,
            explanation_mode: "concise".into(),
            retrieval_budget: RetrievalBudget::default(),
            source_priority: vec![
                "entity".into(),
                "fact".into(),
                "document_chunk".into(),
                "fts".into(),
            ],
        }
    }

    pub fn with_step(mut self, step: RetrievalStep) -> Self {
        self.retrieval_steps.push(step);
        self
    }

    pub fn with_peer_consult(mut self, required: bool) -> Self {
        self.requires_peer_consult = required;
        self
    }

    pub fn with_web_fallback(mut self, allowed: bool) -> Self {
        self.allows_web_fallback = allowed;
        self
    }

    /// Brief summary for LLM prompt: intent and steps used.
    pub fn plan_summary(&self) -> String {
        let intent_str = format!("{:?}", self.intent)
            .replace('_', " ")
            .to_lowercase();
        let step_names: Vec<String> = self
            .retrieval_steps
            .iter()
            .map(|s| format!("{:?}", s.source_type).replace('_', " "))
            .collect();
        if step_names.is_empty() {
            format!("Intent: {}.", intent_str)
        } else {
            format!("Intent: {}. Steps: {}.", intent_str, step_names.join(", "))
        }
    }
}
