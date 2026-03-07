# Adding a New Connector

## Connector Trait

```rust
pub trait Connector: Send + Sync {
    fn id(&self) -> &str;
    fn inspect_schema(&self, path: &Path) -> Result<Vec<TableInfo>>;
    fn ingest_batch(&self, path: &Path, table: &str, offset: u64, limit: u64) -> Result<IngestBatchResult>;
}
```

---

## Steps

1. **Implement Connector** in `node_connectors`
2. **Register ConnectorType** in `proto/events.proto` (or datasets.proto)
3. **Wire in node_api** — `connector_for_type()` maps type to connector instance
4. **Add discovery** (if file-based) — `node_discovery` scans for new path patterns
5. **Add tests** — Fixture data, schema inspection, ingest_batch

---

## TableInfo

- `table_name`: String
- `columns`: Vec<SchemaColumn> (name, type, sample)
- `row_count_estimate`: Optional

---

## IngestBatchResult

- `table_name`, `offset`
- `rows`: Vec of (entity_id, BTreeMap<column, value>)
- Serialize rows to JSON for CAS storage

---

## References

- `crates/node_connectors` — Existing connectors
- [docs/ingestion/connectors.md](../ingestion/connectors.md) — Connector interface
