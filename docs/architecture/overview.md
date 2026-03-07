# MeshMind Architecture Overview

## What Is MeshMind?

MeshMind is a **Distributed Autonomous Intelligence Platform** that learns from:

- **Documents** — PDFs, Word, markdown, runbooks, wikis
- **Databases** — SQLite, CSV, JSON
- **Business systems** — Invoices, customers, quotes, accounts
- **User outcomes** — Confirmed answers, successful flows
- **Web research** — When needed and policy allows (with citations)

It answers questions locally first, consults peers when helpful, and falls back to web research only when necessary—all policy-gated and auditable.

---

## Hybrid Memory Engine

MeshMind uses a three-tier storage architecture:

```
Event Log (source of truth)
     ↓
CAS (content-addressed storage)
     ↓
SQLite (query layer)
```

| Layer | Role |
|-------|------|
| **Event Log** | Immutable, append-only, hash-chained. Every state change is an event. |
| **CAS** | Content-addressed blobs (SHA-256). Deduplication, integrity on read. |
| **SQLite** | Rebuildable materialized views, FTS5 full-text search. |

Views are projections from events; they can be rebuilt from events + CAS alone.

---

## High-Level Flow

```
User Question
     ↓
Router (local / peers / web)
     ↓
Local Memory / DB / Peers / Web
     ↓
Evidence
     ↓
LLM (Ollama or Mock)
     ↓
Answer + Events (CaseDraft, optional CaseConfirmed)
```

---

## Key Design Principles

1. **Local-first** — Data stays on nodes; only derived artifacts (entity cards, facts, models) are exchanged when policy permits.

2. **Policy-gated** — Every ingest, share, web research, and training action is gated by policy (tenant, sensitivity, shareability).

3. **Reproducible training** — Training consumes DatasetManifest (CAS-stored, provenance-tracked), never raw sources directly.

4. **Distributed collaboration** — Peers discover via mDNS (LAN) or relay (WAN); consult flows forward questions and aggregate answers.

5. **Bounded and rollbackable** — Training has hard caps (steps, minutes, items); models are versioned with instant rollback.

---

## Component Map

```
┌──────────────────────────────────────────────────────────┐
│              Tauri Desktop UI (Vite + JS)                 │
│  Dashboard │ Ask │ Sources │ Datasets │ Models │ Peers   │
├──────────────────────────────────────────────────────────┤
│                      node_app                             │
│            (config, bootstrap, run loops)                 │
├──────────────────────────────────────────────────────────┤
│                      node_api                             │
│   (axum HTTP: status, ask, search, admin, etc.)          │
├──────────┬──────────┬──────────┬─────────────────────────┤
│ node_ai  │node_mesh │node_repl │     node_research       │
│(inference│(mDNS +   │(gossip + │     (web fetch +        │
│ backend) │ mTLS)    │  pull)   │      summarize)         │
├──────────┴──────────┴──────────┴─────────────────────────┤
│   node_policy  +  node_trainer  +  node_federated        │
├──────────────────────────────────────────────────────────┤
│                    node_storage                           │
│      (CAS + EventLog + SQLite + FTS5 + Snapshots)        │
├──────────────────────────────────────────────────────────┤
│ node_discovery → node_connectors → node_ingest           │
│                                      → node_datasets     │
└──────────────────────────────────────────────────────────┘
```

---

## See Also

- [Entity Graph](entity-graph.md) — Entity cards, relationships
- [Storage](storage.md) — Disk layout, integrity, rebuild
- [Event Log](event-log.md) — Event types, hash chain
- [CAS](cas.md) — Content-addressed storage
- [Replication](replication.md) — Pull-based replication
- [Mesh Network](mesh-network.md) — Discovery, transport
- [Security](security.md) — mTLS, policy, redaction
