# Distributed Memory Fabric

## Overview

MeshMind's distributed memory fabric extends the local Event Log + CAS + SQLite architecture with **knowledge shards**—logical partitions of memory that enable targeted routing and subscription across nodes.

---

## Shard Concepts

| Shard Type | Key Format | Example |
|------------|------------|---------|
| Tenant | `tenant:{tenant_id}` | `tenant:public`, `tenant:acme` |
| Entity type | `entity_type:{type}` | `entity_type:customer`, `entity_type:invoice` |
| Artifact class | `artifact_class:{class}` | `artifact_class:document`, `artifact_class:fact` |
| Site/region | `site:{region}` | `site:UK` (reserved for future) |
| Public | `public` | Shared, policy-allowed content |

---

## Shard Assignment

Each event/artifact is associated with one or more shard keys when projected:

- **CaseCreated** → `tenant:{tenant_id}`, `public` (if shareable)
- **ArtifactPublished** → `tenant:{tenant_id}`, `entity_type:{entity_type}` (if present), `artifact_class:{document|fact|...}`, `public` (if shareable)

Assignment happens implicitly in the projector; no separate event is required.

---

## SQLite Views

| View | Purpose |
|------|---------|
| `shards_view` | Catalog of known shards (shard_key, shard_kind, created_at_ms) |
| `shard_membership_view` | Which cases/artifacts belong to which shards |
| `shard_subscriptions_view` | Which nodes subscribe to or host which shards |

---

## Events

- **SHARD_SUBSCRIPTION_ADDED** — Node subscribes to a shard. Payload: `shard_key`, `node_id`, `capability` (host|cache|query), `last_seen_ms`.

---

## API Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/v1/shards` | GET | List shards (optional `limit`) |
| `/v1/shards/for-question` | GET | Shards relevant to a question (`?q=...`) |
| `/v1/shards/:key/members` | GET | Members of a shard (optional `member_type`, `limit`) |
| `/v1/shards/:key/subscriptions` | GET | Nodes subscribed to a shard (optional `capability`) |
| `/v1/shards/subscribe` | POST | Subscribe this node to a shard (body: `{ shard_key, capability }`) |

---

## Query Routing (Phase 2)

Shard-aware routing is implemented. When a question is asked (POST /ask):

1. `shards::peers_for_question(conn, question)` computes which nodes host shards relevant to the question (based on `shards_for_question` + `nodes_for_shard`).
2. If any such peers exist, only those peers are consulted via `consult_peers_routed`; otherwise behavior falls back to broadcast (all reachable peers).

This reduces fanout: peers without matching shard subscriptions are not contacted.

---

## Mergeable State (Phase 3)

CRDT-like lightweight state for tags, counters, and annotations. Events are mergeable across nodes for convergent replication.

### Event Types

| Event | Payload | Merge semantics |
|-------|---------|-----------------|
| MERGEABLE_TAG_UPDATED | object_type, object_id, tag, op (add\|remove) | 2P-Set (add/remove sets) |
| MERGEABLE_COUNTER_UPDATED | object_type, object_id, counter_key, delta | PN-Counter (sum deltas) |
| MERGEABLE_ANNOTATION_UPDATED | object_type, object_id, annotation_key, value | LWW (last write wins) |

### SQLite Tables

| Table | Purpose |
|-------|---------|
| mergeable_tag_events | Tag add/remove events (event_id, object_type, object_id, tag, op, node_id, ts_ms) |
| mergeable_counter_deltas | Counter deltas per node (event_id, object_type, object_id, counter_key, node_id, delta, ts_ms) |
| mergeable_annotations_view | Current annotation values (object_type, object_id, annotation_key, value, node_id, ts_ms) |

### API Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/v1/mergeable/:object_type/:object_id/tags` | GET | Current tags for object |
| `/v1/mergeable/:object_type/:object_id/counters` | GET | Counter totals (counter_key → value) |
| `/v1/mergeable/:object_type/:object_id/annotations` | GET | Annotations (key → value) |

