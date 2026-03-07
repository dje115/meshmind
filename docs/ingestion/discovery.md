# MeshMind Discovery

## Overview

Discovery scans configured directories for data sources and emits `DATA_SOURCE_DISCOVERED` events. It does not ingest; that happens after approval.

---

## Lifecycle

```
Discover (scan dirs) → Classify → Approve → Ingest → Normalize → Learn
```

---

## Configuration

**Scan roots**: `data/scan_roots.json` or default path (e.g. `C:\Users\david\Documents\Meshtest`).

**DiscoveryConfig**:

- `scan_dirs`: Root paths to scan (expanded from scan roots)
- `scan_sqlite`, `scan_csv`, `scan_json`, `scan_images`, `scan_documents`: Toggles per type

---

## Connector Types

| Value | Name | Extension / pattern |
|-------|------|---------------------|
| 1 | SQLITE_DB | `.db`, `.sqlite` |
| 2 | CSV_FOLDER | Folder of `.csv` |
| 3 | JSON_FOLDER | Folder of `.json` |
| 7 | IMAGE_FOLDER | jpg, png, tiff, heic, webp |
| 8 | DOCUMENT_FOLDER | pdf, docx, txt, md, rtf |
| 9 | ONEDRIVE | OAuth-based |

---

## Events

- `DATA_SOURCE_DISCOVERED` → `sources_view` (status = `discovered`)

---

## References

- `crates/node_discovery` — Implementation
- [Connectors](connectors.md) — Inspection and ingest
- [docs/ingestion/normalization.md](normalization.md) — Entity cards, facts
