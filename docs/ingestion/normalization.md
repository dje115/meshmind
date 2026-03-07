# MeshMind Normalization

## Overview

Raw ingested rows and documents are normalized into structured knowledge: **Entity Cards**, **Fact Records**, and **Document Summaries**. These feed search, retrieval, and training.

---

## Entity Cards

An **Entity Card** is a normalized document representing a single business entity.

| Field | Description |
|-------|-------------|
| `entity_type` | customer, supplier, invoice, order, property, tenant, etc. |
| `entity_key` | Stable identifier (e.g. customer_id, invoice_no) |
| `attributes` | Approved columns only (redacted per SourceProfile) |
| `derived_summary` | Optional text summary for retrieval |
| `source_ref` | Origin (source_id, table, row) |
| `content_ref` | CAS hash of full card body |

Stored in CAS; metadata in `documents_view`.

---

## Fact Records

**Fact Records** store simple numeric aggregates per ingestion run:

- **Counts**: row counts per table, entity counts per type
- **Sums / Avg / Min / Max**: for numeric columns
- **Time-window counts**: if timestamp columns exist

Each fact is small JSON in CAS, indexed in `facts_view`.

---

## Document Summaries

For document ingestion (PDF, DOCX, etc.):

- Text extracted and stored as artifact
- Title/summary built from filename or first lines
- Optional entity card per doc for cross-reference

---

## Mapping Rules

SourceProfile `mapping_rules_json`:

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

- `entity_type`: Override inferred type
- `entity_key_col`: Column for entity_key
- `timestamp_col`: For time-window facts
- `include_cols` / `exclude_cols`: Column filter

---

## References

- [Entity Cards](entity-cards.md) — Detailed entity card model
- [docs/intelligence/business-intelligence.md](../intelligence/business-intelligence.md) — Use cases
- `crates/node_ingest` — Pipeline
- `crates/node_storage/src/projector.rs` — documents_view, facts_view
