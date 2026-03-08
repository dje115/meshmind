//! Rule-based relationship extraction from document chunks.
//!
//! Infers entity-to-entity relationships from co-occurrence and context.
//! Conservative heuristics; LLM can supplement when enabled.

use std::collections::HashSet;

use super::ExtractedEntity;

/// Relationship types extracted from document chunks.
pub const RELATIONSHIP_TYPES: &[&str] = &[
    "works_for",
    "contact_for",
    "prepared_by",
    "sent_to",
    "for_customer",
    "references_quote",
    "has_quote_number",
    "has_invoice_number",
    "has_total",
    "includes_product",
    "installed_at",
    "located_in",
    "mentions",
    "related_to",
];

/// A single extracted relationship.
#[derive(Debug, Clone)]
pub struct ExtractedRelationship {
    pub from_entity_id: String,
    pub from_entity_value: String,
    pub relationship_type: String,
    pub to_entity_id: String,
    pub to_entity_value: String,
    pub confidence: f32,
    pub extraction_method: String,
}

/// Maximum character distance between entities to consider related.
const PROXIMITY_CHARS: usize = 150;

/// Extract relationships from entities and chunk text.
/// Conservative rules; returns only high-confidence relationships.
pub fn extract_relationships(
    entities: &[ExtractedEntity],
    chunk_text: &str,
) -> Vec<ExtractedRelationship> {
    let mut rels = Vec::new();
    let text_lower = chunk_text.to_lowercase();

    if entities.len() < 2 {
        return rels;
    }

    // Build positions: (entity_index, char_offset of first occurrence)
    let positions: Vec<(usize, usize)> = entities
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            let pos = chunk_text.find(&e.entity_value)?;
            Some((i, pos))
        })
        .collect();

    // Helper: entities within PROXIMITY_CHARS
    let within_proximity = |i: usize, j: usize| -> bool {
        let pos_i = match positions.iter().find(|(idx, _)| *idx == i) {
            Some((_, p)) => *p,
            None => return false,
        };
        let pos_j = match positions.iter().find(|(idx, _)| *idx == j) {
            Some((_, p)) => *p,
            None => return false,
        };
        let dist = (pos_i as i64 - pos_j as i64).unsigned_abs();
        dist <= PROXIMITY_CHARS as u64
    };

    // Check for "prepared by X", "sent to X", "for X", "to X"
    let prepared_by_ctx = text_lower.contains("prepared by") || text_lower.contains("created by");
    let sent_to_ctx = text_lower.contains("sent to")
        || text_lower.contains("attention:")
        || text_lower.contains("to:");
    let for_customer_ctx = text_lower.contains("for")
        || text_lower.contains("customer")
        || text_lower.contains("bill to")
        || text_lower.contains("quote for");

    let mut seen: HashSet<(String, String, String)> = HashSet::new();

    for (i, from) in entities.iter().enumerate() {
        for (j, to) in entities.iter().enumerate() {
            if i == j {
                continue;
            }
            if !within_proximity(i, j) {
                continue;
            }

            // Person + Company proximity
            if (from.entity_type == "person" && to.entity_type == "company")
                || (from.entity_type == "company" && to.entity_type == "person")
            {
                let (person, company) = if from.entity_type == "person" {
                    (from, to)
                } else {
                    (to, from)
                };
                let rel_type = if prepared_by_ctx {
                    "prepared_by"
                } else if sent_to_ctx {
                    "sent_to"
                } else {
                    "works_for"
                };
                let key = (
                    person.entity_id(),
                    rel_type.to_string(),
                    company.entity_id(),
                );
                if seen.contains(&key) {
                    continue;
                }
                seen.insert(key);
                rels.push(ExtractedRelationship {
                    from_entity_id: person.entity_id(),
                    from_entity_value: person.entity_value.clone(),
                    relationship_type: rel_type.to_string(),
                    to_entity_id: company.entity_id(),
                    to_entity_value: company.entity_value.clone(),
                    confidence: 0.75,
                    extraction_method: "rule_based".into(),
                });
            }

            // Quote/Invoice number + Company -> for_customer
            let from_doc =
                from.entity_type == "quote_number" || from.entity_type == "invoice_number";
            let to_doc = to.entity_type == "quote_number" || to.entity_type == "invoice_number";
            let doc_and_company = from_doc && to.entity_type == "company";
            let company_and_doc = from.entity_type == "company" && to_doc;
            if for_customer_ctx && (doc_and_company || company_and_doc) {
                let (doc_entity, company) =
                    if from.entity_type == "quote_number" || from.entity_type == "invoice_number" {
                        (from, to)
                    } else {
                        (to, from)
                    };
                let key = (
                    doc_entity.entity_id(),
                    "for_customer".into(),
                    company.entity_id(),
                );
                if !seen.contains(&key) {
                    seen.insert(key);
                    rels.push(ExtractedRelationship {
                        from_entity_id: doc_entity.entity_id(),
                        from_entity_value: doc_entity.entity_value.clone(),
                        relationship_type: "for_customer".into(),
                        to_entity_id: company.entity_id(),
                        to_entity_value: company.entity_value.clone(),
                        confidence: 0.7,
                        extraction_method: "rule_based".into(),
                    });
                }
            }

            // Quote number entity -> has_quote_number (self-referential via context)
            // Invoice + Quote -> references_quote
            if from.entity_type == "invoice_number" && to.entity_type == "quote_number" {
                let key = (from.entity_id(), "references_quote".into(), to.entity_id());
                if !seen.contains(&key) && text_lower.contains("quote") {
                    seen.insert(key);
                    rels.push(ExtractedRelationship {
                        from_entity_id: from.entity_id(),
                        from_entity_value: from.entity_value.clone(),
                        relationship_type: "references_quote".into(),
                        to_entity_id: to.entity_id(),
                        to_entity_value: to.entity_value.clone(),
                        confidence: 0.72,
                        extraction_method: "rule_based".into(),
                    });
                }
            }

            // Money near quote/invoice -> has_total
            if (from.entity_type == "money"
                && (to.entity_type == "quote_number" || to.entity_type == "invoice_number"))
                || (to.entity_type == "money"
                    && (from.entity_type == "quote_number" || from.entity_type == "invoice_number"))
            {
                let (doc_entity, money) = if from.entity_type == "money" {
                    (to, from)
                } else {
                    (from, to)
                };
                let rel_type = "has_total";
                let key = (doc_entity.entity_id(), rel_type.into(), money.entity_id());
                if !seen.contains(&key)
                    && (text_lower.contains("total")
                        || text_lower.contains("sum")
                        || text_lower.contains("amount"))
                {
                    seen.insert(key);
                    rels.push(ExtractedRelationship {
                        from_entity_id: doc_entity.entity_id(),
                        from_entity_value: doc_entity.entity_value.clone(),
                        relationship_type: rel_type.to_string(),
                        to_entity_id: money.entity_id(),
                        to_entity_value: money.entity_value.clone(),
                        confidence: 0.78,
                        extraction_method: "rule_based".into(),
                    });
                }
            }

            // Quote/Invoice + has_quote_number / has_invoice_number (document -> number)
            if from.entity_type == "quote_number" && to.entity_type == "quote_number" && i != j {
                continue; // same type, skip
            }
            if from.entity_type == "invoice_number" && to.entity_type == "invoice_number" && i != j
            {
                continue;
            }

            // Product + Location -> located_in or installed_at
            if (from.entity_type == "product" && to.entity_type == "location")
                || (from.entity_type == "location" && to.entity_type == "product")
            {
                let (product, loc) = if from.entity_type == "product" {
                    (from, to)
                } else {
                    (to, from)
                };
                let rel_type = if text_lower.contains("install")
                    || text_lower.contains("fitted")
                    || text_lower.contains("mount")
                {
                    "installed_at"
                } else {
                    "located_in"
                };
                let key = (product.entity_id(), rel_type.into(), loc.entity_id());
                if !seen.contains(&key) {
                    seen.insert(key);
                    rels.push(ExtractedRelationship {
                        from_entity_id: product.entity_id(),
                        from_entity_value: product.entity_value.clone(),
                        relationship_type: rel_type.to_string(),
                        to_entity_id: loc.entity_id(),
                        to_entity_value: loc.entity_value.clone(),
                        confidence: 0.7,
                        extraction_method: "rule_based".into(),
                    });
                }
            }

            // Quote/Invoice + Product -> includes_product
            let line_item_ctx = text_lower.contains("qty")
                || text_lower.contains("description")
                || text_lower.contains("item")
                || text_lower.contains("quantity");
            let from_doc =
                from.entity_type == "quote_number" || from.entity_type == "invoice_number";
            let to_doc = to.entity_type == "quote_number" || to.entity_type == "invoice_number";
            let doc_and_product = from_doc && to.entity_type == "product";
            let product_and_doc = to_doc && from.entity_type == "product";
            if line_item_ctx && (doc_and_product || product_and_doc) {
                let (doc_entity, product) =
                    if from.entity_type == "quote_number" || from.entity_type == "invoice_number" {
                        (from, to)
                    } else {
                        (to, from)
                    };
                let key = (
                    doc_entity.entity_id(),
                    "includes_product".into(),
                    product.entity_id(),
                );
                if !seen.contains(&key) {
                    seen.insert(key);
                    rels.push(ExtractedRelationship {
                        from_entity_id: doc_entity.entity_id(),
                        from_entity_value: doc_entity.entity_value.clone(),
                        relationship_type: "includes_product".into(),
                        to_entity_id: product.entity_id(),
                        to_entity_value: product.entity_value.clone(),
                        confidence: 0.72,
                        extraction_method: "rule_based".into(),
                    });
                }
            }
        }
    }

    rels
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::ClassificationMethod;
    use crate::ExtractedEntity;
    use crate::ExtractionMethod;

    fn entity(ty: &str, value: &str, norm: &str) -> ExtractedEntity {
        ExtractedEntity {
            entity_type: ty.into(),
            entity_value: value.into(),
            normalized_value: norm.into(),
            confidence: 0.9,
            extraction_method: ExtractionMethod::RuleBased,
            classification_method: ClassificationMethod::RuleBased,
        }
    }

    #[test]
    fn works_for_person_company() {
        let entities = vec![
            entity("person", "Gavin Anthony", "gavin anthony"),
            entity(
                "company",
                "Complete Cabling Systems Ltd",
                "complete cabling systems ltd",
            ),
        ];
        let text = "Gavin Anthony from Complete Cabling Systems Ltd prepared this quote.";
        let rels = extract_relationships(&entities, text);
        let works_for = rels.iter().find(|r| r.relationship_type == "works_for");
        assert!(works_for.is_some(), "expected works_for");
        let r = works_for.unwrap();
        assert_eq!(r.from_entity_value, "Gavin Anthony");
        assert_eq!(r.to_entity_value, "Complete Cabling Systems Ltd");
    }

    #[test]
    fn for_customer_quote_company() {
        let entities = vec![
            entity("quote_number", "1234", "1234"),
            entity("company", "Becketts Foods", "becketts foods"),
        ];
        let text = "Quote 1234 for customer Becketts Foods. Bill to: Becketts Foods.";
        let rels = extract_relationships(&entities, text);
        let fc = rels.iter().find(|r| r.relationship_type == "for_customer");
        assert!(fc.is_some(), "expected for_customer");
        let r = fc.unwrap();
        assert_eq!(r.from_entity_value, "1234");
        assert_eq!(r.to_entity_value, "Becketts Foods");
    }

    #[test]
    fn has_total_invoice_money() {
        let entities = vec![
            entity("invoice_number", "4567", "4567"),
            entity("money", "£1035.00", "1035.00"),
        ];
        let text = "Invoice 4567. Total amount: £1035.00";
        let rels = extract_relationships(&entities, text);
        let ht = rels.iter().find(|r| r.relationship_type == "has_total");
        assert!(ht.is_some(), "expected has_total");
        let r = ht.unwrap();
        assert_eq!(r.from_entity_value, "4567");
        assert_eq!(r.to_entity_value, "£1035.00");
    }

    #[test]
    fn includes_product_quote() {
        let entities = vec![
            entity("quote_number", "1234", "1234"),
            entity(
                "product",
                "White Flexible Conduit",
                "white flexible conduit",
            ),
        ];
        let text = "Quote 1234. Qty 10, Description: White Flexible Conduit, Item total £50.";
        let rels = extract_relationships(&entities, text);
        let ip = rels
            .iter()
            .find(|r| r.relationship_type == "includes_product");
        assert!(ip.is_some(), "expected includes_product");
        let r = ip.unwrap();
        assert_eq!(r.from_entity_value, "1234");
        assert_eq!(r.to_entity_value, "White Flexible Conduit");
    }

    #[test]
    fn located_in_product_location() {
        let entities = vec![
            entity("product", "Cat6 Cable", "cat6 cable"),
            entity("location", "Server Room", "server room"),
        ];
        let text = "Cat6 Cable installed in Server Room.";
        let rels = extract_relationships(&entities, text);
        let inst = rels.iter().find(|r| r.relationship_type == "installed_at");
        assert!(inst.is_some(), "expected installed_at (install context)");
        let r = inst.unwrap();
        assert_eq!(r.from_entity_value, "Cat6 Cable");
        assert_eq!(r.to_entity_value, "Server Room");
    }

    #[test]
    fn located_in_without_install_context() {
        let entities = vec![
            entity("product", "Cat6 Cable", "cat6 cable"),
            entity("location", "Server Room", "server room"),
        ];
        let text = "Cat6 Cable in Server Room.";
        let rels = extract_relationships(&entities, text);
        let loc = rels.iter().find(|r| r.relationship_type == "located_in");
        assert!(loc.is_some(), "expected located_in");
    }
}
