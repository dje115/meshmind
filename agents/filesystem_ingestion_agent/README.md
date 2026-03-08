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

Local-only. No cloud services. All extraction and OCR run locally (pdftoppm, tesseract when available).
