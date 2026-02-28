# MeshMind Mesh Networking

## Discovery

- **LAN**: mDNS service advertisement and browsing (`_meshmind._tcp.local.`)
- **WAN**: Rendezvous server for Internet-mode peer discovery

## Membership

States: Alive → Suspect → Dead → Quarantined

Partial peer view capped at 30 (configurable).

## Transport

- **Phase 1**: TCP + mTLS (direct LAN connections)
- **Phase 2**: Relay transport (WAN via rendezvous server)
- **HybridTransport**: Tries direct TCP first, falls back to relay
- **Phase 3** (future): QUIC + mTLS (abstraction designed from start)

## Internet Mode

When `relay_addr` and `relay_port` are configured in `meshmind.toml`, the node:
1. Registers with the rendezvous/relay server via mTLS
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

### Wire Protocol

All relay communication uses length-prefixed `RelayWireFrame` over TCP+mTLS.
Message types: Register, Heartbeat, Discover, Relay (envelope forwarding).
