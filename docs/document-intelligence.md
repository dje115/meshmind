# Document Intelligence (Phase A + B)

This document describes document chunking, full-text indexing, entity extraction, and retrieval improvements implemented in Phase A and Phase B. It focuses on how documents are ingested, chunked, indexed, and used by the Ask pipeline.

---

## 1. Chunking Strategy

### Overview

Large documents are no longer ingested as single blobs. Instead, the **DocumentConnector** splits documents into overlapping fragments (chunks) that become logical document fragments.

### Parameters

| Parameter    | Value  | Description                                  |
|-------------|--------|----------------------------------------------|
| Chunk size  | 1500   | Target characters per chunk                  |
| Overlap     | 200    | Characters of overlap between consecutive chunks |

### Chunk Structure

Each chunk is represented as a row with:

- `document_id` — ID of the parent document (e.g. filename)
- `chunk_index` — 0-based index within the document
- `chunk_text` — The extracted text for this fragment
- `source_file` — Original filename
- `page_number` — Page number (if available, e.g. from PDF)
- `ocr_used` — `1` when OCR fallback was used (scanned PDF), `0` otherwise

### Behavior

- **Small documents** (≤ chunk size): Remain a single chunk.
- **Large documents**: Split into multiple chunks with 200-character overlap.
- **Overlap**: Ensures phrases near chunk boundaries are not lost and preserves context.

---

## 2. Why Chunking Improves Retrieval

### Before (Single Blob)

- Only the first ~500 characters (summary) were indexed in `artifacts_fts`.
- Phrases in the middle or end of a document could not be found.
- Long documents contributed little to search relevance.

### After (Chunking)

- Every chunk is indexed in `documents_fts`.
- Phrases anywhere in the document are searchable.
- Multiple chunks from the same document can contribute evidence to a query.
- The Ask pipeline can return several relevant fragments instead of one truncated summary.

---

## 3. FTS Indexing Changes

### `documents_fts` Table

A new FTS5 virtual table indexes document chunks:

| Column       | Indexed | Description                            |
|--------------|---------|----------------------------------------|
| artifact_id  | No      | Artifact ID of the chunk               |
| document_id  | No      | Parent document ID                     |
| chunk_index  | No      | Chunk index within the document        |
| chunk_text   | Yes     | Full chunk text (searchable)           |

### Population

- On `ArtifactPublished` for `document_subtype == "document_chunk"`, the projector extracts `document_id`, `chunk_index`, and `chunk_text` from `entity_attributes_json` and inserts into `documents_fts`.
- On `ArtifactDeprecated`, matching rows are removed.

### `search_all` Integration

`search_all` now queries three sources and merges by rank:

1. `cases_fts` — Case titles, summaries, tags
2. `artifacts_fts` — Artifact titles, summaries
3. `documents_fts` — Document chunk text

Document chunk hits are returned as `SearchHit` with `hit_type = "document_chunk"` and `summary = chunk_text`, so they can be used as context evidence in the Ask pipeline.

---

## 4. Document Summary Improvements

### `build_artifact_summary`

- **Cap**: 800–1000 characters (was 500).
- **Chunks**: Prefer `chunk_text` over `content_text` when available.
- **Metadata**: Includes `source_file` and `page_number` when present.
- **Entity cards**: Fallback to key-value pairs for non-document artifacts.

---

## 5. Projection Updates

### `documents_view`

`documents_view` includes:

- `document_id`, `version`, `document_type`
- `entity_type`, `entity_key` (when known)
- `title`, `summary`, `content_hash`, `source_id`, `table_name`
- `created_at_ms`

### Rebuild

Rebuilding projections from the event log (e.g. via `replay_events`) recreates:

- `documents_view` rows for each document chunk
- `documents_fts` entries for full-text search

---

## 6. Ask Pipeline Behavior

- **FTS retrieval**: `search_all` returns cases, artifacts, and document chunks.
- **Context bullets**: For `document_chunk` hits, `chunk_text` (via `summary`) is included in the context with a per-chunk budget of up to 1500 characters.
- **Evidence**: Chunk hits contribute to `context_used` and evidence, so large documents can provide multiple evidence fragments.

