# MeshMind Security

## Identity

Each node has a unique **NodeId** derived from its TLS certificate fingerprint (SHA-256).

---

## Transport Security

- **mTLS** required for all peer communication
- **Dev CA** for local development
- Certificate pinning planned for production

---

## Policy Engine

Every message carries:

- `tenant_id` — Tenant isolation
- `sensitivity` — Public (1), Internal (2), Restricted (3)
- `shareable` — Whether artifact can be shared

**Default-deny**: Nothing replicates unless explicitly allowed by policy.

---

## Sensitivity Levels

| Level | Name | Replication |
|-------|------|-------------|
| 1 | Public | Allowed (if policy permits) |
| 2 | Internal | Allowed within tenant |
| 3 | Restricted | Blocked by default |

---

## Redaction

- Web research requires `redaction_required: true`
- Redaction rules strip sensitive content before web queries
- PII/secrets detection during classification suggests Restricted

---

## References

- `crates/node_crypto` — mTLS, dev CA
- `crates/node_policy` — Policy evaluation
- `docs/operations/configuration.md` — Policy configuration
