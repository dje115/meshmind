# Entity Graph (Phase B — Document-Derived Entities)

This document describes entity extraction from document chunks, normalization, and document–entity relationships implemented in Phase B. It complements the structured entity graph from table rows (see [architecture/entity-graph.md](architecture/entity-graph.md)).

---

## 1. Overview

Documents now produce **structured entities** (people, companies, emails, money, invoice numbers, etc.) extracted from chunk text. Entities are stored in `entities_view`, linked to documents via `documents_entities_view`, and queryable via `list_entities_by_type`, `search_entities_by_value`, and `list_documents_for_entity`.

---

## 2. Entity Types

| Type           | Description                         | Example                          |
|----------------|-------------------------------------|----------------------------------|
| `person`       | Person name (title + name, or 2–3 words) | John Smith, Dr. Jane Doe       |
| `company`      | Organization (Ltd, Inc, Corp, etc.) | Acme Corporation Ltd             |
| `email`        | Email address                       | john@example.com                 |
| `phone`        | UK/international phone number       | +44 20 7123 4567                 |
| `money`        | Currency amount (£, $, €)           | £1,234.56                        |
| `date`         | Date in common formats              | 2024-01-15                       |
| `location`     | Location (optional heuristic)       | London                           |
| `product`      | Product name (optional heuristic)   | Widget Pro                       |
| `quote_number` | Quote reference                     | QT-2024-001                      |
| `invoice_number` | Invoice reference                 | INV-2024-001                     |

---

## 3. Hybrid Classification Pipeline

Entity processing is split into two stages:

1. **Candidate extraction** — Same regexes and span detection as before; title-case 2–3 word phrases are emitted as **unknown** (not assumed person).
2. **Entity classification** — Determines final type: strong rules → vocabulary lookup → optional LLM → fallback.

### Classification order

1. **Strong rules** — If a phrase matches, type is assigned immediately (no LLM):
   - **Company**: contains Ltd, Limited, Plc, LLP, Inc, Corp, Systems, Foods, Services, Solutions, Group, Holdings, Company, Cabling.
   - **Organization / legal scheme**: contains scheme, ombudsman, protection service, deposit protection, property ombudsman (e.g. Tenancy Deposit Scheme, Property Ombudsman).
   - **Location**: contains room, office, warehouse, kitchen, plant room, server room, changing room, temp room.
   - **Address**: ends with or contains street, road, lane, avenue, drive, court, close, way, place, terrace, gardens, crescent, row, square, hill, park, farm, harborough (e.g. Nelson Street, Holt Lane, Market Harborough, Bridge Farm).
   - **Product**: contains conduit, panel, patch, cable, trunking, fixing, machine, access point, camera, router, switch, cabinet.
   - **Money**: £, $, € or GBP/USD/EUR.
   - **Email**: contains `@`.
   - **Phone**: numeric phone patterns.
   - **Date**: existing date patterns.
2. **Context-aware person** — When context contains tenant, landlord, signed, witness, by, name, contact, prepared by, sent to, attention, and phrase looks like a 2–3 word title-case name (e.g. David Evans, Andrew Boddy), classify as person.
3. **Vocabulary lookup** — Normalized phrase is looked up in `entity_vocabulary`. If found with confidence ≥ 0.8, that type is reused and `occurrence_count` is incremented.
4. **LLM classification** — Unknown/ambiguous entities fall through to the LLM when strong rules, context-aware person, and vocabulary fail. The LLM is given the phrase and short context and returns one of the allowed entity types with confidence. Only non-unknown types with confidence ≥ 0.5 are accepted.
5. **Fallback** — If still unresolved, initial type (e.g. person from title) is kept when confidence ≥ 0.75; otherwise type is `unknown` and the entity is retained for later reclassification.

### Provenance

Each entity record includes:

- **extraction_method** — How the span was found: `rule_based` | `llm_assisted`.
- **classification_method** — How the type was chosen: `rule_based` | `vocabulary_lookup` | `llm_assisted` | `corrected`.
- **confidence** — 0.0–1.0.

---

## 4. Vocabulary Learning

A persistent **entity_vocabulary** table stores previously classified phrases so the system learns domain terms.

| Column             | Description                                      |
|--------------------|--------------------------------------------------|
| normalized_phrase  | Lowercase, collapsed whitespace (primary key)     |
| entity_type        | person, company, product, location, etc.         |
| confidence         | 0.0–1.0                                          |
| first_seen / last_seen | Timestamps (ms)                            |
| occurrence_count   | Number of times this phrase was seen             |
| source_method      | rule_based \| llm_assisted \| corrected         |

