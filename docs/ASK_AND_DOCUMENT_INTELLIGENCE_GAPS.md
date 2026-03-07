# Ask and Document Intelligence — Gap Analysis (Phase 0)

This document provides a precise implementation gap report for improving document intelligence extraction, entity graph quality, planner-first ask pipeline, business-intelligence question support, web fallback decision logic, and training-signal capture. It is grounded in the actual codebase as of v2.0.

---

## 1. Current State

### 1.1 Document Ingestion

**How it works today:**

- **Connectors** (`node_connectors`): SQLite, CSV, JSON, Image (EXIF metadata), Document (PDF, DOCX, TXT, MD, RTF), OneDrive (OAuth, partial ingest).
- **DocumentConnector**:
  - PDF: `pdf_oxide` text extraction (no OCR)
  - DOCX: `docx_rust` paragraph/run text extraction
  - Output: `IngestRow` with columns `filename`, `file_path`, `file_type`, `file_size_bytes`, `page_count`, `content_text` (truncated to 100KB per file)
  - No chunking, no semantic extraction, no NER
- **Ingest pipeline** (`node_ingest`):
  - Rows → `ArtifactPublished` (artifact_type=Document, document_subtype=entity_card or inferred)
  - Structured tables: `entity_type` from table name inference or mapping hints; `entity_key` from id/customer_id/etc.; FK columns → `EntityRelationshipRecorded`
  - Documents table: each file = one row → one artifact with `document_subtype` and `content_text` in `entity_attributes_json`
- **Projection** (`node_storage/projector`):
  - `artifacts_view`, `artifacts_fts` (title, summary)
  - `documents_view` (document_id, entity_type, entity_key, title, summary)
  - `entity_cards_view` only for `entity_card` subtype (structured rows, not document-folder output)

**Artifacts FTS:** `artifacts_fts` indexes `title` and `summary`. For Document artifacts, `build_artifact_summary` uses `content_text` if present, truncated to 500 chars. So only a prefix of document content is searchable.

### 1.2 Ask Flow

**Order of operations** (`node_api` `handle_ask`):

1. **FTS retrieval**: `search_all` (cases_fts + artifacts_fts) with `to_fts5_query` (keyword OR, stop-word removal)
2. **Business intent**: `classify_business_intent` (keyword-based) → `search_entity_cards` + `query_facts` when intent matches
3. **Web search**: If `wants_web_search` OR (`context_hits.is_empty()` AND `looks_like_general_knowledge_question`)
4. **Context assembly**: `build_context_bullets` (fetches CAS content for hits, adaptive sizing)
5. **Single LLM call**: One prompt, one answer
6. **Peer consult**: If `local_confidence < 0.6` and transport exists; shard-aware routing via `peers_for_question`
7. **Response**: answer, confidence, context_used, case_id, source_types, evidence, missing_data_warnings

**No planner.** Retrieval is done first; there is no decision about which retrieval strategy to use before executing it. Evidence is always merged and sent to the LLM in one shot.

### 1.3 Entity Extraction

**What exists:**

- **Structured sources**: Entity cards from table rows. `entity_type` inferred from table name (e.g. `invoices` → invoice) or mapping hints. `entity_key` from `id`, `customer_id`, etc. Relationships from FK columns (`customer_id` → `belongs_to_customer`).
- **documents_view / entity_cards_view**: Populated only for artifacts with `document_subtype == "entity_card"`, i.e. structured ingest, not DocumentConnector output.
- **Entity search**: `search_entity_cards(conn, entity_type, limit)` — filter by entity_type, ordered by created_at_ms. No semantic search.
- **Facts**: `query_facts(conn, metric_filter, limit)` — filter by metric. No aggregation by dimension at query time.

**What does not exist:** No NER, no people/company extraction from unstructured text, no LLM-based entity resolution, no cross-document entity linking.

### 1.4 Structured Projections

| View | Content | FTS |
|------|---------|-----|
| cases_view | case_id, title, summary | cases_fts |
| artifacts_view | artifact_id, type, title, summary, content_hash | artifacts_fts |
| documents_view | document_id, entity_type, entity_key, title, summary | (via artifacts_fts) |
| entity_cards_view | entity_id, entity_type, attributes_json | No |
| entity_relationships_view | from_entity_id, to_entity_id, relationship_type | No |
| facts_view | fact_id, metric, value_json, dimensions_json | No |
| web_briefs_view | question, summary, sources | Yes (artifact) |

