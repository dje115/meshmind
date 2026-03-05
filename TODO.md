# MeshMind TODO List

**Last updated**: 2026-03-04

---

## Completed

- [x] Backend: GET/POST /admin/config/general and POST /admin/restart
- [x] Backend: OAuth flow for OneDrive (start + callback)
- [x] OneDrive connector: support refresh_token only (built-in client_id)
- [x] UI: General tab form + Restart button
- [x] UI: OneDrive tab - Sign in with Microsoft / Disconnect
- [x] Documentation: Azure app registration, redirect URI, restart

---

## OneDrive & Settings

- [ ] OneDrive: full discovery + ingest via Graph API (folders as tables)
- [ ] OneDrive: rate limiting (429, Retry-After)
- [ ] Config: relay_addr, relay_port in General tab (if not already exposed)

---

## Relay Server

- [ ] Add standalone `meshmind-relay` binary (node_relay crate has `run_relay_server()`)
- [ ] Documentation for relay deployment
- [ ] Test HybridTransport with relay fallback in e2e

---

## Email Connector (Outlook/M365 via Graph)

- [ ] Implement `EmailConnector`: OAuth2, folders as tables, fetch messages
- [ ] Privacy filters: date range, folder, sender
- [ ] Optional: ingest attachments (PDF, DOCX) via DocumentConnector

---

## Chat & Web Search

- [ ] Web search triggers (already done: "search the web", "look it up online")
- [ ] Context drift fix (already done: follow-up handling)
- [ ] WebBrief integration in peer consult
- [ ] Peer consult: refine budgets and escalation

---

## Training

- [ ] Replace simulated training with real on-device training
- [ ] Define target: router (local vs peer vs web) or classifier
- [ ] Wire trained model into Ask flow

---

## PDF / Document Quality

- [ ] Extraction improvements if needed (currently using pdf_oxide)
- [ ] OCR for scanned PDFs (optional)

---

## Infrastructure

- [ ] System tray / minimize to tray
- [ ] UI served from exe (resolve ui/dist path)
- [ ] Error UX: loading/retry boundaries
- [ ] Stabilise tests on CI (Linux/macOS)

---

## Documentation

- [ ] Keep HANDOVER.md and REVIEW_AND_ROADMAP.md updated
- [ ] Azure app setup: ensure redirect URI and scopes documented
