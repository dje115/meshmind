# Use Case: Operations Insights

## Problem

Operations teams need visibility into SLA breaches, supplier performance, capacity, and operational anomalies.

---

## Example Questions

- "Which tickets breached SLA last month?"
- "Show supplier delivery performance"
- "What's our average resolution time by category?"
- "Unusual patterns in job completion?"

---

## How MeshMind Answers

1. **Entity cards** — Tickets, suppliers, jobs
2. **Facts** — Resolution time, breach count, delay averages
3. **FTS** — Search tickets, jobs
4. **LLM** — Synthesize with evidence

---

## Data Sources

- Support tickets
- Supplier/order data
- Job logs
- SLA definitions

---

## Normalization

- **Entity cards** — Ticket, supplier, job
- **Facts** — resolution_time_hours, breach_count, avg_delay

---

## Training Signals

- CASE_CONFIRMED for operational queries
- Anomaly detection improvements

---

## Outputs

- SLA breach list
- Supplier scorecard
- Resolution time reports
- Anomaly flags