Events are created via `POST /admin/event` with the appropriate payload.

---

## Federated Learning Fabric (Phase 4)

Federated rounds enable model delta aggregation across nodes without sharing raw data. Policy-gated via `can_share_deltas()`.

### API Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/v1/admin/federated/status` | GET | Config (min/max participants, aggregation) |
| `/v1/admin/federated/rounds` | POST | Start a round (body: model_id, round_number?, min/max?) |
| `/v1/admin/federated/rounds/:round_id` | GET | Round status |
| `/v1/admin/federated/rounds/:round_id/deltas` | POST | Submit delta (body: delta_id, model_id, base_version, cas_hash, metrics, from_node) |
| `/v1/admin/federated/rounds/:round_id/aggregate` | POST | Aggregate and complete round |

### Cross-Node Flow

1. Coordinator starts a round via `POST /admin/federated/rounds`.
2. Remote peers discover coordinator via relay/peer directory and POST their delta to `POST /admin/federated/rounds/:round_id/deltas` (requires admin auth).
3. Coordinator aggregates when enough deltas received via `POST .../aggregate`.
4. Events: `FEDERATED_ROUND_STARTED`, `TRAIN_DELTA_PUBLISHED`, `FEDERATED_ROUND_COMPLETED` are appended and projected.

---

## Proactive Insight Engine (Phase 5)

Scheduled and on-demand insights, alerts, benchmarks, anomalies. Events: `INSIGHT_GENERATED`, `ANOMALY_DETECTED`, `ALERT_RAISED`, `BENCHMARK_UPDATED`. Views: `insights_view`, `alerts_view`, `benchmarks_view`, `anomalies_view`. Trigger generation via `POST /admin/insights/run` with `{ "schedule": "hourly"|"daily"|"weekly"|"monthly"|"manual" }`.

See [proactive-insights.md](proactive-insights.md).

---

## Outcome-Driven Learning (Phase 6)

First-class outcome events for quote and case lifecycle. All feed into `ThisTenantConfirmed` dataset preset for router/ranking/pricing training.

| Event | Payload | Use |
|-------|---------|-----|
| CASE_FAILED | case_id, reason | Case did not succeed |
| QUOTE_ACCEPTED | quote_id, case_id, value_summary, confidence | Quote won |
| QUOTE_LOST | quote_id, case_id, reason | Quote lost |
| QUOTE_REVISED | quote_id, case_id, revision_reason | Quote revised |

- **outcomes_view**: Unified table (outcome_id, outcome_type, case_id, quote_id, outcome_value, reason, confidence, created_at_ms).
- **POST /v1/outcomes**: Record outcome (body: `outcome_type`, `case_id`, `quote_id`, `reason`, etc.).
- **Dataset preset**: `ThisTenantConfirmed` includes CaseConfirmed, CaseFailed, QuoteAccepted, QuoteLost, QuoteRevised.

---

## Distributed BI (Phase 7)

The ask flow (POST /ask) returns structured provenance:

- **source_types**: Which sources contributed — `local`, `peer`, `web`, `insight`, `business_system`
- **evidence**: Per-item evidence with `id`, `source_type`, optional `title`
- **missing_data_warnings**: Structured warnings when data may be incomplete (e.g. no local matches, peers had no knowledge, business intent but no entity data)

Shard-aware routing (Phase 2) limits peer consult to nodes hosting relevant shards.

---

## References

- [DISTRIBUTED_MEMORY_GAPS.md](DISTRIBUTED_MEMORY_GAPS.md) — Gap analysis and roadmap
- [federated-learning.md](federated-learning.md) — Federated round lifecycle and cross-node flow
- [proactive-insights.md](proactive-insights.md) — Proactive insight engine
- [architecture/storage.md](architecture/storage.md) — SQLite schema
- [architecture/event-log.md](architecture/event-log.md) — Event types
