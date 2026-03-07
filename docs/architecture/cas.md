# MeshMind CAS (Content-Addressed Storage)

## Overview

CAS stores blobs by their SHA-256 hash. Deduplication is inherent: identical content maps to the same object. Integrity is verified on every read.

---

## Layout

```
objects/sha256/<aa>/<bb>/<full_32_byte_hex_hash>
```

- First 2 hex chars → first subdir (reduces fan-out)
- Next 2 hex chars → second subdir
- Filename = full 64-char hex hash

Example: hash `abcd1234...` → `objects/sha256/ab/cd/abcd1234...`

---

## Operations

| Operation | Description |
|-----------|-------------|
| `put_bytes(bytes)` | Compute SHA-256, write to path, return hash |
| `get_bytes(hash)` | Read from path, verify hash on read, return bytes |
| `exists(hash)` | Check if object exists |

---

## Integrity

On **get**: bytes are read, SHA-256 recomputed. If it does not match the requested hash, `IntegrityFailure` is returned.

---

## Stored Content Types

- Artifact bodies (JSON rows, document text)
- Schema snapshots
- Model bundles (weights, config)
- Dataset manifests
- Web brief summaries

---

## References

- `crates/node_storage/src/cas.rs` — Implementation
- `proto/cas.proto` — CAS object headers (if used)
