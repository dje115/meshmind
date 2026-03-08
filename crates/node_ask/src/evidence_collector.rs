//! EvidenceCollector: executes AskPlan and returns structured evidence.

use rusqlite::Connection;

use node_storage::cas::CasStore;
use node_storage::search;

use crate::ask_plan::{AskPlan, RetrievalSource, RetrievalStep};

const STOP_WORDS: &[&str] = &[
    "a", "an", "the", "is", "are", "was", "were", "be", "been", "have", "has", "had", "do", "does",
    "did", "will", "would", "shall", "should", "may", "might", "can", "could", "of", "in", "on",
    "at", "to", "for", "with", "from", "by", "about", "and", "but", "or",
];

/// Convert question to FTS5 query (keywords OR).
pub fn to_fts5_query(text: &str) -> String {
    let keywords: Vec<&str> = text
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|w| w.len() > 1 && !STOP_WORDS.contains(&w.to_lowercase().as_str()))
        .collect();
    if keywords.is_empty() {
        text.split_whitespace().next().unwrap_or("*").to_string()
    } else {
        keywords.join(" OR ")
    }
}

/// A single evidence item with provenance.
#[derive(Debug, Clone)]
pub struct EvidenceItem {
    pub source_type: String,
    pub source_id: String,
    pub title: Option<String>,
    pub payload: String,
    pub confidence_hint: f32,
}

/// Collected evidence from executing the plan.
#[derive(Debug, Clone, Default)]
pub struct Evidence {
    pub items: Vec<EvidenceItem>,
    pub entity_ids: Vec<String>,
    pub context_bullets: Vec<String>,
    pub search_hits: Vec<search::SearchHit>,
}

impl Evidence {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
            && self.entity_ids.is_empty()
            && self.context_bullets.is_empty()
            && self.search_hits.is_empty()
    }
}

/// Execute the plan and collect evidence. Does not perform peer consult or web.
pub fn collect_evidence(
    conn: &Connection,
    cas: &CasStore,
    plan: &AskPlan,
    question: &str,
    max_chunk_chars: usize,
) -> Result<Evidence, node_storage::search::SearchError> {
    let mut evidence = Evidence::default();
    let budget = &plan.retrieval_budget;
    let mut total_chars = 0usize;

    for step in plan.retrieval_steps.iter().take(budget.max_steps) {
        match &step.source_type {
            RetrievalSource::EntityQuery => {
                let (items, ids) = execute_entity_step(conn, step, budget)?;
                for item in items {
                    evidence.items.push(item);
                }
                evidence.entity_ids.extend(ids);
            }
            RetrievalSource::EntityCards => {
                let (items, ids) = execute_entity_cards_step(conn, step, budget)?;
                for item in items {
                    evidence.items.push(item);
                }
                evidence.entity_ids.extend(ids);
            }
            RetrievalSource::FactQuery => {
                let items = execute_fact_step(conn, step, budget)?;
                for item in items {
                    evidence.items.push(item);
                }
            }
            RetrievalSource::FtsSearch | RetrievalSource::DocumentChunk => {
                let hits = search::search_all(conn, &step.query, step.limit.min(budget.max_hits))?;
                for h in hits {
                    evidence.search_hits.push(h);
                }
            }
            RetrievalSource::DocumentReference
            | RetrievalSource::PeerConsult
            | RetrievalSource::WebFallback => {
                // Handled by orchestrator
            }
        }
    }

    // Build context bullets from search hits and items
    evidence.context_bullets = build_context_bullets(
        &evidence.search_hits,
        &evidence.items,
        cas,
        question,
        max_chunk_chars.min(budget.max_chunk_chars),
        &mut total_chars,
    );

    Ok(evidence)
}

