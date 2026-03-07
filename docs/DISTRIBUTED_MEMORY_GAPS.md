# Distributed Memory Fabric — Gap Review

## Executive Summary

MeshMind has a solid local-first architecture: Event Log (source of truth), CAS (immutable blobs), SQLite (rebuildable views), policy-gated ingest/share/train, pluggable inference, and dataset manifests for training provenance. To evolve into a **distributed memory fabric** and **proactive business intelligence platform**, several extensions are needed. This document catalogs what exists, what is missing, and what the implementation will add.

---

## 1. What Already Exists

### 1.1 Core Memory Engine

| Component | Status | Location |
|-----------|--------|----------|
| Event Log | ✅ Full | `node_storage/event_log.rs` — append-only, hash-chained, segment rotation |
| CAS | ✅ Full | `node_storage/cas.rs` — SHA-256 content-addressed blobs, deduplication |
| SQLite views | ✅ Full | `node_storage/sqlite_views.rs` — cases, artifacts, sources, ingests, datasets, entities, facts |
| Projector | ✅ Full | `node_storage/projector.rs` — event → view projection, rebuildable |
| FTS5 search | ✅ Full | `node_storage/search.rs` — cases_fts, artifacts_fts |

### 1.2 Policy and Provenance

| Component | Status | Location |
|-----------|--------|----------|
| Policy engine | ✅ Full | `node_policy` — tenant, sensitivity, ingest, training, replication, web gates |
| Dataset manifests | ✅ Full | `node_datasets` — CAS-stored manifests, presets, provenance |
| Redaction / PII | ✅ Full | Connector classification, column-level redaction rules |

### 1.3 Distributed / Mesh

| Component | Status | Location |
|-----------|--------|----------|
| mDNS discovery | ✅ Full | `node_mesh` — LAN peer discovery |
| Relay (WAN) | ✅ Full | `node_relay` — Register, Heartbeat, Discover, relay envelopes |
| Hybrid transport | ✅ Full | `node_mesh/relay_transport` — direct TCP first, relay fallback |
| Peer consult | ✅ Full | `node_mesh/consult` — ASK forwarding to peers, TTL, context budget |
| Replication | ✅ Full | `node_repl` — gossip meta, pull segments, pull CAS, policy-gated |

### 1.4 Federated Learning

| Component | Status | Location |
|-----------|--------|----------|
| Federated coordinator | ✅ Full | `node_federated` — start_round, submit_delta, aggregate |
| Train delta events | ✅ Full | `proto/events.proto` — TRAIN_DELTA_PUBLISHED, TRAIN_DELTA_APPLIED |
| Round lifecycle | ✅ Full | FEDERATED_ROUND_STARTED, FEDERATED_ROUND_COMPLETED |

### 1.5 Entity Graph & BI (Partial)

| Component | Status | Location |
|-----------|--------|----------|
| Entity cards | ✅ Full | `entity_cards_view`, `entity_relationships_view` |
| Facts | ✅ Full | `facts_view` |
| BI intent in ask | ✅ Partial | `node_api` — entity/fact retrieval on intent match |
| Proactive insights | ✅ Minimal | `GET /insights` — overdue invoices, quotes, metrics |
| Outcome events | ✅ Partial | CASE_CONFIRMED exists; QUOTE_ACCEPTED/LOST not yet |

---

## 2. What Is Missing for Large-Scale Distributed Memory

### 2.1 Shard Model

**Gap:** No explicit shard or partition concept. Events and artifacts are implicitly global; there is no way to:

- Associate an event/artifact with a logical shard (e.g. `tenant:X`, `entity:customer`, `site:UK`)
- Advertise "I hold shard X"
- Subscribe to "I want shard X cached/queried"
- Route a question to nodes that hold relevant shards (instead of broadcasting to all peers)

**Impact:** Peer consult and replication operate on a flat model. All reachable peers are considered equal for ASK and pull; there is no targeted routing.

