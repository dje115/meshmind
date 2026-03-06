# Policy Examples

Copy/paste examples for common MeshMind policy configurations. Policy is configured via `PolicyConfig` in `node_app` and wired to the policy engine. These examples show the intended configuration pattern; extend `meshmind.toml` as needed to load policy from file.

---

## 1. Public-Only Replication

Only `tenant_id="public"` artifacts can be replicated. No other tenants allowed.

```rust
PolicyConfig {
    allowed_tenant_ids: vec![],
    max_replication_sensitivity: Sensitivity::Internal as i32,  // Public + Internal
    allow_web: false,
    research_web_capable: false,
    allow_train: false,
    allow_ingest: false,
    approved_sources: vec![],
    global_redact_columns: vec![],
    dataset_presets: vec![
        "public_shareable_only".into(),
        "this_tenant_confirmed".into(),
        "all_approved_no_restricted".into(),
        "numeric_only".into(),
    ],
}
```

Effect: `public` and `internal` sensitivity replicate; `restricted` never does. No web research, no training, no ingestion.

---

## 2. Single-Tenant Internal

Internal node for one tenant; replicate only that tenant’s data.

```rust
PolicyConfig {
    allowed_tenant_ids: vec!["acme-corp".into()],
    max_replication_sensitivity: Sensitivity::Internal as i32,
    allow_web: false,
    research_web_capable: false,
    allow_train: true,
    allow_ingest: true,
    approved_sources: vec!["source-db-main".into(), "source-csv-invoices".into()],
    global_redact_columns: vec!["ssn".into(), "api_key".into(), "password".into()],
    dataset_presets: vec![
        "public_shareable_only".into(),
        "this_tenant_confirmed".into(),
        "all_approved_no_restricted".into(),
        "numeric_only".into(),
    ],
}
```

Effect: Replicates `public` and `acme-corp` + `internal`; training and ingestion allowed for approved sources; SSN, api_key, password always redacted.

---

## 3. Enable Web Research via Research Node Only

Research node: web research allowed; normal nodes deny it.

```rust
// On research-capable node:
PolicyConfig {
    allowed_tenant_ids: vec!["public".into()],
    allow_web: true,
    research_web_capable: true,   // This node has web capability
    allow_train: false,
    allow_ingest: false,
    ..Default::default()
}

// On standard nodes:
PolicyConfig {
    allow_web: false,
    research_web_capable: false,
    ..Default::default()
}
```

Effect: Only nodes with `allow_web: true` and `research_web_capable: true` can run web research. Caller must pass `allow_web: true` and `redaction_required: true` in the request.

---

## 4. Allow Training but Block Raw Retention

Training allowed; raw row retention controlled via SourceProfile.

```rust
PolicyConfig {
    allow_train: true,
    allow_ingest: true,
    approved_sources: vec!["source-analytics".into()],
    dataset_presets: vec![
        "public_shareable_only".into(),
        "this_tenant_confirmed".into(),
        "all_approved_no_restricted".into(),
        "numeric_only".into(),
    ],
    ..Default::default()
}
```

Effect: Use `SourceProfile.allow_raw_retention: false` for sensitive sources. Ingest produces entity cards and facts for training, but raw rows are not persisted long-term. Training uses dataset manifests, not raw sources.

---

## 5. Allow Federated Deltas but Deny Raw Data Sharing

Share model updates (deltas) only; no raw data replication.

```rust
PolicyConfig {
    allowed_tenant_ids: vec!["federation-partners".into()],
    allow_train: true,
    allow_ingest: false,
    max_replication_sensitivity: Sensitivity::Internal as i32,
    ..Default::default()
}
```

Effect: `can_share_deltas()` allows federated learning updates. Raw data sharing is blocked by `shareable: false` on artifacts and by not ingesting; only model deltas move between nodes.

---

## Policy Fields Reference

| Field | Description |
|-------|-------------|
| `allowed_tenant_ids` | Tenant IDs allowed for replication (in addition to `public`) |
| `allow_web` | Node-level: web research allowed |
| `research_web_capable` | Node has web research capability |
| `allow_train` | Node-level: training jobs allowed |
| `allow_ingest` | Node-level: data ingestion allowed |
| `approved_sources` | Source IDs allowed for ingestion |
| `max_replication_sensitivity` | 1=Public, 2=Internal, 3=Restricted max |
| `global_redact_columns` | Column names always redacted |
| `dataset_presets` | Presets allowed for dataset building |

---

## References

- `crates/node_policy` — Policy engine implementation
- `docs/spec.md` — Architecture overview
- `docs/ingestion.md` — SourceProfile and approval workflow
