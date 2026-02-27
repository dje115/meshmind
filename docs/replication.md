# MeshMind Replication

Pull-based, policy-aware replication.

## Flow

1. Gossip segment metadata and small object hashes
2. Identify missing segments and CAS objects
3. Pull with budgets
4. Verify event hash chain on import
5. Policy gate on acceptance

## Guarantees

- Eventual consistency within tenant boundary
- Hash chain integrity verified end-to-end
- Policy engine decides accept/reject per event and object
