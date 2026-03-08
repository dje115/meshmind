# Source Provenance

Evidence in Ask responses carries provenance so users can trace answers back to source documents and open originals.

## Provenance Fields

| Field | Purpose |
|-------|---------|
| `source_locator` | Machine-readable reference (path, id, external key) |
| `source_open_target` | URI or deep-link to open original (file://, outlook://, etc.) |
| `source_origin_label` | Human-readable label (e.g. "C:\Quotes\quote.docx") |

## In AskResponse Evidence

When evidence comes from document chunks (FTS search), each `EvidenceItem` may include:

```json
{
  "id": "ing-agent-xxx-documents-/path/to/doc.pdf-chunk-0",
  "source_type": "local",
  "title": "/path/to/doc.pdf (chunk 0)",
  "source_locator": "/path/to/doc.pdf",
  "source_open_target": "file:///path/to/doc.pdf",
  "source_origin_label": "C:\\Quotes\\doc.pdf"
}
```

The UI can use `source_open_target` for "Open original" and `source_origin_label` for display.

## API Endpoints

| Endpoint | Description |
|----------|-------------|
| `GET /v1/evidence/:id/source` | Fetch provenance for an evidence artifact_id (admin auth) |
| `GET /v1/source-items/:id` | Look up source item by document_id or file path (admin auth) |

## Storage

- **document_chunks_view**: Stores `source_locator`, `source_open_target`, `source_origin_label` (populated from ingest `entity_attributes_json`)
- **Ingest contract**: `IngestedItem` includes these fields; agents send them when publishing

## User Questions

- "Where did this come from?" → provenance in evidence
- "Show me that document" → source_open_target
- "Open the original file" → file:// or app-specific URI