### 1.5 Question Types Supported

- **FTS**: Free-text over cases + artifacts (title, summary, tags)
- **Business intent** (keyword): customer, invoice, quote, account, job, revenue, profit, margin, pricing → entity cards + facts
- **Broad requests**: "summarize", "trends", "across all" → fallback to wider FTS
- **Document-specific**: "read IT3000.docx" → `extract_document_refs_from_question` + content fetch
- **Web**: "search the web", "look it up online", or empty + general-knowledge prefixes

### 1.6 Web Fallback

**Triggers:**

- `wants_web_search`: Contains "search the web", "look it up online", "search online", etc.
- OR: `context_hits.is_empty()` AND `looks_like_general_knowledge_question` (prefixes: "who is", "what is", "when did", etc.)

**Policy:** `can_research_web(allow_web, redaction_required)`. Node must have `research_web_capable`.

**Implementation:** DuckDuckGo HTML scrape → fetch first URL → LLM summarize. No learned decision; purely heuristic.

### 1.7 Training Signals

**Captured today:**

- `CaseConfirmed` via `POST /ask/confirm` (case_id, outcome: accepted|rejected|edited, confidence)
- `CaseFailed`, `QuoteAccepted`, `QuoteLost`, `QuoteRevised` via `POST /v1/outcomes`

**Dataset preset `ThisTenantConfirmed`:** Includes CaseConfirmed, CaseFailed, QuoteAccepted, QuoteLost, QuoteRevised.

**Trainer** (`node_trainer`): `run_job` is **simulated**. Returns fixed score 0.85; no actual model training. ModelRegistry stores versions but no real weights. **RouterClassifier, TaggerClassifier, Ranker are not implemented** — documented in `router-model.md` and `training.md` but never wired into the ask flow. No code path loads a trained router model to decide LOCAL/PEER/WEB.

---

## 2. Gaps

### 2.1 Document Intelligence Extraction

| Gap | Detail |
|-----|--------|
| No chunking | Documents ingested as single blobs. Long PDFs/DOCX truncated at 100KB. No overlap, no semantic boundaries. |
| No structured extraction | No tables, lists, or key-value pairs extracted from documents. |
| No NER from text | No people, companies, dates, amounts extracted from unstructured content. |
| Limited FTS coverage | Only title + summary (500-char prefix of content_text) in artifacts_fts. Full document body not indexed. |
| No semantic embeddings | No vector search; FTS only. |
| OCR | No OCR for scanned PDFs (TODO.md notes this). |

### 2.2 Entity Graph / People–Company Extraction

| Gap | Detail |
|-----|--------|
| No NER from documents | Entity cards come only from structured tables. DocumentConnector output does not produce entity cards. |
| No people/company extraction | No extraction of person names, organizations from text. |
| FK-only relationships | Relationships only from known FK columns (customer_id, quote_id, etc.). No inferred links. |
| No entity resolution | No deduplication or linking of "Acme Corp" vs "Acme Corporation". |
| No entity_type from content | Entity type comes from schema, not from document content. |

### 2.3 Planner-First Ask Flow

| Gap | Detail |
|-----|--------|
| No planner | Retrieval runs before any strategy decision. No "plan then execute" step. |
| Retrieval order fixed | Always: FTS → business intent → (maybe) web. No conditional branches. |
| No retrieval budget | No decision about how much to retrieve, which sources to query first. |
| Single evidence merge | All evidence merged into one prompt. No multi-step reasoning or iterative retrieval. |

### 2.4 Structured Business-Intelligence Queries

| Gap | Detail |
|-----|--------|
| Keyword-only intent | `classify_business_intent` uses substring matching. No semantic classifier. |
| No SQL/aggregation | Facts are pre-aggregated (row_count, sum/avg/min/max per column). No ad-hoc aggregations (e.g. "revenue by customer", "top 5 quotes by margin"). |
| Entity filter only | `search_entity_cards` filters by entity_type. No "customer X", "invoice for Y" semantic filters. |
| No join across entities | No query that joins invoices + customers at ask time. |

