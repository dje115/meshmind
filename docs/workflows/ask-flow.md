# MeshMind Ask Flow

## Decision Ladder

When a user asks a question, MeshMind follows this order:

1. **Local retrieval** — FTS search across cases, artifacts, documents
2. **Peer consult** — If router says so, forward to peers (max 3 hops)
3. **Business system queries** — Query entity cards, facts (structured)
4. **Web research fallback** — If router says so AND policy allows AND node has research capability

---

## Router Role

The RouterClassifier (when trained) influences:

- Whether to ask peers
- Whether to permit web fallback
- Still policy-gated: web research requires `allow_web` + `research_web_capable` + `redaction_required`

---

## Full Flow

```
User question
     ↓
Router decision (local / peers / web)
     ↓
Retrieve evidence (FTS + entity cards + facts)
     ↓
Rank evidence (if Ranker available)
     ↓
Assemble prompt with context
     ↓
LLM generate answer
     ↓
Store CaseDraft event
     ↓
Return answer + confidence + context_used + case_id + source_types + evidence + missing_data_warnings
     ↓
Optional: POST /v1/ask/confirm with case_id, outcome → CaseConfirmed (training signal)
Optional: POST /v1/outcomes for quote/case outcomes (QUOTE_ACCEPTED, QUOTE_LOST, CASE_FAILED)
```

---

## Response Fields (Distributed BI)

- **source_types** — Which sources contributed: `local`, `peer`, `web`, `insight`, `business_system`
- **evidence** — Per-item provenance: `id`, `source_type`, optional `title`
- **missing_data_warnings** — Structured warnings when data may be incomplete

Peer consult uses **shard-aware routing**: only peers hosting shards relevant to the question are contacted (see [distributed-memory.md](../distributed-memory.md)).

## Evidence Types

- **Cases** — Prior runbooks, resolved cases
- **Artifacts** — Ingested documents, rows
- **Documents** — Entity cards, runbooks (documents_view)
- **Facts** — Aggregates from facts_view
- **Web briefs** — Cached web research (if policy allows)

---

## Confirmation

- **case_id** — Returned with every answer; use for confirmation
- **POST /v1/ask/confirm** — Body: `{ case_id, outcome, confidence? }` (outcome: "accepted"|"rejected"|"edited")
- **CaseConfirmed** — Emitted when confirmed; feeds `this_tenant_confirmed` dataset preset for router training

---

## References

- [docs/intelligence/training.md](../intelligence/training.md) — RouterClassifier
- [docs/workflows/web-research.md](web-research.md) — Web fallback
- [docs/workflows/peer-consult.md](peer-consult.md) — Peer forwarding
- [docs/distributed-memory.md](../distributed-memory.md) — Shard routing, BI provenance
