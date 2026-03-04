# MeshMind Full Review and Roadmap

## Current State Summary

- **Backend**: 19 Rust crates, Axum HTTP API, SQLite + event log storage, mTLS mesh, pull-based replication
- **Frontend**: Tauri v2 + Vite 5 + vanilla JS, store.js for central state
- **API**: `/v1/` prefix, SSE streaming for chat, rate limiting (120/min on ask and chat)
- **Connectors**: SQLite, CSV, JSON, Image (EXIF), Document (PDF, DOCX, TXT, MD, RTF)

## Test Coverage

- Unit tests in each crate (node_api, node_connectors, node_ingest, node_storage, etc.)
- E2E mesh test: `crates/node_app/tests/e2e_mesh.rs`
- Document query integration test: `crates/node_app/tests/e2e_document_queries.rs`
- API HTTP tests: status, peers, search, ask, admin endpoints, research policy denial, train status 404
- Browser E2E: Playwright in `ui/e2e/ask.spec.js` (requires node_app running)

## Issues Found and Fixed

- **Windows toolchain**: `cargo test` fails on Windows when `gcc.exe` is not in PATH (libsqlite3-sys, ring, aws-lc-sys). This is an environment setup issue; use MSYS2/MinGW or WSL for full test runs.
- Research endpoint and policy wiring added; WebBrief events are now projected into SQLite views.

## Next Steps Roadmap

### Short-term

- Stabilise tests on CI (Linux/macOS)
- Improve error UX (loading/retry boundaries)
- Add more seed fixtures (e.g. minimal DOCX) if needed

### Mid-term: OneDrive and Email Integration

#### OneDrive Direct Integration

**Current**: Only `%USERPROFILE%\OneDrive` (local synced folder) is in default scan dirs.

**Direct API** would allow sync without local OneDrive app, access to shared/team files.

1. Add `graph-rs-sdk` or `onedrive-api` to node_connectors
2. Implement `OneDriveConnector`: OAuth2, `inspect_schema` (folders as tables), `ingest_batch` (download via Graph API)
3. Config: `onedrive_client_id`, `onedrive_tenant_id`, `onedrive_refresh_token`
4. Respect Graph API rate limits (429, Retry-After)

#### Email Integration (Outlook/Microsoft 365)

1. Use `graph-rs-sdk` for Mail API: `GET /me/mailFolders/inbox/messages`
2. Implement `EmailConnector`: OAuth2, folders as tables, fetch messages, extract subject/body
3. Optional: ingest attachments (PDF, DOCX) via DocumentConnector
4. Privacy: support filters (date range, folder, sender)

**Shared auth**: OneDrive and Mail use Microsoft Graph; one OAuth2 app can access both.

### Long-term

- Federated learning UI (API exists; expand UI)
- Training progress streaming (SSE or WebSocket)
- API rate limit configuration (per-route, per-IP)
