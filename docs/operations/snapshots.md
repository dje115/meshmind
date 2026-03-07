# MeshMind Snapshots

## Overview

Snapshots capture SQLite state at a point in time for fast recovery. Stored in `data/snapshots/`.

---

## Create Snapshot

1. Create SQLite dump
2. Store dump in CAS
3. Write snapshot file with metadata (last_applied_event_hash, CAS ref)
4. Snapshot file: `snapshot_<hash>.bin`

---

## Restore Snapshot

1. Read snapshot metadata
2. Fetch SQLite dump from CAS
3. Copy to `sqlite/meshmind.db`
4. Replay events from event log *after* `last_applied_event_hash`
5. Apply via projector

---

## When to Use

- **Recovery** — After SQLite corruption
- **Migration** — Move node to new machine
- **Audit** — Point-in-time state

---

## Retention

No automatic retention. Prune old snapshots manually. Keep at least one for recovery.

---

## References

- `crates/node_storage/src/snapshot.rs`
- [docs/architecture/storage.md](../architecture/storage.md) — Rebuild procedure
