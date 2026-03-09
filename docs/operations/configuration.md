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

## Agent Sources

Ingestion agents (e.g. filesystem agent) fetch configuration from the main app via `GET /v1/ingest/agent/config` (admin auth). Configure watched folders in `meshmind.toml`:

```toml
[[agent_sources]]
source_id = "docs-inbox"
path = "C:/Users/david/Documents/Inbox"
recursion = true
include_patterns = ["*"]
exclude_patterns = ["*.tmp", "*.bak"]
ocr_enabled = true
llm_helper_enabled = false

[[agent_sources]]
source_id = "reports"
path = "C:/Data/Reports"
recursion = true
```

| Field | Default | Description |
|-------|---------|-------------|
| `source_id` | (required) | MeshMind source identifier |
| `path` | (required) | Root path (filesystem) or endpoint |
| `recursion` | true | Recursive folder scan |
| `include_patterns` | `["*"]` | Glob patterns to include |
| `exclude_patterns` | `[]` | Glob patterns to exclude |
| `max_file_size` | 0 | Max file size in bytes (0 = no limit) |
| `ocr_enabled` | true | OCR for scanned PDFs |
| `llm_helper_enabled` | false | Ingestion-time LLM helper |
| `rate_limit` | 0 | Max items per minute (0 = no limit) |
| `concurrency_limit` | 0 | Max concurrent extractions |
| `retry_limit` | 0 | Retry limit for failed extractions |
| `polling_interval_secs` | 0 | Polling interval when mode is "poll" |

---

## Ingestion Agent Service Control

The main app can start and stop the filesystem ingestion agent (watch mode):

| Endpoint | Method | Description |
|----------|--------|-------------|
| `POST /v1/admin/ingest-agent/start` | Admin | Start the agent (runs `python main.py --watch`) |
| `POST /v1/admin/ingest-agent/stop` | Admin | Stop the agent |
| `GET /v1/admin/ingest-agent/status` | Admin | Returns `{ "status": "running" \| "stopped", "agent_available": true \| false }` |

Config in `meshmind.toml`:
- `ingest_agent_python` — Python executable (default: `"python"`)
- `ingest_agent_script` — Path to `main.py` (default: `./agents/filesystem_ingestion_agent/main.py`)

---

## Settings UI

The Settings page in the UI configures backend, Ollama, mDNS, relay. Changes take effect after restart.

---

## References

- [docs/architecture/security.md](../architecture/security.md) — Policy
- [docs/workflows/ask-flow.md](../workflows/ask-flow.md) — Ask flow
