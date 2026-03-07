# MeshMind Business Intelligence

## What Is "Business Intelligence" in MeshMind?

Beyond traditional BI dashboards and ETL pipelines, MeshMind treats **Business Intelligence** as:

> *Structured extraction of entities, facts, and patterns from ingested data—then answering questions, surfacing insights, and training lightweight classifiers—without requiring raw data sharing or centralization.*

Key principles:

1. **Local-first**: Data stays on nodes; only derived artifacts (entity cards, facts, models) are exchanged when policy permits.
2. **Query over summaries**: Search and retrieval operate on normalized Entity Cards and Fact Tables, not raw rows.
3. **Policy-aware**: PII, secrets, and sensitivity levels gate what can be shared and trained on.
4. **Training-ready**: Entity cards and facts feed dataset manifests for on-device training.

---

## Types of Questions MeshMind Answers

| Category | Example Questions |
|----------|-------------------|
| **Customer** | "What customers have had fibre installs?", "Which customers haven't ordered in 12 months?" |
| **Accounting** | "Which invoices are overdue?", "Show revenue vs profit last quarter", "What changed month-on-month?" |
| **Operational** | "Which quotes were won/lost?", "SLA breaches last month?" |
| **Pricing/Quoting** | "What have we historically charged for a 12-port cabling install?", "Similar past quotes for X" |
| **Trend** | "Profit and loss trends", "Most common products together", "High value / inactive / risky customers" |

---

## Entity Types

| Type | Description |
|------|-------------|
| Customer | End customer or client |
| Supplier | Vendor, partner |
| Product | Product or service |
| Invoice | Invoice record |
| Quote | Quote or proposal |
| Job | Work order, project |
| Account | Accounting entity |
| Transaction | Financial transaction |

Entity cards allow cross-system reasoning when shared keys (e.g. customer_id) exist.

---

## Entity Cards

See [docs/ingestion/entity-cards.md](../ingestion/entity-cards.md) for schema and mapping rules.

---

## Fact Tables

Facts store aggregates: counts, sums, averages, time-window metrics. Used for:

- Training signals (e.g. "AR overdue count increased")
- Anomaly baselines
- Compliance checks ("How many records last 30 days?")

---

## Example Use Cases

| Use Case | Data Source | Entity Cards | Facts | Outputs |
|----------|-------------|--------------|-------|---------|
| AR Overdue | invoices | Invoice cards | Overdue count, aging | "Top 5 overdue customers" |
| Churn Risk | CRM, tickets | Customer cards | Ticket count, last_contact_days | Churn risk list |
| Invoice Anomalies | invoices, POs | Invoice + PO cards | Avg amount, std_dev | Anomaly audit list |
| SLA Breaches | tickets | Ticket cards | Resolution time, breach count | SLA reports |
| Compliance | PII tables | Redacted cards | Retention counts | Compliance summaries |
| Forecasting | historical | N/A | Time-series | Simple forecasts |

---

## References

- [docs/ingestion/normalization.md](../ingestion/normalization.md) — Entity cards, facts
- [docs/use-cases/](../use-cases/) — Detailed scenarios
- [docs/workflows/ask-flow.md](../workflows/ask-flow.md) — How questions are answered
