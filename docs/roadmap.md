# MeshMind Roadmap

## Phase 0: Workspace + CI + Docs
- [x] 0.1 Create workspace skeleton and crates
- [x] 0.2 Add CI pipeline (fmt, clippy, test)
- [x] 0.3 Write docs/spec.md summarizing architecture

## Phase 1: Protobuf Foundation
- [x] 1.1 Add all proto files
- [x] 1.2 Implement node_proto with prost-build
- [x] 1.3 Add serialization tests (24 tests)

## Phase 2: Storage Engine
- [x] 2.1 Implement CAS + tests (9 tests)
- [x] 2.2 Implement EventLog + tests (6 tests)
- [x] 2.3 Implement SQLite schema + projector + tests (11 tests)
- [x] 2.4 Implement FTS search + tests (8 tests)

## Phase 3: Snapshots + Rebuild
- [x] 3.1 Snapshot writer/restore + tests (3 tests)

## Phase 4: Replication (in-process simulation)
- [x] 4.1 node_repl with mock transport + policy hooks (8 tests)
- [x] 4.2 Tests: replicate A→B, verify state equivalence
- [x] 4.3 Policy gates on import (tenant, sensitivity)

## Phase 5: Policy Engine
- [x] 5.1 Implement node_policy rules + tests (22 tests)
- [x] 5.2 Enforce in replication + web/train

## Phase 6: Crypto + LAN Mesh
- [x] 6.1 node_crypto: dev CA, node certs, mTLS config (7 tests)
- [x] 6.2 node_mesh: membership states, peer directory (11 tests)
- [x] 6.3 Mock transport abstraction for testing

## Phase 7: Node API + Wiring
- [x] 7.1 Axum API: /status, /peers, /search, /ask, /admin/* (6 tests)
- [x] 7.2 node_app: main binary with config + bootstrap

## Phase 8: Inference Plugin + Ollama + Mock
- [x] 8.1 Implement InferenceBackend trait + factory (2 tests)
- [x] 8.2 Implement Ollama backend (HTTP) (3 tests)
- [x] 8.3 Implement Mock backend (tests) (6 tests)
- [x] 8.4 /ask path with FTS retrieval + prompt assembly

## Phase 9: Peer Consult Reasoning
- [x] 9.1 ASK/ANSWER/REFUSE envelope building (8 tests)
- [x] 9.2 Budget enforcement (ttl_hops, deadline_ms, max_context_bytes)
- [x] 9.3 consult_peers() with mock transport + confidence merge

## Phase 10: Web Research
- [x] 10.1 node_research: fetch, summarize, store WebBrief (2 tests)
- [x] 10.2 Policy gating on web research
- [ ] 10.3 Route web queries to research_web capable node

## Phase 11: Tiny Trainer
- [x] 11.1 Model registry with versioning + rollback (6 tests)
- [x] 11.2 Training job queue with policy + eval gates
- [x] 11.3 Dataset manifests + bounded CPU jobs

## Wire-up: Full Mesh Integration
- [x] TCP+mTLS transport wired into node_app main binary
- [x] mDNS discovery started on boot, auto-populates PeerDirectory
- [x] Envelope handler: PING/PONG, HELLO, ASK/ANSWER/REFUSE
- [x] /ask endpoint consults peers when local confidence is low
- [x] Periodic replication loop pings peers + gossip handshake
- [x] End-to-end integration tests: 4 tests (ping, ask-peer, status-with-mesh, bidirectional)
- [x] Node identity derived from mTLS certificate fingerprint

## Phase 12: Tauri UI
- [ ] 12.1 Build tray UI + connect to API

## Phase 13: Internet Mode (optional)
- [ ] 13.1 Rendezvous + relay binaries
- [ ] 13.2 Update mesh for internet mode

---

**Current status**: 159 tests, 0 failures, 0 clippy warnings across 13 crates.
All core phases (0-11) plus full mesh wiring implemented. Remaining: Tauri UI (Phase 12), Internet Mode (Phase 13).
