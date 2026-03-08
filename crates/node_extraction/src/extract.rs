//! Rule-based candidate extraction from document chunks.
//! Classification (strong rules, vocabulary, LLM) is in classify.rs.

use regex::Regex;
use std::collections::HashSet;

use super::{EntityCandidate, ExtractedEntity, ExtractionMethod};
use crate::classify::{strong_rule_classify, ClassificationMethod};
use crate::normalize::normalize_value;

/// Configuration for entity extraction.
#[derive(Debug, Clone)]
pub struct ExtractionConfig {
    /// Enable LLM-assisted extraction when rule-based finds few entities.
    pub enable_llm_entity_extraction: bool,
    /// Minimum chunk length (chars) to consider LLM extraction.
    pub llm_chunk_length_threshold: usize,
    /// Rule-based entity count below which to try LLM.
    pub llm_entity_count_threshold: usize,
    /// Enable LLM-assisted relationship extraction when rule-based finds few relationships.
    pub enable_llm_relationship_extraction: bool,
    /// Rule-based relationship count below which to try LLM (when 2+ entities).
    pub llm_relationship_count_threshold: usize,
}

impl Default for ExtractionConfig {
    fn default() -> Self {
        Self {
            // Enable LLM by default when backend is available; improves classification for legal/property docs.
            enable_llm_entity_extraction: true,
            llm_chunk_length_threshold: 500,
            llm_entity_count_threshold: 2,
            enable_llm_relationship_extraction: false,
            llm_relationship_count_threshold: 2,
        }
    }
}

/// Result of extraction on a chunk.
#[derive(Debug, Clone)]
pub struct ExtractionResult {
    pub entities: Vec<ExtractedEntity>,
}

