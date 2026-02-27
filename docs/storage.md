# MeshMind Storage

## Overview

Three-tier hybrid memory engine: Event Log → CAS → SQLite.

## CAS (Content-Addressed Store)

- Path: `data/objects/sha256/xx/yy/<hash>`
- Operations: put_bytes, get_bytes, has
- SHA-256 verified on read

## Event Log

- Path: `data/events/active.log` + `segments/` + `index/`
- Length-prefixed protobuf EventEnvelope records
- Hash chain: prev_hash → event_hash
- Segment rotation by size threshold

## SQLite Views

- Path: `data/sqlite/meshmind.db`
- Tables: cases_view, artifacts_view, web_briefs_view, peers_view, models_view, audit_view
- FTS5 full-text search on title, summary, excerpt
- Rebuildable from events + CAS