- After each classification, the phrase is stored or updated (increment `occurrence_count`, update `last_seen`).
- When the user corrects an entity via the Debug Panel, the vocabulary is updated with `source_method = corrected` and `confidence = 1.0`, so corrections permanently override earlier guesses.
- **GET /v1/debug/vocabulary** returns the learned vocabulary for inspection (phrase, entity_type, confidence, occurrence_count, source_method).

---

## 5. Rule-Based Extraction (Candidates)

Extraction runs on each document chunk during ingestion. Rule-based extractors are **deterministic** and cover:

### Email
- Standard email regex: `[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}`

### Phone
- UK (`+44`, `0044`, `0`) and international style
- Digit count 10–15; avoids noisy patterns

### Money
- Currency symbols: £, $, €
- Patterns: `£1,234.56`, `$99.99`, `1234.56 GBP`

### Date
- Common formats: `DD/MM/YYYY`, `YYYY-MM-DD`, `Jan 15, 2024`

### Quote / Invoice Numbers
- Patterns: `quote no: X`, `invoice no: X`, `inv no. X`

### Company
- Suffixes: Ltd, Limited, Plc, LLP, Inc, Corp, Corporation, Incorporated
- Capitalized multi-word phrases with these suffixes

### Person
- Title prefixes: Mr, Mrs, Ms, Miss, Dr, Prof → high-confidence person.
- Two or three capitalized words (avoiding company suffixes) → emitted as **unknown** and then classified by strong rules, vocabulary, or LLM (so "White Flexible Conduit" becomes product, "Server Room" becomes location, "John Smith" can remain person).

---

## 6. Optional LLM-Assisted Classification

When enabled, the LLM is used **only to classify** ambiguous candidates (unknown type), not to extract spans.

### When the LLM is used

- Candidate not classified by strong rules, context-aware person rules, or vocabulary.
- **Unknown entities fall through to LLM** — they are not a dead-end; the LLM gets a chance to classify them.
- Phrase length &lt; 60 characters.
- Entity count per chunk below a safety threshold (e.g. 15).
- `enable_llm_entity_extraction` is `true` (default) and a backend is configured.

The LLM is given the phrase and short surrounding context and returns a single type and confidence. Results are cached in the vocabulary for future reuse. Only results with type ≠ unknown and confidence ≥ 0.5 are accepted.

---

## 7. Entity Normalization

Values are normalized for deduplication:

| Type            | Normalization                                             |
|-----------------|-----------------------------------------------------------|
| `email`         | Lowercase                                                 |
| `phone`         | Digits only                                               |
| `company`       | Lowercase; `Corporation` → `corp`, `Limited` → `ltd`      |
| `person`        | Lowercase                                                 |
| `money`         | Digits, decimal, comma only (strip currency symbols)      |
| `invoice_number` / `quote_number` | Uppercase, collapse whitespace                  |

---

## 8. Merge and Deduplication

- Identical `normalized_value` within the same chunk → merge
- Rule-based results are preferred over LLM on conflict
- Entities are unique per `(entity_type, normalized_value)` per chunk

---

## 9. Document → Entity Relationships

### Views

| View                             | Purpose                                                           |
|----------------------------------|-------------------------------------------------------------------|
| `entities_view`                  | All extracted entities (entity_id, entity_type, entity_value, normalized_value, document_id, chunk_index, confidence, extraction_method, classification_method) |
| `documents_entities_view`        | Document–entity links (document_id, entity_id, entity_type, entity_value, chunk_index) |
| `extracted_entity_relationships_view` | Entity-to-entity relationships from chunks (from_entity_id, relationship_type, to_entity_id, source_document_id, chunk_index, confidence, extraction_method) |
| `people_view`                    | `entities_view WHERE entity_type = 'person'`                      |
| `companies_view`                 | `entities_view WHERE entity_type = 'company'`                     |
| `entity_relationships_view`      | Includes `document:X` → `entity:Y` with `relationship_type = 'mentions'` |

### Rebuild

All projections rebuild from the event log and CAS. `ExtractedEntityRecorded` events are projected into `entities_view`, `documents_entities_view`, and `entity_relationships_view`. `ExtractedRelationshipRecorded` events are projected into `extracted_entity_relationships_view`.

---

## 10. Relationship Extraction Pipeline

Relationships between entities (e.g. person → works_for → company, quote → for_customer → company) are extracted from document chunks during ingestion.

### Relationship Types

