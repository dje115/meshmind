# MeshMind Architecture Specification

## Vision

MeshMind is a local-first, CPU-friendly, peer-to-peer AI node that can store and replicate knowledge safely, collaborate with other nodes to answer questions, learn over time via memory artifacts and bounded on-device training, and optionally use web research with citations. Windows-first, then Linux/macOS.

## Core Design Principles

### P1. Hybrid Memory Engine

The storage layer has three tiers:

- **Event Log** (append-only): The immutable source of truth. Every state change is an `EventEnvelope` written to a length-prefixed protobuf log. Events form a hash chain (`prev_hash` → `event_hash`). Logs rotate into sealed segments by size threshold.
- **CAS** (Content-Addressed Store): Large content bodies stored by SHA-256 hash under `data/objects/sha256/xx/yy/<hash>`. Deduplication is inherent. Integrity verified on read.
- **SQLite** (Materialized Views): Tables (`cases_view`, `artifacts_view`, `web_briefs_view`, `peers_view`, `models_view`, `audit_view`) with FTS5 full-text search. Entirely rebuildable by replaying events + CAS. Snapshots allow fast recovery.

### P2. Scalability

- Partial peer view capped at 30 (configurable). No full mesh.
- Membership states: Alive / Suspect / Dead / Quarantined.
- Hard budgets on all distributed work: `ttl_hops`, `max_peers`, `deadline_ms`, `max_context_bytes`.

### P3. Security & Policy

- Every message and artifact carries `tenant_id` + `sensitivity`.
- `shareable` defaults to `false`.
- Default-deny replication except `tenant_id="public"`.
- Web research gated on: `allow_web` policy flag + `research_web` node capability + mandatory redaction.
- Training gated on: `allow_train` policy flag + dataset provenance tracking.

### P4. Swap-friendly Inference

- All LLM interaction goes through the `InferenceBackend` trait.
- Shipped backends: Ollama (HTTP), Mock (deterministic, for tests).
- Designed to swap in llama.cpp embedded or custom runtimes later.
- Backend has no direct DB or mesh access; prompt assembly lives in the application layer.

### P5. On-device Training

- Bounded CPU training jobs with caps on time, steps, data size, and threads.
- Evaluation gates: must beat baseline, pass regression prompts.
- Versioned model bundles stored in CAS as artifacts.
- Instant rollback to previous version.
- Trained weights default non-shareable across nodes.

## Tech Stack

| Component | Choice |
|---|---|
| Language | Rust stable |
| Async | tokio |
| HTTP / API | axum |
| Serialization | Protobuf via prost |
| SQLite | rusqlite (FTS5 enabled) |
| TLS | rustls (mTLS) |
| LAN Discovery | mDNS |
| Transport | TCP+mTLS (LAN POC); QUIC+mTLS planned |
| Logging | tracing |
| CI | GitHub Actions |
| UI | Tauri (later phase) |

## Crate Map

| Crate | Responsibility |
|---|---|
| `node_proto` | Protobuf schemas + generated types |
| `node_crypto` | Node identity, mTLS config, dev CA |
| `node_storage` | CAS + event log + SQLite views + snapshots |
| `node_policy` | Policy evaluation, redaction, permissions |
| `node_repl` | Pull-based replication (gossip + pull segments + pull objects) |
| `node_mesh` | LAN discovery, membership, peer directory, transport |
| `node_ai` | InferenceBackend trait + factory + prompt helpers |
| `node_ai_ollama` | Ollama HTTP backend |
| `node_ai_mock` | Mock backend for tests |
| `node_research` | Web research, citations, WebBrief artifacts |
| `node_trainer` | Training jobs, dataset manifests, eval gates, model registry |
| `node_api` | Axum localhost API |
| `node_app` | Wiring: config, start loops, run node |

## Protocol

All wire messages use Protobuf. See `proto/` for schemas:

- `common.proto` — Shared types (Timestamp, Sensitivity, TenantId, NodeId, HashRef, Budget)
- `events.proto` — EventEnvelope with all event types
- `cas.proto` — CAS object headers and manifests
- `snapshot.proto` — Snapshot file format
- `replication.proto` — Gossip meta, pull segments, pull CAS objects
- `mesh.proto` — Peer-to-peer envelope (Hello, Ping/Pong, Ask/Answer, Refuse)
- `research.proto` — Web research request/response
- `training.proto` — Training config, dataset manifest, result

## Replication Model

Pull-based and policy-aware:

1. Nodes gossip segment metadata and small object hashes.
2. Receiver identifies missing segments and CAS objects.
3. Pull requests include budgets.
4. Imported events have their hash chain verified.
5. Policy engine gates acceptance of remote events and objects.

## API Endpoints

Localhost-only axum server:

| Method | Path | Auth | Description |
|---|---|---|---|
| GET | `/status` | none | Node status |
| GET | `/peers` | none | Connected peers |
| GET | `/search?q=` | none | FTS search |
| POST | `/ask` | none | Ask question (retrieval + inference + peer consult) |
| POST | `/admin/event` | token | Append event |
| POST | `/admin/train` | token | Start training job |
| GET | `/admin/logs` | token | Recent audit events |

## Phases

See `docs/roadmap.md` for the ordered implementation plan.
