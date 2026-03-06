# MeshMind Storage

## Overview

MeshMind uses a three-tier hybrid memory engine: **Event Log → CAS → SQLite**. The Event Log is the immutable source of truth; CAS stores content-addressed blobs; SQLite provides rebuildable materialized views and full-text search.

---

## Disk Layout

All paths are relative to `data_dir` (default `./data` from `meshmind.toml`).

```
data/
├── events/
│   ├── active.log              # Current writable log
│   ├── segments/               # Rotated sealed segments
│   │   └── segment_0000000123.log
│   └── index/                  # (reserved for lightweight indexes)
├── objects/
│   └── sha256/
│       └── <2>/<2>/<full_hash> # CAS objects (e.g. ab/cd/abcdef1234...)
├── sqlite/
│   └── meshmind.db             # Materialized views + FTS5
├── snapshots/                  # Snapshot files for recovery
│   └── snapshot_<hash>.bin
├── identity/                   # Node certs (dev CA or provisioned)
└── scan_roots.json             # User-configured scan folders
```

### Event Log Layout

| Path | Purpose |
|------|---------|
| `events/active.log` | Append-only current log. Length-prefixed protobuf `EventEnvelope` records. |
| `events/segments/` | Rotated segments. Naming: `segment_NNNNNNNNNN.log` where N = event count at rotation. |
| `events/index/` | Reserved for future lightweight indexes. |

### CAS Layout

| Path Pattern | Purpose |
|--------------|---------|
| `objects/sha256/xx/yy/<full_hash>` | Object stored by SHA-256 hash. First 2 chars = subdir 1, next 2 = subdir 2. Limits directory fan-out. |
| `objects/sha256/xx/yy/<hash>.tmp` | Temp file during write; renamed to `<hash>` on success. |

### SQLite Path

| Path | Purpose |
|------|---------|
| `sqlite/meshmind.db` | Single DB file. WAL mode. All views + FTS5 virtual tables. |

---

## Integrity Model

### Hash Chain (Event Log)

Each event has:

- `prev_hash`: SHA-256 of the *previous* event's hash input (deterministic serialization excluding `event_hash`)
- `event_hash`: SHA-256 of the event (excluding `event_hash` itself)

Chain verification:

1. Replay events in order.
2. For each event: `event_hash` must match recomputed hash of serialized form.
3. Each `prev_hash` must equal the previous event's `event_hash`.
4. First event has empty `prev_hash`.

Use `EventLog::verify_chain()` to validate.

### CAS Verification

- On **write**: SHA-256 of content → hex hash used as filename.
- On **read**: Recompute SHA-256 of bytes; fail with `IntegrityFailure` if mismatch.

Deduplication is automatic: identical content maps to the same hash.

---

## Rebuild Procedure

### From Snapshot + Segments

1. **Restore snapshot**: `restore_snapshot(snap_path, cas, db_path)` copies the SQLite dump from CAS to `db_path`. Returns `last_applied_event_hash`.
2. **Replay tail**: Replay events from the event log starting *after* the snapshot's `last_applied_event_hash`. Apply each via `projector::apply_event()`.
3. **Verify**: Optionally run `verify_chain()` on the event log.

### Full Rebuild (No Snapshot)

1. Create a fresh SQLite DB with `create_schema()`.
2. Replay all events via `EventLog::replay()`.
3. Apply each event via `projector::apply_event()`.

---

## Failure Modes and Recovery

| Failure | Symptoms | Recovery Steps |
|---------|----------|----------------|
| **Hash chain broken** | `ChainBroken` error on append or verify | Identify corrupted segment/file. Restore from last known-good snapshot, replay only verified segments. May require manual segment truncation. |
| **CAS integrity failure** | `IntegrityFailure` on `get_bytes()` | Object on disk is corrupted. Re-fetch from a peer (replication) or re-ingest source. |
| **SQLite corruption** | DB open fails, checksum errors | Delete `meshmind.db` (and `-wal`/`-shm`). Restore from snapshot or full rebuild from event log. |
| **Active log truncated** | Partial write, `prev_hash` mismatch | Restore from snapshot. Replay segments + partial active if recoverable. |
| **Out of disk space** | I/O errors on append or CAS put | Free space. Event log append is atomic (length + body); partial writes leave log consistent up to last complete record. |

### Compaction / Retention

- **Event log**: No automatic compaction. Segments are sealed by size (default 4 MB) and never rewritten.
- **CAS**: No GC by default. Objects are content-addressed; orphan detection would require scanning event log and manifests.
- **Snapshots**: Old snapshots can be pruned manually. Keep at least one for recovery.
- **SQLite**: WAL mode; `checkpoint` can be run to truncate WAL. No automatic retention policy.

---

## SQLite Materialized Views

| View | Purpose |
|------|---------|
| `cases_view` | Cases (from CASE_CREATED, etc.) |
| `artifacts_view` | Ingested artifacts, document versions |
| `web_briefs_view` | Web research briefs with citations |
| `peers_view` | Discovered peer nodes |
| `models_view` | Trained model versions |
| `audit_view` | Event audit trail |
| `sources_view` | Discovered data sources |
| `source_profiles_view` | Approval profiles per source |
| `ingests_view` | Ingest job history |
| `datasets_view` | Dataset manifest metadata |
| `federated_view` | Federated learning rounds |
| `conversations_view` | Chat conversations |
| `messages_view` | Chat messages |

### FTS5 Tables

- `cases_fts` — case_id, title, summary, tags
- `artifacts_fts` — artifact_id, title, summary
- `messages_fts` — message_id, content

---

## References

- `crates/node_storage/src/event_log.rs` — Event log implementation
- `crates/node_storage/src/cas.rs` — CAS implementation
- `crates/node_storage/src/snapshot.rs` — Snapshot create/restore
- `crates/node_storage/src/projector.rs` — Event → view projection
