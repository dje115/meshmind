# MeshMind Roadmap (v2 — Extended Spec)

## Phase 0: Workspace + CI + Docs
- [x] 0.1 Create workspace skeleton and crates (original 13 + 5 new)
- [x] 0.2 Add CI pipeline (fmt, clippy, test)
- [x] 0.3 Add config struct and config file loader in node_app
- [x] 0.4 Add new crate stubs: node_discovery, node_connectors, node_ingest, node_datasets, node_federated

## Phase 1: Protobuf Foundation (existing + new)
- [x] 1.1 Original proto files: common, cas, events, snapshot, replication, mesh, research, training
- [x] 1.2 Extend events.proto: 10 new EventType values + payload messages
- [x] 1.3 Create datasets.proto: DiscoveredSource, ColumnClassification, SchemaSnapshot, SourceProfile, DatasetManifest, IngestCheckpoint, IngestBatch
- [x] 1.4 Create federated.proto: FederatedRoundConfig, ModelDelta, AggregateResult, RoundParticipant
- [x] 1.5 Extend training.proto: expanded DatasetManifest, EvalGate, EvalResult
- [x] 1.6 Serialization tests for all message categories (49 tests)

## Phase 2: Hybrid Memory Engine (CAS + Event Log + SQLite views)
- [x] 2.1 CAS with dedupe + verify (9 tests)
- [x] 2.2 EventLog with segments + index + hash chain (6 tests)
- [x] 2.3 SQLite schema + projector with new event types (11 tests)
- [x] 2.4 FTS search (8 tests)
- [x] 2.5 New SQLite views: sources_view, source_profiles_view, ingests_view, datasets_view, federated_view + projector writes

## Phase 3: Snapshots + Restore
- [x] 3.1 Snapshot writer/restore (3 tests)

## Phase 4: Replication Core (in-process)
- [x] 4.1 node_repl with mock transport + policy hooks (8 tests)
- [x] 4.2 Two-node replication integration test

## Phase 5: Policy Engine
- [x] 5.1 node_policy rules + tests (22 tests)
- [x] 5.2 Ingestion approval gating per source (can_ingest_source)
- [x] 5.3 Column-level redaction rules (should_redact_column)
- [x] 5.4 Training dataset policy presets (can_build_dataset, can_share_deltas)

## Phase 6: Discovery + Catalog
- [x] 6.1 node_discovery: scan directories for SQLite/CSV/JSON sources (5 tests)
- [x] 6.2 Emit DATA_SOURCE_DISCOVERED events
- [x] 6.3 Project into sources_view

## Phase 7: Schema Inspection + Classification
- [x] 7.1 Connector trait + implementations: SQLiteConnector, CsvFolderConnector, JsonFolderConnector (9 tests)
- [x] 7.2 Schema inspection (inspect_schema) + ingest batching (ingest_batch)
- [x] 7.3 PII/secrets classifier (email, phone, address, ssn, api_key, token, password patterns)
- [x] 7.4 Admin approval API endpoint: POST /admin/sources/approve

## Phase 8: Ingestion Pipelines
- [x] 8.1 node_ingest: run per approved SourceProfile with batching + checkpointing (3 tests)
- [x] 8.2 Normalize rows → Documents in CAS (JSON serialized)
- [x] 8.3 Emit ARTIFACT_PUBLISHED (type=DOCUMENT) and INGEST_STARTED/COMPLETED events

## Phase 9: Dataset Manifests
- [x] 9.1 node_datasets builder: select content by preset, produce DatasetManifest with provenance (4 tests)
- [x] 9.2 Emit DATASET_MANIFEST_CREATED, store manifest in CAS

## Phase 10: Inference Plugin + /ask (existing)
- [x] 10.1 InferenceBackend trait + factory (2 tests)
- [x] 10.2 Ollama backend (3 tests)
- [x] 10.3 Mock backend (6 tests)
- [x] 10.4 /ask with FTS retrieval + prompt assembly + peer consult escalation

## Phase 11: LAN Mesh (existing)
- [x] 11.1 node_crypto: dev CA + mTLS (7 tests)
- [x] 11.2 node_mesh: mDNS, peer directory, membership (11 tests)
- [x] 11.3 TCP+mTLS transport (3 tests)
- [x] 11.4 Peer consult with budgets (8 tests)
- [x] 11.5 Full mesh wiring in node_app (4 e2e tests)

## Phase 12: Web Research Fallback (existing)
- [x] 12.1 node_research: fetch, summarize, WebBrief (2 tests)
- [x] 12.2 Policy gating (allow_web + research_web capability)

## Phase 13: Easy Training (Local) — Extended
- [x] 13.1 Model registry + rollback (6 tests)
- [x] 13.2 Training job queue with policy + eval gates
- [x] 13.3 Admin API: POST /admin/train with dataset presets
- [x] 13.4 Admin API: GET /admin/models, POST /admin/models/rollback
- [x] 13.5 Admin API: GET /admin/datasets

## Phase 14: Federated Training
- [x] 14.1 node_federated: FederatedCoordinator with start_round, submit_delta, aggregate (5 tests)
- [x] 14.2 Event builders: round_started, round_completed, delta_published
- [x] 14.3 Policy gate: can_share_deltas check

## Phase 15: Tauri UI
- [x] 15.1 Vite frontend: Dashboard, Ask, Sources, Datasets, Models, Peers, Audit Log
- [x] 15.2 Dark theme with modern design, status polling, toast notifications
- [x] 15.3 Train modal with dataset preset selection
- [x] 15.4 Tauri v2 backend scaffold (src-tauri)
- [x] 15.5 Wired to localhost API (http://127.0.0.1:3000)

---

**Current status**: 228 tests, 0 failures, 0 clippy warnings across 18 crates.
All phases complete (v2 spec).

**API endpoints implemented**:
- GET /status, /peers, /search, /admin/sources, /admin/models, /admin/datasets, /admin/logs
- POST /ask, /admin/event, /admin/sources/approve, /admin/train, /admin/models/rollback

**Crate summary (18 crates)**:
| Crate | Purpose | Tests |
|---|---|---|
| node_proto | Protobuf types (9 proto files) | 49 |
| node_crypto | Dev CA + mTLS helpers | 7 |
| node_storage | CAS + EventLog + SQLite + Projector | 39 |
| node_policy | Policy engine (replication, ingest, train, web) | 24 |
| node_repl | Replication protocol | 8 |
| node_mesh | TCP+mTLS transport, mDNS, peer directory, consult | 31 |
| node_ai | InferenceBackend trait | 2 |
| node_ai_ollama | Ollama HTTP backend | 3 |
| node_ai_mock | Mock backend for tests | 6 |
| node_research | Web research + WebBrief | 2 |
| node_discovery | Source scanning (SQLite/CSV/JSON) | 5 |
| node_connectors | Connector trait + SQLite/CSV/JSON impls + classifier | 9 |
| node_ingest | Ingestion pipelines | 3 |
| node_datasets | Dataset manifest builder | 4 |
| node_trainer | Model registry + training jobs | 6 |
| node_federated | Federated learning coordinator | 5 |
| node_api | Axum HTTP API (12 endpoints) | 10 |
| node_app | Main binary + mesh integration | 4 e2e |
