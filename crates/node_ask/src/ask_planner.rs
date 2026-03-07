//! Rule-based AskPlanner: classifies intent and builds AskPlan.

use crate::ask_plan::{AskIntent, AskPlan, RetrievalSource, RetrievalStep};

/// Rule-based planner. Deterministic, easy to test.
pub struct AskPlanner;

impl AskPlanner {
    pub fn new() -> Self {
        Self
    }

    /// Build an AskPlan for the given question.
    pub fn plan(&self, question: &str) -> AskPlan {
        let lower = question.trim().to_lowercase();

        // 1. Web freshness / general knowledge
        if Self::wants_web_search(&lower) || Self::looks_like_freshness_question(&lower) {
            return Self::plan_web_freshness(question, &lower);
        }

        // 2. Document entity intents (list people, companies, etc.)
        if let Some(plan) = Self::plan_document_entity(&lower) {
            return plan;
        }

        // 3. Document lookup / summarize
        if Self::looks_like_document_specific(&lower) || Self::looks_like_summarize(&lower) {
            return Self::plan_document_lookup(question, &lower);
        }

        // 4. Pricing / quote history
        if Self::looks_like_pricing_history(&lower) {
            return Self::plan_pricing_history(&lower);
        }

        // 5. Customer / invoice / quote / accounting
        if let Some(plan) = Self::plan_business_intent(&lower) {
            return plan;
        }

        // 6. Default: FTS search
        Self::plan_general_fts(question, &lower)
    }

    fn wants_web_search(lower: &str) -> bool {
        const TRIGGERS: &[&str] = &[
            "search the web",
            "search the internet",
            "search online",
            "look it up online",
            "look up online",
            "check online",
            "google it",
        ];
        TRIGGERS.iter().any(|t| lower.contains(t))
    }

    fn looks_like_freshness_question(lower: &str) -> bool {
        // "what happened yesterday in Iran", "news about X"
        if lower.len() < 10 || lower.len() > 150 {
            return false;
        }
        let freshness = [
            "yesterday",
            "today",
            "last week",
            "recent",
            "latest",
            "news about",
        ];
        let general = ["what happened", "what is happening", "current", "breaking"];
        freshness.iter().any(|f| lower.contains(f))
            && (general.iter().any(|g| lower.contains(g))
                || lower.starts_with("what ")
                || lower.starts_with("when did "))
    }

    fn plan_web_freshness(_question: &str, lower: &str) -> AskPlan {
        let mut plan = AskPlan::new(AskIntent::WebFreshnessNeeded)
            .with_web_fallback(true)
            .with_peer_consult(false);

        // Optional: try local FTS first for any matching content
        if !Self::wants_web_search(lower) {
            let kw: String = lower
                .split_whitespace()
                .filter(|w| w.len() > 2)
                .take(5)
                .collect::<Vec<_>>()
                .join(" OR ");
            if !kw.is_empty() {
                plan.retrieval_steps.push(RetrievalStep {
                    source_type: RetrievalSource::FtsSearch,
                    query: kw,
                    filters: vec![],
                    limit: 20,
                    required: false,
                });
            }
        }
        plan
    }

    fn plan_document_entity(lower: &str) -> Option<AskPlan> {
        let entity_type = if lower.contains("who appears")
            || lower.contains("what people")
            || lower.contains("people mentioned")
        {
            "person"
        } else if lower.contains("which companies")
            || lower.contains("what companies")
            || lower.contains("companies appear")
        {
            "company"
        } else if lower.contains("list emails") || lower.contains("emails found") {
            "email"
        } else if lower.contains("invoice numbers") || lower.contains("invoice no") {
            "invoice_number"
        } else if lower.contains("quote numbers") || lower.contains("quote no") {
            "quote_number"
        } else if lower.contains("documents mentioning")
            || lower.contains("documents that mention")
            || lower.contains("mentioning ")
        {
            let rest = if let Some(i) = lower.find("mentioning ") {
                lower[i + 11..].trim()
            } else if let Some(i) = lower.find("that mention ") {
                lower[i + 13..].trim()
            } else {
                return None;
            };
            let entity: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == ' ' || *c == '.' || *c == '-')
                .collect();
            let entity = entity.trim().trim_end_matches(['?', '.']);
            if entity.len() < 2 {
                return None;
            }
            let mut plan = AskPlan::new(AskIntent::DocumentLookup);
            plan.retrieval_steps.push(RetrievalStep {
                source_type: RetrievalSource::EntityQuery,
                query: format!("documents_mentioning:{}", entity),
                filters: vec![],
                limit: 20,
                required: true,
            });
            return Some(plan);
        } else {
            return None;
        };

