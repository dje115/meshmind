//! Optional LLM-assisted entity extraction from document chunks.
//!
//! Used when enable_llm_entity_extraction is true and rule-based extraction
//! finds few entities in a long chunk.

use std::sync::Arc;

use node_ai::{GenerateRequest, InferenceBackend};

use super::extract::{extract_entities, merge_entities, ExtractionConfig, ExtractionResult};
use crate::{normalize::normalize_value, ExtractedEntity, ExtractionMethod};

const ENTITY_EXTRACTION_PROMPT: &str = r#"Extract entities from the following document text.

Return ONLY valid JSON with these exact keys (use empty arrays [] if none found):
{
  "people": [],
  "companies": [],
  "emails": [],
  "phones": [],
  "money": [],
  "dates": [],
  "locations": [],
  "products": [],
  "invoice_numbers": [],
  "quote_numbers": []
}

Rules: Only include entities that clearly appear in the text. No fabrication.
Text:
"#;

/// Map LLM JSON keys to our entity types.
const KEY_TO_TYPE: &[(&str, &str)] = &[
    ("people", "person"),
    ("companies", "company"),
    ("emails", "email"),
    ("phones", "phone"),
    ("money", "money"),
    ("dates", "date"),
    ("locations", "location"),
    ("products", "product"),
    ("invoice_numbers", "invoice_number"),
    ("quote_numbers", "quote_number"),
];

#[derive(serde::Deserialize, Default)]
struct LlmEntityResponse {
    #[serde(default)]
    people: Vec<String>,
    #[serde(default)]
    companies: Vec<String>,
    #[serde(default)]
    emails: Vec<String>,
    #[serde(default)]
    phones: Vec<String>,
    #[serde(default)]
    money: Vec<String>,
    #[serde(default)]
    dates: Vec<String>,
    #[serde(default)]
    locations: Vec<String>,
    #[serde(default)]
    products: Vec<String>,
    #[serde(default)]
    invoice_numbers: Vec<String>,
    #[serde(default)]
    quote_numbers: Vec<String>,
}

/// Parse LLM JSON response into ExtractedEntity list.
/// Discards invalid or empty values.
pub fn parse_llm_entity_json(json: &str) -> Vec<ExtractedEntity> {
    let parsed: LlmEntityResponse = match serde_json::from_str(json) {
        Ok(p) => p,
        Err(_) => return vec![],
    };

    let mut entities = Vec::new();
    let rows: &[(&str, Vec<String>)] = &[
        ("people", parsed.people),
        ("companies", parsed.companies),
        ("emails", parsed.emails),
        ("phones", parsed.phones),
        ("money", parsed.money),
        ("dates", parsed.dates),
        ("locations", parsed.locations),
        ("products", parsed.products),
        ("invoice_numbers", parsed.invoice_numbers),
        ("quote_numbers", parsed.quote_numbers),
    ];

    for (key, values) in rows {
        let entity_type = KEY_TO_TYPE
            .iter()
            .find(|(k, _)| *k == *key)
            .map(|(_, t)| *t)
            .unwrap_or("unknown");
        for v in values {
            let v = v.trim();
            if v.is_empty() || v.len() > 200 {
                continue;
            }
            let normalized = normalize_value(entity_type, v);
            entities.push(ExtractedEntity {
                entity_type: entity_type.to_string(),
                entity_value: v.to_string(),
                normalized_value: normalized,
                confidence: 0.7, // Lower than rule-based; LLM may hallucinate
                extraction_method: ExtractionMethod::LlmAssisted,
            });
        }
    }
    entities
}

/// Call LLM to extract entities from text. Returns entities with extraction_method = llm_assisted.
pub async fn extract_entities_llm_async(
    backend: &Arc<dyn InferenceBackend>,
    text: &str,
) -> Vec<ExtractedEntity> {
    let prompt = format!("{ENTITY_EXTRACTION_PROMPT}\n{text}");
    let req = GenerateRequest {
        prompt,
        system: Some("You are an entity extraction assistant. Reply only with valid JSON.".into()),
        max_tokens: 1024,
        temperature: 0.1,
        stop: vec![],
    };
    match backend.generate(req).await {
        Ok(resp) => parse_llm_entity_json(&resp.text),
        Err(_) => vec![],
    }
}

/// Extract entities with optional LLM augmentation.
/// When config enables LLM and conditions are met, calls backend and merges results.
pub async fn extract_entities_with_llm(
    backend: Option<&Arc<dyn InferenceBackend>>,
    text: &str,
    config: &ExtractionConfig,
) -> ExtractionResult {
    let rule_result = extract_entities(text);
    let rule_entities = rule_result.entities;

    let use_llm = config.enable_llm_entity_extraction
        && backend.is_some()
        && text.len() >= config.llm_chunk_length_threshold
        && rule_entities.len() < config.llm_entity_count_threshold;

    if !use_llm {
        return ExtractionResult {
            entities: rule_entities,
        };
    }

    let backend = backend.unwrap();
    let llm_entities = extract_entities_llm_async(backend, text).await;
    let merged = merge_entities(rule_entities, llm_entities);
    ExtractionResult { entities: merged }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_llm_json() {
        let json = r#"{
          "people": ["John Smith", "Jane Doe"],
          "companies": ["Acme Ltd"],
          "emails": [],
          "phones": [],
          "money": [],
          "dates": [],
          "locations": [],
          "products": [],
          "invoice_numbers": ["INV-001"],
          "quote_numbers": []
        }"#;
        let entities = parse_llm_entity_json(json);
        assert!(entities
            .iter()
            .any(|e| e.entity_type == "person" && e.entity_value.contains("John")));
        assert!(entities.iter().any(|e| e.entity_type == "company"));
        assert!(entities.iter().any(|e| e.entity_type == "invoice_number"));
        assert!(entities
            .iter()
            .all(|e| e.extraction_method == ExtractionMethod::LlmAssisted));
        assert!(entities.iter().all(|e| e.confidence == 0.7));
    }

    #[test]
    fn parse_invalid_json_returns_empty() {
        let entities = parse_llm_entity_json("not json");
        assert!(entities.is_empty());
    }

    #[test]
    fn parse_empty_objects_returns_empty() {
        let entities = parse_llm_entity_json(
            r#"{"people":[],"companies":[],"emails":[],"phones":[],"money":[],"dates":[],"locations":[],"products":[],"invoice_numbers":[],"quote_numbers":[]}"#,
        );
        assert!(entities.is_empty());
    }

    #[tokio::test]
    async fn extract_entities_llm_mock_backend() {
        use node_ai_mock::MockBackend;
        use std::sync::Arc;

        let backend: Arc<dyn InferenceBackend> = Arc::new(MockBackend::new());
        let text = "Contact Alice Smith at test@example.com. Invoice INV-001 from Test Corp.";
        let entities = extract_entities_llm_async(&backend, text).await;
        assert!(!entities.is_empty());
        assert!(entities
            .iter()
            .any(|e| e.entity_type == "person" && e.entity_value.contains("Alice")));
        assert!(entities.iter().any(|e| e.entity_type == "company"));
        assert!(entities.iter().any(|e| e.entity_type == "invoice_number"));
        assert!(entities
            .iter()
            .all(|e| e.extraction_method == ExtractionMethod::LlmAssisted));
    }
}
