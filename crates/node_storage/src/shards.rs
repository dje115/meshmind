//! Knowledge shard queries for distributed memory.
//!
//! Shard keys: tenant:X | entity_type:customer | site:UK | artifact_class:document | public

use rusqlite::Connection;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ShardError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

pub type Result<T> = std::result::Result<T, ShardError>;

#[derive(Debug, Clone)]
pub struct ShardRow {
    pub shard_key: String,
    pub shard_kind: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone)]
pub struct ShardMembershipRow {
    pub shard_key: String,
    pub member_type: String,
    pub member_id: String,
    pub node_id: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone)]
pub struct ShardSubscriptionRow {
    pub shard_key: String,
    pub node_id: String,
    pub capability: String,
    pub last_seen_ms: i64,
}

/// List all known shards.
pub fn list_shards(conn: &Connection, limit: usize) -> Result<Vec<ShardRow>> {
    let mut stmt = conn.prepare(
        "SELECT shard_key, shard_kind, created_at_ms FROM shards_view
         ORDER BY created_at_ms DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map([limit as i64], |row| {
        Ok(ShardRow {
            shard_key: row.get(0)?,
            shard_kind: row.get(1)?,
            created_at_ms: row.get(2)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// Peers that host or cache shards relevant to a question. Use for shard-routed consult.
pub fn peers_for_question(conn: &Connection, question: &str) -> Result<Vec<String>> {
    let shard_keys = shards_for_question(conn, question)?;
    let mut node_ids = std::collections::HashSet::new();
    for key in shard_keys {
        let subs = nodes_for_shard(conn, &key, None)?;
        for s in subs {
            if !s.node_id.is_empty()
                && (s.capability == "host" || s.capability == "cache" || s.capability == "query")
            {
                node_ids.insert(s.node_id);
            }
        }
    }
    Ok(node_ids.into_iter().collect())
}

/// List shards relevant to a question (entity_type, tenant, etc. derived from keywords).
pub fn shards_for_question(conn: &Connection, question: &str) -> Result<Vec<String>> {
    let lower = question.to_lowercase();
    let mut relevant = Vec::new();

    // Entity type hints
    for et in ["customer", "invoice", "quote", "account", "job", "order"] {
        if lower.contains(et) {
            relevant.push(format!("entity_type:{}", et));
        }
    }

    // If no hints, return public and common shards
    if relevant.is_empty() {
        relevant.push("public".into());
        relevant.push("tenant:public".into());
    }

    // Filter to shards that exist
    let mut found = Vec::new();
    for key in &relevant {
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM shards_view WHERE shard_key = ?1",
                [key],
                |row| row.get::<_, i32>(0).map(|v| v == 1),
            )
            .unwrap_or(false);
        if exists {
            found.push(key.clone());
        }
    }

    Ok(found)
}

/// List nodes that subscribe to or host a shard.
pub fn nodes_for_shard(
    conn: &Connection,
    shard_key: &str,
    capability_filter: Option<&str>,
) -> Result<Vec<ShardSubscriptionRow>> {
    if let Some(c) = capability_filter.filter(|s| !s.is_empty()) {
        let mut stmt = conn.prepare(
            "SELECT shard_key, node_id, capability, last_seen_ms
             FROM shard_subscriptions_view
             WHERE shard_key = ?1 AND capability = ?2
             ORDER BY last_seen_ms DESC",
        )?;
        let rows = stmt.query_map(rusqlite::params![shard_key, c], |row| {
            Ok(ShardSubscriptionRow {
                shard_key: row.get(0)?,
                node_id: row.get(1)?,
                capability: row.get(2)?,
                last_seen_ms: row.get(3)?,
            })
        })?;
        return Ok(rows.filter_map(|r| r.ok()).collect());
    }
    let mut stmt = conn.prepare(
        "SELECT shard_key, node_id, capability, last_seen_ms
         FROM shard_subscriptions_view
         WHERE shard_key = ?1
         ORDER BY last_seen_ms DESC",
    )?;
    let rows = stmt.query_map([shard_key], |row| {
        Ok(ShardSubscriptionRow {
            shard_key: row.get(0)?,
            node_id: row.get(1)?,
            capability: row.get(2)?,
            last_seen_ms: row.get(3)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

/// List members of a shard.
pub fn members_of_shard(
    conn: &Connection,
    shard_key: &str,
    member_type_filter: Option<&str>,
    limit: usize,
) -> Result<Vec<ShardMembershipRow>> {
    let limit_i = limit as i64;
    if let Some(t) = member_type_filter.filter(|s| !s.is_empty()) {
        let mut stmt = conn.prepare(
            "SELECT shard_key, member_type, member_id, node_id, created_at_ms
             FROM shard_membership_view
             WHERE shard_key = ?1 AND member_type = ?2
             ORDER BY created_at_ms DESC LIMIT ?3",
        )?;
        let rows = stmt.query_map(rusqlite::params![shard_key, t, limit_i], |row| {
            Ok(ShardMembershipRow {
                shard_key: row.get(0)?,
                member_type: row.get(1)?,
                member_id: row.get(2)?,
                node_id: row.get(3)?,
                created_at_ms: row.get(4)?,
            })
        })?;
        return Ok(rows.filter_map(|r| r.ok()).collect());
    }
    let mut stmt = conn.prepare(
        "SELECT shard_key, member_type, member_id, node_id, created_at_ms
         FROM shard_membership_view
         WHERE shard_key = ?1
         ORDER BY created_at_ms DESC LIMIT ?2",
    )?;
    let rows = stmt.query_map(rusqlite::params![shard_key, limit_i], |row| {
        Ok(ShardMembershipRow {
            shard_key: row.get(0)?,
            member_type: row.get(1)?,
            member_id: row.get(2)?,
            node_id: row.get(3)?,
            created_at_ms: row.get(4)?,
        })
    })?;
    Ok(rows.filter_map(|r| r.ok()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projector;
    use crate::sqlite_views;
    use node_proto::common::*;
    use node_proto::events::*;

    fn setup_with_shards() -> Connection {
        use node_proto::events::ArtifactPublished;
        use node_proto::events::ArtifactType;

        let conn = Connection::open_in_memory().unwrap();
        sqlite_views::create_schema(&conn).unwrap();
        let event = node_proto::events::EventEnvelope {
            event_id: "e1".into(),
            r#type: node_proto::events::EventType::CaseCreated as i32,
            ts: Some(Timestamp { unix_ms: 1000 }),
            node_id: Some(NodeId {
                value: "node-1".into(),
            }),
            tenant_id: Some(TenantId {
                value: "public".into(),
            }),
            sensitivity: node_proto::common::Sensitivity::Public as i32,
            payload: Some(event_envelope::Payload::CaseCreated(CaseCreated {
                case_id: "case-1".into(),
                title: "Test".into(),
                summary: "S".into(),
                content_ref: None,
                shareable: true,
            })),
            ..Default::default()
        };
        projector::apply_event(&conn, &event).unwrap();

        let ap_event = node_proto::events::EventEnvelope {
            event_id: "e2".into(),
            r#type: node_proto::events::EventType::ArtifactPublished as i32,
            ts: Some(Timestamp { unix_ms: 2000 }),
            node_id: Some(NodeId {
                value: "node-1".into(),
            }),
            tenant_id: Some(TenantId {
                value: "public".into(),
            }),
            sensitivity: node_proto::common::Sensitivity::Public as i32,
            payload: Some(event_envelope::Payload::ArtifactPublished(
                ArtifactPublished {
                    artifact_id: "art-1".into(),
                    artifact_type: ArtifactType::Document as i32,
                    version: 1,
                    title: "Customer".into(),
                    summary: "".into(),
                    document_subtype: "entity_card".into(),
                    entity_type: "customer".into(),
                    entity_key: "c1".into(),
                    source_ref: "src-1".into(),
                    table_name: "customers".into(),
                    ..Default::default()
                },
            )),
            ..Default::default()
        };
        projector::apply_event(&conn, &ap_event).unwrap();
        conn
    }

    #[test]
    fn list_shards_returns_populated() {
        let conn = setup_with_shards();
        let shards = list_shards(&conn, 50).unwrap();
        assert!(!shards.is_empty());
        assert!(shards
            .iter()
            .any(|s| s.shard_key == "tenant:public" || s.shard_key == "public"));
    }

    #[test]
    fn shards_for_question_customer() {
        let conn = setup_with_shards();
        let keys = shards_for_question(&conn, "which customers have invoices?").unwrap();
        assert!(keys.contains(&"entity_type:customer".to_string()));
    }

    #[test]
    fn members_of_shard_query() {
        let conn = setup_with_shards();
        let members = members_of_shard(&conn, "tenant:public", Some("case"), 10).unwrap();
        assert!(!members.is_empty());
        assert!(members.iter().any(|m| m.member_id == "case-1"));
    }
}