### 2.2 Memory Subscriptions and Content Routing

**Gap:** Nodes do not advertise what shards they host or subscribe to. Query routing is broadcast-style (all peers) rather than shard-aware.

**Impact:** Unnecessary fanout, wasted bandwidth, and latency when many peers are present. No way to "pull only shard X" or "ask only nodes holding customer data."

### 2.3 Mergeable State Layer

**Gap:** No CRDT-like mergeable state for lightweight replicated objects (tags, counters, trust, annotations, task status).

**Impact:** Entity tags, trust scores, and shared annotations must be handled as full events; no lightweight convergent merge for high-churn metadata.

### 2.4 Proactive Insight Engine

**Gap:** No scheduled analysis jobs. `GET /insights` is on-demand, rule-based. No:

- Trend change / anomaly detection
- Benchmark updates (pricing, margin, win-rate)
- Scheduled alerts (overdue, inactivity, pricing shift)

**Impact:** Intelligence is purely reactive (user asks); no background discovery of insights.

### 2.5 Outcome-Driven Learning Loop

**Gap:** CASE_CONFIRMED exists but QUOTE_ACCEPTED, QUOTE_LOST, QUOTE_REVISED, CASE_FAILED are not first-class. Router/ranking/pricing training does not consistently consume these outcomes.

**Impact:** Learning from user feedback is partial; quote and case outcomes are underused.

### 2.6 Distributed Business Intelligence

**Gap:** Ask flow queries local memory and consults peers, but:

- No explicit "source types" (local, peer, web, insight, business system) in response
- No shard-based routing to peers with relevant business data
- Missing-data warnings are implicit, not structured

**Impact:** BI answers lack provenance and structured evidence; cross-node BI is best-effort.

---

## 3. What This Implementation Will Add

| Phase | Additions | Status |
|-------|-----------|--------|
| **Phase 0** | This gap document | ✅ Done |
| **Phase 1** | Shard model: tenant/entity-type/site/artifact-class/public shards; `shards_view`, `shard_membership_view`, `shard_subscriptions_view`; subscribe, advertise, query APIs; rebuild from events | ✅ Done |
| **Phase 2** | Memory subscriptions: nodes advertise shards they host; subscribe to shards; route questions to shard-relevant peers only (no broadcast) | ✅ Done |
| **Phase 3** | CRDT-like mergeable state: mergeable state objects, event types, deterministic merge, projected views (tags, counters, trust, annotations) | ✅ Done |
| **Phase 4** | Federated learning fabric: extend training across nodes; model delta publish/aggregate; policy-gated (no raw data by default) | ✅ Done |
| **Phase 5** | Proactive insight engine: scheduled jobs (hourly/daily/weekly/monthly); INSIGHT_GENERATED, ANOMALY_DETECTED, ALERT_RAISED, BENCHMARK_UPDATED; insights_view, alerts_view, benchmarks_view | ✅ Done |
| **Phase 6** | Outcome-driven learning: QUOTE_ACCEPTED, QUOTE_LOST, QUOTE_REVISED, CASE_FAILED; feed router/ranking/pricing training | ✅ Done |
| **Phase 7** | Distributed BI: shard-aware ask flow; evidence, confidence, source types, missing-data warnings | ✅ Done |
| **Phase 8** | Documentation: distributed-memory.md, proactive-insights.md, entity-graph.md, federated-learning.md, use-cases | ✅ Done |

---

## 4. Architecture Preservation

All extensions will:

- Use Event Log as source of truth (new event types)
- Store content in CAS when applicable
- Project to SQLite views (rebuildable)
- Respect policy gates (tenant, sensitivity, shareability)
- Use dataset manifests for training provenance
- Remain pluggable for inference backends
- Preserve Windows 11 compatibility

No rewrites of existing event types, projector paths, or replication logic. Extensions are additive.
