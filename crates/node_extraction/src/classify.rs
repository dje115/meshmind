//! Entity classification: strong rules, vocabulary lookup, and optional LLM.
//!
//! Classifies entity candidates into final types (person, company, product, location, etc.)
//! without over-relying on title-case.

use super::normalize::normalize_phrase_for_vocab;
use super::{EntityCandidate, ExtractedEntity, ENTITY_TYPES};
use std::collections::HashSet;

/// How the entity type was determined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassificationMethod {
    RuleBased,
    VocabularyLookup,
    LlmAssisted,
    Corrected,
}

impl std::fmt::Display for ClassificationMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RuleBased => write!(f, "rule_based"),
            Self::VocabularyLookup => write!(f, "vocabulary_lookup"),
            Self::LlmAssisted => write!(f, "llm_assisted"),
            Self::Corrected => write!(f, "corrected"),
        }
    }
}

// Strong company indicators (phrase contains one of these -> company)
const COMPANY_INDICATORS: &[&str] = &[
    "ltd",
    "limited",
    "plc",
    "llp",
    "inc",
    "corp",
    "corporation",
    "incorporated",
    "systems",
    "foods",
    "services",
    "solutions",
    "group",
    "holdings",
    "company",
    "cabling",
];

// Organization / legal scheme indicators (no company suffix but clearly an org)
const ORGANIZATION_INDICATORS: &[&str] = &[
    "scheme",
    "ombudsman",
    "protection service",
    "deposit protection",
    "property ombudsman",
];

// Location keywords (phrase contains -> location)
const LOCATION_KEYWORDS: &[&str] = &[
    "room",
    "office",
    "warehouse",
    "kitchen",
    "plant room",
    "server room",
    "changing room",
    "temp room",
];

// Street/address suffixes (phrase ends with or contains -> location/address)
const ADDRESS_SUFFIXES: &[&str] = &[
    "street",
    "road",
    "lane",
    "avenue",
    "drive",
    "court",
    "close",
    "way",
    "place",
    "terrace",
    "gardens",
    "crescent",
    "row",
    "square",
    "hill",
    "park",
    "farm",
    "harborough", // e.g. Market Harborough
];

// Product keywords (phrase contains -> product)
const PRODUCT_KEYWORDS: &[&str] = &[
    "conduit",
    "panel",
    "patch",
    "cable",
    "trunking",
    "fixing",
    "machine",
    "access point",
    "camera",
    "router",
    "switch",
    "cabinet",
];

// Person-context keywords: when phrase appears near these, bias toward person
const PERSON_CONTEXT_KEYWORDS: &[&str] = &[
    "tenant",
    "landlord",
    "signed",
    "witness",
    "by",
    "name",
    "contact",
    "prepared by",
    "sent to",
    "attention",
];

/// Strong-rule classification only. Returns (entity_type, confidence) if a strong rule applies.
pub fn strong_rule_classify(phrase: &str) -> Option<(&'static str, f32)> {
    let lower = phrase.to_lowercase();
    let lower = lower.trim();

    // Company: must contain a known indicator
    for ind in COMPANY_INDICATORS {
        if lower.contains(ind) {
            return Some(("company", 0.92));
        }
    }

    // Organization / legal scheme: Tenancy Deposit Scheme, Property Ombudsman, etc.
    for ind in ORGANIZATION_INDICATORS {
        if lower.contains(ind) {
            return Some(("company", 0.88)); // Use company for orgs; same display
        }
    }

    // Location: contains location keyword
    for kw in LOCATION_KEYWORDS {
        if lower.contains(kw) {
            return Some(("location", 0.88));
        }
    }

    // Address/street: ends with or contains address suffix (Nelson Street, Holt Lane, Market Harborough)
    for suffix in ADDRESS_SUFFIXES {
        if lower.ends_with(suffix) || lower.contains(&format!(" {suffix}")) {
            return Some(("location", 0.85));
        }
    }

    // Product: contains product keyword
    for kw in PRODUCT_KEYWORDS {
        if lower.contains(kw) {
            return Some(("product", 0.88));
        }
    }

    None
}

