# MeshMind Documentation Index

Complete index of all documentation sections. Use this as a map to find what you need.

---

## Distributed Memory Fabric

| Document | Description |
|----------|-------------|
| [distributed-memory.md](distributed-memory.md) | Shards, mergeable state, query routing, federated, insights, outcomes, BI |
| [DISTRIBUTED_MEMORY_GAPS.md](DISTRIBUTED_MEMORY_GAPS.md) | Gap analysis and roadmap |
| [proactive-insights.md](proactive-insights.md) | Scheduled insights, alerts, benchmarks |
| [federated-learning.md](federated-learning.md) | Federated rounds, delta aggregation |

---

## Architecture

| Document | Description |
|----------|-------------|
| [architecture/overview.md](architecture/overview.md) | What MeshMind is, hybrid memory engine, design principles |
| [architecture/entity-graph.md](architecture/entity-graph.md) | Entity cards, relationships, entity_cards_view |
| [architecture/storage.md](architecture/storage.md) | Disk layout, integrity, rebuild, failure modes |
| [architecture/event-log.md](architecture/event-log.md) | Event types, hash chain, projection |
| [architecture/cas.md](architecture/cas.md) | Content-addressed storage |
| [architecture/replication.md](architecture/replication.md) | Pull-based replication |
| [architecture/mesh-network.md](architecture/mesh-network.md) | Discovery, transport, membership |
| [architecture/security.md](architecture/security.md) | mTLS, policy, redaction |

---

## Ingestion

| Document | Description |
|----------|-------------|
| [ingestion/discovery.md](ingestion/discovery.md) | Source discovery, scan config |
| [ingestion/connectors.md](ingestion/connectors.md) | Connector interface, SQLite/CSV/JSON/Document |
| [ingestion/normalization.md](ingestion/normalization.md) | Entity cards, facts, document summaries |
| [ingestion/entity-cards.md](ingestion/entity-cards.md) | Entity card schema, mapping rules |
| [INGESTION_AGENT_ARCHITECTURE.md](INGESTION_AGENT_ARCHITECTURE.md) | Agent boundaries, IngestedItem contract, core vs agent responsibilities |
| [source-provenance.md](source-provenance.md) | Source locators, evidence provenance, open targets |

---

## Intelligence

| Document | Description |
|----------|-------------|
| [intelligence/business-intelligence.md](intelligence/business-intelligence.md) | BI concepts, entity types, use cases |
| [intelligence/training.md](intelligence/training.md) | Dataset manifests, job lifecycle, models |
| [intelligence/router-model.md](intelligence/router-model.md) | RouterClassifier |
| [intelligence/tagging-model.md](intelligence/tagging-model.md) | TaggerClassifier |
| [intelligence/ranking-model.md](intelligence/ranking-model.md) | Ranker (optional) |

---

## Workflows

| Document | Description |
|----------|-------------|
| [ask-planner.md](ask-planner.md) | Planner-first ask flow, AskPlan, evidence collection (Phase C) |
| [workflows/ask-flow.md](workflows/ask-flow.md) | Decision ladder, evidence, confirmation |
| [workflows/web-research.md](workflows/web-research.md) | Web fallback, gates, flow |
| [workflows/peer-consult.md](workflows/peer-consult.md) | Peer forwarding, budgets |
| [workflows/dataset-manifests.md](workflows/dataset-manifests.md) | Manifest build, presets |

---

## Use Cases

| Document | Description |
|----------|-------------|
| [use-cases/quoting.md](use-cases/quoting.md) | Quote intelligence, pricing history |
| [use-cases/customer-intelligence.md](use-cases/customer-intelligence.md) | Customer questions, churn, value |
| [use-cases/accounting-questions.md](use-cases/accounting-questions.md) | Invoices, P&L, trends |
| [use-cases/operations-insights.md](use-cases/operations-insights.md) | SLA, suppliers, resolution time |

---

## Operations

| Document | Description |
|----------|-------------|
| [operations/configuration.md](operations/configuration.md) | meshmind.toml, scan roots |
| [operations/snapshots.md](operations/snapshots.md) | Create, restore snapshots |
| [operations/recovery.md](operations/recovery.md) | Failure modes, rebuild |
| [operations/scaling.md](operations/scaling.md) | Single node, LAN, WAN |
| [operations/policy-examples.md](operations/policy-examples.md) | Policy config examples |

---

## Development

| Document | Description |
|----------|-------------|
| [development/crate-architecture.md](development/crate-architecture.md) | Crate map, project structure |
| [development/adding-connectors.md](development/adding-connectors.md) | How to add a connector |
| [development/source-agents.md](development/source-agents.md) | Source agent model (FilesystemAgent, XeroAgent, etc.) |
| [development/adding-models.md](development/adding-models.md) | How to add an ML model |
| [development/testing.md](development/testing.md) | Test structure, commands |

---

## Debug & Operations

| Document | Description |
|----------|-------------|
| [debug-panel.md](debug-panel.md) | Debug UI, ingestion tab, documents/chunks/entities, ask sessions |

---

## Legacy / Root Docs

These remain at `docs/` root for backwards compatibility:

| Document | Description |
|----------|-------------|
| [spec.md](spec.md) | Full architecture specification |
| [protocol.md](protocol.md) | Wire protocol |
| [storage.md](storage.md) | (Superseded by architecture/storage.md) |
| [ingestion.md](ingestion.md) | (Superseded by ingestion/*) |
| [training.md](training.md) | (Superseded by intelligence/training.md) |
| [business-intelligence.md](business-intelligence.md) | (Superseded by intelligence/business-intelligence.md) |
| [use-cases.md](use-cases.md) | (Extended by use-cases/*) |
| [policy-examples.md](policy-examples.md) | Full policy examples |
| [roadmap.md](roadmap.md) | Implementation progress |
| [mesh.md](mesh.md) | (Superseded by architecture/mesh-network.md) |
| [replication.md](replication.md) | (Superseded by architecture/replication.md) |
| [security.md](security.md) | (Superseded by architecture/security.md) |
| [research.md](research.md) | (See workflows/web-research.md) |
| [inference.md](inference.md) | Inference backends |

---

## Quick Links

- **Getting started**: [README](../README.md)
- **Architecture overview**: [architecture/overview.md](architecture/overview.md)
- **Ask flow**: [workflows/ask-flow.md](workflows/ask-flow.md)
- **Training**: [intelligence/training.md](intelligence/training.md)
- **Configuration**: [operations/configuration.md](operations/configuration.md)
