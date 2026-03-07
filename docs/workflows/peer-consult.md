# MeshMind Peer Consult

## Overview

Nodes can forward ASK requests to peers when the router decides local retrieval is insufficient.

---

## Flow

1. Router outputs ASK_PEERS
2. Node selects peer(s) by capability
3. Forwards question with budgets:
   - `ttl_hops` — Max forwarding depth (e.g. 3)
   - `deadline_ms` — Max time to wait
   - `max_context_bytes` — Context size limit
   - `max_answer_bytes` — Answer size limit
4. Peer retrieves locally, generates answer
5. Answer returned to caller
6. Caller may aggregate multiple peer answers

---

## Policy

- Peer must be reachable (Alive)
- Policy gates what can be shared (tenant, sensitivity)
- Budgets prevent runaway recursion

---

## References

- `crates/node_mesh` — Transport, consult
- [docs/architecture/mesh-network.md](../architecture/mesh-network.md) — Discovery, transport
- [docs/workflows/ask-flow.md](ask-flow.md) — Decision ladder