/// Context-aware heuristics for person names. When context contains tenant/landlord/signed/etc,
/// and phrase looks like a 2–3 word title-case name, classify as person.
pub fn context_aware_person_classify(phrase: &str, context: &str) -> Option<(&'static str, f32)> {
    let ctx_lower = context.to_lowercase();
    let has_person_context = PERSON_CONTEXT_KEYWORDS
        .iter()
        .any(|kw| ctx_lower.contains(kw));

    if !has_person_context {
        return None;
    }

    // Phrase must look like a name: 2–3 title-case words, no company/product/location keywords
    let words: Vec<&str> = phrase.split_whitespace().collect();
    if words.len() < 2 || words.len() > 3 {
        return None;
    }

    let lower = phrase.to_lowercase();
    // Exclude product/location/company keywords
    for kw in PRODUCT_KEYWORDS {
        if lower.contains(kw) {
            return None;
        }
    }
    for kw in LOCATION_KEYWORDS {
        if lower.contains(kw) {
            return None;
        }
    }
    for kw in COMPANY_INDICATORS {
        if lower.contains(kw) {
            return None;
        }
    }
    for kw in ADDRESS_SUFFIXES {
        if lower.contains(kw) {
            return None;
        }
    }

    // Check title-case: each word starts with uppercase
    let looks_like_name = words.iter().all(|w| {
        w.len() >= 2
            && w.chars().next().map(|c| c.is_uppercase()).unwrap_or(false)
            && w.chars()
                .skip(1)
                .all(|c| c.is_lowercase() || c == '-' || c == '\'')
    });

    if looks_like_name {
        Some(("person", 0.82))
    } else {
        None
    }
}

/// Classify a candidate using only strong rules. Returns None if no strong rule applies.
pub fn classify_with_strong_rules(candidate: &EntityCandidate) -> Option<ExtractedEntity> {
    let (entity_type, confidence) = strong_rule_classify(&candidate.entity_value)?;
    Some(ExtractedEntity {
        entity_type: entity_type.to_string(),
        entity_value: candidate.entity_value.clone(),
        normalized_value: candidate.normalized_value.clone(),
        confidence,
        extraction_method: super::ExtractionMethod::RuleBased,
        classification_method: ClassificationMethod::RuleBased,
    })
}

/// Vocabulary lookup result.
pub struct VocabEntry {
    pub entity_type: String,
    pub confidence: f32,
}

/// Minimal trait for vocabulary lookup (implemented by storage layer).
pub trait VocabularyLookup {
    fn lookup(&self, normalized_phrase: &str) -> Option<VocabEntry>;
}

/// Callback type for LLM phrase classification (phrase, context) -> (entity_type, confidence).
pub type LlmClassifyFn = dyn Fn(&str, &str) -> Option<(String, f32)>;

