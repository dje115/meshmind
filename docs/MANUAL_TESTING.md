# MeshMind Manual Testing Guide

## Prerequisites

- **Ollama** running (for real AI responses) or use mock backend
- **node_app** started: `cargo run -p node_app` (listens on http://127.0.0.1:9900)
- Optional: Add local files (Word docs, PDFs, invoices) to a folder for ingestion

## Testing Document Queries

### 1. Add Local Files

Place business-relevant files in a folder, e.g.:

- `~/Documents/invoices/` - PDF or TXT invoices
- `~/Documents/reports/` - Word (.docx) or PDF reports
- Or use `seed/public/documents/` (already contains sample invoice_001.txt, etc.)

Supported formats: **PDF, DOCX, TXT, MD, RTF**.

### 2. Configure Scan Directories

Default scan dirs include `Documents`, `OneDrive`, `Desktop`, `Downloads`.  
To add a custom folder, edit `meshmind.toml` or set `data_dir` / scan paths.

### 3. Scan, Approve, Ingest

1. Open the UI: http://127.0.0.1:9900
2. Go to **Sources**
3. Click **Scan Sources** (or it may auto-scan on load)
4. Approve the sources you want (green checkmark)
5. Click **Ingest All** (or ingest individual sources)

### 4. Ask Questions

In the **Ask** page, try:

- "How many invoices do I have?"
- "How many Word documents do I have?"
- "List my invoices"
- "Summarize my documents"
- "What invoices mention Acme?"

The system uses FTS search over ingested content and an LLM (Ollama) to answer.

## API verification script

From the repo root, run `.\scripts\verify-e2e-flow.ps1` to exercise scan → approve → ingest → ask via the API. Requires `meshmind` built and `ui/dist` present. The script starts the server, runs the flow, and stops it.

## Troubleshooting

- **No context / "no data found"**: Ensure you've scanned, approved, and ingested. Check Sources for approved items.
- **Server crash during ingest**: Rare with pdf_oxide (replaced pdf-extract); if it occurs, try excluding problematic folders from scan dirs.
- **Ollama not responding**: Start Ollama (`ollama serve`) and pull a model (e.g. `ollama pull llama3.2:3b`).
- **Empty search results**: Verify files are in supported formats and were successfully ingested (check ingest logs).
- **PDF content not searchable**: Text-based PDFs are extracted via pdf_oxide. Scanned/image-only PDFs have no embedded text—OCR support would be needed for those.
