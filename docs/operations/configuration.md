# MeshMind Configuration

## Configuration File

MeshMind reads from `meshmind.toml` in the working directory. All fields are optional.

```toml
data_dir = "./data"
listen = "127.0.0.1:9900"
mesh_port = 9901
backend = "mock"
ollama_endpoint = "http://localhost:11434"
ollama_model = "llama3.2:3b"
admin_token = "auto-generated-if-not-set"
enable_mdns = true
replication_interval_secs = 30
relay_addr = "relay.example.com"
relay_port = 9902
relay_only = false
public_addr = "203.0.113.10:9901"
expose_admin_token = true
```

---

## Default Values

| Key | Default | Description |
|-----|---------|-------------|
| `data_dir` | `./data` | Local storage directory |
| `listen` | `127.0.0.1:9900` | HTTP API address |
| `mesh_port` | 9901 | Mesh TCP port |
| `backend` | mock | Inference backend (mock, ollama) |
| `ollama_endpoint` | http://localhost:11434 | Ollama API URL |
| `ollama_model` | llama3.2:3b | Ollama model name |
| `admin_token` | (random UUID) | Admin API token |
| `enable_mdns` | true | LAN discovery |
| `replication_interval_secs` | 30 | Replication poll interval |

---

## Scan Roots

User-configured scan folders: `data/scan_roots.json`

```json
["C:\\Users\\you\\Documents\\Meshtest"]
```

---

## Settings UI

The Settings page in the UI configures backend, Ollama, mDNS, relay. Changes take effect after restart.

---

## References

- [docs/architecture/security.md](../architecture/security.md) — Policy
- [docs/workflows/ask-flow.md](../workflows/ask-flow.md) — Ask flow
