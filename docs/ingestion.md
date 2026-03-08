# MeshMind Ingestion

## Lifecycle: Discover → Classify → Approve → Ingest → Normalize → Learn

```
┌─────────────┐   ┌─────────────┐   ┌─────────────┐   ┌─────────────┐   ┌─────────────┐
│  Discover   │ → │  Classify   │ → │  Approve    │ → │  Ingest     │ → │  Learn      │
│ (scan dirs) │   │ (PII/secret)│   │ (profile)   │   │ (connector) │   │ (training)  │
└─────────────┘   └─────────────┘   └─────────────┘   └─────────────┘   └─────────────┘
      │                   │                 │                 │                 │
 DATA_SOURCE_         DATA_SOURCE_      DATA_SOURCE_    INGEST_STARTED/   Dataset manifest
 DISCOVERED           CLASSIFIED        APPROVED        INGEST_COMPLETED  for training
```

---

## 1. Discover

**Component**: `node_discovery`

Scans configured directories for data sources. Emits `DATA_SOURCE_DISCOVERED` for each found source. Does not ingest.

**Events**: `EventType::DataSourceDiscovered`  
**Projection**: `sources_view` (status = `discovered`)

**Connector types** (from `ConnectorType` enum):

| Value | Name | Extension / pattern |
|-------|------|---------------------|
| 1 | SQLITE_DB | `.db`, `.sqlite` |
| 2 | CSV_FOLDER | Folder of `.csv` |
| 3 | JSON_FOLDER | Folder of `.json` |
| 4 | POSTGRES | (future) |
| 5 | MYSQL | (future) |
| 6 | ODBC | (future) |
| 7 | IMAGE_FOLDER | jpg, png, tiff, heic, webp |
| 8 | DOCUMENT_FOLDER | pdf, docx, doc, xls, xlsx, pptx, ppt, txt, md, rtf (see Supported Document Formats) |
| 9 | ONEDRIVE | OAuth-based |

**Configuration** (`DiscoveryConfig`):

- `scan_dirs`: Root paths to scan (from `scan_roots.json` or config)
- `scan_sqlite`, `scan_csv`, `scan_json`, `scan_images`, `scan_documents`: Toggles per type

---

## 2. Classify

**Component**: `node_connectors::classify_column`

Schema inspection and column-level classification. Emits `DATA_SOURCE_CLASSIFIED` with:

- `suggested_sensitivity`: 1=Public, 2=Internal, 3=Restricted
- `pii_detected`, `secrets_detected`
- `schema_snapshot_ref`: CAS hash of schema snapshot

**Projection**: `sources_view` (status = `classified`, `pii_detected`, `secrets_detected`, `sensitivity`)

### PII/Secrets Detection (rule-based)

**PII patterns** (column name substring match, case-insensitive):

- email, phone, address, name, dob, date_of_birth, ssn, social_security
- iban, sort_code, card_number
- gps_latitude, gps_longitude, gps_altitude
- file_path, location

→ `is_pii: true`, `suggested_sensitivity: 3` (Restricted)

**Secret patterns**:

- api_key, token, password, secret, credential

→ `is_secret: true`, `suggested_sensitivity: 3` (Restricted)

**Default** (no match): `suggested_sensitivity: 1` (Public)

---

## 3. Approve

**Component**: Admin API `POST /admin/sources/approve`

Admin approves a source with a `SourceProfile` (or defaults). Emits `DATA_SOURCE_APPROVED`.

**Payload** (`DataSourceApproved`):

- `source_id`
- `allowed_tables`: Tables to ingest (empty = all)
- `row_limit`: Max rows per table (0 = no limit)
- `source_profile_ref`: Optional CAS ref to full profile

**SourceProfile** (`source_profiles_view`):

- `allowed_tables_json`: JSON array of table names
- `row_limit`
- `allow_raw_retention`: Whether raw rows are retained
- `allow_training`: Whether data can be used for training
- `max_sensitivity`
- `redaction_policy_json`
- `mapping_rules_json`: Per-table mapping hints (see Mapping rules below)

### Mapping Rules

`mapping_rules_json` format:

```json
{
  "tables": {
    "invoices": {
      "entity_type": "invoice",
      "entity_key_col": "invoice_id",
      "timestamp_col": "created_at",
      "include_cols": ["invoice_id", "customer_id", "amount", "due_date"],
      "exclude_cols": ["internal_notes"]
    },
    "customers": {
      "entity_type": "customer",
      "entity_key_col": "id"
    }
  }
}
```

- `entity_type`: Override inferred entity type (e.g. customer, invoice).
- `entity_key_col`: Column to use as entity_key (stable business ID).
- `timestamp_col`: Column for time-window facts (optional).
- `include_cols` / `exclude_cols`: Column filter (optional). If absent, all columns are used.

**Projection**: `sources_view` (status = `approved`), `source_profiles_view`

---

## 4. Ingest

**Component**: `node_ingest` + `node_connectors` (legacy) **or** source agents (agent path)

