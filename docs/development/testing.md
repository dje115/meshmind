# MeshMind Testing

## Commands

```bash
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

---

## Test Structure

- **Unit tests** — In each crate `#[cfg(test)] mod tests`
- **Integration tests** — `crates/node_app/tests/`, `crates/node_mesh/tests/`
- **E2E** — Multi-node mesh, ask flow, replication

---

## Key Test Areas

| Crate | Focus |
|-------|-------|
| node_proto | Protobuf roundtrip, enum coverage |
| node_storage | CAS, event log, projector, FTS, snapshots |
| node_policy | Tenant, sensitivity, gates |
| node_mesh | Membership, transport, consult |
| node_repl | Gossip, pull, policy gates |
| node_connectors | Inspect, ingest, PII |
| node_api | HTTP endpoints |
| node_trainer | Registry, rollback, eval |
| node_app | Config, e2e mesh |

---

## Fixtures

- Sample SQLite DBs, CSV folders, JSON
- Event log fixtures for projector tests
- Mock transport for mesh tests

---

## References

- README.md — Test coverage table
- `.github/workflows/ci.yml` — CI pipeline
