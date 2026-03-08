# MeshMind Debug Panel

The Debug Panel provides inspection capabilities for document ingestion, OCR status, chunks, extracted entities, and Ask planner output. It supports correction feedback for self-learning.

## Access

- **UI**: Navigate to **Debug** in the sidebar (admin-protected).
- **API**: Debug endpoints are under `/v1/debug/` and require admin auth (`Authorization: Bearer <token>`).

## Debug Endpoints

| Endpoint | Description |
|----------|-------------|
| `GET /v1/debug/documents` | List ingested documents with OCR status, chunk count, entity count |
| `GET /v1/debug/documents/:id` | Detailed document metadata, chunks, entities |
| `GET /v1/debug/documents/:id/chunks` | Chunk list with previews, OCR status, page numbers |
| `GET /v1/debug/documents/:id/entities` | Entities extracted from the document |
| `GET /v1/debug/ingest-results?source_id=X` | Per-file ingest results (ingested, skipped, failed) |
| `GET /v1/debug/ingest/sources` | List sources with last ingest status |
| `GET /v1/debug/ingest/jobs` | List ingest jobs (ingests_view) |
| `GET /v1/debug/ingest/items?source_id=&ingest_id=&limit=` | Per-file items from ingest_file_results |
| `GET /v1/debug/ingest/items/:id` | Single ingest item by file_path or filename |
| `GET /v1/debug/ask/:case_id` | Ask session: question, plan, evidence, confidence, source types |
| `GET /v1/debug/entities?entity_type=X` | List entities, optionally filtered by type |
| `GET /v1/debug/vocabulary?limit=N` | Learned phrase → type vocabulary (phrase, entity_type, confidence, occurrence_count, source_method) |

## Inspecting OCR Results

When a scanned PDF is ingested with OCR fallback:

1. **Documents** tab shows `ocr_used: true` for documents that used OCR.
2. **Document Detail** shows per-chunk OCR status and page numbers.
3. Chunks from `document_chunks_view` include `source_file`, `page_number`, `ocr_used`.

If `document_chunks_view` is empty (older ingest), the API falls back to `documents_fts` (no OCR metadata).

## Ingestion Tab

The **Ingestion** tab (Debug panel) shows sources, jobs, and per-file items:

- **Sources** — All sources with last ingest status
- **Jobs** — Ingest jobs from `ingests_view`
- **Items** — Per-file results with optional source filter

## Inspecting Ingest Results

The **Ingest Results** tab shows per-file status from the last document folder ingest:

- **filename**, **detected_type** — File and format
- **status** — `ingested`, `skipped_unsupported`, `failed_extraction`, `failed_ocr`, `failed_unknown`
- **ocr_attempted** — Whether OCR was tried (scanned PDFs)
- **chunks_created** — Number of chunks produced
- **failure_reason** — When status is failed, the reason (e.g. "OCR failed", "Format not supported")

Select a document source from the dropdown and click Load. See [ingestion.md](ingestion.md) for supported formats and failure reporting.

## Inspecting Chunks and Entities

- **Chunks**: `chunk_text_preview` (first 200 chars), `source_file`, `page_number`, `ocr_used`.
- **Entities**: `entity_type`, `entity_value`, `normalized_value`, `extraction_method`, `classification_method` (rule_based | vocabulary_lookup | llm_assisted | corrected), `confidence`, `source_document_id`, `chunk_index`.

## Classification Summary and LLM Usage

The **document detail** (`GET /v1/debug/documents/:id`) includes a `classification_summary` object:

- **rule_based** — Count of entities classified by strong rules or context-aware heuristics.
- **vocabulary_lookup** — Count classified by learned vocabulary.
- **llm_assisted** — Count classified by the LLM (when unknown entities reached the LLM helper).
- **corrected** — Count with user corrections.
- **unknown_count** — Count still unresolved (entity_type = unknown).

Use this to verify that the LLM classifier is being used: when `llm_assisted` &gt; 0, unknown entities are reaching the LLM and being resolved. If `unknown_count` is high and `llm_assisted` is 0, check that an inference backend is configured and `enable_llm_entity_extraction` is true (default).

## Inspecting Ask Plan and Evidence

After an Ask request, use the returned `case_id` with `GET /v1/debug/ask/:case_id` to see:

- **question** — The user question
- **plan_json** — Intent, retrieval steps, source types
- **evidence_json** — Evidence items with provenance
- **confidence** — Answer confidence
- **web_fallback_used** — Whether web search was used
- **peer_consult_used** — Whether peer mesh consult was used

## Correction Feedback

### Entity Correction

```
POST /v1/debug/documents/:id/entities/correct
Body: { "entity_id": "...", "corrected_value": "...", "corrected_type": "...", "is_valid": true }
```

- `corrected_value` — New value; omit to keep original.
- `corrected_type` — Override entity type (e.g. `person` → `company`).
- `is_valid` — `false` marks the entity as incorrect (filtered in effective view).

### Chunk (OCR) Correction

```
POST /v1/debug/documents/:id/chunks/:chunk_index/correct
Body: { "corrected_text": "...", "note": "optional note" }
```

### Classification Correction

```
POST /v1/debug/documents/:id/classification/correct
Body: { "corrected_document_type": "...", "corrected_entity_type": "..." }
```

## Effective Views

- **effective_entities_view** — Same as `entities_view` but uses corrected values when a correction exists. Original extraction is preserved in `entities_view`.
- Corrections are stored in `corrections_view` with full provenance (timestamp, source_user).

## Workflow Example

1. **Ingest** a scanned PDF → OCR fallback runs.
2. **Debug → Documents** → See the document with `ocr_used: true`.
3. **Document Detail** → Inspect chunks and entities.
4. **Correct** a bad extraction via POST to `/entities/correct`.
5. **effective_entities_view** → Query uses the corrected value.
6. **Future training** — Dataset builders can read `corrections_view` and `effective_entities_view` for supervised learning.

## Future Training Integration

Corrections include:

- `original_value` / `corrected_value` — For value corrections
- `corrected_type` — For type changes
- `is_valid` — For invalid-entity annotations
- `source_user`, `created_at_ms` — Provenance

These fields are suitable for building training datasets that prefer human-corrected extractions.
