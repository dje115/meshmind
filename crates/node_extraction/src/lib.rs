//! Entity extraction from document chunks.
//!
//! Supports rule-based extraction (primary) and optional LLM-assisted extraction.
//! Produces structured entity records for person, company, email, phone, money, etc.

mod extract;
mod extract_llm;
mod normalize;

pub use extract::{extract_entities, merge_entities, ExtractionConfig, ExtractionResult};
pub use extract_llm::{
    extract_entities_llm_async, extract_entities_with_llm, parse_llm_entity_json,
};
pub use normalize::normalize_value;

/// Supported entity types from document text.
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
];

/// Extraction method: rule-based or LLM-assisted.
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

/// A single extracted entity.
#[derive(Debug, Clone)]
pub struct ExtractedEntity {
    pub entity_type: String,
    pub entity_value: String,
    pub normalized_value: String,
    pub confidence: f32,
    pub extraction_method: ExtractionMethod,
}

impl ExtractedEntity {
    /// Build entity_id: {entity_type}:{normalized_value}
    pub fn entity_id(&self) -> String {
        format!("{}:{}", self.entity_type, self.normalized_value)
    }
}