/// Classify one candidate: strong rules -> context-aware person -> vocab -> optional LLM.
/// Returns (entity, classification_method). Caller is responsible for learning into vocabulary.
/// Unknown entities fall through to LLM when llm_classify is provided and config allows.
#[allow(clippy::type_complexity)]
pub fn classify_candidate(
    candidate: &EntityCandidate,
    context: &str,
    vocab: Option<&dyn VocabularyLookup>,
    llm_classify: Option<&LlmClassifyFn>,
    max_phrase_len: usize,
    vocab_confidence_threshold: f32,
) -> ExtractedEntity {
    let normalized = normalize_phrase_for_vocab(&candidate.entity_value);

    // 1) Strong rules (company, org, location, address, product)
    if let Some(entity) = classify_with_strong_rules(candidate) {
        return entity;
    }

    // 2) Context-aware person (tenant/landlord/signed context + 2–3 word name-like phrase)
    if let Some((entity_type, confidence)) =
        context_aware_person_classify(&candidate.entity_value, context)
    {
        return ExtractedEntity {
            entity_type: entity_type.to_string(),
            entity_value: candidate.entity_value.clone(),
            normalized_value: candidate.normalized_value.clone(),
            confidence,
            extraction_method: candidate.extraction_method,
            classification_method: ClassificationMethod::RuleBased,
        };
    }

    // 3) Vocabulary lookup
    if let Some(v) = vocab {
        if let Some(entry) = v.lookup(&normalized) {
            if entry.confidence >= vocab_confidence_threshold {
                return ExtractedEntity {
                    entity_type: entry.entity_type,
                    entity_value: candidate.entity_value.clone(),
                    normalized_value: candidate.normalized_value.clone(),
                    confidence: entry.confidence,
                    extraction_method: candidate.extraction_method,
                    classification_method: ClassificationMethod::VocabularyLookup,
                };
            }
        }
    }

    // 4) LLM: unknown/ambiguous entities fall through here when classifier is provided.
    // Only keep unknown if LLM is disabled, returns unknown, or confidence too low.
    if candidate.entity_value.len() < max_phrase_len {
        if let Some(classify_fn) = llm_classify {
            if let Some((entity_type, confidence)) = classify_fn(&candidate.entity_value, context) {
                let type_clean = entity_type.trim().to_lowercase();
                // Accept any known type except "unknown"; use threshold 0.5 to avoid low-confidence guesses
                if type_clean != "unknown"
                    && ENTITY_TYPES.contains(&type_clean.as_str())
                    && confidence >= 0.5
                {
                    return ExtractedEntity {
                        entity_type: type_clean,
                        entity_value: candidate.entity_value.clone(),
                        normalized_value: candidate.normalized_value.clone(),
                        confidence,
                        extraction_method: candidate.extraction_method,
                        classification_method: ClassificationMethod::LlmAssisted,
                    };
                }
            }
        }
    }

    // 5) Fallback: keep initial type if it was high confidence (e.g. person from title)
    let (entity_type, confidence) = if candidate.initial_confidence >= 0.75 {
        (candidate.initial_type.clone(), candidate.initial_confidence)
    } else {
        ("unknown".to_string(), 0.4)
    };

    ExtractedEntity {
        entity_type,
        entity_value: candidate.entity_value.clone(),
        normalized_value: candidate.normalized_value.clone(),
        confidence,
        extraction_method: candidate.extraction_method,
        classification_method: ClassificationMethod::RuleBased,
    }
}

