# MeshMind Mesh Networking

## Discovery

- LAN: mDNS service advertisement and browsing
- Internet: Rendezvous server (future phase)

## Membership

States: Alive → Suspect → Dead → Quarantined

Partial peer view capped at 30 (configurable).

## Transport

- Phase 1: TCP + mTLS
- Phase 2: QUIC + mTLS (abstraction designed from start)