fn execute_entity_step(
    conn: &Connection,
    step: &RetrievalStep,
    budget: &crate::ask_plan::RetrievalBudget,
) -> Result<(Vec<EvidenceItem>, Vec<String>), node_storage::search::SearchError> {
    let mut items = Vec::new();
    let mut ids = Vec::new();

    // Check for documents_mentioning:X
    let query = step.query.trim();
    if query.starts_with("documents_mentioning:") {
        let mention = query.trim_start_matches("documents_mentioning:");
        let norm = node_extraction::normalize_value("company", mention);
        let docs = search::list_documents_for_entity(
            conn,
            &norm,
            None,
            step.limit.min(budget.max_entity_results),
        )?;
        if docs.is_empty() {
            let docs2 = search::list_documents_for_entity(
                conn,
                &mention.to_lowercase(),
                None,
                step.limit.min(budget.max_entity_results),
            )?;
            for (doc_id, etype, eval) in docs2 {
                items.push(EvidenceItem {
                    source_type: "document".into(),
                    source_id: format!("doc:{}", doc_id),
                    title: Some(format!("{}: {}", etype, eval)),
                    payload: format!("Document '{}' mentions {}: {}", doc_id, etype, eval),
                    confidence_hint: 0.85,
                });
                ids.push(format!("doc:{}", doc_id));
            }
        } else {
            for (doc_id, etype, eval) in docs {
                items.push(EvidenceItem {
                    source_type: "document".into(),
                    source_id: format!("doc:{}", doc_id),
                    title: Some(format!("{}: {}", etype, eval)),
                    payload: format!("Document '{}' mentions {}: {}", doc_id, etype, eval),
                    confidence_hint: 0.85,
                });
                ids.push(format!("doc:{}", doc_id));
            }
        }
        return Ok((items, ids));
    }

    // entity_type from filters or query
    let entity_type = step
        .filters
        .iter()
        .find(|(k, _)| k == "entity_type")
        .map(|(_, v)| v.as_str())
        .unwrap_or(query);

    let entities = search::list_entities_by_type(
        conn,
        entity_type,
        step.limit.min(budget.max_entity_results),
    )?;

    let mut seen = std::collections::HashSet::new();
    for e in entities {
        if seen.insert(e.normalized_value.clone()) {
            let label = format!("{}: {}", e.entity_type, e.entity_value);
            items.push(EvidenceItem {
                source_type: "entity".into(),
                source_id: e.entity_id.clone(),
                title: Some(label.clone()),
                payload: format!("- {}", label),
                confidence_hint: e.confidence,
            });
            ids.push(e.entity_id);
        }
    }
    Ok((items, ids))
}

fn execute_entity_cards_step(
    conn: &Connection,
    step: &RetrievalStep,
    budget: &crate::ask_plan::RetrievalBudget,
) -> Result<(Vec<EvidenceItem>, Vec<String>), node_storage::search::SearchError> {
    let entity_type = step
        .filters
        .iter()
        .find(|(k, _)| k == "entity_type")
        .map(|(_, v)| v.as_str());
    let hits =
        search::search_entity_cards(conn, entity_type, step.limit.min(budget.max_entity_results))?;
    let mut items = Vec::new();
    let mut ids = Vec::new();
    for h in hits {
        items.push(EvidenceItem {
            source_type: "entity_card".into(),
            source_id: h.entity_id.clone(),
            title: None,
            payload: format!(
                "[Entity {} {}] {}",
                h.entity_type, h.entity_id, h.attributes_json
            ),
            confidence_hint: 0.9,
        });
        ids.push(h.entity_id);
    }
    Ok((items, ids))
}

fn execute_fact_step(
    conn: &Connection,
    step: &RetrievalStep,
    budget: &crate::ask_plan::RetrievalBudget,
) -> Result<Vec<EvidenceItem>, node_storage::search::SearchError> {
    let metric = step
        .filters
        .iter()
        .find(|(k, _)| k == "metric")
        .map(|(_, v)| v.as_str());
    let filter = if step.query == "facts" {
        None
    } else {
        metric.or(Some(step.query.as_str()))
    };
    let hits = search::query_facts(conn, filter, step.limit.min(budget.max_fact_results))?;
    let mut items = Vec::new();
    for h in hits {
        items.push(EvidenceItem {
            source_type: "fact".into(),
            source_id: h.fact_id.clone(),
            title: None,
            payload: format!(
                "[Fact {}] metric={} value={} dimensions={}",
                h.fact_id, h.metric, h.value_json, h.dimensions_json
            ),
            confidence_hint: 0.9,
        });
    }
    Ok(items)
}

