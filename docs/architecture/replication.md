# MeshMind Replication

## Overview

Pull-based, policy-aware replication. Nodes exchange event log segments and CAS objects; the policy engine gates what is accepted.

---

## Flow

```
1. Gossip   — Nodes exchange GossipMeta (segment IDs, small CAS hashes)
2. Diff     — Receiver computes missing segments and objects
3. Pull     — PullSegmentsRequest / PullCasObjectsRequest with budgets
4. Verify   — Hash chain verified on imported events
5. Gate     — Policy accepts/denies per event and object
```

---

## Guarantees

- **Eventual consistency** within tenant boundary
- **Hash chain integrity** verified end-to-end
- **Policy engine** decides accept/reject per event and object

---

## Policy Gates

- `tenant_id` — Only allowed tenants replicate
- `sensitivity` — `max_replication_sensitivity` caps what is accepted
- `shareable` — Artifacts must be shareable to replicate

---

## Budgets

Pull requests include budgets to prevent unbounded transfer:

- Max segments
- Max CAS objects
- Max bytes

---

## References

- `crates/node_repl` — Replication implementation
- `crates/node_policy` — Policy gates
- `docs/architecture/security.md` — Security model
