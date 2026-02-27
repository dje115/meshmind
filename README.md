# MeshMind

A production-grade, local-first, cross-platform distributed AI node system built in Rust. MeshMind nodes store and replicate knowledge safely, collaborate peer-to-peer to answer questions, learn over time via bounded on-device training, and optionally perform web research with citations.

---

## Table of Contents

- [Features](#features)
- [Architecture](#architecture)
- [Prerequisites](#prerequisites)
- [Installation](#installation)
  - [Windows](#windows)
  - [Linux](#linux)
  - [macOS](#macos)
- [Building](#building)
- [Running](#running)
- [Configuration](#configuration)
- [API Reference](#api-reference)
- [Testing](#testing)
- [Project Structure](#project-structure)
- [Crate Map](#crate-map)
- [Design Principles](#design-principles)
- [Protocol & Schemas](#protocol--schemas)
- [Replication Model](#replication-model)
- [Security Model](#security-model)
- [Inference Backends](#inference-backends)
- [Roadmap](#roadmap)
- [Contributing](#contributing)
- [License](#license)

---

## Features

- **Hybrid Memory Engine** — Append-only event log + content-addressed store (CAS) + SQLite materialized views with FTS5 full-text search
- **Peer-to-Peer Mesh** — mDNS LAN discovery, membership states (Alive/Suspect/Dead/Quarantined), pull-based replication
- **Policy Engine** — Tenant isolation, sensitivity levels, default-deny replication, per-artifact shareability
- **Pluggable AI Inference** — Swap backends at runtime (Ollama, mock, future: llama.cpp)
- **Peer Consult** — ASK/ANSWER forwarding across nodes with budget enforcement (TTL hops, deadlines, context limits)
- **Web Research** — Fetch, summarize, extract citations, store as WebBrief artifacts with policy gating
- **On-device Training** — Bounded CPU training jobs with eval gates, versioned models, instant rollback
- **mTLS Security** — Certificate-based node identity, dev CA for local development, mutual TLS for all peer communication
- **Cross-platform** — Windows-first, works on Linux and macOS. CI tests on both Windows and Ubuntu.

---

## Architecture

```
┌──────────────────────────────────────────────────┐
│                   node_app                        │
│         (config, bootstrap, run loops)            │
├──────────────────────────────────────────────────┤
│                   node_api                        │
│    (axum HTTP: /status /peers /search /ask)       │
├──────────┬──────────┬──────────┬─────────────────┤
│ node_ai  │node_mesh │node_repl │  node_research  │
│(inference│(discovery│(gossip + │  (web fetch +   │
│ backend) │ + peers) │  pull)   │   summarize)    │
├──────────┴──────────┴──────────┴─────────────────┤
│              node_policy  +  node_trainer         │
│       (policy gates, eval gates, models)          │
├──────────────────────────────────────────────────┤
│              node_storage                         │
│   (CAS + EventLog + SQLite + FTS5 + Snapshots)   │
├──────────────────────────────────────────────────┤
│  node_proto (protobuf)  │  node_crypto (mTLS)    │
└──────────────────────────────────────────────────┘
```

---

## Prerequisites

### Required

| Dependency | Version | Purpose |
|---|---|---|
| **Rust** | stable 1.75+ | Language toolchain |
| **Cargo** | (ships with Rust) | Build system & package manager |
| **Protocol Buffers compiler** (`protoc`) | 25.x+ | Compiles `.proto` schemas at build time |
| **C compiler** | GCC 12+ or MSVC 2022+ | Required by native dependencies (SQLite, ring) |

### Optional

| Dependency | Version | Purpose |
|---|---|---|
| **Ollama** | 0.1+ | Local LLM inference (if using `ollama` backend) |
| **Git** | 2.x+ | Version control |

### Platform-specific Notes

#### Windows

You need **one of** these C toolchains:

- **Option A (Recommended): MSYS2 + MinGW-w64**
  ```powershell
  # Install MSYS2
  winget install MSYS2.MSYS2

  # In MSYS2 UCRT64 terminal:
  pacman -S mingw-w64-x86_64-toolchain mingw-w64-x86_64-protobuf

  # Add to PATH (PowerShell):
  $env:PATH = "C:\msys64\mingw64\bin;C:\msys64\usr\bin;$env:PATH"
  $env:CC = "C:\msys64\mingw64\bin\gcc.exe"
  $env:PROTOC = "C:\msys64\mingw64\bin\protoc.exe"
  ```
  Then create `.cargo/config.toml` in the project root:
  ```toml
  [build]
  target = "x86_64-pc-windows-gnu"

  [target.x86_64-pc-windows-gnu]
  linker = "C:\\msys64\\mingw64\\bin\\gcc.exe"
  ```

- **Option B: Visual Studio Build Tools**
  ```powershell
  # Install via Visual Studio Installer
  # Required workloads:
  #   - "Desktop development with C++"
  #   - Windows SDK (e.g., Windows 11 SDK 22621)
  # Then install protoc separately:
  choco install protoc
  # OR download from https://github.com/protocolbuffers/protobuf/releases
  ```

#### Linux (Ubuntu/Debian)

```bash
# Install build essentials and protoc
sudo apt update
sudo apt install -y build-essential protobuf-compiler pkg-config

# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env
```

#### Linux (Fedora/RHEL)

```bash
sudo dnf install -y gcc gcc-c++ protobuf-compiler protobuf-devel
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env
```

#### macOS

```bash
# Install Xcode command line tools (provides clang)
xcode-select --install

# Install protoc via Homebrew
brew install protobuf

# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env
```

---

## Installation

```bash
# Clone the repository
git clone https://github.com/dje115/meshmind.git
cd meshmind

# Build all crates
cargo build --workspace

# Run all tests to verify installation
cargo test --workspace
```

---

## Building

```bash
# Debug build (fast compilation)
cargo build --workspace

# Release build (optimized)
cargo build --workspace --release

# Build just the main binary
cargo build -p node_app --release
```

The compiled binary will be at `target/release/meshmind` (or `meshmind.exe` on Windows).

---

## Running

### Quick Start with Mock Backend

```bash
# Run with default settings (mock AI backend, localhost:9900)
cargo run -p node_app

# Or with the compiled binary
./target/release/meshmind
```

### With Ollama (local LLM)

1. Install and run [Ollama](https://ollama.ai)
2. Pull a model: `ollama pull llama3.2:3b`
3. Create `meshmind.toml`:

```toml
data_dir = "./data"
listen = "127.0.0.1:9900"
backend = "ollama"
ollama_endpoint = "http://localhost:11434"
ollama_model = "llama3.2:3b"
```

4. Run: `cargo run -p node_app`

### Verify It's Running

```bash
# Check node status
curl http://localhost:9900/status

# Ask a question
curl -X POST http://localhost:9900/ask \
  -H "Content-Type: application/json" \
  -d '{"question": "What is DNS?", "max_tokens": 256}'

# Search the knowledge base
curl "http://localhost:9900/search?q=certificate"

# View connected peers
curl http://localhost:9900/peers
```

---

## Configuration

MeshMind reads from `meshmind.toml` in the working directory. All fields are optional with sensible defaults.

```toml
# Directory for event log, CAS objects, SQLite database, and snapshots
data_dir = "./data"

# HTTP API listen address (localhost only for security)
listen = "127.0.0.1:9900"

# Inference backend: "mock" or "ollama"
backend = "mock"

# Ollama settings (only used when backend = "ollama")
ollama_endpoint = "http://localhost:11434"
ollama_model = "llama3.2:3b"

# Admin token for /admin/* endpoints (auto-generated if not set)
admin_token = "your-secret-token-here"
```

### Default Values

| Key | Default | Description |
|---|---|---|
| `data_dir` | `./data` | Local data storage directory |
| `listen` | `127.0.0.1:9900` | API listen address |
| `backend` | `mock` | AI inference backend |
| `ollama_endpoint` | `http://localhost:11434` | Ollama API URL |
| `ollama_model` | `llama3.2:3b` | Ollama model name |
| `admin_token` | (random UUID) | Token for admin endpoints |

---

## API Reference

All endpoints are localhost-only.

### Public Endpoints

#### `GET /status`
Returns node status including event count, peer count, and backend info.

```json
{
  "node_id": "node-a1b2c3d4",
  "status": "running",
  "event_count": 42,
  "peer_count": 3,
  "backend": "ollama"
}
```

#### `GET /peers`
Returns list of known peers and their membership state.

```json
[
  {
    "node_id": "node-e5f6g7h8",
    "address": "192.168.1.10",
    "port": 9000,
    "state": "Alive",
    "capabilities": ["inference"],
    "rtt_ms": 12
  }
]
```

#### `GET /search?q=<query>&limit=<n>`
Full-text search across the knowledge base using SQLite FTS5.

| Parameter | Required | Default | Description |
|---|---|---|---|
| `q` | yes | — | Search query |
| `limit` | no | 20 | Max results |

#### `POST /ask`
Ask a question. The node retrieves relevant context from FTS5, assembles a prompt, and calls the inference backend.

**Request:**
```json
{
  "question": "How do I fix a DNS timeout?",
  "max_tokens": 512
}
```

**Response:**
```json
{
  "answer": "DNS timeouts can be caused by...",
  "confidence": 0.7,
  "model": "llama3.2:3b",
  "context_used": ["case-001", "case-007"]
}
```

### Admin Endpoints

#### `POST /admin/event`
Append a new event to the event log and project it to SQLite views.

**Request:**
```json
{
  "event_id": "evt-001",
  "event_type": "case_created",
  "title": "DNS Timeout Issue",
  "summary": "Customer reported intermittent DNS timeouts...",
  "tenant_id": "public",
  "tags": ["dns", "networking"]
}
```

#### `GET /admin/logs?n=<count>`
Fetch recent audit log entries.

---

## Testing

```bash
# Run all tests (146 tests across 13 crates)
cargo test --workspace

# Run tests for a specific crate
cargo test -p node_storage
cargo test -p node_repl
cargo test -p node_mesh

# Run with output
cargo test --workspace -- --nocapture

# Check formatting
cargo fmt --all -- --check

# Run clippy lints
cargo clippy --workspace --all-targets -- -D warnings
```

### Test Coverage by Crate

| Crate | Tests | Coverage |
|---|---|---|
| `node_proto` | 24 | Protobuf roundtrip serialization, enum coverage |
| `node_storage` | 39 | CAS put/get/dedup, event log append/replay/chain, projector, FTS search, snapshots |
| `node_policy` | 22 | All policy rules: tenant, sensitivity, share, web, train |
| `node_mesh` | 19 | Membership states, peer directory, transport mock, peer consult |
| `node_repl` | 8 | Gossip, segment pull, CAS pull, full A→B replication, policy gates |
| `node_crypto` | 7 | CA generation, node certs, mTLS server/client config |
| `node_ai` | 2 | Default config/request validation |
| `node_ai_mock` | 6 | Health check, deterministic responses |
| `node_ai_ollama` | 3 | Backend config, timeout handling |
| `node_api` | 6 | All HTTP endpoints via tower::ServiceExt |
| `node_trainer` | 6 | Model registry, versioning, rollback, eval gates |
| `node_research` | 2 | Source extraction, policy gating |
| **Total** | **146** | **0 failures, 0 clippy warnings** |

---

## Project Structure

```
meshmind/
├── .cargo/
│   └── config.toml          # Cargo build configuration
├── .github/
│   └── workflows/
│       └── ci.yml            # GitHub Actions CI (Windows + Ubuntu)
├── crates/
│   ├── node_proto/           # Protobuf types (prost-build)
│   ├── node_crypto/          # mTLS, dev CA, node identity
│   ├── node_storage/         # CAS, EventLog, SQLite, FTS5, Snapshots
│   │   └── src/
│   │       ├── cas.rs        # Content-addressed store
│   │       ├── event_log.rs  # Append-only event log with hash chain
│   │       ├── sqlite_views.rs # Schema definition
│   │       ├── projector.rs  # Event → SQLite materialization
│   │       ├── search.rs     # FTS5 full-text search
│   │       └── snapshot.rs   # Snapshot create/restore
│   ├── node_policy/          # Policy evaluation engine
│   ├── node_repl/            # Pull-based replication
│   ├── node_mesh/            # Peer discovery, membership, transport
│   │   └── src/
│   │       ├── membership.rs # Alive/Suspect/Dead/Quarantined
│   │       ├── peer_dir.rs   # Capped peer directory with eviction
│   │       ├── transport.rs  # Transport trait + mock
│   │       └── consult.rs    # ASK/ANSWER peer forwarding
│   ├── node_ai/              # InferenceBackend trait
│   ├── node_ai_ollama/       # Ollama HTTP client
│   ├── node_ai_mock/         # Deterministic mock backend
│   ├── node_research/        # Web research + WebBrief
│   ├── node_trainer/         # Training jobs, model registry
│   ├── node_api/             # Axum HTTP API
│   └── node_app/             # Main binary entrypoint
├── docs/
│   ├── spec.md               # Full architecture specification
│   ├── protocol.md           # Wire protocol documentation
│   ├── storage.md            # Storage engine details
│   ├── replication.md        # Replication model
│   ├── security.md           # Security model
│   ├── mesh.md               # Mesh networking
│   ├── inference.md          # Inference backend design
│   ├── research.md           # Web research system
│   ├── training.md           # Training system
│   └── roadmap.md            # Implementation progress
├── proto/
│   ├── common.proto          # Shared types
│   ├── cas.proto             # CAS object headers
│   ├── events.proto          # Event envelope + all event types
│   ├── snapshot.proto        # Snapshot format
│   ├── replication.proto     # Gossip + pull protocol
│   ├── mesh.proto            # Peer envelope (Hello, Ask, Answer)
│   ├── research.proto        # Web research messages
│   └── training.proto        # Training messages
├── Cargo.toml                # Workspace manifest
├── .gitignore
├── meshmind.toml              # Node configuration (user-created)
└── README.md
```

---

## Crate Map

| Crate | Dependencies | Purpose |
|---|---|---|
| `node_proto` | prost | Protobuf schema compilation and generated Rust types |
| `node_crypto` | rustls, rcgen, sha2 | Dev CA, node certificates, mTLS ServerConfig/ClientConfig |
| `node_storage` | rusqlite, sha2 | CAS blob store, append-only event log, SQLite views, FTS5 search, snapshots |
| `node_policy` | node_proto | Tenant/sensitivity/share policy evaluation, web/train gates |
| `node_repl` | node_storage, node_policy | Gossip metadata, segment/CAS pulling, policy-gated import |
| `node_mesh` | node_proto, node_crypto | mDNS discovery, membership FSM, peer directory, transport trait, peer consult |
| `node_ai` | async-trait | `InferenceBackend` trait definition |
| `node_ai_ollama` | reqwest, node_ai | Ollama HTTP API client |
| `node_ai_mock` | node_ai | Deterministic mock for testing |
| `node_research` | node_ai, node_storage, reqwest | Web fetch, AI summarization, WebBrief events |
| `node_trainer` | node_policy | Model registry, training jobs, eval gates, rollback |
| `node_api` | axum, node_storage, node_ai, node_mesh | HTTP API endpoints |
| `node_app` | all crates | Binary: config loading, storage init, server startup |

---

## Design Principles

### 1. Hybrid Memory Engine
- **Event Log**: Immutable, append-only. Hash-chained events (`prev_hash` → `event_hash`). Rotates into sealed segments.
- **CAS**: Content-addressed by SHA-256. Deduplication is inherent. Integrity checked on every read.
- **SQLite**: Materialized views rebuilt from events. FTS5 for full-text search. Projector applies events to views.

### 2. Scalability
- Partial peer view capped at 30 (configurable `max_peers`).
- Membership state machine: Alive → Suspect → Dead, with Quarantined for misbehaving peers.
- Hard budgets: `ttl_hops`, `deadline_ms`, `max_context_bytes`, `max_answer_bytes`.

### 3. Security & Policy
- Every artifact carries `tenant_id` + `sensitivity` (Public/Internal/Restricted).
- Default-deny replication: only `tenant_id="public"` replicates by default.
- Web research requires explicit policy flag + node capability + redaction.
- Training requires explicit policy flag + dataset provenance.

### 4. Swap-friendly Inference
- `InferenceBackend` trait with `health_check()` and `generate()`.
- No direct DB/mesh access from backends — prompt assembly lives in the API layer.
- Currently ships Ollama (HTTP) and Mock backends.

### 5. On-device Training
- Bounded CPU jobs with time/step/data caps.
- Eval gates: new model must beat baseline by configurable threshold.
- Versioned model bundles in CAS with instant rollback.
- Trained weights default to non-shareable.

---

## Protocol & Schemas

All wire messages use Protocol Buffers. See the `proto/` directory for full schemas.

| Schema | Contents |
|---|---|
| `common.proto` | Timestamp, Sensitivity, TenantId, NodeId, HashRef, Budget |
| `events.proto` | EventEnvelope with 16 event types (cases, artifacts, web briefs, peers, training, audit) |
| `cas.proto` | CAS object headers and manifests |
| `snapshot.proto` | Snapshot file format |
| `replication.proto` | GossipMeta, PullSegments, PullCasObjects |
| `mesh.proto` | Envelope with Hello, Ping/Pong, Ask/Answer, Refuse + PolicyFlags |
| `research.proto` | ResearchRequest, ResearchResponse |
| `training.proto` | TrainingConfig, DatasetManifest, TrainingResult |

---

## Replication Model

Pull-based, policy-aware replication:

1. **Gossip**: Nodes exchange `GossipMeta` listing available segments and small CAS object hashes
2. **Diff**: Receiver computes missing segments and objects
3. **Pull**: Receiver sends `PullSegmentsRequest` / `PullCasObjectsRequest` with budgets
4. **Verify**: Imported events have their hash chain verified
5. **Gate**: Policy engine accepts/denies each event and object based on tenant, sensitivity, and shareability

---

## Security Model

- **Node Identity**: Each node has a unique ID derived from its TLS certificate fingerprint (SHA-256)
- **mTLS**: All peer communication uses mutual TLS with a shared dev CA
- **Tenant Isolation**: Events and artifacts are tagged with `tenant_id`; replication respects tenant boundaries
- **Sensitivity Levels**: Public, Internal, Restricted — controls what can be replicated
- **Default-Deny**: Nothing replicates unless explicitly allowed by policy
- **Audit Trail**: All state changes logged in the append-only event log

---

## Inference Backends

### Mock Backend
Deterministic responses for testing. No external dependencies.

### Ollama Backend
Connects to a local [Ollama](https://ollama.ai) instance via HTTP API.

```toml
backend = "ollama"
ollama_endpoint = "http://localhost:11434"
ollama_model = "llama3.2:3b"
```

Supported Ollama models: any model available via `ollama pull`, e.g.:
- `llama3.2:3b` (recommended for CPU)
- `mistral:7b`
- `phi3:mini`
- `gemma2:2b`

---

## Roadmap

See [docs/roadmap.md](docs/roadmap.md) for detailed progress.

| Phase | Status | Description |
|---|---|---|
| 0. Workspace + CI | Done | Cargo workspace, GitHub Actions, documentation |
| 1. Protobuf | Done | All 8 proto schemas, prost-build, 24 roundtrip tests |
| 2. Storage | Done | CAS, EventLog, SQLite, FTS5, projector — 39 tests |
| 3. Snapshots | Done | Snapshot create/restore with event replay |
| 4. Replication | Done | Pull-based with policy gates, A→B equivalence test |
| 5. Policy | Done | Tenant/sensitivity/share/web/train gates — 22 tests |
| 6. Crypto + Mesh | Done | Dev CA, mTLS, membership, peer directory — 19 tests |
| 7. API + Wiring | Done | Axum endpoints, main binary, TOML config |
| 8. Inference | Done | Backend trait, Ollama, Mock, /ask with retrieval |
| 9. Peer Consult | Done | ASK/ANSWER forwarding with budgets |
| 10. Web Research | Done | Fetch, summarize, WebBrief, policy gating |
| 11. Training | Done | Model registry, eval gates, rollback — 6 tests |
| 12. Tauri UI | Planned | Desktop tray app |
| 13. Internet Mode | Planned | Rendezvous + relay for WAN |

---

## Contributing

1. Fork the repository
2. Create a feature branch: `git checkout -b feature/my-feature`
3. Ensure all checks pass:
   ```bash
   cargo fmt --all -- --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   ```
4. Commit with a descriptive message
5. Open a Pull Request

### Code Style
- Follow standard Rust conventions (`cargo fmt`)
- All public items must have doc comments
- No clippy warnings allowed
- Tests required for new functionality

---

## License

MIT