### Legacy Path (Admin Ingest)

Runs per approved source. Uses connector’s `inspect_schema` and `ingest_batch`.

**Events**:

- `INGEST_STARTED`: Job started
- `ARTIFACT_PUBLISHED`: Per row/document (content in CAS)
- `INGEST_COMPLETED`: Job finished

**Flow**:

1. Load connector for source’s `connector_type`
2. For each allowed table: batch read (offset/limit), serialize rows to JSON
3. Store JSON in CAS, emit `ARTIFACT_PUBLISHED` with `content_ref`
4. Project into `artifacts_view`

**Config** (`IngestConfig`):

- `batch_size`: Rows per batch (default 100)
- `max_rows_per_table`: Cap per table (default 10_000)

### Agent Path (Source Agents)

Source agents (e.g. `agents/filesystem_ingestion_agent/`) run as separate processes. They:

1. Discover items (e.g. scan folders)
2. Extract content (PDF, Office, OCR)
3. Normalize to `IngestedItem` contract
4. POST to `POST /v1/ingest/items/batch`

See [INGESTION_AGENT_ARCHITECTURE.md](INGESTION_AGENT_ARCHITECTURE.md) and [development/source-agents.md](development/source-agents.md).

---

## 5. Normalize / Learn

- **Normalize**: Artifacts are stored as JSON blobs. Title/summary built from columns (e.g. `content_text`, `filename`).
- **Learn**: `node_datasets` builds dataset manifests from artifacts for training. See `docs/training.md`.

---

## Document Folder Ingest

When ingesting a document folder source, the DocumentConnector:

1. **Recursively scans** the folder and all subfolders for document files
2. **Processes every supported file** in the tree
3. **Reports unsupported files** as `skipped_unsupported` (not silently ignored)
4. **Records failed extractions** (empty files, OCR failures) with status and reason
5. **Uses unique document IDs** (relative path, e.g. `subdir/report.pdf`) to avoid collisions

Per-file results are stored in `ingest_file_results` and visible in the Debug panel **Ingest Results** tab.

### Supported Document Formats

| Format | Extension | Notes |
|--------|-----------|-------|
| PDF | `.pdf` | Text extraction; OCR fallback for scanned PDFs (requires pdftoppm + tesseract) |
| Word | `.docx` | Supported (docx-rust) |
| Legacy Word | `.doc` | Supported (litchi) — experimental |
| Excel | `.xls`, `.xlsx` | Supported (calamine) — cell values extracted as text |
| PowerPoint | `.pptx` | Supported (undoc) |
| Legacy PowerPoint | `.ppt` | Supported (litchi) — experimental |
| Plain text | `.txt`, `.md`, `.rtf` | Supported |
| Visio | `.vsd`, `.vsdx` | **Not supported** — no mature Rust parser; use external tool to convert |

### Failure Reporting

Each candidate file gets a status:

| Status | Meaning |
|--------|---------|
| `ingested` | Text extracted and chunks created |
| `skipped_unsupported` | Format not supported (e.g. .vsd, .vsdx) |
| `failed_extraction` | Extraction attempted but yielded no text |
| `failed_ocr` | Scanned PDF; OCR attempted but failed or yielded nothing |
| `failed_unknown` | Other failure |

Failed files produce a visible record in the ingest results, including `failure_reason` and `ocr_attempted`. See Debug panel → Ingest Results.

---

## Connector Interface

```rust
pub trait Connector: Send + Sync {
    fn id(&self) -> &str;
    fn inspect_schema(&self, path: &Path) -> Result<Vec<TableInfo>>;
    fn ingest_batch(&self, path: &Path, table: &str, offset: u64, limit: u64) -> Result<IngestBatchResult>;
}
```

**TableInfo**: `table_name`, `columns` (SchemaColumn), `row_count_estimate`  
**IngestBatchResult**: `table_name`, `rows` (entity_id + columns BTreeMap), `offset`

**Implemented connectors**:

- `SQLiteConnector`
- `CsvFolderConnector`
- `JsonFolderConnector`
- `ImageConnector` (EXIF/GPS metadata)
- `DocumentConnector` (PDF, DOCX, DOC, XLS, XLSX, PPTX, PPT, TXT, Markdown, RTF text extraction)
- `OneDriveConnector` (OAuth, file listing)

---

## Default Sensitivity Mapping

| Classification | Sensitivity | Replication |
|----------------|-------------|-------------|
| normal | Public (1) | Allowed (if policy permits) |
| pii | Restricted (3) | Blocked by default |
| secret | Restricted (3) | Blocked by default |

Policy engine: `RESTRICTED` blocks replication. `max_replication_sensitivity` caps what can be replicated.

---

## References

- `crates/node_discovery` — Discovery
- `crates/node_connectors` — Connectors + `classify_column`
- `crates/node_ingest` — Ingest pipeline
- `crates/node_storage/src/projector.rs` — Event projection
- `proto/events.proto` — Event payloads
