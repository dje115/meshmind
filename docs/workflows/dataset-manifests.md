# MeshMind Dataset Manifests

## Overview

Dataset manifests are the **only** legal input to training. They describe what data is included, with full provenance.

---

## Contents

- `manifest_id`, `created_at`, `tenant_id`
- `dataset_type` (preset name)
- `selection_rules` — Sources, tables, event hash range
- `cas_object_hashes` — CAS refs included
- `schema_snapshot_refs` — Schema versions
- `redaction_rules_applied`
- `counts` / `stats`
- `feature_schema_version`

---

## Presets

- `public_shareable_only` — Only shareable artifacts
- `this_tenant_confirmed` — Confirmed cases for this tenant
- `all_approved_no_restricted` — All approved, exclude Restricted
- `numeric_only` — Numeric facts only

---

## Build Flow

1. Admin calls `POST /admin/datasets/build` (or equivalent)
2. node_datasets builds manifest from:
   - Confirmed cases + outcomes
   - Entity cards
   - Facts
3. Manifest stored in CAS
4. Emit `DATASET_MANIFEST_CREATED` event

---

## References

- `crates/node_datasets` — Manifest builder
- [docs/intelligence/training.md](../intelligence/training.md) — Training consumption