### 2.5 Web Fallback Decision Quality

| Gap | Detail |
|-----|--------|
| Heuristic only | Triggers are keywords and prefix patterns. No learned model. |
| No confidence-based escalation | Web is triggered by empty context + general-knowledge pattern, or explicit "search" phrase. Not by "local answer confidence too low". |
| No RouterClassifier | RouterClassifier is documented but not implemented. Ask flow never consults a router model. |
| Policy gates only | Policy can deny web, but the decision to *try* web is heuristic. |

### 2.6 Answer Outcome Capture for Training

| Gap | Detail |
|-----|--------|
| CaseConfirmed exists | POST /ask/confirm emits CaseConfirmed. |
| Outcomes exist | POST /v1/outcomes emits CaseFailed, QuoteAccepted, etc. |
| Trainer simulated | No real training. RouterClassifier not trained, not loaded. |
| No wiring | Even if a router model existed, ask flow does not call it. |
| Outcome→router link missing | CaseConfirmed is in ThisTenantConfirmed preset, but no training loop consumes it to produce a router model. |

---

## 3. Recommended Implementation Order

### Phase A: Document Intelligence (First)

**Rationale:** Better document representation is foundational. Entity extraction and BI queries depend on it. Current 500-char summary limit severely restricts document usefulness.

**Scope:**

1. Chunking for long documents (overlap, semantic boundaries or fixed-size)
2. Full content indexing in FTS (or separate documents_fts) so full text is searchable
3. Optional: table/key-value extraction from structured documents (PDF tables, DOCX tables)

**Dependencies:** None. Can be done in DocumentConnector + ingest + projector.

**Defer:** Semantic embeddings, NER (Phase B/C).

---

### Phase B: Entity Graph Quality

**Rationale:** Enables "who", "which company", "invoices for customer X" without full-text scan. Depends on Phase A if we want entity extraction from documents.

**Scope:**

1. NER from document content (people, orgs, dates) → entity_cards_view, entity_relationships_view
2. Entity resolution / deduplication (optional, can start simple)
3. Document-derived entity cards (not just table rows)

**Dependencies:** Phase A for document chunks; optional LLM or NER library.

**Defer:** Full knowledge-graph reasoning.

---

### Phase C: Planner-First Ask

**Rationale:** Deciding *what* to retrieve before retrieving improves efficiency and answer quality. Should come after we have multiple evidence types to choose from.

**Scope:**

1. Planner step: given question, produce retrieval plan (which sources, in what order, with what filters)
2. Execute plan: conditional retrieval (e.g. BI query first if intent is BI, else FTS)
3. Optional: iterative retrieval (retrieve → re-plan → retrieve again)

**Dependencies:** Phases A/B for richer evidence; can start with rule-based planner.

**Defer:** Learned planner; multi-turn reasoning.

---

### Phase D: Structured BI Queries

**Rationale:** "Revenue by customer", "top 5 quotes" require aggregation and joins. Builds on entity graph.

**Scope:**

1. Intent → structured query mapping (e.g. "revenue by customer" → query facts + entity cards with dimension)
2. Simple aggregation at query time (GROUP BY dimension, ORDER BY value LIMIT)
3. Optional: natural language → SQL or query-builder (higher risk)

**Dependencies:** Phase B (entity graph); facts_view already has dimensions_json.

**Defer:** Full SQL generation; complex joins.

---

### Phase E: Web Fallback Decision Quality

**Rationale:** Avoid unnecessary web calls; escalate when local is insufficient. Benefits from RouterClassifier.

**Scope:**

1. Implement RouterClassifier training loop (consume ThisTenantConfirmed, produce LOCAL/PEER/WEB decision)
2. Wire RouterClassifier into ask flow *before* retrieval (or before web step)
3. Replace heuristic web trigger with router output (policy still gates)

**Dependencies:** Training signals (CaseConfirmed, etc.) exist. Trainer must be extended from simulation to real training.

**Defer:** Federated router; A/B testing.

---

### Phase F: Training-Signal Capture (Parallel / Early)

**Rationale:** More and richer outcomes improve all learned components. Can be improved in parallel with Phases A–E.

**Scope:**

