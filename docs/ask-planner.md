# Ask Planner — Planner-First Ask Flow

Phase C introduces a **planner-first** ask flow: the system decides *what* to retrieve and *how* before executing retrieval. This improves answer quality by using structured evidence first and avoiding mixed-context overload.

---

## Planner-First vs Retrieval-First

### Before (Retrieval-First)

- **Fixed order**: FTS → business intent augmentation → document entity intent → web fallback
- **All evidence merged early**: Entity bullets, fact bullets, and document chunks merged into one context block
- **LLM overloaded**: Model must reason over mixed evidence and guess which source is authoritative

### After (Planner-First)

- **Plan first**: `AskPlanner` classifies intent and produces an `AskPlan`
- **Conditional retrieval**: `EvidenceCollector` executes only the steps in the plan
- **Structured evidence**: Entity, fact, and document evidence are clearly separated
- **Planner-controlled fallbacks**: Peer consult and web fallback are explicitly allowed or disallowed by the plan

---

## Flow Overview

```
question
  → AskPlanner
  → AskPlan
  → EvidenceCollector (executes plan steps)
  → optional peer consult (if plan.requires_peer_consult)
  → optional web fallback (if plan.allows_web_fallback)
  → LLM explanation/formatting
  → response with evidence + confidence
```

---

## AskPlan Structure

| Field | Description |
|-------|-------------|
| `intent` | Classified intent (see Supported Intents) |
| `retrieval_steps[]` | Ordered list of retrieval steps to execute |
| `requires_peer_consult` | Whether peer consult may be used when local confidence is low |
| `allows_web_fallback` | Whether web search is permitted |
| `explanation_mode` | How the LLM should explain: "concise" \| "detailed" \| "cite_sources" |
| `retrieval_budget` | Limits: max_steps, max_hits, max_chunk_chars, max_entity_results, max_fact_results |
| `source_priority[]` | Merge order: e.g. ["entity", "fact", "document_chunk", "fts"] |

### RetrievalStep

| Field | Description |
|-------|-------------|
| `source_type` | EntityQuery, FactQuery, EntityCards, FtsSearch, DocumentChunk, DocumentReference, PeerConsult, WebFallback |
| `query` | Query string or action (e.g. "person", "revenue", FTS query) |
| `filters` | Optional filters (entity_type, metric) |
| `limit` | Result limit |
| `required` | Whether this step is required |

---

## Supported Intents

| Intent | Description | Example Questions |
|--------|-------------|-------------------|
| `list_entities` | List people, companies, emails, etc. | "Who appears in my documents?", "Which companies are mentioned?" |
| `count_entities` | Count by type | — |
| `document_lookup` | Summarize or find specific document | "Summarize document X", "Documents mentioning Acme" |
| `customer_history` | Customer-related queries | "Customer X orders" |
| `quote_history` | Quote/proposal queries | "Quotes for customer Y" |
| `pricing_history` | Historical pricing | "What have we charged for Cat6 installs?" |
| `invoice_status` | Invoice queries | "Overdue invoices" |
| `accounting_summary` | Revenue, profit, margin | "Revenue last quarter" |
| `profit_loss_query` | P&L questions | "Profit and loss trends" |
| `trend_change` | Trend analysis | — |
| `anomaly_question` | Anomaly detection | — |
| `general_document_summary` | Broad FTS search | General document questions |
| `web_freshness_needed` | Fresh/general knowledge | "What happened in Iran yesterday?" |
| `unknown` | Fallback | — |

---

## Evidence Collection Flow

1. **EntityQuery**: `list_entities_by_type` for person, company, email, invoice_number, quote_number; or `list_documents_for_entity` for "documents mentioning X"
2. **EntityCards**: `search_entity_cards` for customer, invoice, quote, etc.
3. **FactQuery**: `query_facts` for revenue, profit, margin, pricing
4. **FtsSearch** / **DocumentChunk**: `search_all` over documents_fts, cases_fts, artifacts_fts
5. **DocumentReference**: Fetch by doc ID (orchestrator-level)
6. **PeerConsult** / **WebFallback**: Handled by the API layer when plan allows

Evidence items include:
- `source_type`, `source_id`, `title`, `payload`, `confidence_hint`

---

## Example Question Plans

### Entity question

**Question:** "Who appears in my documents?"

**Plan:**
- intent = `list_entities`
- retrieval_steps = [EntityQuery(person)]
- no peer, no web

### Document question

**Question:** "Summarize document invoice.pdf"

**Plan:**
- intent = `document_lookup`
- retrieval_steps = [DocumentChunk(fts_query), FtsSearch(fts_query)]
- no peer, no web

### Pricing history question

**Question:** "What have we historically charged for Cat6 installs?"

**Plan:**
- intent = `pricing_history`
- retrieval_steps = [FactQuery(pricing), EntityCards(quote), DocumentChunk(cat6 install)]
- peer consult allowed
- no web

### Web freshness question

**Question:** "What happened in Iran yesterday?"

**Plan:**
- intent = `web_freshness_needed`
- retrieval_steps = [optional FtsSearch for local match]
- web fallback allowed
- no peer

---

## Retrieval Budget

The planner enforces budgets to avoid over-fetching:

| Field | Default | Purpose |
|-------|---------|---------|
| max_steps | 5 | Limit retrieval steps executed |
| max_hits | 100 | Limit FTS/document hits |
| max_chunk_chars | 95,000 | Limit total chunk text in prompt |
| max_entity_results | 50 | Limit entity query results |
| max_fact_results | 20 | Limit fact query results |

---

## Prompt Assembly

The LLM receives:
- **Plan summary**: Intent and steps used
- **Evidence sections**: Entity graph and facts first, then document chunks
- **Web results** (if applicable): Clearly labeled
- **Question**: Original user question

The LLM's role: **explain**, **summarize**, **format**, **highlight assumptions** — not guess the evidence source.

---

## Implementation

- **AskPlanner**: Rule-based, deterministic, in `node_ask::ask_planner`
- **EvidenceCollector**: `node_ask::evidence_collector::collect_evidence`
- **handle_ask**: `node_api` — builds plan, collects evidence, assembles prompt, runs LLM, optionally consults peers or web

---

## References

- [docs/ASK_AND_DOCUMENT_INTELLIGENCE_GAPS.md](ASK_AND_DOCUMENT_INTELLIGENCE_GAPS.md) — Phase C spec
- [docs/workflows/ask-flow.md](workflows/ask-flow.md) — Ask flow overview
- [docs/business-intelligence.md](business-intelligence.md) — Entity cards, facts
- [docs/use-cases.md](use-cases.md) — Example scenarios