        let mut plan = AskPlan::new(AskIntent::ListEntities);
        plan.retrieval_steps.push(RetrievalStep {
            source_type: RetrievalSource::EntityQuery,
            query: entity_type.to_string(),
            filters: vec![("entity_type".into(), entity_type.into())],
            limit: 50,
            required: true,
        });
        Some(plan)
    }

    fn looks_like_document_specific(lower: &str) -> bool {
        lower.contains("read the content")
            || lower.contains("read the document")
            || lower.contains("contents of")
            || lower.contains("content of this")
            || lower.contains(".docx")
            || lower.contains(".pdf")
            || lower.contains(".txt")
            || lower.contains("summarize document")
            || lower.contains("summarize the document")
    }

    fn looks_like_summarize(lower: &str) -> bool {
        lower.contains("summarize") && (lower.contains("document") || lower.contains("doc"))
    }

    fn plan_document_lookup(question: &str, _lower: &str) -> AskPlan {
        let mut plan = AskPlan::new(AskIntent::DocumentLookup);
        let fts_query = crate::evidence_collector::to_fts5_query(question);
        plan.retrieval_steps.push(RetrievalStep {
            source_type: RetrievalSource::DocumentChunk,
            query: fts_query.clone(),
            filters: vec![],
            limit: 30,
            required: true,
        });
        plan.retrieval_steps.push(RetrievalStep {
            source_type: RetrievalSource::FtsSearch,
            query: fts_query,
            filters: vec![],
            limit: 20,
            required: false,
        });
        plan
    }

    fn looks_like_pricing_history(lower: &str) -> bool {
        lower.contains("historically charged")
            || lower.contains("charge for")
            || (lower.contains("pricing") && lower.contains("history"))
            || (lower.contains("what have we")
                && (lower.contains("charged") || lower.contains("price")))
    }

    fn plan_pricing_history(lower: &str) -> AskPlan {
        let mut plan = AskPlan::new(AskIntent::PricingHistory)
            .with_peer_consult(true)
            .with_web_fallback(false);

        plan.retrieval_steps.push(RetrievalStep {
            source_type: RetrievalSource::FactQuery,
            query: "pricing".into(),
            filters: vec![("metric".into(), "pricing".into())],
            limit: 15,
            required: true,
        });
        plan.retrieval_steps.push(RetrievalStep {
            source_type: RetrievalSource::EntityCards,
            query: "quote".into(),
            filters: vec![("entity_type".into(), "quote".into())],
            limit: 20,
            required: false,
        });
        // FTS for related documents
        let kw = if lower.contains("cat6") {
            "cat6 install"
        } else {
            "quote price"
        };
        plan.retrieval_steps.push(RetrievalStep {
            source_type: RetrievalSource::DocumentChunk,
            query: kw.to_string(),
            filters: vec![],
            limit: 15,
            required: false,
        });
        plan
    }

    fn plan_business_intent(lower: &str) -> Option<AskPlan> {
        let mut entity_types = Vec::new();
        if lower.contains("customer") {
            entity_types.push("customer");
        }
        if lower.contains("invoice") || lower.contains("overdue") {
            entity_types.push("invoice");
        }
        if lower.contains("quote") || lower.contains("proposal") {
            entity_types.push("quote");
        }

        let mut metrics = Vec::new();
        if lower.contains("revenue") {
            metrics.push("revenue");
        }
        if lower.contains("profit") {
            metrics.push("profit");
        }
        if lower.contains("margin") {
            metrics.push("margin");
        }

        if entity_types.is_empty() && metrics.is_empty() {
            return None;
        }

        let intent = if lower.contains("profit") && lower.contains("loss") {
            AskIntent::ProfitLossQuery
        } else if lower.contains("accounting") || lower.contains("summary") {
            AskIntent::AccountingSummary
        } else if !entity_types.is_empty() && entity_types.contains(&"customer") {
            AskIntent::CustomerHistory
        } else if !entity_types.is_empty() && entity_types.contains(&"quote") {
            AskIntent::QuoteHistory
        } else if !entity_types.is_empty() && entity_types.contains(&"invoice") {
            AskIntent::InvoiceStatus
        } else {
            AskIntent::AccountingSummary
        };

        let mut plan = AskPlan::new(intent).with_peer_consult(true);

        for et in &entity_types {
            plan.retrieval_steps.push(RetrievalStep {
                source_type: RetrievalSource::EntityCards,
                query: et.to_string(),
                filters: vec![("entity_type".into(), et.to_string())],
                limit: 20,
                required: true,
            });
        }
        for m in &metrics {
            plan.retrieval_steps.push(RetrievalStep {
                source_type: RetrievalSource::FactQuery,
                query: m.to_string(),
                filters: vec![("metric".into(), m.to_string())],
                limit: 15,
                required: true,
            });
        }
        if metrics.is_empty()
            && (lower.contains("revenue") || lower.contains("profit") || lower.contains("margin"))
        {
            plan.retrieval_steps.push(RetrievalStep {
                source_type: RetrievalSource::FactQuery,
                query: "facts".into(),
                filters: vec![],
                limit: 15,
                required: true,
            });
        }

        // Add FTS for document context
        let kw: String = lower
            .split_whitespace()
            .filter(|w| w.len() > 2)
            .take(5)
            .collect::<Vec<_>>()
            .join(" OR ");
        if !kw.is_empty() {
            plan.retrieval_steps.push(RetrievalStep {
                source_type: RetrievalSource::FtsSearch,
                query: kw,
                filters: vec![],
                limit: 20,
                required: false,
            });
        }
        Some(plan)
    }

    fn plan_general_fts(question: &str, lower: &str) -> AskPlan {
        let mut plan = AskPlan::new(AskIntent::GeneralDocumentSummary);
        let fts_query = crate::evidence_collector::to_fts5_query(question);
        plan.retrieval_steps.push(RetrievalStep {
            source_type: RetrievalSource::FtsSearch,
            query: fts_query.clone(),
            filters: vec![],
            limit: 100,
            required: true,
        });
        plan.retrieval_steps.push(RetrievalStep {
            source_type: RetrievalSource::DocumentChunk,
            query: fts_query,
            filters: vec![],
            limit: 30,
            required: false,
        });
        plan.requires_peer_consult = true;
        plan.allows_web_fallback = context_hits_likely_empty(lower);
        plan
    }
}