| Type                | Description                              | Example                                |
|---------------------|------------------------------------------|----------------------------------------|
| `works_for`         | Person employed by company               | Gavin Anthony → Complete Cabling Systems Ltd |
| `contact_for`       | Person is contact for company            |                                       |
| `prepared_by`       | Document prepared by person              |                                       |
| `sent_to`           | Document sent to person/company          |                                       |
| `for_customer`      | Quote/invoice for company                | Quote 1234 → Becketts Foods            |
| `references_quote`  | Invoice references quote                 | Invoice 4567 → Quote 1234              |
| `has_quote_number`  | Document has quote number                |                                       |
| `has_invoice_number`| Document has invoice number              |                                       |
| `has_total`         | Quote/invoice has total amount           | Invoice 4567 → £1035.00                |
| `includes_product`  | Quote/invoice includes product           | Quote 1234 → White Flexible Conduit    |
| `installed_at`      | Product installed at location            | Cat6 Cable → Server Room               |
| `located_in`        | Product/location relationship            | Product → Server Room                  |
| `mentions`          | General mention                          |                                       |
| `related_to`        | General related                          |                                       |

### Rule-Based Extraction

Deterministic heuristics infer relationships from co-occurrence and context:

- Person near company in header/signature/contact block → `works_for` or `contact_for`
- Quote/invoice number near company with "for", "customer", "bill to" → `for_customer`
- Money near quote/invoice with "total", "sum", "amount" → `has_total`
- Products in line-item sections (qty, description, item) → `includes_product`
- Product + location with "install", "fitted", "mount" → `installed_at`, else `located_in`
- Invoice + quote with "quote" in context → `references_quote`

Rules are conservative; entities must be within ~150 characters in the chunk text.

### Optional LLM-Assisted Relationship Extraction

When `enable_llm_relationship_extraction` is `true` in `ExtractionConfig` and a backend is configured:

- **When used**: Chunk has 2+ entities, rule-based extraction found fewer than `llm_relationship_count_threshold` (default 2) relationships.
- **Input**: Short chunk text, list of extracted entities, allowed relationship types.
- **Output**: Strict JSON array `[{"from":"...","relationship":"...","to":"...","confidence":0.0}]`
- **Validation**: Only relationships where both from and to match an entity in the chunk are stored. Invalid relationship types are rejected.

---

## 11. Query Support

| Function                                              | Purpose                                              |
|-------------------------------------------------------|------------------------------------------------------|
| `list_entities_by_type(conn, type, limit)`             | List entities of a given type                        |
| `search_entities_by_value(conn, query, type_opt, limit)` | Search by value (LIKE)                             |
| `list_documents_for_entity(conn, normalized_value, type_opt, limit)` | Documents mentioning an entity (by normalized value) |
| `count_entity_mentions(conn, type)`                    | Count entity mentions by type                        |
| `list_relationships_for_entity(conn, entity_id, limit)`| Relationships where entity is from or to             |
| `list_related_entities(conn, entity_value, rel_type_opt, limit)` | Related entities by value, optionally filtered by relationship type |
| `list_documents_for_related_entities(conn, entity_value, limit)` | Documents containing relationships for an entity   |
| `count_relationships_by_type(conn, relationship_type)` | Count relationships by type                          |

---

## 12. Example Queries

### Who appears in my documents?
```
list_entities_by_type(conn, "person", 50)
```

### Show documents mentioning ABC Ltd
```
list_documents_for_entity(conn, "abc corp ltd", Some("company"), 20)
```
(Use normalized value: `ABC Corporation Ltd` → `abc corp ltd`)

### List all emails found in documents
```
list_entities_by_type(conn, "email", 100)
```

### What invoice numbers appear?
```
list_entities_by_type(conn, "invoice_number", 50)
```

### What is related to Becketts Foods?
```
list_related_entities(conn, "Becketts Foods", None, 50)
```

### Which products are linked to this quote?
```
list_related_entities(conn, "1234", Some("includes_product"), 20)
```

### Which person works for Complete Cabling Systems Ltd?
```
list_related_entities(conn, "Complete Cabling Systems Ltd", Some("works_for"), 20)
```

### List relationships for an entity by ID
```
list_relationships_for_entity(conn, "person:gavin-anthony", 50)
```

---

## 13. Debug Visibility

- **GET /v1/debug/documents/:id/relationships** — Relationships extracted from that document.
- **GET /v1/debug/relationships?entity_type=&relationship_type=** — All relationships, optionally filtered by entity type prefix (e.g. `person`) and/or relationship type.

Each response shows: `from`, `relationship`, `to`, `extraction_method`, `confidence`, `source_document_id`.

---

## 14. Ask Pipeline Support

When the question clearly requests entities, the Ask pipeline uses structured queries first:

- **Intent patterns**: "who appears", "what people", "which companies", "show documents mentioning X", "list emails", "what invoice numbers", "what quote numbers", "what is related to X", "which products are in this quote", "who works for X"
- **Flow**: Classify intent → run structured entity queries → add results to context → LLM formats the answer
- **Evidence**: Structured entity results are returned in `AskResponse`; the LLM does not discover entities from raw text.

---

## References

- [document-intelligence.md](document-intelligence.md) — Document chunking, FTS, Phase A/B
- [architecture/entity-graph.md](architecture/entity-graph.md) — Structured entity cards from tables
