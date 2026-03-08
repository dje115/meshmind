//! Entity extraction from document chunks.
//!
//! Two-stage pipeline: CandidateExtraction (find spans) and EntityClassification (assign type).
//! Supports strong rules, vocabulary lookup, and optional LLM-assisted classification.

mod classify;
mod extract;
mod extract_llm;
mod normalize;
mod relationships;

pub use classify::{
    classify_candidate, classify_with_strong_rules, strong_rule_classify, ClassificationMethod,
    VocabEntry, VocabularyLookup,
};
pub use extract::{
    extract_candidates, extract_entities, merge_entities, ExtractionConfig, ExtractionResult,
};
pub use extract_llm::{
    classify_phrase_llm_async, extract_entities_llm_async, extract_entities_with_llm,
    extract_relationships_llm_async, extract_relationships_with_llm, parse_llm_entity_json,
    parse_llm_relationship_json,
};
pub use normalize::normalize_value;
pub use relationships::{extract_relationships, ExtractedRelationship, RELATIONSHIP_TYPES};

/// Supported entity types from document text (includes "unknown" for unresolved candidates).
pub const ENTITY_TYPES: &[&str] = &[
    "person",
    "company",
    "email",
    "phone",
    "money",
    "date",
    "location",
    "product",
    "quote_number",
    "invoice_number",
    "unknown",
];

/// Extraction method: rule-based or LLM-assisted (how the span was found).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractionMethod {
    RuleBased,
    LlmAssisted,
}

impl std::fmt::Display for ExtractionMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RuleBased => write!(f, "rule_based"),
            Self::LlmAssisted => write!(f, "llm_assisted"),
        }
    }
}

/// A candidate phrase from candidate extraction (before classification).
#[derive(Debug, Clone)]
pub struct EntityCandidate {
    pub entity_value: String,
    pub normalized_value: String,
    pub initial_type: String,
    pub initial_confidence: f32,
    pub extraction_method: ExtractionMethod,
}

/// A single extracted entity with classification provenance.
#[derive(Debug, Clone)]
pub struct ExtractedEntity {
    pub entity_type: String,
    pub entity_value: String,
    pub normalized_value: String,
    pub confidence: f32,
    pub extraction_method: ExtractionMethod,
    pub classification_method: ClassificationMethod,
}

impl ExtractedEntity {
    /// Build entity_id: {entity_type}:{normalized_value}
    pub fn entity_id(&self) -> String {
        format!("{}:{}", self.entity_type, self.normalized_value)
    }
}
