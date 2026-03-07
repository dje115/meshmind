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
| `GET /v1/debug/ask/:case_id` | Ask session: question, plan, evidence, confidence, source types |
| `GET /v1/debug/entities?entity_type=X` | List entities, optionally filtered by type |

## Inspecting OCR Results

When a scanned PDF is ingested with OCR fallback:

1. **Documents** tab shows `ocr_used: true` for documents that used OCR.
2. **Document Detail** shows per-chunk OCR status and page numbers.
3. Chunks from `document_chunks_view` include `source_file`, `page_number`, `ocr_used`.

If `document_chunks_view` is empty (older ingest), the API falls back to `documents_fts` (no OCR metadata).

## Inspecting Chunks and Entities

- **Chunks**: `chunk_text_preview` (first 200 chars), `source_file`, `page_number`, `ocr_used`.
- **Entities**: `entity_type`, `entity_value`, `normalized_value`, `extraction_method`, `confidence`, `source_document_id`, `chunk_index`.

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
