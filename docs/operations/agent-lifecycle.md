# Ingestion Agent Lifecycle

How the main app controls the filesystem ingestion agent: start, stop, and monitor.

## Overview

- **Main app** (MeshMind) owns configuration and process lifecycle
- **Agent** (Python) watches folders, extracts content, POSTs to core
- All communication is localhost HTTP/JSON

## Configuration

Configure the agent in `meshmind.toml`:

```toml
[ingest_agent]
python = "python"           # or "python3"
script = "agents/filesystem_ingestion_agent/main.py"

[[agent_sources]]
source_id = "docs"
path = "C:/Documents/MeshMind"
recursion = true
include_patterns = ["*"]
exclude_patterns = ["*.tmp", ".git"]
ocr_enabled = true
llm_helper_enabled = false
```

See [configuration.md](configuration.md) for full options.

## Starting the Agent

1. **From UI**: Debug → Ingestion → Agent tab → Start
2. **From API**: `POST /v1/admin/ingest-agent/start` (admin auth)
3. **Manually**: `cargo run --release` starts the app; use the UI or API to start the agent

When started, the main app spawns the agent as a child process with `--watch`. The agent fetches config from `GET /v1/ingest/agent/config` and monitors the configured folders.

## Stopping the Agent

1. **From UI**: Debug → Ingestion → Agent tab → Stop
2. **From API**: `POST /v1/admin/ingest-agent/stop` (admin auth)
3. **Implicitly**: When the main app shuts down, the agent process is terminated

## Status

- **API**: `GET /v1/admin/ingest-agent/status` returns `{ status: "running" | "stopped", agent_available: bool }`
- **UI**: Debug → Ingestion → Agent tab shows current status and Start/Stop buttons

## Agent Availability

`agent_available` is true when the main app can locate the agent script (via `ingest_agent_script` in config). If false, Start will fail with an error; configure `ingest_agent_script` in meshmind.toml.

## Troubleshooting

| Symptom | Cause |
|---------|-------|
| Agent not available | `ingest_agent_script` not set or script not found |
| Start fails | Agent script missing, Python not in PATH, or port conflict |
| No files ingested | No `[[agent_sources]]` in config, or paths don't exist |
| Publish errors | Check `MESHMIND_ADMIN_TOKEN`; main app must be running on API URL |
