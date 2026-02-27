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
- [ ] 2.5 Add new SQLite views: sources_view, source_profiles_view, ingests_view, datasets_view, federated_view

## Phase 3: Snapshots + Restore
- [x] 3.1 Snapshot writer/restore (3 tests)

## Phase 4: Replication Core (in-process)
- [x] 4.1 node_repl with mock transport + policy hooks (8 tests)
- [x] 4.2 Two-node replication integration test

## Phase 5: Policy Engine
- [x] 5.1 node_policy rules + tests (22 tests)
- [ ] 5.2 Extend: ingestion approval gating per source
- [ ] 5.3 Extend: column-level redaction rules
- [ ] 5.4 Extend: training dataset policy presets

## Phase 6: Discovery + Catalog (NEW)
- [ ] 6.1 node_discovery: scan directories for SQLite/CSV/JSON sources
- [ ] 6.2 Emit DATA_SOURCE_DISCOVERED events
- [ ] 6.3 Project into sources_view

## Phase 7: Schema Inspection + Classification (NEW)
- [ ] 7.1 Connector trait + implementations: SQLiteConnector, CsvFolderConnector, JsonFolderConnector
- [ ] 7.2 Schema snapshots stored in CAS
- [ ] 7.3 PII/secrets classifier (email, phone, address, api_key, token, password patterns)
- [ ] 7.4 Emit DATA_SOURCE_CLASSIFIED events
- [ ] 7.5 Admin approval API: create SourceProfile, emit DATA_SOURCE_APPROVED

## Phase 8: Ingestion Pipelines (NEW)
- [ ] 8.1 node_ingest: run per approved SourceProfile with checkpointing
- [ ] 8.2 Normalize rows → Documents/Facts in CAS
- [ ] 8.3 Emit ARTIFACT_PUBLISHED (type=DOCUMENT) and INGEST_STARTED/COMPLETED
- [ ] 8.4 Tests with sample SQLite and CSV fixtures

## Phase 9: Dataset Manifests (NEW)
- [ ] 9.1 node_datasets builder: select content, produce DatasetManifest with provenance
- [ ] 9.2 Emit DATASET_MANIFEST_CREATED

## Phase 10: Inference Plugin + /ask (existing)
- [x] 10.1 InferenceBackend trait + factory (2 tests)
- [x] 10.2 Ollama backend (3 tests)
- [x] 10.3 Mock backend (6 tests)
- [x] 10.4 /ask with FTS retrieval + prompt assembly

## Phase 11: LAN Mesh (existing)
- [x] 11.1 node_crypto: dev CA + mTLS (7 tests)
- [x] 11.2 node_mesh: mDNS, peer directory, membership (11 tests)
- [x] 11.3 TCP+mTLS transport (3 tests)
- [x] 11.4 Peer consult with budgets (8 tests)
- [x] 11.5 Full mesh wiring in node_app (4 e2e tests)

## Phase 12: Web Research Fallback (existing)
- [x] 12.1 node_research: fetch, summarize, WebBrief (2 tests)
- [x] 12.2 Policy gating (allow_web + research_web capability)
- [ ] 12.3 Route to research_web capable peer

## Phase 13: Easy Training (Local) — Extended
- [x] 13.1 Model registry + rollback (6 tests)
- [x] 13.2 Training job queue with policy + eval gates
- [ ] 13.3 Build training dataset from DatasetManifest
- [ ] 13.4 Router/classifier training (CPU-friendly)
- [ ] 13.5 API: POST /admin/train with dataset presets

## Phase 14: Federated Training (NEW)
- [ ] 14.1 node_federated: publish model deltas, apply on aggregator
- [ ] 14.2 Track rounds in events + views
- [ ] 14.3 Integration test: 3-node federated round

## Phase 15: Tauri UI
- [ ] 15.1 Build tray UI: sources, approvals, ingest, train, ask, peers
- [ ] 15.2 Wire to localhost API

---

**Current status**: 184 tests, 0 failures, 0 clippy warnings across 18 crates.
Phases 0-1 complete (v2 spec). Phases 2-5, 10-12 carried forward from v1.
Next: Phase 6 (Discovery + Catalog).
