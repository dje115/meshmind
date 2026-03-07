# MeshMind RouterClassifier

## Purpose

The RouterClassifier decides how to handle a question:

- **LOCAL_ONLY** — Answer from local retrieval only
- **ASK_PEERS** — Forward to peers for consult
- **WEB_RESEARCH** — Allow web research fallback (still policy-gated)

---

## Inputs

- Question features (length, keywords, domain hints)
- Retrieval counts by type (cases, artifacts, documents)
- Connector availability
- Peer availability / capability
- Freshness flags
- Prior success/failure labels (from CASE_CONFIRMED)

---

## Outputs

- **Action**: LOCAL_ONLY | ASK_PEERS | WEB_RESEARCH
- **Optional**: Which peer capability to use

---

## Implementation

- CPU-friendly: logistic regression or small MLP
- Weights serialized to JSON
- Stored as MODEL_BUNDLE in CAS
- Integrated into ask pipeline

---

## Training Data

- Derived from CASE_CONFIRMED and successful flows
- Dataset manifest built from confirmed cases + outcomes

---

## References

- [docs/intelligence/training.md](training.md) — Training system
- [docs/workflows/ask-flow.md](../workflows/ask-flow.md) — Integration in ask flow
