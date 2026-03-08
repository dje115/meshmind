# Source Agents

Source agents are separate processes that discover content, extract text, normalize to the shared contract, and POST to MeshMind core. They do **not** own Event Log, CAS, projections, or training.

## Pattern

```
┌─────────────────────────────────────────────────────────────────────────┐
│                        MeshMind Core (Rust)                              │
│  Event Log │ CAS │ Projector │ Ask │ Policies │ Training                 │
└─────────────────────────────────────────────────────────────────────────┘
                                    ▲
                                    │ POST /v1/ingest/items/batch
                                    │ (IngestedItem JSON)
┌─────────────────────────────────────────────────────────────────────────┐
│  Source Agents (separate processes)                                      │
│  ┌──────────────────┐ ┌─────────────────┐ ┌─────────────────┐            │
│  │ FilesystemAgent  │ │ XeroAgent       │ │ OutlookAgent    │  ...       │
│  │ (Python)         │ │ (future)        │ │ (future)        │            │
│  └──────────────────┘ └─────────────────┘ └─────────────────┘            │
└─────────────────────────────────────────────────────────────────────────┘
```

## Agent Responsibilities

Each agent:

1. **Discovers** items from its source (filesystem, API, mailbox, etc.)
2. **Extracts** content locally (no cloud)
3. **Normalizes** into the shared `IngestedItem` contract
4. **Publishes** via `POST /v1/ingest/items/batch` to core
5. **Provenance** — fills `source_locator`, `source_open_target`, `source_origin_label`

## Implemented Agents

| Agent | Location | Source Type | Status |
|-------|----------|-------------|--------|
| Filesystem | `agents/filesystem_ingestion_agent/` | filesystem | Implemented |

## Future Agents

| Agent | Source | Notes |
|-------|--------|-------|
| XeroAgent | Xero API | Invoices, contacts; `xero://` open target |
| OutlookAgent | Microsoft Graph | Mail items; `outlook://` open target |
| OneDriveAgent | OneDrive API | Files; core has OneDriveConnector; could move to agent |

## Contract

All agents use the same `IngestedItem` contract (`node_ingest_contract`, `contract_models.py`):

- `source_id`, `source_type`, `item_id`
- `source_locator`, `source_open_target`, `source_origin_label`
- `extracted_text`, `chunks`, `content_type`, `ocr_used`
- `content_hash`, `source_modified_at`, `ingested_at`

See [INGESTION_AGENT_ARCHITECTURE.md](../INGESTION_AGENT_ARCHITECTURE.md) and [source-provenance.md](../source-provenance.md).

## Adding a New Source Agent

1. Create a new directory under `agents/` (e.g. `agents/xero_ingestion_agent/`)
2. Implement: discovery → extraction → normalization → publish
3. Use `contract_models.py` (or port types) for `IngestedItem`
4. POST to `{MESHMIND_API_URL}/v1/ingest/items/batch` with `Authorization: Bearer {MESHMIND_ADMIN_TOKEN}`
5. Ensure `source_locator`, `source_open_target`, `source_origin_label` are set for provenance