1. Ensure CaseConfirmed, CaseFailed, QuoteAccepted, etc. are fully projected and queryable
2. Link case_id to retrieval context (which sources were used) for better training features
3. Extend DatasetManifest / presets if new outcome types are added
4. Replace Trainer simulation with real RouterClassifier training

**Dependencies:** None for capture; Trainer changes for consumption.

---

## 4. Concrete Code Touchpoints

| Phase | Files / Modules |
|-------|-----------------|
| **A: Document intelligence** | `node_connectors/src/lib.rs` (DocumentConnector: chunking, full-text handling), `node_ingest` (chunk → artifact or extend artifact schema), `node_storage/sqlite_views.rs` (documents_fts or expand artifacts_fts), `node_storage/projector.rs` (FTS update for document content) |
| **B: Entity graph** | `node_connectors` or new `node_extraction` (NER), `node_ingest` (emit EntityRelationshipRecorded from NER), `node_storage/projector.rs` (entity_cards_view from document-derived entities), `proto/events.proto` (if new event types) |
| **C: Planner** | `node_api/src/lib.rs` (new `plan_retrieval` step before `handle_ask` body), new module `node_planner` or logic in node_api, `handle_ask` refactor to accept plan |
| **D: BI queries** | `node_storage/search.rs` (query_facts by dimension, aggregate), `node_api` (classify_business_intent → structured query builder), `handle_ask` (route to BI path) |
| **E: Web fallback** | `node_trainer` (real RouterClassifier training), `node_api` (load router, call before web step), `node_ai` or new crate (model format, inference) |
| **F: Training signals** | `node_storage/projector.rs` (outcomes_view completeness), `node_datasets` (preset for retrieval-context), `node_trainer` (run_job implementation, model serialization) |

---

## 5. Testing Gaps

| Area | Missing Tests | Integration Needs |
|------|---------------|-------------------|
| Document ingestion | Chunking correctness, full-content FTS hit | E2E: ingest PDF → search by phrase in middle of doc |
| Entity extraction | NER accuracy, relationship correctness | E2E: ingest doc with "Acme Corp" → entity card created |
| Planner | Plan quality, execution fidelity | E2E: question → plan → correct retrieval path |
| BI queries | Aggregation, dimension filter | E2E: "revenue by customer" → correct facts returned |
| Web fallback | Router decision vs heuristic | E2E: router says LOCAL → no web call; router says WEB → web called |
| Training | End-to-end: CaseConfirmed → manifest → train → model in use | E2E: confirm case → build preset → train → ask uses router |

**Unit tests needed:**

- Chunking boundaries and overlap
- classify_business_intent expansion (more intents)
- Plan→retrieval mapping
- Router inference (when model exists)

---

## 6. Risks

| Risk | Mitigation |
|------|------------|
| **Overusing LLM** | Prefer rule-based or lightweight models for routing, intent, NER where possible. Reserve LLM for generation and complex extraction. |
| **Ingesting unstructured blobs without normalization** | Chunk and structure before storage. Enforce document_subtype and entity_type. |
| **Schema drift** | Version event types and views. Migration path for projector. |
| **Poor provenance** | Every artifact and fact carries source_ref, ingest_id. Ensure evidence chain in AskResponse. |
| **Weak training labels** | CaseConfirmed outcome is coarse. Consider confidence, edit distance, or finer signals. Complement with synthetic or human review. |
| **Router overfitting** | Small dataset. Use simple model (logistic regression), eval gate, and rollback. |
| **Full-content FTS cost** | Index size growth. Consider separate documents_fts, or tiered indexing (summary vs full). |

---

## Summary

- **Document intelligence:** Chunking, full-content indexing, and optional structured extraction are missing. Current 500-char summary limit is a major gap.
- **Entity graph:** Only structured tables produce entity cards. No NER or people/company extraction from documents.
- **Planner:** No planner. Retrieval order is fixed; no "plan then execute".
- **BI queries:** Keyword intent and pre-aggregated facts only. No ad-hoc aggregation or joins.
- **Web fallback:** Heuristic triggers. RouterClassifier is documented but not implemented or wired.
- **Training:** CaseConfirmed and outcomes are captured. Trainer is simulated; no real RouterClassifier training or integration.

**Recommended first phase:** Document intelligence (chunking + full-content FTS). This unblocks entity extraction from documents and improves retrieval quality with minimal architectural change.