---

## 7. PDF Text Extraction and OCR Fallback

### PDF Text Extraction

PDFs are processed using **pdf_oxide** for direct text extraction. The extractor iterates over each page and extracts embedded text. Text is truncated to 100KB per document to avoid oversized context.

### OCR Fallback for Scanned PDFs

When a PDF yields very little extractable text (fewer than 200 characters), it is treated as a **scanned document**. The system automatically runs an OCR fallback:

1. **Render pages to images** — Uses `pdftoppm` (from poppler-utils) to convert each PDF page to PNG at 300 DPI.
2. **Run OCR** — Uses **Tesseract OCR** on each page image to recognize text.
3. **Combine results** — Collected text from all pages is merged and passed through the normal pipeline.

**Requirements**: `pdftoppm` (poppler-utils) and `tesseract` must be installed and available in PATH.

| Platform | Install |
|----------|---------|
| Ubuntu/Debian | `apt install poppler-utils tesseract-ocr` |
| macOS | `brew install poppler tesseract` |
| Windows | [Poppler for Windows](https://github.com/oschwartz10612/poppler-windows/releases), [Tesseract](https://github.com/UB-Mannheim/tesseract/wiki) |

When OCR is used, chunk metadata includes `ocr_used = true` and `page_number` for provenance.

### Supported Formats

| Format | Extension | Extraction Method |
|--------|-----------|-------------------|
| PDF | `.pdf` | pdf_oxide (direct) → OCR fallback if &lt; 200 chars |
| Word | `.docx` | docx-rust (paragraph/run text) |
| Legacy Word | `.doc` | litchi (experimental) |
| Excel | `.xls`, `.xlsx` | calamine (cell values) |
| PowerPoint | `.pptx` | undoc |
| Legacy PowerPoint | `.ppt` | litchi (experimental) |
| Plain text | `.txt`, `.md`, `.rtf` | Direct file read |

---

## 8. Example Ingestion Pipeline

1. **Scan**: DocumentConnector discovers PDF, DOCX, TXT, MD, RTF files in a folder.
2. **Extract**: Text is extracted (e.g. `pdf_oxide` for PDFs; OCR fallback for scanned PDFs).
3. **Chunk**: Text is split with `chunk_text()` (1500 chars, 200 overlap).
4. **Emit**: One `ArtifactPublished` per chunk with `document_subtype = "document_chunk"` and `entity_attributes_json` containing `document_id`, `chunk_index`, `chunk_text`, `source_file`, `page_number`.
5. **Project**: Projector updates `artifacts_view`, `documents_view`, and `documents_fts`.
6. **Search**: `search_documents_fts` and `search_all` return chunk hits for queries.

---

## 9. Entity Extraction (Phase B)

### Overview

Document chunks are now processed for **entity extraction** during ingestion. Extracted entities (people, companies, emails, phones, money, dates, invoice/quote numbers) are stored in `entities_view` and linked to documents via `documents_entities_view`.

### Hybrid Classification Pipeline

1. **Candidate extraction** — Regexes and span detection; title-case 2–3 word phrases are emitted as *unknown* (not assumed person).
2. **Strong rule classification** — Company (Ltd, Inc, Systems, …), organization/legal scheme (Tenancy Deposit Scheme, Property Ombudsman), location (room, office, …), address (street, lane, harborough, farm, …), product (conduit, panel, cable, …), money, email, phone, date.
3. **Context-aware person** — When context contains tenant, landlord, signed, witness, contact, etc., and phrase looks like a 2–3 word name, classify as person.
4. **Vocabulary lookup** — Normalized phrase is looked up in `entity_vocabulary`. If found with confidence ≥ 0.8, that type is reused.
5. **LLM-assisted classification** — **Unknown entities fall through to the LLM** when rules and vocabulary fail. The LLM is enabled by default (`enable_llm_entity_extraction = true`) when a backend is configured. Phrase length &lt; 60 chars, under per-chunk safety threshold. Result is cached in vocabulary.
6. **Vocabulary learning** — After classification, phrase and type are stored in `entity_vocabulary` (or occurrence_count updated). User corrections set `source_method = corrected` and `confidence = 1.0`.

### Events

- **ExtractedEntityRecorded**: Emitted per entity per chunk with `entity_id`, `entity_type`, `entity_value`, `normalized_value`, `source_document_id`, `chunk_index`, `confidence`, `extraction_method` (rule_based | llm_assisted), `classification_method` (rule_based | vocabulary_lookup | llm_assisted | corrected).
- **ExtractedRelationshipRecorded**: Emitted per relationship with `from_entity_id`, `to_entity_id`, `relationship_type` (works_for, for_customer, includes_product, has_total, etc.), `source_document_id`, `chunk_index`, `confidence`, `extraction_method` (rule_based | llm_assisted).

### Relationship Extraction

Relationships between entities (person → works_for → company, quote → for_customer → company, quote → includes_product → product) are extracted via:

1. **Rule-based extraction** — Deterministic heuristics (proximity, context keywords like "prepared by", "bill to", "total", "qty", "install").
2. **Optional LLM-assisted extraction** — When `enable_llm_relationship_extraction` is true, backend is configured, and rule-based finds few relationships in chunks with 2+ entities.

### Example Queries

- "Who appears in my documents?" → `list_entities_by_type(conn, "person", 50)`
- "Show documents mentioning ABC Ltd" → `list_documents_for_entity(conn, "abc corp ltd", Some("company"), 20)`
- "List all emails found" → `list_entities_by_type(conn, "email", 100)`
- "What is related to Becketts Foods?" → `list_related_entities(conn, "Becketts Foods", None, 50)`
- "Which products are in quote 1234?" → `list_related_entities(conn, "1234", Some("includes_product"), 20)`

See [entity-graph.md](entity-graph.md) for full details.

---

## 10. Debug Panel and Correction Feedback

### Debug API and UI

The Debug Panel allows inspection of:

- **Relationships** — GET /v1/debug/documents/:id/relationships (per-document), GET /v1/debug/relationships (filterable by entity_type, relationship_type)

- **OCR status** — Which documents used OCR, per-chunk OCR flags
- **Chunks** — `document_id`, `chunk_index`, `chunk_text` preview, `source_file`, `page_number`
- **Entities** — Extracted entities with type, value, confidence, extraction method, classification method
- **Ask plan** — Intent, retrieval steps, evidence, source types, web/peer fallback

See [debug-panel.md](debug-panel.md) for endpoints and usage.

### Vocabulary and Corrections

- **GET /v1/debug/vocabulary** — Returns learned phrase → type mappings (phrase, entity_type, confidence, occurrence_count, source_method).
- When a user corrects an entity type via the Debug Panel, the correction is stored in `corrections_view` **and** the phrase is upserted into `entity_vocabulary` with `source_method = corrected` and `confidence = 1.0`, so future classifications reuse the corrected type.

### Correction Storage

Corrections are stored in `corrections_view`:

- **Entity correction** — `corrected_value`, `corrected_type`, `is_valid` (marks incorrect extractions)
- **Chunk (OCR) correction** — `corrected_text` for OCR errors
- **Classification correction** — Document type or entity type override

Each correction includes `source_user`, `created_at_ms` for provenance.

### Effective Entities View

`effective_entities_view` mirrors `entities_view` but uses corrected values when a correction exists. Original extraction remains in `entities_view`. Queries that prefer human-corrected data should use `effective_entities_view`.

### Future Training

Corrections are queryable by DatasetManifest builders. Fields `original_value`, `corrected_value`, `corrected_type`, `is_valid`, `source_user`, `created_at_ms` provide full provenance for supervised learning.

---

## 11. Out of Scope (Phase C)

- Planner logic for retrieval strategy selection
- Learned entity resolution or cross-document linking
