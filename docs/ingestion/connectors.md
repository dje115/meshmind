# MeshMind Connectors

## Overview

Connectors inspect schemas and ingest data from approved sources. Each connector implements the `Connector` trait.

---

## Connector Interface

```rust
pub trait Connector: Send + Sync {
    fn id(&self) -> &str;
    fn inspect_schema(&self, path: &Path) -> Result<Vec<TableInfo>>;
    fn ingest_batch(&self, path: &Path, table: &str, offset: u64, limit: u64) -> Result<IngestBatchResult>;
}
```

- **TableInfo**: `table_name`, `columns` (SchemaColumn), `row_count_estimate`
- **IngestBatchResult**: `table_name`, `rows` (entity_id + columns BTreeMap), `offset`

---

## Implemented Connectors

| Connector | Path Type | Tables |
|-----------|-----------|--------|
| SQLiteConnector | `.db`, `.sqlite` | Schema from PRAGMA |
| CsvFolderConnector | Folder of `.csv` | One "table" per CSV file |
| JsonFolderConnector | Folder of `.json` | One "table" per JSON file |
| ImageConnector | Folder of images | EXIF/GPS metadata |
| DocumentConnector | Folder of PDF/DOCX/TXT/MD | Text extraction |
| OneDriveConnector | OAuth | Files/folders via Graph API |

---

## Classification

`classify_column` inspects column names for PII and secrets:

**PII patterns** (case-insensitive): email, phone, address, name, dob, ssn, iban, sort_code, card_number, gps_*, file_path, location  
→ `is_pii: true`, `suggested_sensitivity: 3` (Restricted)

**Secret patterns**: api_key, token, password, secret, credential  
→ `is_secret: true`, `suggested_sensitivity: 3`

**Default**: `suggested_sensitivity: 1` (Public)

---

## SourceProfile Approval

Admin approves via `POST /admin/sources/approve`:

- `allowed_tables`: Tables to ingest (empty = all)
- `row_limit`: Max rows per table (0 = no limit)
- `source_profile_ref`: Optional CAS ref to full profile

SourceProfile in `source_profiles_view`:

- `allowed_tables_json`, `row_limit`
- `allow_raw_retention`, `allow_training`
- `max_sensitivity`, `redaction_policy_json`
- `mapping_rules_json` — See [Entity Cards](entity-cards.md)

---

## References

- `crates/node_connectors` — Implementation
- [Discovery](discovery.md) — Scan and discover
- [Entity Cards](entity-cards.md) — Mapping rules, normalization
