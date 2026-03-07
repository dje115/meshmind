# MeshMind Ranker

## Purpose

Rank retrieved evidence using confirmed successful answers. Improves relevance of context passed to the LLM.

---

## Inputs

- Retrieved evidence items
- Question features
- Prior success labels (which evidence led to confirmed good answers)

---

## Outputs

- Ranked list of evidence items

---

## Implementation

- Optional in v1; may be deferred
- CPU-feasible (e.g. learned scoring function)

---

## References

- [docs/intelligence/training.md](training.md) — Training system
- [docs/workflows/ask-flow.md](../workflows/ask-flow.md) — Evidence ranking in ask flow
