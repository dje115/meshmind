# MeshMind Entity Cards

## What Are Entity Cards?

Entity Cards are normalized documents representing single business entities from ingested sources. They enable semantic search ("Find invoices for customer X") and consistent representation across tables.

---

## Schema

| Field | Description |
|-------|-------------|
| `entity_type` | customer, supplier, invoice, order, property, tenant, etc. |
| `entity_key` | Stable identifier (e.g. customer_id, invoice_no) |
| `attributes` | Approved columns only (redacted per SourceProfile) |
| `derived_summary` | Optional text summary for retrieval (rule-based or LLM) |
| `source_ref` | Origin (source_id, table, row) |
| `content_ref` | CAS hash of full card body (markdown or JSON) |
| `table_name` | Source table (for DB connectors) |

---

## Inference Rules

When mapping rules are absent, best-effort inference:

- Table name hints: `customers` → customer, `invoices` → invoice, `orders` → order
- Primary key column: first unique column or `id`
- Timestamp: `created_at`, `updated_at`, `date`, `timestamp`

---

## Storage

- **Body**: JSON or markdown in CAS
- **Metadata**: `documents_view` (artifact_id, entity_type, entity_key, source_ref, etc.)
- **FTS**: Searchable via artifacts_fts where applicable

---

## References

- [Normalization](normalization.md) — Pipeline
- [Connectors](connectors.md) — Mapping rules in SourceProfile
- [docs/intelligence/business-intelligence.md](../intelligence/business-intelligence.md) — Use cases
