# MeshMind Security

## Identity

Each node has a unique NodeId derived from its TLS certificate.

## Transport Security

- mTLS required for all peer communication
- Dev CA for local development
- Certificate pinning planned

## Policy Engine

- tenant_id + sensitivity on every message
- Default deny replication except public tenant
- shareable flag controls artifact sharing
- RESTRICTED sensitivity blocks replication

## Redaction

- Web research requires redaction_required=true
- Redaction rules strip sensitive content before web queries
