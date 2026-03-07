# Use Case: Customer Intelligence

## Problem

Businesses need to answer questions about customers across systems: who has had X, who is at risk, who is high value, who is inactive.

---

## Example Questions

- "What customers have had fibre installs?"
- "Which customers haven't ordered anything in 12 months?"
- "Which customers are high value / inactive / risky?"
- "What products or services are most common together for customer X?"

---

## How MeshMind Answers

1. **Entity cards** — Customer cards with cross-refs to orders, jobs, support
2. **Facts** — Last order date, order count, ticket count, revenue
3. **FTS** — Search over customer attributes, notes
4. **LLM** — Synthesize answer with evidence

---

## Data Sources

- CRM (customers, contacts)
- Invoices, orders
- Support tickets
- Job history

---

## Normalization

- **CustomerRecord** — customer_id, attributes, derived summary
- **Entity cards** — One per customer with approved columns
- **Facts** — days_since_contact, order_count_12m, ticket_count

---

## Training Signals

- CASE_CONFIRMED for customer queries
- Improves routing and tagging

---

## Outputs

- Customer list with attributes
- Risk/activity indicators
- Suggested outreach
