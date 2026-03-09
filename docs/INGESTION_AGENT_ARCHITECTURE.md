# Ingestion Agent Architecture

This document describes the current ingestion flow, what will move to separate ingestion agents, what stays in MeshMind core, and the proposed local-only architecture with full source provenance.

---

## 1. Current Ingestion Flow

### End-to-End

```
┌─────────────┐   ┌─────────────┐   ┌─────────────┐   ┌─────────────┐   ┌─────────────┐
│  Discover   │ → │  Classify   │ → │  Approve    │ → │  Ingest     │ → │  Learn      │
│ (scan dirs) │   │ (PII/secret)│   │ (profile)   │   │ (connector) │   │ (training)  │
└─────────────┘   └─────────────┘   └─────────────┘   └─────────────┘   └─────────────┘
      │                   │                 │                 │                 │
 node_discovery     node_connectors    Admin API        node_ingest      node_trainer
 EventLog            EventLog          EventLog         EventLog          (simulated)
 sources_view        sources_view      sources_view     CAS, projector   datasets
```

### Ingest Trigger

- **Admin API**: `POST /admin/ingest` with `{ source_id }`
- Handler loads source from `sources_view` (connector_type, path_or_uri, status)
- Picks connector (DocumentConnector, CsvFolderConnector, SQLiteConnector, etc.) and calls `node_ingest::run_ingest`

### Ingest Execution (node_ingest)

1. Emit `IngestStarted` event
2. Loop over tables → `connector.ingest_batch(path, table, offset, limit)`
3. For each row:
   - Put content JSON in CAS
   - Emit `ArtifactPublished` (with source_ref, document_subtype, entity_type, entity_key)
   - Apply projector (artifacts_view, documents_view, document_chunks_view, etc.)
   - For document chunks: run entity extraction (candidates → classify → ExtractedEntityRecorded)
4. Emit `IngestCompleted`
5. If DocumentConnector: persist `file_results` to `ingest_file_results` table

### Connector Contract

```rust
trait Connector {
    fn id(&self) -> &str;
    fn inspect_schema(&self, path: &Path) -> Result<Vec<TableInfo>>;
    fn ingest_batch(&self, path: &Path, table: &str, offset: u64, limit: u64) -> Result<IngestBatchResult>;
}
```

`IngestBatchResult`: `table_name`, `rows: Vec<IngestRow>`, `offset`, `file_results` (optional, DocumentConnector only)

---

## 2. Current node_connectors Responsibilities

| Connector | inspect_schema | ingest_batch |
|-----------|----------------|--------------|
| SQLiteConnector | List tables, columns | SELECT * LIMIT/OFFSET |
| CsvFolderConnector | List CSV files as tables | Read CSV rows |
| JsonFolderConnector | List JSON files | Parse JSON, emit rows |
| ImageConnector | List image files | Extract EXIF metadata |
| **DocumentConnector** | Walk folder, count doc files | **Walk folder, extract text, chunk, produce IngestRow** |
| OneDriveConnector | OAuth, list folders | Download files, extract |

### DocumentConnector (what should move to agent)

- **Folder walking**: `WalkDir::new(path)` → collects files, filters by extension
- **File reading**: `extract_text_from_file(path)` for PDF, DOCX, DOC, XLS, XLSX, PPTX, PPT, TXT, MD, RTF
- **Extraction**: pdf_oxide (text), pdftoppm + tesseract (OCR for scanned PDFs), docx-rust, undoc, litchi, rtf-parser
- **Chunking**: `chunk_text(text, CHUNK_SIZE, CHUNK_OVERLAP)`
- **Output**: `IngestRow` with columns: document_id, chunk_index, chunk_text, source_file, page_number, filename, file_path, file_type, file_size_bytes, content_text, ocr_used
- **file_results**: Per-file status (Ingested, SkippedUnsupported, FailedExtraction, FailedOcr, FailedUnknown)

All of this is **file-system–specific, extraction-specific** logic that belongs in a local agent, not in core storage/projection.

---

## 3. What Should Move Out of Core

