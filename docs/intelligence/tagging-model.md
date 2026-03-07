# MeshMind TaggerClassifier

## Purpose

Assign business tags and suggested sensitivity/entity types to improve organization and dataset selection.

---

## Inputs

- Document text features
- Schema hints (column names, types)
- Source type

---

## Outputs

- Tags (e.g. "invoice", "customer", "technical")
- Suggested sensitivity (Public / Internal / Restricted)
- Entity type hints

---

## Implementation

- Bag-of-words + linear classifier
- CPU-cheap
- Optional in v1

---

## References

- [docs/intelligence/training.md](training.md) — Training system
