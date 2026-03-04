# MeshMind Handover Document

**Last updated**: 2026-03-04  
**Purpose**: When you relaunch Cursor, use this document to continue work. Priority: OneDrive connector → Email connector → Training implementation.

---

## Current State

### What's Done

- **Backend**: 19 Rust crates, Axum API, SQLite + event log, mTLS mesh
- **Connectors**: SQLite, CSV, JSON, Image (EXIF), Document (PDF, DOCX, TXT, MD, RTF)
- **PDF extraction**: Switched from `pdf-extract` to `pdf_oxide` (more robust, 100% pass rate, no panics)
- **Windows build**: `rust-toolchain.toml` pins `stable-x86_64-pc-windows-gnu`; use `--target x86_64-pc-windows-gnu` and MinGW in PATH (`C:\msys64\mingw64\bin`)
- **CI**: GitHub Actions on ubuntu, macos, windows (with MSYS2 MinGW); E2E Playwright job
- **Scripts**: `scripts/build-windows.ps1`, `scripts/verify-e2e-flow.ps1`
- **Docs**: `docs/SETUP_WINDOWS.md`, `docs/MANUAL_TESTING.md`

### Build & Test (Windows)

```powershell
$env:PATH = "C:\msys64\mingw64\bin;" + $env:PATH
cargo test --workspace --target x86_64-pc-windows-gnu
# Or: .\scripts\build-windows.ps1
```

### Run App

```powershell
.\target\x86_64-pc-windows-gnu\debug\meshmind.exe
# Or: cargo run -p node_app --target x86_64-pc-windows-gnu
# Open http://127.0.0.1:9900
```

---

## Priority 1: OneDrive Connector

**Goal**: Direct OneDrive API sync (no local OneDrive client required), including shared/team files.

**Reference**: `docs/REVIEW_AND_ROADMAP.md` (OneDrive Direct Integration)

### Steps

1. **Add dependency** to `crates/node_connectors/Cargo.toml`:
   - `graph-rs-sdk` or `graph-client` (Microsoft Graph API)
   - Or lighter: `reqwest` + manual OAuth2 + Graph REST calls

2. **Implement `OneDriveConnector`** (implement `Connector` trait):
   - `inspect_schema(&self, path: &Path)` – list folders as "tables" (or use path as drive root)
   - `ingest_batch(&self, ...)` – download files via Graph API (`GET /me/drive/root/children`, `GET /me/drive/items/{id}/content`)

3. **OAuth2 flow**:
   - Config: `onedrive_client_id`, `onedrive_tenant_id`, `onedrive_refresh_token`
   - Use device code or auth code flow; store refresh token in config/secure storage

4. **Discovery integration**:
   - Add OneDrive as a source type in `node_discovery` (connector_type 9 or similar)
   - Wire in `node_api` / `connector_for_type` for the new connector

5. **Rate limiting**: Handle 429 responses with `Retry-After` header

### Key Paths

- `crates/node_connectors/src/lib.rs` – Connector trait, DocumentConnector, etc.
- `crates/node_discovery/src/lib.rs` – source types, scan logic
- `crates/node_api/src/lib.rs` – `connector_for_type`, scan/ingest handlers

---

## Priority 2: Email Connector (Outlook / Microsoft 365)

**Goal**: Ingest email from Outlook/M365 via Microsoft Graph Mail API.

**Reference**: `docs/REVIEW_AND_ROADMAP.md` (Email Integration)

### Steps

1. **Shared auth**: One OAuth2 app can cover both OneDrive and Mail – reuse Graph client/tokens

2. **Implement `EmailConnector`**:
   - `inspect_schema` – mail folders as tables (Inbox, Sent, etc.)
   - `ingest_batch` – fetch messages via `GET /me/mailFolders/{id}/messages`, extract subject, body, sender, date

3. **Privacy**:
   - Filters: date range, folder, sender
   - Redaction: PII in policy engine

4. **Attachments (optional)**:
   - Download PDF/DOCX attachments and run through DocumentConnector

### Key Paths

- Same as OneDrive for connector + discovery + API wiring

---

## Priority 3: Get Training Working

**Goal**: Replace simulated training with real on-device training (router/classifier).

**Reference**: `docs/training.md`, `crates/node_trainer/src/lib.rs`

### Current State

- `Trainer::run_job` **simulates** training (fake score 0.85)
- Model registry, eval gate, rollback all work
- **Trained models are NOT used** by Ask/search – Ask uses FTS + Ollama only

### Steps

1. **Define training target**:
   - Router: routes queries to best source (local FTS vs peer consult)
   - Classifier: e.g. document type, intent, or relevance

2. **Implement real training loop** in `node_trainer`:
   - Load dataset from manifest (CAS refs → documents)
   - Use CPU-feasible model (e.g. `linfa`, `candle` CPU, or simple sklearn-style logic in Rust)
   - Bounded: `max_steps`, `max_minutes`, `max_dataset_items`

3. **Eval gate**:
   - Regression prompts or held-out eval set
   - Only register model if score beats baseline by threshold

4. **Wire trained model into Ask flow** (optional for v1):
   - Use router model to decide: local FTS vs peer consult vs web research
   - Or use classifier to filter/rank FTS results

### Key Paths

- `crates/node_trainer/src/lib.rs` – `Trainer::run_job`, `ModelRegistry`
- `crates/node_datasets` – manifest builder, preset logic
- `crates/node_api/src/lib.rs` – `handle_ask`, peer consult logic

---

## File Map (Relevant to Handover)

| Path | Purpose |
|------|---------|
| `crates/node_connectors/src/lib.rs` | Connector implementations (add OneDrive, Email) |
| `crates/node_discovery/src/lib.rs` | Source scanning, connector types |
| `crates/node_trainer/src/lib.rs` | Training job, model registry (replace simulation) |
| `crates/node_api/src/lib.rs` | HTTP API, connector_for_type, handle_ask |
| `docs/REVIEW_AND_ROADMAP.md` | Full roadmap, OneDrive/Email spec |
| `docs/SETUP_WINDOWS.md` | Windows build instructions |
| `docs/MANUAL_TESTING.md` | Manual test flow, verification script |

---

## Quick Reference

- **Ask flow**: FTS search → top 10 hits → context bullets → Ollama prompt → answer
- **Ingestion**: Scan → Approve → Ingest (no training needed for Ask)
- **Training**: Produces versioned model bundles; currently not used by Ask
