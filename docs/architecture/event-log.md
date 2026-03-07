# MeshMind Event Log

## Overview

The Event Log is the **source of truth** for all MeshMind state. Every state change is recorded as an immutable, append-only event with a hash chain for integrity.

---

## Event Envelope

Each event is a protobuf `EventEnvelope`:

| Field | Description |
|-------|-------------|
| `event_id` | Unique identifier (UUID) |
| `type` | EventType enum (case, artifact, source, ingest, etc.) |
| `ts` | Timestamp (unix_ms) |
| `node_id` | Node that created the event |
| `sensitivity` | Public / Internal / Restricted |
| `prev_hash` | Hash of previous event (chain link) |
| `event_hash` | SHA-256 of this event (excluding event_hash) |
| `payload` | OneOf with typed event payload |

---

## Event Types

| Type | Payload | Purpose |
|------|---------|---------|
| `CASE_CREATED` | CaseCreated | New case/runbook |
| `ARTIFACT_PUBLISHED` | ArtifactPublished | Ingested row/doc, model bundle |
| `DATA_SOURCE_DISCOVERED` | DataSourceDiscovered | Scan found a source |
| `DATA_SOURCE_CLASSIFIED` | DataSourceClassified | PII/schema classification |
| `DATA_SOURCE_APPROVED` | DataSourceApproved | Admin approved for ingest |
| `INGEST_STARTED` | IngestStarted | Ingest job began |
| `INGEST_COMPLETED` | IngestCompleted | Ingest job finished |
| `TRAIN_JOB_STARTED` | TrainJobStarted | Training began |
| `TRAIN_JOB_COMPLETED` | TrainJobCompleted | Training finished |
| `MODEL_PROMOTED` | ModelPromoted | Model version promoted |
| `MODEL_ROLLED_BACK` | ModelRolledBack | Rollback to prior version |
| `WEB_BRIEF_CREATED` | WebBriefCreated | Web research result |
| `PEER_DISCOVERED` | PeerDiscovered | Peer node found |
| ... | ... | (see proto/events.proto) |

---

## Hash Chain

```
Event 1: prev_hash="", event_hash=H1
Event 2: prev_hash=H1, event_hash=H2
Event 3: prev_hash=H2, event_hash=H3
...
```

Verification ensures:

1. Each `event_hash` matches recomputed hash of the serialized event.
2. Each `prev_hash` equals the previous event's `event_hash`.

---

## Segment Rotation

When `active.log` exceeds a size threshold (e.g. 4 MB), it is rotated:

1. Rename `active.log` → `segments/segment_N.log` (N = event count)
2. Create new empty `active.log`
3. Continue appending to `active.log`

Segments are never modified; they form an immutable history.

---

## Projection

The **projector** in `node_storage` applies each event to SQLite views:

- `EventLog::replay()` iterates events in order
- `projector::apply_event()` updates `cases_view`, `artifacts_view`, `sources_view`, etc.

Views are always derivable from events; no business logic lives in SQLite alone.

---

## References

- `proto/events.proto` — Full event schema
- `crates/node_storage/src/event_log.rs` — Implementation
- `crates/node_storage/src/projector.rs` — Projection logic
