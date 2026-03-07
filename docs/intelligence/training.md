# MeshMind On-Device Training

## Why Training Exists

Training improves:

- **Routing** — Whether to answer locally, ask peers, or do web research
- **Classification** — Tags, sensitivity, entity types
- **Ranking** — Order of retrieved evidence

All training is CPU-feasible, bounded, and rollbackable.

---

## Bounds

All training jobs have hard caps:

- `max_steps`, `max_minutes`, `max_dataset_items`, `max_threads`, `max_bytes`

---

## DatasetManifest

Training **MUST** consume a DatasetManifest; never raw sources directly.

Manifest (CAS-stored) includes:

- `manifest_id`, `created_at`, `tenant_id`
- `dataset_type` (preset name)
- `selection_rules` — Sources, tables, artifacts
- `event_hash_range` — Event range used
- `cas_object_hashes` — CAS refs included
- `schema_snapshot_refs` — Schema versions
- `redaction_rules_applied`
- `counts` / `stats`
- `feature_schema_version`

### Example DatasetManifest (JSON)

```json
{
  "manifest_id": "ds-abc123",
  "created_at": "2025-03-06T12:00:00Z",
  "tenant_id": "public",
  "dataset_type": "this_tenant_confirmed",
  "selection_rules": {
    "sources": ["source-1", "source-2"],
    "event_hash_range": ["hash-start", "hash-end"],
    "artifacts_included": 150
  },
  "cas_object_hashes": ["sha256:abc...", "sha256:def..."],
  "redaction_rules_applied": ["pii_columns_redacted"],
  "counts": { "items": 150, "sources": 2 }
}
```

---

## Training Job Lifecycle

```
queued → running → evaluating → completed | failed
                    ↓
               promoted (optional)
```

| State | Description |
|-------|-------------|
| queued | Job accepted, waiting for resources |
| running | Training in progress |
| evaluating | Eval gate: must beat baseline |
| completed | Job finished, model published |
| failed | Job failed or eval gate failed |
| promoted | Model version promoted to active |

---

## Events

| Event | Purpose |
|-------|---------|
| `TRAIN_JOB_STARTED` | Job began |
| `TRAIN_JOB_COMPLETED` | Job finished |
| `MODEL_PROMOTED` | Model version promoted |
| `MODEL_ROLLED_BACK` | Rollback to prior version |

---

## Model Registry

- **MODEL_BUNDLE** — Trained weights + config in CAS
- **models_view** — model_id, version, model_bundle_hash, promoted, rolled_back

---

## Models

### RouterClassifier

- **Inputs**: Question features, retrieval counts by type, connector availability, peer availability, freshness flags
- **Output**: Action (LOCAL_ONLY / ASK_PEERS / WEB_RESEARCH)
- **Implementation**: Logistic regression or small MLP, JSON-serialized weights

### TaggerClassifier

- **Inputs**: Document text features, schema hints
- **Output**: Tags, suggested sensitivity
- **Implementation**: Bag-of-words + linear, CPU-cheap

### Ranker (if feasible)

- **Purpose**: Rank retrieved evidence using confirmed successful answers

---

## Training Signals

| Signal | Source |
|--------|--------|
| CASE_CONFIRMED | User confirmed answer success |
| WEB_BRIEF_CREATED | Web research produced useful result |
| PEER_ANSWER_RECEIVED | Peer consult succeeded |

---

## Evaluation Gates

- New model must beat baseline by configurable threshold
- Regression test prompts validate behavior
- Policy can deny training (`allow_train: false`)

---

## Rollback

Instant rollback to any previous model version via `MODEL_ROLLED_BACK` event. No retraining required.

---

## References

- `crates/node_trainer` — Job queue, registry, eval
- `crates/node_datasets` — Manifest builder
- [docs/workflows/dataset-manifests.md](../workflows/dataset-manifests.md) — Manifest build flow
- [Router Model](router-model.md) — RouterClassifier details