impl Default for AskPlanner {
    fn default() -> Self {
        Self::new()
    }
}

/// Heuristic: empty FTS likely for general-knowledge questions.
fn context_hits_likely_empty(lower: &str) -> bool {
    let prefixes = [
        "who is ",
        "what is ",
        "when did ",
        "where is ",
        "why did ",
        "how does ",
    ];
    prefixes.iter().any(|p| lower.starts_with(p)) && lower.len() < 80
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_entity_question_people() {
        let p = AskPlanner::new();
        let plan = p.plan("Who appears in my documents?");
        assert_eq!(plan.intent, AskIntent::ListEntities);
        assert_eq!(plan.retrieval_steps.len(), 1);
        assert!(matches!(
            plan.retrieval_steps[0].source_type,
            RetrievalSource::EntityQuery
        ));
        assert!(!plan.allows_web_fallback);
    }

    #[test]
    fn classify_entity_question_companies() {
        let p = AskPlanner::new();
        let plan = p.plan("Which companies are mentioned?");
        assert_eq!(plan.intent, AskIntent::ListEntities);
        assert!(matches!(
            plan.retrieval_steps[0].source_type,
            RetrievalSource::EntityQuery
        ));
    }

    #[test]
    fn classify_pricing_question() {
        let p = AskPlanner::new();
        let plan = p.plan("What have we historically charged for Cat6 installs?");
        assert_eq!(plan.intent, AskIntent::PricingHistory);
        assert!(plan.requires_peer_consult);
        assert!(!plan.allows_web_fallback);
        let has_fact = plan
            .retrieval_steps
            .iter()
            .any(|s| matches!(s.source_type, RetrievalSource::FactQuery));
        assert!(has_fact);
    }

    #[test]
    fn classify_document_lookup() {
        let p = AskPlanner::new();
        let plan = p.plan("Summarize document invoice.pdf");
        assert_eq!(plan.intent, AskIntent::DocumentLookup);
        let has_fts = plan.retrieval_steps.iter().any(|s| {
            matches!(
                s.source_type,
                RetrievalSource::FtsSearch | RetrievalSource::DocumentChunk
            )
        });
        assert!(has_fts);
    }

    #[test]
    fn classify_web_freshness_question() {
        let p = AskPlanner::new();
        let plan = p.plan("What happened in Iran yesterday?");
        assert_eq!(plan.intent, AskIntent::WebFreshnessNeeded);
        assert!(plan.allows_web_fallback);
    }

    #[test]
    fn classify_documents_mentioning() {
        let p = AskPlanner::new();
        let plan = p.plan("Documents mentioning Acme Corp");
        assert_eq!(plan.intent, AskIntent::DocumentLookup);
        assert!(plan
            .retrieval_steps
            .iter()
            .any(|s| s.query.starts_with("documents_mentioning:")));
    }
}