| Responsibility | Current Location | Target |
|----------------|------------------|--------|
| Folder walking | node_connectors DocumentConnector | Ingestion agent |
| File watching (new/changed/deleted) | Not implemented | Ingestion agent |
| Text extraction (PDF, Office, etc.) | node_connectors | Ingestion agent |
| OCR (pdftoppm, tesseract) | node_connectors/pdf_ocr | Ingestion agent |
| Chunking | node_connectors | Ingestion agent |
| Content hashing / change detection | Not implemented | Ingestion agent |
| Extraction retries | Not implemented | Ingestion agent |
| Per-file status before send | node_connectors file_results | Ingestion agent |
| Source provenance collection | Minimal (source_ref, source_file) | Ingestion agent |

---

## 4. What Should Remain in Core

| Responsibility | Location | Notes |
|----------------|----------|-------|
| CAS write | node_storage | Content-addressable storage |
| Event creation (ArtifactPublished, etc.) | node_ingest | Event Log is source of truth |
| Projections (artifacts_view, documents_view, etc.) | node_storage/projector | Rebuildable from events |
| Entity extraction from chunks | node_extraction, node_ingest | Post-ingest; can stay or be pluggable |
| Planner / Ask pipeline | node_ask | Uses projections |
| Training orchestration | node_trainer | Uses datasets |
| Policies (ingest/share/web/train) | node_policy | Gating |
| Debug UI | node_api, UI | Visibility |
| Discovery (scan for sources) | node_discovery | Can stay or become agent-driven |
| Source approval | Admin API | Core gates what may be ingested |

---

## 5. Responsibility Boundaries (Formal)

### A. MeshMind Core Responsibilities

- CAS (content-addressed storage)
- Event log (source of truth)
- SQLite projections (rebuildable from events)
- Entity graph and relationships
- Planner / Ask pipeline
- Training orchestration
- Policy engine (ingest/share/web/train)
- Answer provenance assembly
- UI / debug / status display

### B. Ingestion Agent Responsibilities

- Watch sources (folders, endpoints)
- Detect changes (new, modified, deleted)
- Read files / fetch records
- OCR (local, no cloud)
- Use ingestion-time LLM helpers where configured
- Normalize to IngestedItem contract
- Publish normalized items to core
- Report status / errors / progress via API or SSE

### C. Main App Responsibilities

- Store ingestion configuration (source watches)
- Start / stop / pause / resume agents
- Set rate limits and concurrency
- Surface live status in UI
- Show ingest debug information (jobs, items, provenance)
- Allow source configuration changes

---

## 6. Proposed Agent Boundaries

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                        MeshMind Core (Rust)                                  │
│  Event Log │ CAS │ Projector │ Ask │ Planner │ Policies │ Training │ Debug  │
└─────────────────────────────────────────────────────────────────────────────┘
                                      ▲
                                      │ localhost HTTP/JSON
                                      │ POST /v1/ingest/items (normalized IngestedItem)
                                      │ POST /v1/ingest/items/batch
                                      │ POST /v1/ingest/jobs
                                      │
