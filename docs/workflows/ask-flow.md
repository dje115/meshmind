# MeshMind Ask Flow

## Planner-First Flow (Phase C)

MeshMind uses a **planner-first** flow: the AskPlanner classifies intent and produces an AskPlan *before* retrieval. The plan controls which evidence sources are used and whether peer consult or web fallback is allowed.

1. **AskPlanner** — Classify intent, build retrieval plan
2. **EvidenceCollector** — Execute plan steps (entity, fact, FTS, document chunks)
3. **Peer consult** — Only if `plan.requires_peer_consult` and local confidence is low
4. **Web fallback** — Only if `plan.allows_web_fallback` and policy allows

See [docs/ask-planner.md](../ask-planner.md) for the full planner specification.

---

## Decision Ladder (Legacy / Training)

When a trained RouterClassifier exists, it will influence:

- Whether to ask peers
- Whether to permit web fallback
- Still policy-gated: web research requires `allow_web` + `research_web_capable` + `redaction_required`

---

## Full Flow

```
User question
     ↓
AskPlanner → AskPlan (intent, retrieval_steps, peer/web flags)
     ↓
EvidenceCollector (execute plan steps)
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