fn build_context_bullets(
    hits: &[search::SearchHit],
    items: &[EvidenceItem],
    cas: &CasStore,
    _question: &str,
    max_chars: usize,
    total_chars: &mut usize,
) -> Vec<String> {
    let mut bullets = Vec::new();

    // Entity/fact evidence first
    let entity_items: Vec<_> = items
        .iter()
        .filter(|i| {
            i.source_type == "entity"
                || i.source_type == "entity_card"
                || i.source_type == "fact"
                || i.source_type == "document"
        })
        .collect();
    if !entity_items.is_empty() {
        let block: String = entity_items
            .iter()
            .map(|i| i.payload.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let take = (max_chars - *total_chars).min(block.len());
        bullets.push(format!(
            "Entity graph and facts:\n{}",
            block.chars().take(take).collect::<String>()
        ));
        *total_chars += take;
    }

    // Search hits (FTS, document chunks)
    for h in hits {
        if *total_chars >= max_chars {
            break;
        }
        let content: String = if let Some(ref hash) = h.content_hash {
            cas.get_bytes(hash)
                .ok()
                .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                .and_then(|j: serde_json::Value| {
                    j.get("content_text")
                        .and_then(|v| v.as_str())
                        .map(String::from)
                })
                .unwrap_or_default()
        } else {
            String::new()
        };

        let text = if !content.is_empty() {
            content
        } else {
            h.summary.clone()
        };
        let take = (max_chars - *total_chars).min(1500).min(text.len());
        let excerpt: String = text.chars().take(take).collect();
        *total_chars += excerpt.len();
        bullets.push(format!("- [{}] {}:\n{}", h.hit_type, h.title, excerpt));
    }

    bullets
}

#[cfg(test)]
mod tests {
    use super::*;
    use node_storage::sqlite_views;
    use rusqlite::Connection;

    fn setup_entity_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        sqlite_views::create_schema(&conn).unwrap();
        conn.execute(
            "INSERT INTO entities_view (entity_id, entity_type, entity_value, normalized_value, document_id, chunk_index, confidence, extraction_method, classification_method, created_at_ms)
             VALUES ('person:jane', 'person', 'Jane Doe', 'jane doe', 'doc1', 0, 0.9, 'rule', 'rule_based', 1000),
                    ('person:john', 'person', 'John Smith', 'john smith', 'doc1', 0, 0.9, 'rule', 'rule_based', 1001)",
            [],
        )
        .unwrap();
        conn
    }

    fn setup_fact_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        sqlite_views::create_schema(&conn).unwrap();
        conn.execute(
            "INSERT INTO facts_view (fact_id, version, metric, value_json, dimensions_json, created_at_ms)
             VALUES ('f1', 1, 'revenue', '{\"amount\":5000}', '{}', 3000)",
            [],
        )
        .unwrap();
        conn
    }

    fn setup_entity_cards_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        sqlite_views::create_schema(&conn).unwrap();
        conn.execute(
            "INSERT INTO entity_cards_view (entity_id, entity_type, attributes_json, created_at_ms)
             VALUES ('customer:1', 'customer', '{\"name\":\"Acme\"}', 1000)",
            [],
        )
        .unwrap();
        conn
    }

    #[test]
    fn executes_entity_query_correctly() {
        let conn = setup_entity_conn();
        let tmp = tempfile::tempdir().unwrap();
        let cas = node_storage::cas::CasStore::open(tmp.path()).unwrap();
        let plan = crate::ask_plan::AskPlan::new(crate::ask_plan::AskIntent::ListEntities)
            .with_step(crate::ask_plan::RetrievalStep {
                source_type: crate::ask_plan::RetrievalSource::EntityQuery,
                query: "person".to_string(),
                filters: vec![("entity_type".into(), "person".into())],
                limit: 50,
                required: true,
            });
        let evidence = collect_evidence(&conn, &cas, &plan, "Who appears?", 50_000).unwrap();
        assert!(!evidence.items.is_empty());
        assert!(evidence.items.iter().any(|i| i.source_type == "entity"));
        assert!(evidence.entity_ids.iter().any(|id| id.contains("person")));
    }

    #[test]
    fn executes_fact_query_correctly() {
        let conn = setup_fact_conn();
        let tmp = tempfile::tempdir().unwrap();
        let cas = node_storage::cas::CasStore::open(tmp.path()).unwrap();
        let plan = crate::ask_plan::AskPlan::new(crate::ask_plan::AskIntent::AccountingSummary)
            .with_step(crate::ask_plan::RetrievalStep {
                source_type: crate::ask_plan::RetrievalSource::FactQuery,
                query: "revenue".to_string(),
                filters: vec![("metric".into(), "revenue".into())],
                limit: 15,
                required: true,
            });
        let evidence = collect_evidence(&conn, &cas, &plan, "What is revenue?", 50_000).unwrap();
        assert!(!evidence.items.is_empty());
        assert!(evidence.items.iter().any(|i| i.source_type == "fact"));
    }

    #[test]
    fn executes_entity_cards_correctly() {
        let conn = setup_entity_cards_conn();
        let tmp = tempfile::tempdir().unwrap();
        let cas = node_storage::cas::CasStore::open(tmp.path()).unwrap();
        let plan = crate::ask_plan::AskPlan::new(crate::ask_plan::AskIntent::CustomerHistory)
            .with_step(crate::ask_plan::RetrievalStep {
                source_type: crate::ask_plan::RetrievalSource::EntityCards,
                query: "customer".to_string(),
                filters: vec![("entity_type".into(), "customer".into())],
                limit: 20,
                required: true,
            });
        let evidence = collect_evidence(&conn, &cas, &plan, "List customers", 50_000).unwrap();
        assert!(!evidence.items.is_empty());
        assert!(evidence
            .items
            .iter()
            .any(|i| i.source_type == "entity_card"));
    }

    #[test]
    fn executes_fts_chunk_retrieval_correctly() {
        use node_proto::common::*;
        use node_proto::events::*;
        use node_storage::projector;

        let conn = Connection::open_in_memory().unwrap();
        sqlite_views::create_schema(&conn).unwrap();
        let attrs = serde_json::json!({
            "document_id": "doc-ftstest",
            "chunk_index": 0,
            "chunk_text": "Budget allocation for Q4 infrastructure"
        });
        let event = EventEnvelope {
            event_id: "e1".into(),
            r#type: EventType::ArtifactPublished as i32,
            ts: Some(Timestamp {
                unix_ms: 1700000000000,
            }),
            node_id: Some(NodeId {
                value: "node-1".into(),
            }),
            tenant_id: Some(TenantId {
                value: "public".into(),
            }),
            sensitivity: Sensitivity::Public as i32,
            event_hash: Some(HashRef {
                sha256: "h1".into(),
            }),
            payload: Some(event_envelope::Payload::ArtifactPublished(
                ArtifactPublished {
                    artifact_id: "doc-ftstest::chunk::0".into(),
                    artifact_type: ArtifactType::Document as i32,
                    version: 1,
                    title: "Doc chunk 0".into(),
                    summary: "Budget section".into(),
                    content_ref: Some(HashRef {
                        sha256: "ch0".into(),
                    }),
                    shareable: false,
                    expires_unix_ms: 0,
                    document_subtype: "document_chunk".into(),
                    entity_attributes_json: attrs.to_string(),
                    ..Default::default()
                },
            )),
            ..Default::default()
        };
        projector::apply_event(&conn, &event).unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let cas = node_storage::cas::CasStore::open(tmp.path()).unwrap();
        let plan = crate::ask_plan::AskPlan::new(crate::ask_plan::AskIntent::DocumentLookup)
            .with_step(crate::ask_plan::RetrievalStep {
                source_type: crate::ask_plan::RetrievalSource::FtsSearch,
                query: "budget infrastructure".to_string(),
                filters: vec![],
                limit: 10,
                required: true,
            });
        let evidence = collect_evidence(&conn, &cas, &plan, "Budget info", 50_000).unwrap();
        assert!(!evidence.search_hits.is_empty());
    }

    /// Integration: planner + collector for entity question uses entity path
    #[test]
    fn integration_who_appears_uses_entity_path() {
        let conn = setup_entity_conn();
        let tmp = tempfile::tempdir().unwrap();
        let cas = node_storage::cas::CasStore::open(tmp.path()).unwrap();
        let planner = crate::ask_planner::AskPlanner::new();
        let plan = planner.plan("Who appears in my documents?");
        assert_eq!(plan.intent, crate::ask_plan::AskIntent::ListEntities);
        let evidence = collect_evidence(&conn, &cas, &plan, "Who appears?", 50_000).unwrap();
        assert!(!evidence.items.is_empty(), "should have entity evidence");
        assert!(evidence.items.iter().any(|i| i.source_type == "entity"));
    }

    /// Integration: pricing question uses pricing path (plan + execute, no crash)
    #[test]
    fn integration_pricing_uses_pricing_path() {
        let conn = setup_fact_conn();
        let tmp = tempfile::tempdir().unwrap();
        let cas = node_storage::cas::CasStore::open(tmp.path()).unwrap();
        let planner = crate::ask_planner::AskPlanner::new();
        let plan = planner.plan("What have we historically charged for Cat6?");
        assert_eq!(plan.intent, crate::ask_plan::AskIntent::PricingHistory);
        let _evidence = collect_evidence(&conn, &cas, &plan, "Pricing?", 50_000).unwrap();
        // Execution succeeds; evidence may be empty if no pricing/quote data matches
        assert!(plan.retrieval_steps.len() >= 2);
    }

    /// Integration: web freshness question allows web, plan has web fallback
    #[test]
    fn integration_web_freshness_allows_web() {
        let planner = crate::ask_planner::AskPlanner::new();
        let plan = planner.plan("What happened yesterday in Iran?");
        assert_eq!(plan.intent, crate::ask_plan::AskIntent::WebFreshnessNeeded);
        assert!(plan.allows_web_fallback);
    }
}
