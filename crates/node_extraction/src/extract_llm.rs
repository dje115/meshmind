//! Optional LLM-assisted entity extraction from document chunks.
//!
//! Used when enable_llm_entity_extraction is true and rule-based extraction
//! finds few entities in a long chunk.

use std::sync::Arc;

use node_ai::{GenerateRequest, InferenceBackend};

use super::extract::{extract_entities, merge_entities, ExtractionConfig, ExtractionResult};
use super::relationships::{ExtractedRelationship, RELATIONSHIP_TYPES};
use crate::classify::ClassificationMethod;
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
                classification_method: ClassificationMethod::LlmAssisted,
            });
        }
    }
    entities
}

/// Call LLM to classify a single phrase. Returns (entity_type, confidence) or None.
pub async fn classify_phrase_llm_async(
    backend: &Arc<dyn InferenceBackend>,
    phrase: &str,
    context: &str,
) -> Option<(String, f32)> {
    let context_trim: String = context.chars().take(300).collect();
    let prompt = format!(
        r#"Classify the entity type of this phrase extracted from a business document.

Phrase:
"{phrase}"

Context:
"{context_trim}"

Choose one type: person, company, product, location, money, email, phone, date, quote_number, invoice_number, unknown.

Return ONLY valid JSON with no other text:
{{"type": "<type>", "confidence": <0-1>}}"#
    );
    let req = GenerateRequest {
        prompt,
        system: Some(
            "You are an entity classification assistant. Reply only with valid JSON.".into(),
        ),
        max_tokens: 64,
        temperature: 0.1,
        stop: vec![],
    };
    let resp = backend.generate(req).await.ok()?;
    let text = resp.text.trim();
    #[derive(serde::Deserialize)]
    struct ClassifyResponse {
        #[serde(rename = "type")]
        entity_type: String,
        confidence: f32,
    }
    let parsed: ClassifyResponse = serde_json::from_str(text).ok()?;
    let confidence = parsed.confidence.clamp(0.0, 1.0);
    Some((parsed.entity_type.trim().to_lowercase(), confidence))
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

const RELATIONSHIP_EXTRACTION_PROMPT: &str = r#"Given document text and a list of extracted entities, identify relationships between them.

Entities (format: "value" [type]):
"#;

/// LLM returns JSON: [{"from": "entity value", "relationship": "type", "to": "entity value", "confidence": 0.0-1.0}]
#[derive(serde::Deserialize)]
struct LlmRelationshipItem {
    from: String,
    relationship: String,
    to: String,
    #[serde(default = "default_confidence")]
    confidence: f32,
}

fn default_confidence() -> f32 {
    0.6
}

/// Parse LLM relationship JSON and validate against entities. Returns only valid relationships
/// where both from and to match an entity in the chunk.
pub fn parse_llm_relationship_json(
    json: &str,
    entities: &[ExtractedEntity],
) -> Vec<ExtractedRelationship> {
    let parsed: Vec<LlmRelationshipItem> = match serde_json::from_str(json) {
        Ok(p) => p,
        Err(_) => return vec![],
    };
    let allowed: std::collections::HashSet<&str> = RELATIONSHIP_TYPES.iter().copied().collect();

    let find_entity = |value: &str| -> Option<&ExtractedEntity> {
        let v = value.trim();
        if v.is_empty() {
            return None;
        }
        let v_lower = v.to_lowercase();
        entities.iter().find(|e| {
            e.entity_value.eq_ignore_ascii_case(v) || e.entity_value.to_lowercase() == v_lower
        })
    };

    let mut out = Vec::new();
    for item in parsed {
        let from_trim = item.from.trim();
        let to_trim = item.to.trim();
        if from_trim.is_empty() || to_trim.is_empty() || from_trim.eq_ignore_ascii_case(to_trim) {
            continue;
        }
        let rel_lower = item.relationship.trim().to_lowercase();
        let rel = rel_lower.as_str();
        if !allowed.contains(rel) {
            continue;
        }
        let Some(from_e) = find_entity(from_trim) else {
            continue;
        };
        let Some(to_e) = find_entity(to_trim) else {
            continue;
        };
        let confidence = item.confidence.clamp(0.0, 1.0);
        out.push(ExtractedRelationship {
            from_entity_id: from_e.entity_id(),
            from_entity_value: from_e.entity_value.clone(),
            relationship_type: rel.to_string(),
            to_entity_id: to_e.entity_id(),
            to_entity_value: to_e.entity_value.clone(),
            confidence,
            extraction_method: "llm_assisted".to_string(),
        });
    }
    out
}

/// Call LLM to extract relationships. Returns only relationships validated against entities.
pub async fn extract_relationships_llm_async(
    backend: &Arc<dyn InferenceBackend>,
    chunk_text: &str,
    entities: &[ExtractedEntity],
) -> Vec<ExtractedRelationship> {
    if entities.len() < 2 {
        return vec![];
    }
    let entity_list: String = entities
        .iter()
        .map(|e| format!(r#"  "{}" [{}]"#, e.entity_value, e.entity_type))
        .collect::<Vec<_>>()
        .join("\n");
    let rel_types = RELATIONSHIP_TYPES.join(", ");
    let prompt = format!(
        r#"{RELATIONSHIP_EXTRACTION_PROMPT}
{entity_list}

Allowed relationship types: {rel_types}

Return ONLY a JSON array of relationships. Each item: {{"from": "<entity value>", "relationship": "<type>", "to": "<entity value>", "confidence": <0-1>}}
Use only entity values from the list above. No fabrication.

Text:
"{chunk_text}""#
    );
    let req = GenerateRequest {
        prompt,
        system: Some(
            "You are a relationship extraction assistant. Reply only with a valid JSON array."
                .into(),
        ),
        max_tokens: 512,
        temperature: 0.1,
        stop: vec![],
    };
    match backend.generate(req).await {
        Ok(resp) => parse_llm_relationship_json(resp.text.trim(), entities),
        Err(_) => vec![],
    }
}

/// Extract relationships with optional LLM augmentation.
/// When config enables LLM, entities >= 2, and rule-based found few, calls backend and merges.
pub async fn extract_relationships_with_llm(
    backend: Option<&Arc<dyn InferenceBackend>>,
    chunk_text: &str,
    entities: &[ExtractedEntity],
    rule_based: Vec<ExtractedRelationship>,
    config: &ExtractionConfig,
) -> Vec<ExtractedRelationship> {
    let use_llm = config.enable_llm_relationship_extraction
        && backend.is_some()
        && entities.len() >= 2
        && rule_based.len() < config.llm_relationship_count_threshold;

    if !use_llm {
        return rule_based;
    }

    let backend = backend.unwrap();
    let llm_rels = extract_relationships_llm_async(backend, chunk_text, entities).await;
    let mut seen = std::collections::HashSet::new();
    for r in &rule_based {
        seen.insert((
            r.from_entity_id.clone(),
            r.relationship_type.clone(),
            r.to_entity_id.clone(),
        ));
    }
    let mut merged = rule_based;
    for r in llm_rels {
        let key = (
            r.from_entity_id.clone(),
            r.relationship_type.clone(),
            r.to_entity_id.clone(),
        );
        if !seen.contains(&key) {
            seen.insert(key);
            merged.push(r);
        }
    }
    merged
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

    #[test]
    fn parse_llm_relationship_json_valid() {
        use crate::classify::ClassificationMethod;
        use crate::ExtractedEntity;

        let entities = vec![
            ExtractedEntity {
                entity_type: "person".into(),
                entity_value: "Alice Smith".into(),
                normalized_value: "alice smith".into(),
                confidence: 0.9,
                extraction_method: ExtractionMethod::RuleBased,
                classification_method: ClassificationMethod::RuleBased,
            },
            ExtractedEntity {
                entity_type: "company".into(),
                entity_value: "Test Corp".into(),
                normalized_value: "test corp".into(),
                confidence: 0.9,
                extraction_method: ExtractionMethod::RuleBased,
                classification_method: ClassificationMethod::RuleBased,
            },
        ];
        let json = r#"[{"from":"Alice Smith","relationship":"works_for","to":"Test Corp","confidence":0.85}]"#;
        let rels = parse_llm_relationship_json(json, &entities);
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].relationship_type, "works_for");
        assert_eq!(rels[0].from_entity_value, "Alice Smith");
        assert_eq!(rels[0].to_entity_value, "Test Corp");
        assert_eq!(rels[0].extraction_method, "llm_assisted");
        assert!((rels[0].confidence - 0.85).abs() < 0.01);
    }

    #[test]
    fn parse_llm_relationship_json_rejects_unmatched_entities() {
        use crate::classify::ClassificationMethod;
        use crate::ExtractedEntity;

        let entities = vec![ExtractedEntity {
            entity_type: "person".into(),
            entity_value: "Alice Smith".into(),
            normalized_value: "alice smith".into(),
            confidence: 0.9,
            extraction_method: ExtractionMethod::RuleBased,
            classification_method: ClassificationMethod::RuleBased,
        }];
        // "Test Corp" is not in entities - should reject
        let json = r#"[{"from":"Alice Smith","relationship":"works_for","to":"Test Corp","confidence":0.85}]"#;
        let rels = parse_llm_relationship_json(json, &entities);
        assert!(rels.is_empty());
    }

    #[test]
    fn parse_llm_relationship_json_rejects_invalid_relationship_type() {
        use crate::classify::ClassificationMethod;
        use crate::ExtractedEntity;

        let entities = vec![
            ExtractedEntity {
                entity_type: "person".into(),
                entity_value: "Alice".into(),
                normalized_value: "alice".into(),
                confidence: 0.9,
                extraction_method: ExtractionMethod::RuleBased,
                classification_method: ClassificationMethod::RuleBased,
            },
            ExtractedEntity {
                entity_type: "company".into(),
                entity_value: "Acme".into(),
                normalized_value: "acme".into(),
                confidence: 0.9,
                extraction_method: ExtractionMethod::RuleBased,
                classification_method: ClassificationMethod::RuleBased,
            },
        ];
        let json =
            r#"[{"from":"Alice","relationship":"invalid_type","to":"Acme","confidence":0.8}]"#;
        let rels = parse_llm_relationship_json(json, &entities);
        assert!(rels.is_empty());
    }

    #[tokio::test]
    async fn extract_relationships_llm_mock_backend() {
        use node_ai_mock::MockBackend;
        use std::sync::Arc;

        use crate::classify::ClassificationMethod;
        use crate::ExtractedEntity;

        let backend: Arc<dyn InferenceBackend> = Arc::new(MockBackend::new());
        let entities = vec![
            ExtractedEntity {
                entity_type: "person".into(),
                entity_value: "Alice Smith".into(),
                normalized_value: "alice smith".into(),
                confidence: 0.9,
                extraction_method: ExtractionMethod::RuleBased,
                classification_method: ClassificationMethod::RuleBased,
            },
            ExtractedEntity {
                entity_type: "company".into(),
                entity_value: "Test Corp".into(),
                normalized_value: "test corp".into(),
                confidence: 0.9,
                extraction_method: ExtractionMethod::RuleBased,
                classification_method: ClassificationMethod::RuleBased,
            },
        ];
        let chunk_text = "Alice Smith works at Test Corp.";
        let rels = extract_relationships_llm_async(&backend, chunk_text, &entities).await;
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].relationship_type, "works_for");
        assert_eq!(rels[0].from_entity_value, "Alice Smith");
        assert_eq!(rels[0].to_entity_value, "Test Corp");
    }
}
