# MeshMind Business Intelligence

## What Is "Business Intelligence" in MeshMind?

Beyond traditional BI dashboards and ETL pipelines, MeshMind treats **Business Intelligence** as:

> *Structured extraction of entities, facts, and patterns from ingested data—then answering questions, surfacing insights, and training lightweight classifiers—without requiring raw data sharing or centralization.*

Key principles:

1. **Local-first**: Data stays on nodes; only derived artifacts (entity cards, facts, models) are exchanged when policy permits.
2. **Query over summaries**: Search and retrieval operate on normalized Entity Cards and Fact Tables, not raw rows.
3. **Policy-aware**: PII, secrets, and sensitivity levels gate what can be shared and trained on.
4. **Training-ready**: Entity cards and facts feed dataset manifests for on-device training (routing, tagging, anomaly detection).

---

## Entity Cards

An **Entity Card** is a normalized document representing a single business entity from ingested sources.

| Field | Description |
|-------|-------------|
| `entity_type` | customer, supplier, invoice, order, property, tenant, etc. |
| `entity_key` | Stable identifier (e.g. customer_id, invoice_no) |
| `attributes` | Approved columns only (redacted per SourceProfile) |
| `derived_summary` | Optional text summary for retrieval (rule-based or LLM) |
| `source_ref` | Origin (source_id, table, row) |
| `content_ref` | CAS hash of full card body (markdown or JSON) |

**Purpose**: Enables semantic search ("Find invoices for customer X") and consistent representation across tables. Stored in CAS; metadata in `documents_view`.

---

## Fact Tables

**Fact Tables** store simple numeric aggregates produced per ingestion run:

- **Counts**: row counts per table, entity counts per type
- **Sums / Avg / Min / Max**: for numeric columns (amounts, quantities)
- **Time-window counts**: if timestamp columns exist (e.g. invoices per month)

Each fact is a small JSON blob in CAS, indexed in `facts_view` for quick lookup. Used for:

- Training signals (e.g. "AR overdue count increased")
- Anomaly baselines
- Compliance checks ("How many records last 30 days?")

---

## Example Use Cases

| Use Case | Data Source | Entity Cards | Facts | Training Signals | Outputs |
|----------|-------------|--------------|-------|------------------|---------|
| **AR Overdue** | invoices table | Invoice cards (customer, due_date, amount) | Overdue count, total aging | Patterns of overdue by customer/region | "Top 5 overdue customers" answer, alerts |
| **Churn Risk** | CRM, support tickets | Customer cards | Ticket count, last_contact_days | High ticket + no contact → churn | Churn risk scores, suggested outreach |
| **Invoice Anomalies** | invoices, POs | Invoice + PO cards | Avg amount, std_dev | Outlier amounts | Anomaly flags, audit list |
| **SLA Breaches** | tickets, timestamps | Ticket cards | Resolution time, breach count | Breach patterns | SLA reports, escalation lists |
| **Compliance Checks** | PII tables, audit logs | Redacted entity cards | Retention counts | Policy violations | Compliance summaries |
| **Forecasting** | historical facts | N/A | Time-series counts/amounts | Trend features | Simple forecasts (e.g. linear) |

These scenarios rely on:

- **Entity Cards** for "who/what" (searchable, retrievable).
- **Fact Tables** for "how much" (aggregates, trends).
- **Training** for routing ("local only" vs "ask peers") and tagging (sensitivity, type).
- **Policy** to gate what data is ingested, retained, and trained on.

---

## References

- `docs/ingestion.md` — Ingest lifecycle, SourceProfile
- `docs/training.md` — Dataset manifests, training pipeline
- `docs/policy-examples.md` — Policy configuration
