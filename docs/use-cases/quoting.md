# Use Case: Quoting Intelligence

## Problem

Sales and engineering need to generate quotes that are consistent with historical pricing, similar jobs, and current market reality.

---

## Example Questions

- "What have we usually charged for a 12-port cabling install?"
- "Show similar past quotes for fibre installation"
- "Draft a quote for X in a similar style to job Y"
- "What's the typical margin for this type of work?"

---

## How MeshMind Answers

1. **Retrieve** — Entity cards for past quotes, line items, totals
2. **Facts** — Aggregates: avg amount by job type, pricing ranges
3. **LLM** — Synthesize answer with citations to similar historical jobs
4. **Web research** (optional) — Verify current best practice / market rates when policy allows

---

## Data Sources

- Quote/proposal documents (PDF, DOCX)
- CRM or job system (SQLite, CSV)
- Invoice history (for realized pricing)

---

## Normalization

- **QuoteRecord** — Quote metadata, totals, assumptions, exclusions, sector, outcome
- **QuoteLineItem** — Line-level pricing
- **Entity cards** — Per quote, per customer

---

## Training Signals

- CASE_CONFIRMED when user accepts a quote suggestion
- Improves routing (when to use local vs web for pricing verification)

---

## Outputs

- Confidence score
- Similar historical jobs used
- Assumptions, exclusions
- Variance from historical pricing
