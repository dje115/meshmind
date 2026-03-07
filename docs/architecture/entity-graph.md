# Entity Intelligence Graph

## Overview

The entity graph enables MeshMind to reason about customers, products, jobs, invoices, quotes, and accounting data across systems. Each entity produces an **EntityCard** stored in CAS and projected into SQLite views.

## Entity Types

| Type | Description |
|------|-------------|
| Customer | End customer or client |
| Supplier | Vendor, partner |
| Product | Product or service |
| Quote | Quote or proposal |
| QuoteLineItem | Line item within a quote |
| Invoice | Invoice record |
| Account | Accounting entity |
| Transaction | Financial transaction |
| Project | Project |
| Job | Work order, project |

## EntityCard Format

```json
{
  "type": "customer",
  "id": "customer:abc-ltd",
  "attributes": {
    "revenue_total": 140000,
    "jobs_completed": 9,
    "invoices_overdue": 2,
    "products_used": ["cat6", "fibre"]
  }
}
```

Entity IDs use the format `{entity_type}:{entity_key}` (e.g. `customer:cust-1`, `invoice:inv-001`).

## Relationships

| Relationship | From | To |
|--------------|------|-----|
| customer -> quotes | customer | quote |
| quote -> line_items | quote | quote_line_item |
| quote -> customer | quote | customer |
| invoice -> quote | invoice | quote |
| invoice -> customer | invoice | customer |
| transaction -> account | transaction | account |

Relationships are emitted as `ENTITY_RELATIONSHIP_RECORDED` events and stored in `entity_relationships_view`. Ingestion infers relationships from FK columns (e.g. `customer_id` -> `belongs_to_customer`).

## SQLite Views

| View | Purpose |
|------|---------|
| `entity_cards_view` | All entity cards (entity_id, entity_type, attributes_json, content_hash, source_id, table_name) |
| `entity_relationships_view` | Relationships (from_entity_id, to_entity_id, relationship_type) |
| `customers_view` | `entity_cards_view WHERE entity_type = 'customer'` |
| `quotes_view` | `entity_cards_view WHERE entity_type = 'quote'` |
| `invoices_view` | `entity_cards_view WHERE entity_type = 'invoice'` |
| `accounts_view` | `entity_cards_view WHERE entity_type = 'account'` |

All views rebuild from the event log.

## Events

- **ArtifactPublished** (DOCUMENT, document_subtype=entity_card): Creates entity card. Requires `entity_type`, `entity_key`, and optionally `entity_attributes_json`.
- **ENTITY_RELATIONSHIP_RECORDED**: Records a relationship between two entities.

## References

- [docs/ingestion/entity-cards.md](../ingestion/entity-cards.md) — Schema and mapping rules
- [docs/intelligence/business-intelligence.md](../intelligence/business-intelligence.md) — BI use cases
