# Federated Learning

## Overview

MeshMind supports federated learning: nodes train locally and share only model deltas (weight updates), not raw data. The coordinator aggregates deltas to produce an improved model. Policy gates ensure training is allowed (`allow_train`, `can_share_deltas`).

## Flow

1. **Start round** — Coordinator calls `POST /v1/admin/federated/rounds` with `model_id`, optional `round_number`, `min_participants`, `max_participants`.
2. **Submit deltas** — Participants (local or remote) train and POST their delta to `POST /v1/admin/federated/rounds/:round_id/deltas`. Delta includes: `delta_id`, `model_id`, `base_version`, `cas_hash` (CAS reference to delta blob), `metrics`, `from_node`.
3. **Aggregate** — When `min_participants` is reached, coordinator calls `POST .../aggregate`. Aggregation averages metrics and stores merged model reference in CAS.
4. **Events** — `FEDERATED_ROUND_STARTED`, `TRAIN_DELTA_PUBLISHED`, `FEDERATED_ROUND_COMPLETED` are recorded in the event log and projected to `federated_view`.

## Cross-Node Participation

Remote peers discover the coordinator via the mesh (relay/peer directory) and POST their delta to the coordinator's API. Requires admin Bearer token. Delta CAS objects must be replicated (e.g. via gossip) so the coordinator can fetch them.

## Configuration

- `FederatedConfig`: `model_id`, `min_participants` (default 2), `max_participants` (default 10), `deadline_seconds`, `aggregation_strategy` (default `fedavg`).
- Policy: `allow_train` must be true for `can_share_deltas()` to allow participation.

## References

- [distributed-memory.md](distributed-memory.md) — Shards, mergeable state, query routing
- [DISTRIBUTED_MEMORY_GAPS.md](DISTRIBUTED_MEMORY_GAPS.md) — Roadmap
