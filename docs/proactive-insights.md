# Proactive Insight Engine

## Overview

The proactive insight engine generates and stores insights, alerts, benchmarks, and anomalies. Events are persisted to the event log and projected to SQLite views. Scheduling is supported via the `schedule` field (hourly, daily, weekly, monthly, manual).

## Event Types

| Event | Payload | View |
|-------|---------|------|
| INSIGHT_GENERATED | insight_id, insight_type, title, summary, entity_ids, confidence, schedule | insights_view |
| ANOMALY_DETECTED | anomaly_id, metric, dimension, expected/actual value, deviation_pct, schedule | anomalies_view |
| ALERT_RAISED | alert_id, alert_type, severity, title, message, entity_ids, schedule | alerts_view |
| BENCHMARK_UPDATED | benchmark_id, metric, dimension, value, time_window, schedule | benchmarks_view |

## API Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/v1/insights` | GET | Combined: proactive insights from views + on-demand (overdue, quotes, financial) |
| `/v1/insights/alerts` | GET | Alerts (optional `?schedule=`, `?limit=`) |
| `/v1/insights/benchmarks` | GET | Benchmarks (optional `?schedule=`, `?limit=`) |
| `/v1/admin/insights/run` | POST | Trigger insight generation (body: `{ "schedule": "hourly" }`) |

## Scheduling

- **Manual**: Call `POST /admin/insights/run` on demand.
- **Scheduled**: Use an external cron or scheduler to call `POST /admin/insights/run` with `{"schedule": "hourly"}` (or daily/weekly/monthly) at the desired intervals.

## References

- [distributed-memory.md](distributed-memory.md)
- [DISTRIBUTED_MEMORY_GAPS.md](DISTRIBUTED_MEMORY_GAPS.md)
