//! Proactive insight engine: insights, alerts, benchmarks, anomalies.

use rusqlite::Connection;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum InsightError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

pub type Result<T> = std::result::Result<T, InsightError>;

#[derive(Debug, Clone)]
pub struct InsightRow {
    pub insight_id: String,
    pub insight_type: String,
    pub title: String,
    pub summary: String,
    pub entity_ids_json: String,
    pub confidence: f32,
    pub schedule: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone)]
pub struct AlertRow {
    pub alert_id: String,
    pub alert_type: String,
    pub severity: String,
    pub title: String,
    pub message: String,
    pub entity_ids_json: String,
    pub schedule: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone)]
pub struct BenchmarkRow {
    pub benchmark_id: String,
    pub metric: String,
    pub dimension: String,
    pub value: f64,
    pub time_window: String,
    pub schedule: String,
    pub created_at_ms: i64,
}

/// List insights (optionally by schedule).
pub fn list_insights(
    conn: &Connection,
    schedule_filter: Option<&str>,
    limit: usize,
) -> Result<Vec<InsightRow>> {
    let rows: Vec<InsightRow> = match schedule_filter.filter(|x| !x.is_empty()) {
        Some(s) => {
            let mut stmt = conn.prepare(
                "SELECT insight_id, insight_type, title, summary, entity_ids_json, confidence, schedule, created_at_ms
                 FROM insights_view WHERE schedule = ?1 ORDER BY created_at_ms DESC LIMIT ?2",
            )?;
            let mapped = stmt.query_map(rusqlite::params![s, limit as i64], |row| {
                Ok(InsightRow {
                    insight_id: row.get(0)?,
                    insight_type: row.get(1)?,
                    title: row.get(2)?,
                    summary: row.get(3)?,
                    entity_ids_json: row.get(4)?,
                    confidence: row.get(5)?,
                    schedule: row.get(6)?,
                    created_at_ms: row.get(7)?,
                })
            })?;
            mapped.filter_map(|r| r.ok()).collect()
        }
        None => {
            let mut stmt = conn.prepare(
                "SELECT insight_id, insight_type, title, summary, entity_ids_json, confidence, schedule, created_at_ms
                 FROM insights_view ORDER BY created_at_ms DESC LIMIT ?1",
            )?;
            let mapped = stmt.query_map(rusqlite::params![limit as i64], |row| {
                Ok(InsightRow {
                    insight_id: row.get(0)?,
                    insight_type: row.get(1)?,
                    title: row.get(2)?,
                    summary: row.get(3)?,
                    entity_ids_json: row.get(4)?,
                    confidence: row.get(5)?,
                    schedule: row.get(6)?,
                    created_at_ms: row.get(7)?,
                })
            })?;
            mapped.filter_map(|r| r.ok()).collect()
        }
    };
    Ok(rows)
}

/// List alerts.
pub fn list_alerts(
    conn: &Connection,
    schedule_filter: Option<&str>,
    limit: usize,
) -> Result<Vec<AlertRow>> {
    let rows: Vec<AlertRow> = match schedule_filter.filter(|x| !x.is_empty()) {
        Some(s) => {
            let mut stmt = conn.prepare(
                "SELECT alert_id, alert_type, severity, title, message, entity_ids_json, schedule, created_at_ms
                 FROM alerts_view WHERE schedule = ?1 ORDER BY created_at_ms DESC LIMIT ?2",
            )?;
            let mapped = stmt.query_map(rusqlite::params![s, limit as i64], |row| {
                Ok(AlertRow {
                    alert_id: row.get(0)?,
                    alert_type: row.get(1)?,
                    severity: row.get(2)?,
                    title: row.get(3)?,
                    message: row.get(4)?,
                    entity_ids_json: row.get(5)?,
                    schedule: row.get(6)?,
                    created_at_ms: row.get(7)?,
                })
            })?;
            mapped.filter_map(|r| r.ok()).collect()
        }
        None => {
            let mut stmt = conn.prepare(
                "SELECT alert_id, alert_type, severity, title, message, entity_ids_json, schedule, created_at_ms
                 FROM alerts_view ORDER BY created_at_ms DESC LIMIT ?1",
            )?;
            let mapped = stmt.query_map(rusqlite::params![limit as i64], |row| {
                Ok(AlertRow {
                    alert_id: row.get(0)?,
                    alert_type: row.get(1)?,
                    severity: row.get(2)?,
                    title: row.get(3)?,
                    message: row.get(4)?,
                    entity_ids_json: row.get(5)?,
                    schedule: row.get(6)?,
                    created_at_ms: row.get(7)?,
                })
            })?;
            mapped.filter_map(|r| r.ok()).collect()
        }
    };
    Ok(rows)
}

/// List benchmarks.
pub fn list_benchmarks(
    conn: &Connection,
    schedule_filter: Option<&str>,
    limit: usize,
) -> Result<Vec<BenchmarkRow>> {
    let rows: Vec<BenchmarkRow> = match schedule_filter.filter(|x| !x.is_empty()) {
        Some(s) => {
            let mut stmt = conn.prepare(
                "SELECT benchmark_id, metric, dimension, value, time_window, schedule, created_at_ms
                 FROM benchmarks_view WHERE schedule = ?1 ORDER BY created_at_ms DESC LIMIT ?2",
            )?;
            let mapped = stmt.query_map(rusqlite::params![s, limit as i64], |row| {
                Ok(BenchmarkRow {
                    benchmark_id: row.get(0)?,
                    metric: row.get(1)?,
                    dimension: row.get(2)?,
                    value: row.get(3)?,
                    time_window: row.get(4)?,
                    schedule: row.get(5)?,
                    created_at_ms: row.get(6)?,
                })
            })?;
            mapped.filter_map(|r| r.ok()).collect()
        }
        None => {
            let mut stmt = conn.prepare(
                "SELECT benchmark_id, metric, dimension, value, time_window, schedule, created_at_ms
                 FROM benchmarks_view ORDER BY created_at_ms DESC LIMIT ?1",
            )?;
            let mapped = stmt.query_map(rusqlite::params![limit as i64], |row| {
                Ok(BenchmarkRow {
                    benchmark_id: row.get(0)?,
                    metric: row.get(1)?,
                    dimension: row.get(2)?,
                    value: row.get(3)?,
                    time_window: row.get(4)?,
                    schedule: row.get(5)?,
                    created_at_ms: row.get(6)?,
                })
            })?;
            mapped.filter_map(|r| r.ok()).collect()
        }
    };
    Ok(rows)
}
