# Adding a New ML Model

## Requirements

- CPU-feasible (no GPU dependency)
- Bounded (steps, minutes, items)
- Serializable to JSON or compact binary
- Stored as MODEL_BUNDLE in CAS

---

## Steps

1. **Define model contract** — Inputs, outputs, weights format
2. **Implement training** in `node_trainer` — Load from DatasetManifest, train, serialize
3. **Implement inference** — Load weights, run forward pass
4. **Wire into pipeline** — e.g. RouterClassifier into ask flow
5. **Add eval gates** — Regression prompts, baseline comparison
6. **Add tests** — Fixture manifest, training job, promotion, rollback

---

## Event Types

- TRAIN_JOB_STARTED
- TRAIN_JOB_COMPLETED
- MODEL_PROMOTED
- MODEL_ROLLED_BACK

---

## Model Registry

- `models_view` — model_id, version, model_bundle_hash, promoted, rolled_back
- Promotion and rollback update view via events

---

## References

- `crates/node_trainer` — Implementation
- [docs/intelligence/training.md](../intelligence/training.md) — Training system
- [docs/intelligence/router-model.md](../intelligence/router-model.md) — RouterClassifier example