┌─────────────────────────────────────────────────────────────────────────────┐
│                 Ingestion Agents (separate processes)                        │
│  ┌─────────────────────┐  ┌─────────────────┐  ┌─────────────────┐         │
│  │ Filesystem Agent    │  │ Xero Agent      │  │ Outlook Agent   │  ...     │
│  │ (Python)            │  │ (future)        │  │ (future)        │         │
│  │ - watch folders     │  │ - list invoices │  │ - list mail     │         │
│  │ - extract text      │  │ - normalize     │  │ - normalize     │         │
│  │ - OCR               │  │ - provenance    │  │ - provenance    │         │
│  │ - chunk             │  │                 │  │                 │         │
│  │ - normalize         │  │                 │  │                 │         │
│  │ - provenance        │  │                 │  │                 │         │
│  └─────────────────────┘  └─────────────────┘  └─────────────────┘         │
└─────────────────────────────────────────────────────────────────────────────┘
```

Each agent:

- Discovers items from its source (files, API records, etc.)
- Extracts content locally (no cloud)
- Normalizes into the shared `IngestedItem` contract
- Sends items to MeshMind core via HTTP/JSON
- Does NOT own Event Log, CAS, projections, or training

---

## 6. Local Communication Model

### Primary: HTTP/JSON

- **Why**: Language-neutral, transport-neutral, easy to test, firewall-friendly for localhost
- **Endpoints** (implemented):
  - `POST /v1/ingest/jobs` — Start ingest job
  - `POST /v1/ingest/items` — Submit single normalized item
  - `POST /v1/ingest/items/batch` — Submit batch (used by agent)
  - `GET /v1/ingest/status` — Job status
  - `GET /v1/ingest/stream` — SSE stream for progress
  - `GET /v1/ingest/sources` — Watched sources
  - `GET /v1/ingest/results` — Per-item results
  - `GET /v1/ingest/agent/config` — Agent sources (admin)

### Secondary: SSE (or WebSocket)

- **Purpose**: Live progress/status streaming for UI only
- **Not** the main ingest contract; agents send normalized items via POST
- SSE endpoint for: current job, file being processed, counts, warnings
- WebSocket acceptable only if UI already uses it for live updates

### Constraints

- localhost only (127.0.0.1, ::1)
- Admin-protected
- No cloud services; all extraction and OCR local

---

## 8. Why HTTP/JSON Primary, SSE/WebSocket Secondary

1. **Durability**: POST is request/response; retries are straightforward.
2. **Language-agnostic**: Python, Rust, Node agents can all use HTTP.
3. **Debuggability**: Logs and proxies can inspect HTTP traffic.
4. **Simplicity**: No connection state for the main data path.
5. **Streaming**: SSE/WS are for human visibility, not for bulk data transfer.

---

## 8. Future Business-System Agents

Same pattern:

- Xero: list invoices → normalize → POST items
- ITQuoter: list quotes → normalize → POST items
- Sage: list records → normalize → POST items
- Outlook: list mail → normalize → POST items
- OneDrive: list files → download locally → extract → POST items

Each emits the same `IngestedItem` shape with:

- `source_type`: filesystem, xero, itquoter, sage, outlook, onedrive, etc.
- `source_locator`: machine-readable reference (path, id, key)
- `source_open_target`: how to open original (file://, outlook://, xero://, etc.)
- `source_origin_label`: human-readable origin

### SourceAgent abstraction (future)

Future agents can implement a common pattern:

1. **Config**: Fetch from `GET /v1/ingest/agent/config` (main app returns `agent_sources` per agent type)
2. **Discover**: List items from source (API, folder, mailbox, etc.)
3. **Extract**: Read content locally; optionally use ingestion-time LLM
4. **Normalize**: Produce `IngestedItem[]` with full provenance
5. **Publish**: `POST /v1/ingest/items/batch`
6. **Watch**: Poll or subscribe for changes; repeat discover→extract→normalize→publish

The main app config may extend `[[agent_sources]]` with `agent_type` (e.g. `filesystem`, `xero`) so each agent only receives its relevant sources.

---

## 10. Source Provenance Storage and Exposure

### Current State

- `ArtifactPublished.source_ref` = source_id
- `entity_attributes_json` includes `source_file`, `filename`, `file_path`
- `ingest_file_results` has per-file status (filename, file_path, status, etc.)
- Evidence items have `source_id` but limited open-target information

### Target State

Every ingested item and evidence item must retain:

| Field | Purpose |
|-------|---------|
| source_type | filesystem, outlook, xero, etc. |
| source_id | MeshMind source identifier (from approved source) |
| source_locator | Machine-readable reference (path, id, external key) |
| source_open_target | URI or deep-link to open original (file://, outlook://, etc.) |
| source_origin_label | Human-readable label (e.g. "C:\Quotes\quote.docx") |
| source_parent | Optional grouping (watched folder, mailbox, tenant) |

### Storage

- **In events**: Extend `ArtifactPublished` or store provenance in `entity_attributes_json` / a new field
- **In projections**: `documents_view`, `document_chunks_view`, new `source_items_view` or `evidence_sources_view`
- **In API**: `GET /v1/evidence/:id/source`, `GET /v1/source-items/:id` — return provenance so UI can show "Open original" / "Where did this come from?"

### User Questions

- "Where did this come from?" → provenance in evidence
- "Show me that document" → source_open_target
- "Open the original file" → file:// or app-specific URI

---

---

## 11. Why Ingestion-Time LLM Helpers Belong in the Agent

Ingestion-time LLM helpers are used only for *upstream extraction and classification*, not for answering questions:

| Agent-side (allowed) | Core-side (not allowed in ingest) |
|----------------------|-----------------------------------|
| Document type classification | General Q&A / Ask planning |
| Entity classification/disambiguation | Answer generation |
| Relationship extraction assistance | Evidence ranking |
| OCR cleanup / post-OCR structuring | Peer consult decisions |
| Metadata enrichment | Training data selection |
| Ambiguity resolution (e.g. "Acme Corp" vs "Acme Corporation") | — |

**Reasons:**

1. **Separation of concerns**: Core is reasoning/memory. Ingestion is extraction/normalization.
2. **Different failure modes**: Agent LLM fails → item is skipped or flagged; core LLM fails → answer quality degrades.
3. **Configurability**: Agents can enable/disable ingestion-time LLM per source; core Ask flow stays simple.
4. **Provenance**: `llm_helper_used` and `llm_helper_steps` are recorded per item; debug can show exactly what the agent did.
5. **Local-only**: Both agent and core use local inference; no cloud.

---

## 12. Why Configuration Belongs in the Main App

| Owner | Responsibility |
|-------|----------------|
| **Main app** | Source watches (folders, endpoints), include/exclude, recursion, OCR on/off, LLM helper on/off, rate limits, concurrency, retry limits, polling interval, agent enabled/disabled, agent startup mode |
| **Agent** | Executes the config; does not invent or override source lists |

**Reasons:**

1. **Single source of truth**: User configures ingestion in one place (UI or meshmind.toml); agent fetches config.
2. **Service lifecycle**: Main app starts/stops agents; it must know what they should do.
3. **Policy gating**: Main app applies policies; config can reflect approval state.
4. **Multi-agent consistency**: Future Xero, Outlook, etc. agents all receive config from main app.
5. **Auditability**: Changes to watched sources are visible in main app logs/config.

---

## 12. Why the Main App Controls Service Lifecycle

- **Start**: Main app spawns the ingestion agent process when enabled.
- **Stop**: Main app terminates the agent on shutdown or user request.
- **Restart**: Main app can restart the agent after config changes or on failure.
- **Pause/Resume**: Main app can signal the agent to pause (e.g. rate limiting).
- **Monitor**: Main app polls agent health and displays status in the UI.

The agent is a **child process** or **supervised service** of the main app, not a standalone daemon that runs independently.

---

## 14. Current Python Agent Path

The filesystem ingestion agent (`agents/filesystem_ingestion_agent/`):

- **One-shot**: `python main.py --one-shot /path/to/folder [--source-id src-1]`
- **Config-from-API**: `--config-from-api` — fetches sources from main app, runs one-shot per source
- **Watch**: `--watch` — runs filesystem watcher, fetches config from `GET /v1/ingest/agent/config`
- **Config**: Main app owns `[[agent_sources]]` in meshmind.toml; agent fetches via config API
- **Env**: `MESHMIND_API_URL`, `MESHMIND_ADMIN_TOKEN`
- **Publish**: POSTs `IngestedItem` to `POST /v1/ingest/items/batch`

---

## 14. Current Legacy Rust Ingestion Path

- **Trigger**: `POST /admin/ingest` with `{ source_id }`
- **Flow**: `node_connectors` (DocumentConnector, etc.) → `node_ingest::run_ingest` → CAS, EventLog, projector
- **Status**: Still supported for SQLite, CSV, JSON, Document folder. Document folder extraction will be deprecated in favor of the filesystem agent.
- **Retention**: Legacy connectors remain for structured data (SQLite, CSV, JSON); document-folder extraction moves to agent.

---

## 16. Summary

| Aspect | Decision |
|--------|----------|
| Core language | Rust |
| First agent language | Python (filesystem watcher + extraction) |
| Primary communication | localhost HTTP/JSON |
| Secondary communication | SSE for progress (optional) |
| Extraction philosophy | Direct reading first, OCR locally, no cloud |
| Provenance | Mandatory; source_locator, source_open_target, source_origin_label |
| Configuration | Main app owns; agent fetches |
| Service lifecycle | Main app starts/stops/monitors agents |
| Ingestion-time LLM | Agent only; core does not use LLM for ingest |
| Cloud | Never; all processing local |