/// Extract candidates (phrase + initial type). Title-case phrases without company suffix
/// get initial_type "unknown" so classification can assign product/location/person.
pub fn extract_candidates(text: &str) -> Vec<EntityCandidate> {
    let mut candidates = Vec::new();

    // Email
    if let Ok(re) = Regex::new(r#"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}"#) {
        for m in re.find_iter(text) {
            let v = m.as_str();
            if v.len() < 100 {
                candidates.push(EntityCandidate {
                    entity_value: v.to_string(),
                    normalized_value: normalize_value("email", v),
                    initial_type: "email".to_string(),
                    initial_confidence: 0.95,
                    extraction_method: ExtractionMethod::RuleBased,
                });
            }
        }
    }

    // Phone
    if let Ok(re) =
        Regex::new(r#"(?:\+44|0044|0)\s*[\d\s\-()]{9,14}\d|(?:\+\d{1,3}\s)?[\d\s\-()]{10,16}\d"#)
    {
        for m in re.find_iter(text) {
            let v = m.as_str().trim();
            let digits: String = v.chars().filter(|c| c.is_ascii_digit()).collect();
            if digits.len() >= 10 && digits.len() <= 15 {
                candidates.push(EntityCandidate {
                    entity_value: v.to_string(),
                    normalized_value: normalize_value("phone", v),
                    initial_type: "phone".to_string(),
                    initial_confidence: 0.9,
                    extraction_method: ExtractionMethod::RuleBased,
                });
            }
        }
    }

    // Money
    if let Ok(re) =
        Regex::new(r#"[$€£]\s*[\d,]+(?:\.\d{2})?|\d+(?:,\d{3})*(?:\.\d{2})?\s*(?:GBP|USD|EUR)"#)
    {
        for m in re.find_iter(text) {
            let v = m.as_str();
            if v.len() < 50 {
                candidates.push(EntityCandidate {
                    entity_value: v.to_string(),
                    normalized_value: normalize_value("money", v),
                    initial_type: "money".to_string(),
                    initial_confidence: 0.92,
                    extraction_method: ExtractionMethod::RuleBased,
                });
            }
        }
    }

    // Date
    if let Ok(re) = Regex::new(
        r#"\d{1,2}[-/]\d{1,2}[-/]\d{2,4}|\d{4}[-/]\d{1,2}[-/]\d{1,2}|(?:Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)[a-z]*\.?\s+\d{1,2},?\s+\d{4}"#,
    ) {
        for m in re.find_iter(text) {
            let v = m.as_str();
            candidates.push(EntityCandidate {
                entity_value: v.to_string(),
                normalized_value: normalize_value("date", v),
                initial_type: "date".to_string(),
                initial_confidence: 0.9,
                extraction_method: ExtractionMethod::RuleBased,
            });
        }
    }

    // Quote / invoice numbers
    if let Ok(re) = Regex::new(
        r#"(?i)(?:quote|quotation)\s*(?:no\.?|#)?\s*:?\s*([A-Z0-9\-/]{3,30})|(?:inv\.?\s*no\.?|invoice\s*(?:no\.?|#)?)\s*:?\s*([A-Z0-9\-/]{3,30})"#,
    ) {
        for cap in re.captures_iter(text) {
            let (entity_type, v) = if let Some(m) = cap.get(1) {
                ("quote_number", m.as_str().trim())
            } else if let Some(m) = cap.get(2) {
                ("invoice_number", m.as_str().trim())
            } else {
                continue;
            };
            if !v.is_empty() {
                candidates.push(EntityCandidate {
                    entity_value: v.to_string(),
                    normalized_value: normalize_value(entity_type, v),
                    initial_type: entity_type.to_string(),
                    initial_confidence: 0.88,
                    extraction_method: ExtractionMethod::RuleBased,
                });
            }
        }
    }

    // Company: Ltd, Limited, Plc, LLP, Inc, Corp (strong rule)
    if let Ok(re) = Regex::new(
        r#"(?:\b[A-Z][a-zA-Z0-9\s&'-]{2,40}(?:Limited|Ltd\.?|Plc|LLP|Inc\.?|Corp\.?|Corporation|Incorporated|Systems|Foods|Services|Solutions|Group|Holdings|Company|Cabling)\b)"#,
    ) {
        for m in re.find_iter(text) {
            let v = m.as_str().trim();
            if v.len() >= 4 && v.len() <= 60 {
                candidates.push(EntityCandidate {
                    entity_value: v.to_string(),
                    normalized_value: normalize_value("company", v),
                    initial_type: "company".to_string(),
                    initial_confidence: 0.9,
                    extraction_method: ExtractionMethod::RuleBased,
                });
            }
        }
    }

    // Person: Mr, Mrs, Ms, Dr + capitalized words (strong: title indicates person)
    if let Ok(re) =
        Regex::new(r#"\b(?:Mr|Mrs|Ms|Miss|Dr|Prof)\.?\s+([A-Z][a-z]+(?:\s+[A-Z][a-z]+){1,2})\b"#)
    {
        for cap in re.captures_iter(text) {
            if let Some(m) = cap.get(1) {
                let v = m.as_str().trim();
                if !v.to_lowercase().contains("limited")
                    && !v.to_lowercase().contains("ltd")
                    && !v.to_lowercase().contains("inc")
                {
                    candidates.push(EntityCandidate {
                        entity_value: v.to_string(),
                        normalized_value: normalize_value("person", v),
                        initial_type: "person".to_string(),
                        initial_confidence: 0.82,
                        extraction_method: ExtractionMethod::RuleBased,
                    });
                }
            }
        }
    }

    // Title-case 2–3 word phrases: emit as "unknown" so classification can assign product/location/person
    if let Ok(re) = Regex::new(r#"\b([A-Z][a-z]+\s+[A-Z][a-z]+(?:\s+[A-Z][a-z]+)?)\b"#) {
        for m in re.find_iter(text) {
            let v = m.as_str().trim();
            let lower = v.to_lowercase();
            if !lower.ends_with("ltd")
                && !lower.ends_with("limited")
                && !lower.ends_with("plc")
                && !lower.ends_with("inc")
                && !lower.ends_with("corp")
                && !lower.ends_with("llp")
                && v.len() >= 4
                && v.len() <= 50
            {
                candidates.push(EntityCandidate {
                    entity_value: v.to_string(),
                    normalized_value: normalize_value("unknown", v),
                    initial_type: "unknown".to_string(),
                    initial_confidence: 0.5,
                    extraction_method: ExtractionMethod::RuleBased,
                });
            }
        }
    }

    // Deduplicate by normalized_value (keep first occurrence)
    let mut seen: HashSet<String> = HashSet::new();
    candidates.retain(|c| {
        if seen.contains(&c.normalized_value) {
            false
        } else {
            seen.insert(c.normalized_value.clone());
            true
        }
    });

    candidates
}

/// Extract entities: candidates + classification (strong rules only, no vocab/LLM).
/// Used when full pipeline with vocab/LLM is not available.
pub fn extract_entities(text: &str) -> ExtractionResult {
    let candidates = extract_candidates(text);
    let entities: Vec<ExtractedEntity> = candidates
        .iter()
        .map(|c| {
            // Strong rule overrides unknown
            if c.initial_type == "unknown" {
                if let Some((entity_type, confidence)) = strong_rule_classify(&c.entity_value) {
                    return ExtractedEntity {
                        entity_type: entity_type.to_string(),
                        entity_value: c.entity_value.clone(),
                        normalized_value: c.normalized_value.clone(),
                        confidence,
                        extraction_method: c.extraction_method,
                        classification_method: ClassificationMethod::RuleBased,
                    };
                }
                // Leave as unknown (low confidence, kept for later reclassification)
                ExtractedEntity {
                    entity_type: "unknown".to_string(),
                    entity_value: c.entity_value.clone(),
                    normalized_value: c.normalized_value.clone(),
                    confidence: 0.4,
                    extraction_method: c.extraction_method,
                    classification_method: ClassificationMethod::RuleBased,
                }
            } else {
                // Already strongly typed
                ExtractedEntity {
                    entity_type: c.initial_type.clone(),
                    entity_value: c.entity_value.clone(),
                    normalized_value: c.normalized_value.clone(),
                    confidence: c.initial_confidence,
                    extraction_method: c.extraction_method,
                    classification_method: ClassificationMethod::RuleBased,
                }
            }
        })
        .collect();

    ExtractionResult { entities }
}

/// Merge rule-based and LLM results. Rule-based preferred on conflict (same key).
pub fn merge_entities(
    rule_based: Vec<ExtractedEntity>,
    llm: Vec<ExtractedEntity>,
) -> Vec<ExtractedEntity> {
    let mut by_key: std::collections::HashMap<String, ExtractedEntity> =
        std::collections::HashMap::new();
    for e in rule_based {
        let key = format!("{}:{}", e.entity_type, e.normalized_value);
        by_key.entry(key).or_insert(e);
    }
    for e in llm {
        let key = format!("{}:{}", e.entity_type, e.normalized_value);
        by_key.entry(key).or_insert(e);
    }
    by_key.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_email() {
        let r = extract_entities("Contact john.doe@example.com for details.");
        let emails: Vec<_> = r
            .entities
            .iter()
            .filter(|e| e.entity_type == "email")
            .collect();
        assert_eq!(emails.len(), 1);
        assert_eq!(emails[0].entity_value, "john.doe@example.com");
    }

    #[test]
    fn extract_phone() {
        let r = extract_entities("Call +44 20 7123 4567 or (020) 7123-4567");
        let phones: Vec<_> = r
            .entities
            .iter()
            .filter(|e| e.entity_type == "phone")
            .collect();
        assert!(!phones.is_empty());
    }

    #[test]
    fn extract_money() {
        let r = extract_entities("Total: £1,234.56 or $99.99");
        let money: Vec<_> = r
            .entities
            .iter()
            .filter(|e| e.entity_type == "money")
            .collect();
        assert!(!money.is_empty());
    }

    #[test]
    fn extract_company() {
        let r = extract_entities("Invoice from Acme Corporation Ltd.");
        let companies: Vec<_> = r
            .entities
            .iter()
            .filter(|e| e.entity_type == "company")
            .collect();
        assert!(!companies.is_empty());
    }

    #[test]
    fn extract_person_with_title() {
        let r = extract_entities("Signed by Mr. John Smith.");
        let people: Vec<_> = r
            .entities
            .iter()
            .filter(|e| e.entity_type == "person")
            .collect();
        assert!(!people.is_empty());
    }

    #[test]
    fn extract_invoice_number() {
        let r = extract_entities("Invoice No: INV-2024-001");
        let inv: Vec<_> = r
            .entities
            .iter()
            .filter(|e| e.entity_type == "invoice_number")
            .collect();
        assert!(!inv.is_empty());
    }

    // Classification accuracy: no incorrect person for product/location phrases
    #[test]
    fn no_person_for_product_phrases() {
        let r =
            extract_entities("Install Cat6 using White Flexible Conduit and Patch Panel Surface.");
        let people: Vec<_> = r
            .entities
            .iter()
            .filter(|e| e.entity_type == "person")
            .collect();
        let products: Vec<_> = r
            .entities
            .iter()
            .filter(|e| e.entity_type == "product")
            .collect();
        assert!(
            people.is_empty(),
            "should not classify product phrases as person"
        );
        assert!(!products.is_empty(), "should classify as product");
    }

    #[test]
    fn no_person_for_location_phrases() {
        let r = extract_entities("Access required to Server Room and Changing Room.");
        let people: Vec<_> = r
            .entities
            .iter()
            .filter(|e| e.entity_type == "person")
            .collect();
        let locations: Vec<_> = r
            .entities
            .iter()
            .filter(|e| e.entity_type == "location")
            .collect();
        assert!(
            people.is_empty(),
            "should not classify location phrases as person"
        );
        assert!(!locations.is_empty(), "should classify as location");
    }

    #[test]
    fn company_phrases_classified_correctly() {
        let r = extract_entities("Invoice from Complete Cabling Systems Ltd and Becketts Foods.");
        let companies: Vec<_> = r
            .entities
            .iter()
            .filter(|e| e.entity_type == "company")
            .collect();
        assert!(companies.len() >= 2, "should classify both as company");
    }

    #[test]
    fn person_with_title_still_classified() {
        let r = extract_entities("Signed by Dr Sarah Davies and Mr Gavin Anthony.");
        let people: Vec<_> = r
            .entities
            .iter()
            .filter(|e| e.entity_type == "person")
            .collect();
        assert!(people.len() >= 2, "title+name should remain person");
    }

    #[test]
    fn money_classified() {
        let r = extract_entities("Total: £1035.00");
        let money: Vec<_> = r
            .entities
            .iter()
            .filter(|e| e.entity_type == "money")
            .collect();
        assert!(!money.is_empty());
    }

    #[test]
    fn unresolved_phrases_stored_as_unknown_not_dropped() {
        let r = extract_entities("Refer to Ambiguous Phrase and Another Odd Term for details.");
        let unknowns: Vec<_> = r
            .entities
            .iter()
            .filter(|e| e.entity_type == "unknown")
            .collect();
        assert!(
            !unknowns.is_empty(),
            "unresolved phrases must be stored as type 'unknown', not dropped"
        );
        for e in &unknowns {
            assert_eq!(e.entity_type, "unknown");
            assert!(e.confidence >= 0.3 && e.confidence <= 0.5);
            assert!(!e.entity_value.is_empty());
            assert!(!e.normalized_value.is_empty());
        }
    }
}
