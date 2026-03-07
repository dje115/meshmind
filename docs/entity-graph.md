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

## 3. Rule-Based Extraction (Primary)

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
- Title prefixes: Mr, Mrs, Ms, Miss, Dr, Prof
- Two or three capitalized words (avoiding company suffixes)

---

## 4. Optional LLM-Assisted Extraction

When enabled, an LLM can augment rule-based extraction for long chunks with few entities.

### Configuration

| Setting                       | Default | Description                                          |
|------------------------------|---------|------------------------------------------------------|
| `enable_llm_entity_extraction` | `false` | Whether to call the LLM for entity extraction       |
| `llm_chunk_length_threshold` | 500     | Minimum chunk length (chars) to consider LLM        |
| `llm_entity_count_threshold` | 2       | Rule-based count below which to try LLM             |

### Behavior

- LLM extraction runs only when:
  - `enable_llm_entity_extraction` is `true`
  - Chunk length ≥ `llm_chunk_length_threshold`
  - Rule-based extraction finds fewer than `llm_entity_count_threshold` entities
- The LLM returns strict JSON with keys: `people`, `companies`, `emails`, `phones`, `money`, `dates`, `locations`, `products`, `invoice_numbers`, `quote_numbers`.
- LLM-extracted entities are stored with `extraction_method = "llm_assisted"` and lower confidence (0.7).
- Hallucinated or invalid entries are discarded during parsing.

---

## 5. Entity Normalization

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

## 6. Merge and Deduplication

- Identical `normalized_value` within the same chunk → merge
- Rule-based results are preferred over LLM on conflict
- Entities are unique per `(entity_type, normalized_value)` per chunk

---

## 7. Document → Entity Relationships

### Views

| View                     | Purpose                                                           |
|--------------------------|-------------------------------------------------------------------|
| `entities_view`          | All extracted entities (entity_id, entity_type, entity_value, normalized_value, document_id, chunk_index, confidence, extraction_method) |
| `documents_entities_view`| Document–entity links (document_id, entity_id, entity_type, entity_value, chunk_index) |
| `people_view`            | `entities_view WHERE entity_type = 'person'`                      |
| `companies_view`         | `entities_view WHERE entity_type = 'company'`                     |
| `entity_relationships_view` | Includes `document:X` → `entity:Y` with `relationship_type = 'mentions'` |

### Rebuild

All projections rebuild from the event log and CAS. `ExtractedEntityRecorded` events are projected into `entities_view`, `documents_entities_view`, and `entity_relationships_view`.

---

## 8. Query Support

| Function                          | Purpose                                              |
|-----------------------------------|------------------------------------------------------|
| `list_entities_by_type(conn, type, limit)` | List entities of a given type                   |
| `search_entities_by_value(conn, query, type_opt, limit)` | Search by value (LIKE)                     |
| `list_documents_for_entity(conn, normalized_value, type_opt, limit)` | Documents mentioning an entity (by normalized value) |
| `count_entity_mentions(conn, type)` | Count entity mentions by type                     |

---

## 9. Example Queries

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

---

## 10. Ask Pipeline Support

When the question clearly requests entities, the Ask pipeline uses structured queries first:

- **Intent patterns**: "who appears", "what people", "which companies", "show documents mentioning X", "list emails", "what invoice numbers", "what quote numbers"
- **Flow**: Classify intent → run structured entity queries → add results to context → LLM formats the answer
- **Evidence**: Structured entity results are returned in `AskResponse`; the LLM does not discover entities from raw text.

---

## References

- [document-intelligence.md](document-intelligence.md) — Document chunking, FTS, Phase A/B
- [architecture/entity-graph.md](architecture/entity-graph.md) — Structured entity cards from tables
