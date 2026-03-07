# MeshMind Crate Architecture

## Crate Map

| Crate | Dependencies | Purpose |
|-------|--------------|---------|
| node_proto | prost | Protobuf types (10 .proto files) |
| node_crypto | rustls, rcgen | mTLS, dev CA, node identity |
| node_storage | rusqlite | CAS, EventLog, SQLite, FTS5, snapshots |
| node_policy | node_proto | Policy evaluation |
| node_repl | node_storage, node_policy | Pull-based replication |
| node_mesh | node_proto, node_crypto | Discovery, transport, consult |
| node_ai | async-trait | InferenceBackend trait |
| node_ai_ollama | node_ai | Ollama HTTP client |
| node_ai_mock | node_ai | Mock backend |
| node_research | node_ai, node_storage | Web fetch, WebBrief |
| node_discovery | node_storage | Scan sources |
| node_connectors | rusqlite, node_policy | Connectors + PII classifier |
| node_ingest | node_connectors, node_storage | Ingestion pipeline |
| node_datasets | node_storage, node_policy | Dataset manifest builder |
| node_trainer | node_policy | Model registry, training jobs |
| node_federated | node_mesh, node_trainer | FedAvg coordinator |
| node_relay | node_proto, node_crypto | Rendezvous + relay server |
| node_api | axum, node_* | HTTP API |
| node_app | all crates | Main binary |

---

## Project Structure

```
meshmind/
├── crates/          # Rust crates
├── proto/           # .proto schemas
├── ui/              # Tauri + Vite UI
├── docs/            # Documentation
├── seed/            # Sample data
├── Cargo.toml
└── meshmind.toml
```

---

## Event Projections

Events from the append-only event log are projected into SQLite materialized views via `projector::apply_event()`. See [architecture/event-log.md](../architecture/event-log.md) and [architecture/storage.md](../architecture/storage.md) for details.

---

## References

- [Adding Connectors](adding-connectors.md)
- [Adding Models](adding-models.md)
- [Testing](testing.md)
