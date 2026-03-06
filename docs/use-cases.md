# MeshMind Use Cases

Each scenario follows: **Data Source → Normalization → Retrieval → Training Signals → Outputs**.

---

## 1. Accounts Receivable Overdue

| Phase | Description |
|-------|-------------|
| **Data Source** | SQLite/CSV: `invoices` (customer_id, due_date, amount, status) |
| **Normalization** | Entity cards per invoice; facts: overdue_count, total_aging, by_customer |
| **Retrieval** | FTS + structured: "invoices overdue", "customer X aging" |
| **Training Signals** | Overdue patterns by customer/region; payment velocity |
| **Outputs** | "Top 5 overdue customers", aging report, suggested follow-ups |

---

## 2. Churn Risk Detection

| Phase | Description |
|-------|-------------|
| **Data Source** | CRM contacts + support tickets (customer_id, last_contact, ticket_count) |
| **Normalization** | Customer entity cards; facts: days_since_contact, ticket_count_30d |
| **Retrieval** | "At-risk customers", "no contact 90 days" |
| **Training Signals** | High ticket + no contact → churn; feature combos |
| **Outputs** | Churn risk list, outreach suggestions |

---

## 3. Invoice Anomaly Detection

| Phase | Description |
|-------|-------------|
| **Data Source** | Invoices + purchase orders (amount, vendor, po_ref) |
| **Normalization** | Invoice + PO entity cards; facts: avg_amount, std_dev per vendor |
| **Retrieval** | "Unusual invoices", "vendor X outliers" |
| **Training Signals** | Amount vs baseline; PO mismatch flags |
| **Outputs** | Anomaly audit list, audit trail excerpts |

---

## 4. SLA Breach Reporting

| Phase | Description |
|-------|-------------|
| **Data Source** | Support tickets (created_at, resolved_at, sla_hours) |
| **Normalization** | Ticket entity cards; facts: resolution_time_hours, breach_count |
| **Retrieval** | "SLA breaches", "open tickets past SLA" |
| **Training Signals** | Breach patterns by team/product |
| **Outputs** | SLA reports, escalation lists |

---

## 5. Compliance Checks

| Phase | Description |
|-------|-------------|
| **Data Source** | PII tables, audit logs (access events, retention metadata) |
| **Normalization** | Redacted entity cards; facts: retention counts, access counts |
| **Retrieval** | "PII access last 30 days", "retention policy violations" |
| **Training Signals** | Policy violation patterns |
| **Outputs** | Compliance summaries, audit exports |

---

## 6. Revenue Forecasting

| Phase | Description |
|-------|-------------|
| **Data Source** | Historical invoices (date, amount) |
| **Normalization** | Facts: monthly sums, running averages |
| **Retrieval** | "Revenue last 12 months", "trend" |
| **Training Signals** | Time-series features for simple models |
| **Outputs** | Forecasts (e.g. linear), trend charts |

---

## 7. Document Q&A over Ingested Files

| Phase | Description |
|-------|-------------|
| **Data Source** | PDF, DOCX, TXT in scan folders |
| **Normalization** | Document artifacts (text extracted); entity cards per doc |
| **Retrieval** | FTS over document text |
| **Training Signals** | User confirmations (CASE_CONFIRMED) |
| **Outputs** | Answer with citations, related docs |

---

## 8. Property Management Dashboards

| Phase | Description |
|-------|-------------|
| **Data Source** | Properties, tenants, leases (property_id, tenant_id, rent, lease_end) |
| **Normalization** | Property + tenant + lease entity cards; facts: occupancy, revenue |
| **Retrieval** | "Vacant units", "leases expiring Q2" |
| **Training Signals** | Occupancy trends, renewal patterns |
| **Outputs** | Dashboard summaries, alerts |

---

## 9. Supplier Performance

| Phase | Description |
|-------|-------------|
| **Data Source** | Orders, shipments (supplier_id, delivery_delay, quality_flags) |
| **Normalization** | Supplier entity cards; facts: avg_delay, defect_rate |
| **Retrieval** | "Late suppliers", "quality issues" |
| **Training Signals** | Supplier scoring features |
| **Outputs** | Supplier scorecard, risk list |

---

## 10. Route Optimization Hints

| Phase | Description |
|-------|-------------|
| **Data Source** | Deliveries (address, timestamp, distance) |
| **Normalization** | Delivery entity cards; facts: route_length, stops_per_day |
| **Retrieval** | "Dense areas", "long routes" |
| **Training Signals** | Route efficiency patterns |
| **Outputs** | Suggested route groupings |

---

## 11. Helpdesk Triage

| Phase | Description |
|-------|-------------|
| **Data Source** | Tickets (category, body, assignee, resolution) |
| **Normalization** | Ticket entity cards; facts: category_counts, resolution_time |
| **Retrieval** | Similar tickets, "category X unresolved" |
| **Training Signals** | RouterClassifier: local vs peer vs web; TaggerClassifier for category |
| **Outputs** | Suggested category, routing decision, similar resolved tickets |

---

## 12. Regulatory Report Generation

| Phase | Description |
|-------|-------------|
| **Data Source** | Transactions, entities (amounts, dates, counterparties) |
| **Normalization** | Entity cards; facts: transaction_counts, thresholds |
| **Retrieval** | "Transactions over threshold", "suspicious activity" |
| **Training Signals** | Threshold breach patterns |
| **Outputs** | Report-ready summaries, audit exports |

---

## 13. Knowledge Base Search

| Phase | Description |
|-------|-------------|
| **Data Source** | Runbooks, wikis, internal docs |
| **Normalization** | Document artifacts + entity cards for key concepts |
| **Retrieval** | FTS + semantic search |
| **Training Signals** | CASE_CONFIRMED for better routing |
| **Outputs** | Answer with runbook citations |

---

## 14. Multi-Source Cross-Reference

| Phase | Description |
|-------|-------------|
| **Data Source** | CRM, ERP, support (shared keys: customer_id, order_id) |
| **Normalization** | Entity cards with cross-refs; facts per source |
| **Retrieval** | "Customer X across systems", "order Y status" |
| **Training Signals** | Resolution patterns, data quality |
| **Outputs** | Unified view, discrepancy flags |

---

## References

- `docs/ingestion.md` — Ingest lifecycle
- `docs/business-intelligence.md` — Entity cards, facts
- `docs/training.md` — Training pipeline
