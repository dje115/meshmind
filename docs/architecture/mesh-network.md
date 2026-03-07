# MeshMind Mesh Networking

## Discovery

- **LAN**: mDNS service advertisement and browsing (`_meshmind._tcp.local.`)
- **WAN**: Rendezvous server for Internet-mode peer discovery

---

## Membership

States: **Alive** → **Suspect** → **Dead** → **Quarantined**

Partial peer view capped at 30 (configurable `max_peers`).

---

## Transport

| Mode | Description |
|------|-------------|
| **TCP + mTLS** | Direct LAN connections |
| **Relay** | WAN via rendezvous server |
| **HybridTransport** | Tries direct TCP first, falls back to relay |

---

## Internet Mode

When `relay_addr` and `relay_port` are configured in `meshmind.toml`:

1. Node registers with rendezvous/relay server via mTLS
2. Sends periodic heartbeats to keep registration alive
3. Discovers WAN peers through the rendezvous directory
4. Can relay envelopes through the server to reach NAT'd peers

### Configuration

```toml
relay_addr = "relay.example.com"
relay_port = 9902
relay_only = false       # true if node cannot accept direct connections
public_addr = "203.0.113.10:9901"  # externally reachable address (if any)
```

---

## Wire Protocol

All relay communication uses length-prefixed `RelayWireFrame` over TCP+mTLS.

Message types: Register, Heartbeat, Discover, Relay (envelope forwarding).

---

## Peer Consult

Nodes can forward ASK requests to peers:

- **TTL hops** — Max forwarding depth
- **Deadline** — Max time to wait
- **Context budget** — Max bytes for context

---

## References

- `crates/node_mesh` — Discovery, transport, consult
- `crates/node_relay` — Rendezvous + relay server
- `proto/mesh.proto` — Peer envelope types
- `proto/relay.proto` — Relay wire protocol
