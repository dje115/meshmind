//! Rule-based and optional LLM entity extraction from document chunks.

use regex::Regex;
use std::collections::HashSet;

use super::{ExtractedEntity, ExtractionMethod};
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
}

impl Default for ExtractionConfig {
    fn default() -> Self {
        Self {
            enable_llm_entity_extraction: false,
            llm_chunk_length_threshold: 500,
            llm_entity_count_threshold: 2,
        }
    }
}

/// Result of extraction on a chunk.
#[derive(Debug, Clone)]
pub struct ExtractionResult {
    pub entities: Vec<ExtractedEntity>,
}

/// Extract entities from chunk text using rule-based patterns.
pub fn extract_entities(text: &str) -> ExtractionResult {
    let mut entities = Vec::new();

    // Email
    if let Ok(re) = Regex::new(r#"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}"#) {
        for m in re.find_iter(text) {
            let v = m.as_str();
            if v.len() < 100 {
                entities.push(ExtractedEntity {
                    entity_type: "email".into(),
                    entity_value: v.to_string(),
                    normalized_value: normalize_value("email", v),
                    confidence: 0.95,
                    extraction_method: ExtractionMethod::RuleBased,
                });
            }
        }
    }

    // Phone: UK/international - avoid noisy patterns
    if let Ok(re) =
        Regex::new(r#"(?:\+44|0044|0)\s*[\d\s\-()]{9,14}\d|(?:\+\d{1,3}\s)?[\d\s\-()]{10,16}\d"#)
    {
        for m in re.find_iter(text) {
            let v = m.as_str().trim();
            let digits: String = v.chars().filter(|c| c.is_ascii_digit()).collect();
            if digits.len() >= 10 && digits.len() <= 15 {
                entities.push(ExtractedEntity {
                    entity_type: "phone".into(),
                    entity_value: v.to_string(),
                    normalized_value: normalize_value("phone", v),
                    confidence: 0.9,
                    extraction_method: ExtractionMethod::RuleBased,
                });
            }
        }
    }

    // Money: £, $, €
    if let Ok(re) =
        Regex::new(r#"[$€£]\s*[\d,]+(?:\.\d{2})?|\d+(?:,\d{3})*(?:\.\d{2})?\s*(?:GBP|USD|EUR)"#)
    {
        for m in re.find_iter(text) {
            let v = m.as_str();
            if v.len() < 50 {
                entities.push(ExtractedEntity {
                    entity_type: "money".into(),
                    entity_value: v.to_string(),
                    normalized_value: normalize_value("money", v),
                    confidence: 0.92,
                    extraction_method: ExtractionMethod::RuleBased,
                });
            }
        }
    }

    // Date: common formats
    if let Ok(re) = Regex::new(
        r#"\d{1,2}[-/]\d{1,2}[-/]\d{2,4}|\d{4}[-/]\d{1,2}[-/]\d{1,2}|(?:Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)[a-z]*\.?\s+\d{1,2},?\s+\d{4}"#,
    ) {
        for m in re.find_iter(text) {
            let v = m.as_str();
            entities.push(ExtractedEntity {
                entity_type: "date".into(),
                entity_value: v.to_string(),
                normalized_value: normalize_value("date", v),
                confidence: 0.9,
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
                entities.push(ExtractedEntity {
                    entity_type: entity_type.into(),
                    entity_value: v.to_string(),
                    normalized_value: normalize_value(entity_type, v),
                    confidence: 0.88,
                    extraction_method: ExtractionMethod::RuleBased,
                });
            }
        }
    }

    // Company: Ltd, Limited, Plc, LLP, Inc, Corp
    if let Ok(re) = Regex::new(
        r#"(?:\b[A-Z][a-zA-Z0-9\s&'-]{2,40}(?:Limited|Ltd\.?|Plc|LLP|Inc\.?|Corp\.?|Corporation|Incorporated)\b)"#,
    ) {
        for m in re.find_iter(text) {
            let v = m.as_str().trim();
            if v.len() >= 4 && v.len() <= 60 {
                entities.push(ExtractedEntity {
                    entity_type: "company".into(),
                    entity_value: v.to_string(),
                    normalized_value: normalize_value("company", v),
                    confidence: 0.85,
                    extraction_method: ExtractionMethod::RuleBased,
                });
            }
        }
    }

    // Person: Mr, Mrs, Ms, Dr + capitalized words
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
                    entities.push(ExtractedEntity {
                        entity_type: "person".into(),
                        entity_value: v.to_string(),
                        normalized_value: normalize_value("person", v),
                        confidence: 0.82,
                        extraction_method: ExtractionMethod::RuleBased,
                    });
                }
            }
        }
    }

    // Person: two or three capitalized words (e.g. "John Smith")
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
                entities.push(ExtractedEntity {
                    entity_type: "person".into(),
                    entity_value: v.to_string(),
                    normalized_value: normalize_value("person", v),
                    confidence: 0.6,
                    extraction_method: ExtractionMethod::RuleBased,
                });
            }
        }
    }

    // Deduplicate by normalized_value, keeping highest confidence
    let mut seen: HashSet<String> = HashSet::new();
    entities.retain(|e| {
        let key = format!("{}:{}", e.entity_type, e.normalized_value);
        if seen.contains(&key) {
            false
        } else {
            seen.insert(key);
            true
        }
    });

    ExtractionResult { entities }
}

/// Merge rule-based and LLM results. Rule-based preferred on conflict.
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
}
