# Dev CA Tools

MeshMind uses programmatic dev CA generation via the `node_crypto` crate.
When a node starts, it automatically generates a self-signed CA and node
certificate if none exist in the data directory.

The CA and node certificates are stored at:
- `data/certs/ca.pem` / `data/certs/ca-key.pem`
- `data/certs/node.pem` / `data/certs/node-key.pem`

For manual certificate generation, use `generate_certs.ps1` (Windows)
or run `cargo run -p node_app -- --generate-certs` once the CLI supports it.
