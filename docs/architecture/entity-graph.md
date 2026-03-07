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

## Distributed Memory Integration

Entity cards and relationships participate in the distributed memory fabric:

- **Shards**: Entity-type shards (e.g. `entity_type:customer`) enable targeted peer routing. Subscribe via `POST /v1/shards/subscribe`.
- **Mergeable state**: Use mergeable tags, counters, and annotations on entities (`object_type: entity`, `object_id: customer:abc-ltd`).
- **Outcomes**: Quote and case outcomes (QUOTE_ACCEPTED, QUOTE_LOST, CASE_FAILED) feed router/ranking training via `ThisTenantConfirmed` preset.

See [distributed-memory.md](../distributed-memory.md).

## Document-Derived Entities (Phase B)

Documents now also produce extracted entities (people, companies, emails, money, invoice numbers, etc.) from chunk text. These are stored in `entities_view` and linked via `documents_entities_view`. See [entity-graph.md](../entity-graph.md) for extraction, normalization, and query support.

## References

- [docs/ingestion/entity-cards.md](../ingestion/entity-cards.md) — Schema and mapping rules
- [docs/intelligence/business-intelligence.md](../intelligence/business-intelligence.md) — BI use cases
- [entity-graph.md](../entity-graph.md) — Document-derived entity extraction (Phase B)
- [distributed-memory.md](../distributed-memory.md) — Shards, mergeable state, outcomes
