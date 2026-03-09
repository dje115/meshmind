# MeshMind Filesystem Ingestion Agent

Local-only ingestion agent that watches folders, extracts content, and sends normalized items to MeshMind core over localhost HTTP/JSON.

## Requirements

- Python 3.10+
- MeshMind core running (API at `http://127.0.0.1:PORT`)

## Installation

```bash
cd agents/filesystem_ingestion_agent
pip install -r requirements.txt
```

## Run Locally

```bash
cd agents/filesystem_ingestion_agent
python main.py
```

Environment variables:

- `MESHMIND_API_URL` — MeshMind core API base URL (default: `http://127.0.0.1:9900`)
- `MESHMIND_ADMIN_TOKEN` — Admin token for ingest API (required for publish)
- `WATCH_DIRS` — Comma-separated folder paths to watch (optional; configure via config file)

## Configuration

The main app owns ingestion configuration. Add `[[agent_sources]]` to `meshmind.toml` (see [docs/operations/configuration.md](../../docs/operations/configuration.md)). The agent fetches config via `GET /v1/ingest/agent/config`.

### One-shot (CLI path)

```bash
python main.py --one-shot /path/to/folder [--source-id src-1]
```

### Config from main app

```bash
python main.py --config-from-api
```

Fetches configured sources from the main app and runs one-shot ingest for each. Requires `MESHMIND_ADMIN_TOKEN`.

Local-only. No cloud services. All extraction and OCR run locally (pdftoppm, tesseract when available).

### Ingestion-time LLM helper

When `llm_helper_enabled = true` in a source config, the agent may use a local LLM (e.g. Ollama) for ingestion-time tasks such as document type classification or entity disambiguation. Results are recorded in `llm_helper_used` and `llm_helper_steps` on each `IngestedItem`. See `llm_helper.py` for the interface; current implementation uses stubs until an Ollama (or similar) backend is wired.
