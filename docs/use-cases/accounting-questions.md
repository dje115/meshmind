# Use Case: Accounting Questions

## Problem

Finance and operations need quick answers about invoices, P&L, aging, and trends without digging through spreadsheets.

---

## Example Questions

- "Which invoices are overdue?"
- "Show profit and loss trends"
- "What changed month-on-month?"
- "Revenue vs profit last quarter"
- "What is the average margin by job type?"

---

## How MeshMind Answers

1. **Facts** — Aggregates: overdue count, totals, by-period sums
2. **Entity cards** — Invoice, transaction cards
3. **FTS** — Search for specific invoices, accounts
4. **LLM** — Synthesize with numbers and citations

---

## Data Sources

- Invoices (SQLite, CSV)
- General ledger / transactions
- P&L snapshots (if ingested)

---

## Normalization

- **InvoiceRecord** — Dates, amounts, status, customer
- **ProfitLossSnapshot** — Period, revenue, profit, breakdown
- **FactRecord** — Time-series metrics, aggregates

---

## Training Signals

- CASE_CONFIRMED for accounting queries
- Improves fact selection and formatting

---

## Outputs

- Overdue list, aging report
- P&L summary, trends
- Month-on-month deltas