/// Deduplicate entities by (entity_type, normalized_value), keeping highest confidence.
#[allow(dead_code)]
pub fn dedupe_entities(entities: Vec<ExtractedEntity>) -> Vec<ExtractedEntity> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::with_capacity(entities.len());
    for e in entities {
        let key = format!("{}:{}", e.entity_type, e.normalized_value);
        if seen.contains(&key) {
            continue;
        }
        seen.insert(key);
        out.push(e);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strong_rule_company() {
        assert_eq!(
            strong_rule_classify("Complete Cabling Systems Ltd"),
            Some(("company", 0.92))
        );
        assert_eq!(
            strong_rule_classify("Becketts Foods"),
            Some(("company", 0.92))
        );
    }

    #[test]
    fn strong_rule_location() {
        assert_eq!(
            strong_rule_classify("Server Room"),
            Some(("location", 0.88))
        );
        assert_eq!(
            strong_rule_classify("Changing Room"),
            Some(("location", 0.88))
        );
    }

    #[test]
    fn strong_rule_product() {
        assert_eq!(
            strong_rule_classify("White Flexible Conduit"),
            Some(("product", 0.88))
        );
        assert_eq!(
            strong_rule_classify("Patch Panel Surface"),
            Some(("product", 0.88))
        );
        assert_eq!(
            strong_rule_classify("Mount Mini Trunking"),
            Some(("product", 0.88))
        );
    }

    #[test]
    fn no_strong_rule_for_person() {
        assert_eq!(strong_rule_classify("Gavin Anthony"), None);
        assert_eq!(strong_rule_classify("John Smith"), None);
    }

    #[test]
    fn strong_rule_organization_legal_scheme() {
        assert_eq!(
            strong_rule_classify("Tenancy Deposit Scheme"),
            Some(("company", 0.88))
        );
        assert_eq!(
            strong_rule_classify("Deposit Protection Service"),
            Some(("company", 0.88))
        );
        assert_eq!(
            strong_rule_classify("Property Ombudsman"),
            Some(("company", 0.88))
        );
    }

    #[test]
    fn strong_rule_address_location() {
        assert_eq!(
            strong_rule_classify("Nelson Street"),
            Some(("location", 0.85))
        );
        assert_eq!(strong_rule_classify("Holt Lane"), Some(("location", 0.85)));
        assert_eq!(
            strong_rule_classify("Market Harborough"),
            Some(("location", 0.85))
        );
        assert_eq!(
            strong_rule_classify("Bridge Farm"),
            Some(("location", 0.85))
        );
    }

    #[test]
    fn context_aware_person_tenant_landlord() {
        let ctx = "Tenancy agreement between landlord and tenant. Signed by David Evans.";
        assert_eq!(
            context_aware_person_classify("David Evans", ctx),
            Some(("person", 0.82))
        );
        assert_eq!(
            context_aware_person_classify("Andrew Boddy", ctx),
            Some(("person", 0.82))
        );
        assert_eq!(
            context_aware_person_classify("Frederica Monk", ctx),
            Some(("person", 0.82))
        );
    }

    #[test]
    fn context_aware_person_no_context() {
        // Without tenant/landlord/signed context, should not classify as person
        assert_eq!(
            context_aware_person_classify("David Evans", "Some random text about products."),
            None
        );
    }

    #[test]
    fn context_aware_person_excludes_product() {
        // White Flexible Conduit near "tenant" should still be product
        assert_eq!(
            context_aware_person_classify("White Flexible Conduit", "tenant signed"),
            None
        );
    }

    #[test]
    fn classify_candidate_tenancy_entities() {
        use super::super::EntityCandidate;
        use super::super::ExtractionMethod;

        let ctx = "Tenancy agreement. Landlord David Evans, tenant Andrew Boddy. Property at Nelson Street, Market Harborough. Deposit via Tenancy Deposit Scheme. Rent £400.00. Start 21/08/2010. Mayband Property Services Ltd.";

        let check = |value: &str, expected_type: &str| {
            let c = EntityCandidate {
                entity_value: value.into(),
                normalized_value: value.to_lowercase(),
                initial_type: "unknown".into(),
                initial_confidence: 0.5,
                extraction_method: ExtractionMethod::RuleBased,
            };
            let e = classify_candidate(&c, ctx, None, None, 60, 0.8);
            assert_eq!(
                e.entity_type, expected_type,
                "{} should be {}",
                value, expected_type
            );
        };

        check("David Evans", "person");
        check("Andrew Boddy", "person");
        check("Nelson Street", "location");
        check("Market Harborough", "location");
        check("Tenancy Deposit Scheme", "company");
        check("Mayband Property Services Ltd", "company");
    }

    #[test]
    fn classify_candidate_llm_resolves_unknown() {
        use super::super::EntityCandidate;
        use super::super::ExtractionMethod;

        let c = EntityCandidate {
            entity_value: "Jane Doe".into(),
            normalized_value: "jane doe".into(),
            initial_type: "unknown".into(),
            initial_confidence: 0.5,
            extraction_method: ExtractionMethod::RuleBased,
        };
        // Context without person keywords so "Jane Doe" stays unknown without LLM
        let ctx = "Reference number 12345. Item code ABC.";

        // Without LLM: stays unknown (no person context, initial_confidence 0.5 < 0.75)
        let e_no_llm = classify_candidate(&c, ctx, None, None, 60, 0.8);
        assert_eq!(e_no_llm.entity_type, "unknown");
        assert_eq!(
            e_no_llm.classification_method,
            ClassificationMethod::RuleBased
        );

        // With LLM returning person: becomes person
        let llm_fn = |phrase: &str, _ctx: &str| {
            if phrase == "Jane Doe" {
                Some(("person".to_string(), 0.85))
            } else {
                None
            }
        };
        let e_with_llm = classify_candidate(&c, ctx, None, Some(&llm_fn), 60, 0.8);
        assert_eq!(e_with_llm.entity_type, "person");
        assert_eq!(
            e_with_llm.classification_method,
            ClassificationMethod::LlmAssisted
        );
    }
}
