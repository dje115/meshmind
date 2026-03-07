# MeshMind Scaling

## Single Node

- Single `meshmind` process
- Local storage, local inference
- Suitable for personal or small team use

---

## LAN mDNS vs Internet Relay

| Aspect | LAN (mDNS) | WAN (Internet Relay) |
|--------|------------|----------------------|
| Discovery | mDNS on local network | Rendezvous server (relay) |
| Topology | Same subnet / office | Distributed, NAT-traversed |
| Use case | Team, office | Remote teams, hybrid |
| Config | `enable_mdns = true` | `relay_addr`, `relay_only` |

---

## Multi-Node (LAN)

- Multiple nodes on same LAN
- mDNS discovery
- Pull-based replication
- Peer consult for questions
- Suitable for team / office deployment

---

## Internet Mode (WAN)

- Rendezvous + relay server
- Nodes register, discover via relay
- HybridTransport: direct when possible, relay when NAT'd
- Suitable for distributed teams

---

## Limits

- **Peer view**: Capped at 30 (configurable)
- **Replication**: Policy-gated, budget-limited
- **Training**: CPU-bounded, single-node jobs
- **Federated**: Multi-node training coordination (FedAvg)

---

## References

- [docs/architecture/mesh-network.md](../architecture/mesh-network.md) — Discovery, relay
- [docs/architecture/replication.md](../architecture/replication.md) — Replication
