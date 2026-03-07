# MeshMind Recovery

## Failure Modes

| Failure | Recovery |
|---------|----------|
| **Hash chain broken** | Restore from last known-good snapshot, replay verified segments |
| **CAS integrity failure** | Re-fetch from peer or re-ingest source |
| **SQLite corruption** | Delete meshmind.db, restore from snapshot or full rebuild |
| **Active log truncated** | Restore from snapshot, replay segments |
| **Out of disk** | Free space; event log is atomic per record |

---

## Full Rebuild (No Snapshot)

1. Create fresh SQLite with `create_schema()`
2. Replay all events via `EventLog::replay()`
3. Apply each via `projector::apply_event()`

---

## From Snapshot

1. Restore snapshot (copies SQLite from CAS)
2. Replay events after `last_applied_event_hash`
3. Verify chain optionally

---

## References

- [docs/architecture/storage.md](../architecture/storage.md) — Failure modes
- [docs/operations/snapshots.md](snapshots.md) — Snapshot create/restore
