//! Axum localhost API: status, peers, search, ask, conversations, admin endpoints.
//!
//! Endpoints:
//! - GET  /status
//! - GET  /peers
//! - GET  /search?q=
//! - POST /ask
//! - GET  /conversations
//! - POST /conversations
//! - GET  /conversations/:id/messages
//! - POST /conversations/:id/messages
//! - DELETE /conversations/:id
//! - POST /admin/event
//! - POST /admin/scan
//! - POST /admin/ingest
//! - POST /admin/train
//! - GET  /admin/logs
//! - GET  /admin/sources
//! - POST /admin/sources/approve
//! - GET  /admin/models
//! - POST /admin/models/rollback
//! - GET  /admin/datasets

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::{delete, get, post};
use axum::Router;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};

use node_ai::InferenceBackend;
use node_connectors::{
    Connector, CsvFolderConnector, DocumentConnector, ImageConnector, JsonFolderConnector,
    SQLiteConnector,
};
use node_datasets::{DatasetBuildConfig, DatasetPreset};
use node_discovery::{DiscoveryConfig, scan_directory, build_discovered_event};
use node_ingest::{IngestConfig, IngestJob};
use node_mesh::transport::Transport;
use node_mesh::{ConsultConfig, PeerDirectory};
use node_storage::cas::CasStore;
use node_storage::event_log::EventLog;
use node_storage::search;

const STOP_WORDS: &[&str] = &[
    "a", "an", "the", "is", "are", "was", "were", "be", "been", "being",
    "have", "has", "had", "do", "does", "did", "will", "would", "shall",
    "should", "may", "might", "can", "could", "am", "i", "me", "my",
    "we", "our", "you", "your", "he", "she", "it", "they", "them",
    "this", "that", "these", "those", "of", "in", "on", "at", "to",
    "for", "with", "from", "by", "about", "into", "through", "during",
    "before", "after", "and", "but", "or", "not", "no", "if", "then",
    "so", "how", "what", "when", "where", "who", "which", "why",
];

fn to_fts5_query(text: &str) -> String {
    let keywords: Vec<&str> = text
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|w| w.len() > 1 && !STOP_WORDS.contains(&w.to_lowercase().as_str()))
        .collect();
    if keywords.is_empty() {
        text.split_whitespace()
            .next()
            .unwrap_or("*")
            .to_string()
    } else {
        keywords.join(" OR ")
    }
}
use node_trainer::{ModelRegistry, Trainer, TrainingJob, JobStatus};

/// Shared application state for all API handlers.
pub struct AppState {
    pub event_log: RwLock<EventLog>,
    pub cas: CasStore,
    pub db_path: std::path::PathBuf,
    pub peer_dir: RwLock<PeerDirectory>,
    pub backend: Arc<dyn InferenceBackend>,
    pub transport: Option<Arc<dyn Transport>>,
    pub consult_config: ConsultConfig,
    pub node_id: String,
    pub admin_token: String,
    pub scan_dirs: Vec<std::path::PathBuf>,
    pub trainer: Arc<Trainer>,
    pub model_registry: Arc<tokio::sync::Mutex<ModelRegistry>>,
}

pub fn build_router(state: Arc<AppState>) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .route("/status", get(handle_status))
        .route("/peers", get(handle_peers))
        .route("/search", get(handle_search))
        .route("/ask", post(handle_ask))
        .route("/admin/event", post(handle_admin_event))
        .route("/admin/logs", get(handle_admin_logs))
        .route("/admin/sources", get(handle_admin_sources))
        .route("/admin/sources/approve", post(handle_admin_approve_source))
        .route("/admin/train", post(handle_admin_train))
        .route("/admin/models", get(handle_admin_models))
        .route("/admin/models/rollback", post(handle_admin_rollback_model))
        .route("/admin/datasets", get(handle_admin_datasets))
        .route("/admin/scan", post(handle_admin_scan))
        .route("/admin/ingest", post(handle_admin_ingest))
        .route("/conversations", get(handle_list_conversations).post(handle_create_conversation))
        .route("/conversations/:id/messages", get(handle_get_messages).post(handle_send_message))
        .route("/conversations/:id", delete(handle_delete_conversation))
        .layer(cors)
        .with_state(state)
}

// ---------- Data types ----------

#[derive(Serialize, Deserialize)]
struct StatusResponse {
    node_id: String,
    status: String,
    event_count: u64,
    peer_count: usize,
    backend: String,
}

#[derive(Serialize, Deserialize)]
struct PeerInfo {
    node_id: String,
    address: String,
    port: u16,
    state: String,
    capabilities: Vec<String>,
    rtt_ms: Option<u32>,
}

#[derive(Deserialize)]
struct SearchParams {
    q: String,
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    20
}

#[derive(Serialize, Deserialize)]
struct SearchResult {
    result_type: String,
    id: String,
    title: String,
    summary: String,
}

#[derive(Serialize, Deserialize)]
struct AskRequest {
    question: String,
    #[serde(default)]
    max_tokens: Option<u32>,
}

#[derive(Serialize, Deserialize)]
struct AskResponse {
    answer: String,
    confidence: f32,
    model: String,
    context_used: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct AdminEventRequest {
    event_id: String,
    #[allow(dead_code)]
    event_type: String,
    title: String,
    summary: String,
    #[serde(default)]
    tenant_id: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct AdminEventResponse {
    event_id: String,
    event_hash: String,
}

#[derive(Deserialize)]
struct LogParams {
    #[serde(default = "default_log_limit")]
    n: usize,
}

fn default_log_limit() -> usize {
    50
}

#[derive(Serialize)]
struct AuditEntry {
    event_id: String,
    event_type: i32,
    summary: String,
    created_at_ms: i64,
}

// ---------- Admin data types ----------

#[derive(Serialize, Deserialize)]
struct SourceRow {
    source_id: String,
    display_name: String,
    connector_type: i32,
    status: String,
    pii_detected: bool,
    estimated_size_bytes: i64,
}

#[derive(Serialize, Deserialize)]
struct ApproveSourceRequest {
    source_id: String,
    #[serde(default)]
    allowed_tables: Vec<String>,
    #[serde(default)]
    row_limit: u32,
}

#[derive(Serialize, Deserialize)]
struct ApproveSourceResponse {
    event_id: String,
}

#[derive(Serialize, Deserialize)]
struct TrainRequest {
    target: String,
    #[serde(default)]
    dataset_preset: String,
}

#[derive(Serialize, Deserialize)]
struct TrainResponse {
    job_id: String,
    status: String,
    dataset_items: u64,
    dataset_manifest_id: String,
    score: Option<f64>,
    model_version: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct ModelRow {
    model_id: String,
    version: i32,
    promoted: bool,
    rolled_back: bool,
}

#[derive(Serialize, Deserialize)]
struct RollbackModelRequest {
    model_id: String,
    from_version: u32,
    to_version: u32,
    reason: String,
}

#[derive(Serialize, Deserialize)]
struct RollbackModelResponse {
    event_id: String,
}

#[derive(Serialize, Deserialize)]
struct DatasetRow {
    manifest_id: String,
    source_id: String,
    preset: String,
    item_count: i64,
    total_bytes: i64,
}

// ---------- Handlers ----------

async fn handle_status(State(state): State<Arc<AppState>>) -> Json<StatusResponse> {
    let event_count = state.event_log.read().await.event_count();
    let peer_count = state.peer_dir.read().await.all_peers().len();

    Json(StatusResponse {
        node_id: state.node_id.clone(),
        status: "running".into(),
        event_count,
        peer_count,
        backend: state.backend.name().to_string(),
    })
}

async fn handle_peers(State(state): State<Arc<AppState>>) -> Json<Vec<PeerInfo>> {
    let dir = state.peer_dir.read().await;
    let peers = dir
        .all_peers()
        .iter()
        .map(|p| PeerInfo {
            node_id: p.node_id.clone(),
            address: p.address.clone(),
            port: p.port,
            state: p.state.to_string(),
            capabilities: p.capabilities.clone(),
            rtt_ms: p.rtt_ms,
        })
        .collect();
    Json(peers)
}

async fn handle_search(
    State(state): State<Arc<AppState>>,
    Query(params): Query<SearchParams>,
) -> Result<Json<Vec<SearchResult>>, StatusCode> {
    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let fts_query = to_fts5_query(&params.q);
    let hits = search::search_all(&conn, &fts_query, params.limit)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let results = hits
        .into_iter()
        .map(|h| SearchResult {
            result_type: h.hit_type,
            id: h.id,
            title: h.title,
            summary: h.summary,
        })
        .collect();

    Ok(Json(results))
}

async fn handle_ask(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AskRequest>,
) -> Result<Json<AskResponse>, StatusCode> {
    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let fts_query = to_fts5_query(&req.question);
    let context_hits = search::search_all(&conn, &fts_query, 10)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let context_bullets: Vec<String> = context_hits
        .iter()
        .map(|h| {
            let preview: String = h.summary.chars().take(300).collect();
            format!("- [{}] {}: {}", h.hit_type, h.title, preview)
        })
        .collect();

    let prompt = if context_bullets.is_empty() {
        format!(
            "The user asked: \"{}\"\n\nNo matching data was found in the local knowledge base. \
             Tell the user you searched but found no relevant results. Suggest they scan and ingest \
             their local data sources first using the Sources panel, then try again.",
            req.question
        )
    } else {
        format!(
            "Context from the user's local knowledge base:\n{}\n\nQuestion: {}\n\nAnswer based on the context above. Be concise and specific.",
            context_bullets.join("\n"),
            req.question
        )
    };

    let system_prompt = "\
You are MeshMind, a local-first AI assistant running on the user's own machine. \
You have access to the user's local knowledge base containing their ingested documents, \
images (with EXIF/GPS metadata), CSV data, SQLite databases, and other files they have scanned. \
When context is provided, answer based on that data. \
Never say you cannot access the user's files -- you CAN, through the knowledge base. \
If no context was found, explain that the knowledge base doesn't have matching data yet \
and suggest they scan and ingest more sources.";

    let gen_req = node_ai::GenerateRequest {
        prompt,
        system: Some(system_prompt.into()),
        max_tokens: req.max_tokens.unwrap_or(1024),
        ..Default::default()
    };

    let gen_resp = state
        .backend
        .generate(gen_req)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let local_confidence: f32 = if context_bullets.is_empty() { 0.3 } else { 0.7 };

    // If local confidence is low and we have a transport, consult peers
    let mut peer_answers = Vec::new();
    if local_confidence < 0.6 {
        if let Some(ref transport) = state.transport {
            let result = node_mesh::consult::consult_peers(
                transport,
                &state.peer_dir,
                &state.consult_config,
                &state.node_id,
                "public",
                &req.question,
                &context_bullets,
            )
            .await;

            for pa in &result.answers {
                peer_answers.push(format!("[{}] {}", pa.peer_id, pa.answer));
            }

            if let Some(best) = result.best_answer {
                if best.confidence > local_confidence {
                    return Ok(Json(AskResponse {
                        answer: best.answer,
                        confidence: best.confidence,
                        model: format!("peer:{}", best.peer_id),
                        context_used: best.evidence_refs,
                    }));
                }
            }
        }
    }

    let answer = if peer_answers.is_empty() {
        gen_resp.text
    } else {
        format!(
            "{}\n\n--- Peer insights ---\n{}",
            gen_resp.text,
            peer_answers.join("\n")
        )
    };

    Ok(Json(AskResponse {
        answer,
        confidence: local_confidence,
        model: gen_resp.model,
        context_used: context_hits.iter().map(|h| h.id.clone()).collect(),
    }))
}

// ---------- Conversation types ----------

#[derive(Serialize, Deserialize)]
struct ConversationSummary {
    conversation_id: String,
    title: String,
    created_at_ms: i64,
    updated_at_ms: i64,
}

#[derive(Serialize, Deserialize)]
struct MessageResponse {
    message_id: String,
    conversation_id: String,
    role: String,
    content: String,
    context_used: Vec<String>,
    model: String,
    confidence: f32,
    created_at_ms: i64,
}

#[derive(Serialize, Deserialize)]
struct SendMessageRequest {
    content: String,
    #[serde(default)]
    max_tokens: Option<u32>,
}

const MESHMIND_SYSTEM_PROMPT: &str = "\
You are MeshMind, a local-first AI assistant running on the user's own machine. \
You have access to the user's local knowledge base containing their ingested documents, \
images (with EXIF/GPS metadata), CSV data, SQLite databases, and other files they have scanned. \
When context is provided, answer based on that data. \
Never say you cannot access the user's files -- you CAN, through the knowledge base. \
If no context was found, explain that the knowledge base doesn't have matching data yet \
and suggest they scan and ingest more sources. \
When conversation history is provided, use it to maintain continuity and give contextual follow-up answers.";

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

// ---------- Conversation handlers ----------

async fn handle_list_conversations(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<ConversationSummary>>, StatusCode> {
    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut stmt = conn
        .prepare(
            "SELECT conversation_id, title, created_at_ms, updated_at_ms
             FROM conversations_view ORDER BY updated_at_ms DESC LIMIT 100",
        )
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let convos = stmt
        .query_map([], |row| {
            Ok(ConversationSummary {
                conversation_id: row.get(0)?,
                title: row.get(1)?,
                created_at_ms: row.get(2)?,
                updated_at_ms: row.get(3)?,
            })
        })
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .filter_map(|r| r.ok())
        .collect();

    Ok(Json(convos))
}

async fn handle_create_conversation(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ConversationSummary>, StatusCode> {
    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let id = uuid::Uuid::new_v4().to_string();
    let ts = now_ms();

    conn.execute(
        "INSERT INTO conversations_view (conversation_id, title, created_at_ms, updated_at_ms)
         VALUES (?1, 'New conversation', ?2, ?2)",
        rusqlite::params![id, ts],
    )
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(ConversationSummary {
        conversation_id: id,
        title: "New conversation".into(),
        created_at_ms: ts,
        updated_at_ms: ts,
    }))
}

async fn handle_get_messages(
    State(state): State<Arc<AppState>>,
    Path(conv_id): Path<String>,
) -> Result<Json<Vec<MessageResponse>>, StatusCode> {
    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut stmt = conn
        .prepare(
            "SELECT message_id, conversation_id, role, content, context_used, model, confidence, created_at_ms
             FROM messages_view WHERE conversation_id = ?1 ORDER BY created_at_ms ASC",
        )
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let msgs = stmt
        .query_map(rusqlite::params![conv_id], |row| {
            let ctx_json: String = row.get(4)?;
            let context_used: Vec<String> =
                serde_json::from_str(&ctx_json).unwrap_or_default();
            Ok(MessageResponse {
                message_id: row.get(0)?,
                conversation_id: row.get(1)?,
                role: row.get(2)?,
                content: row.get(3)?,
                context_used,
                model: row.get(5)?,
                confidence: row.get(6)?,
                created_at_ms: row.get(7)?,
            })
        })
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .filter_map(|r| r.ok())
        .collect();

    Ok(Json(msgs))
}

async fn handle_delete_conversation(
    State(state): State<Arc<AppState>>,
    Path(conv_id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let msg_ids: Vec<String> = conn
        .prepare("SELECT message_id FROM messages_view WHERE conversation_id = ?1")
        .and_then(|mut s| {
            let ids = s
                .query_map(rusqlite::params![&conv_id], |row| row.get(0))?
                .filter_map(|r| r.ok())
                .collect();
            Ok(ids)
        })
        .unwrap_or_default();

    for mid in &msg_ids {
        let _ = conn.execute("DELETE FROM messages_fts WHERE message_id = ?1", rusqlite::params![mid]);
    }
    let _ = conn.execute(
        "DELETE FROM messages_view WHERE conversation_id = ?1",
        rusqlite::params![conv_id],
    );
    let _ = conn.execute(
        "DELETE FROM conversations_view WHERE conversation_id = ?1",
        rusqlite::params![conv_id],
    );

    Ok(StatusCode::NO_CONTENT)
}

async fn handle_send_message(
    State(state): State<Arc<AppState>>,
    Path(conv_id): Path<String>,
    Json(req): Json<SendMessageRequest>,
) -> Result<Json<MessageResponse>, StatusCode> {
    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let ts = now_ms();
    let user_msg_id = uuid::Uuid::new_v4().to_string();

    // 1. Store user message
    conn.execute(
        "INSERT INTO messages_view (message_id, conversation_id, role, content, created_at_ms)
         VALUES (?1, ?2, 'user', ?3, ?4)",
        rusqlite::params![user_msg_id, conv_id, req.content, ts],
    )
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    conn.execute(
        "INSERT INTO messages_fts (message_id, content) VALUES (?1, ?2)",
        rusqlite::params![user_msg_id, req.content],
    )
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // 2. Load conversation history (last 10 messages)
    let history: Vec<(String, String)> = conn
        .prepare(
            "SELECT role, content FROM messages_view
             WHERE conversation_id = ?1 ORDER BY created_at_ms DESC LIMIT 10",
        )
        .and_then(|mut s| {
            let rows: Vec<(String, String)> = s
                .query_map(rusqlite::params![conv_id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .filter_map(|r| r.ok())
                .collect();
            Ok(rows)
        })
        .unwrap_or_default()
        .into_iter()
        .rev()
        .collect();

    // 3. RAG search on knowledge base
    let fts_query = to_fts5_query(&req.content);
    let context_hits = search::search_all(&conn, &fts_query, 10)
        .unwrap_or_default();

    let context_bullets: Vec<String> = context_hits
        .iter()
        .map(|h| {
            let preview: String = h.summary.chars().take(300).collect();
            format!("- [{}] {}: {}", h.hit_type, h.title, preview)
        })
        .collect();

    // 4. Cross-session search (past assistant answers from OTHER conversations)
    let cross_session: Vec<String> = conn
        .prepare(
            "SELECT m.content FROM messages_fts f
             JOIN messages_view m ON m.message_id = f.message_id
             WHERE f.content MATCH ?1 AND m.role = 'assistant' AND m.conversation_id != ?2
             ORDER BY rank LIMIT 3",
        )
        .and_then(|mut s| {
            let rows: Vec<String> = s
                .query_map(rusqlite::params![&fts_query, &conv_id], |row| row.get(0))?
                .filter_map(|r| r.ok())
                .collect();
            Ok(rows)
        })
        .unwrap_or_default();

    // 5. Build multi-turn prompt
    let mut prompt_parts = Vec::new();

    if !context_bullets.is_empty() {
        prompt_parts.push(format!(
            "Knowledge base context:\n{}",
            context_bullets.join("\n")
        ));
    }

    if !cross_session.is_empty() {
        let cross_bullets: Vec<String> = cross_session
            .iter()
            .map(|a| {
                let preview: String = a.chars().take(200).collect();
                format!("- {}", preview)
            })
            .collect();
        prompt_parts.push(format!(
            "Relevant answers from previous conversations:\n{}",
            cross_bullets.join("\n")
        ));
    }

    // Add conversation history (skip the current user message, it's the last in history)
    let hist_len = history.len();
    if hist_len > 1 {
        let mut hist_lines = Vec::new();
        for (role, content) in &history[..hist_len - 1] {
            let label = if role == "user" { "User" } else { "Assistant" };
            let preview: String = content.chars().take(400).collect();
            hist_lines.push(format!("{}: {}", label, preview));
        }
        prompt_parts.push(format!(
            "Conversation history:\n{}",
            hist_lines.join("\n")
        ));
    }

    prompt_parts.push(format!("User: {}", req.content));

    if context_bullets.is_empty() && cross_session.is_empty() && hist_len <= 1 {
        prompt_parts.push(
            "No matching data was found in the knowledge base. \
             Tell the user you searched but found no relevant results. \
             Suggest they scan and ingest local data sources first."
                .to_string(),
        );
    }

    let prompt = prompt_parts.join("\n\n");

    // 6. Send to LLM
    let gen_req = node_ai::GenerateRequest {
        prompt,
        system: Some(MESHMIND_SYSTEM_PROMPT.into()),
        max_tokens: req.max_tokens.unwrap_or(1024),
        ..Default::default()
    };

    let gen_resp = state
        .backend
        .generate(gen_req)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let confidence: f32 = if context_bullets.is_empty() && cross_session.is_empty() {
        0.3
    } else {
        0.7
    };

    // 7. Store assistant response
    let asst_msg_id = uuid::Uuid::new_v4().to_string();
    let asst_ts = now_ms();
    let ctx_json = serde_json::to_string(
        &context_hits.iter().map(|h| h.id.clone()).collect::<Vec<_>>(),
    )
    .unwrap_or_else(|_| "[]".into());

    conn.execute(
        "INSERT INTO messages_view (message_id, conversation_id, role, content, context_used, model, confidence, created_at_ms)
         VALUES (?1, ?2, 'assistant', ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            asst_msg_id,
            conv_id,
            gen_resp.text,
            ctx_json,
            gen_resp.model,
            confidence,
            asst_ts,
        ],
    )
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    conn.execute(
        "INSERT INTO messages_fts (message_id, content) VALUES (?1, ?2)",
        rusqlite::params![asst_msg_id, gen_resp.text],
    )
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // 8. Auto-title from first user message
    let msg_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM messages_view WHERE conversation_id = ?1",
            rusqlite::params![conv_id],
            |row| row.get(0),
        )
        .unwrap_or(0);

    if msg_count <= 2 {
        let title: String = req.content.chars().take(60).collect();
        let _ = conn.execute(
            "UPDATE conversations_view SET title = ?1, updated_at_ms = ?2 WHERE conversation_id = ?3",
            rusqlite::params![title, asst_ts, conv_id],
        );
    } else {
        let _ = conn.execute(
            "UPDATE conversations_view SET updated_at_ms = ?1 WHERE conversation_id = ?2",
            rusqlite::params![asst_ts, conv_id],
        );
    }

    Ok(Json(MessageResponse {
        message_id: asst_msg_id,
        conversation_id: conv_id,
        role: "assistant".into(),
        content: gen_resp.text,
        context_used: context_hits.iter().map(|h| h.id.clone()).collect(),
        model: gen_resp.model,
        confidence,
        created_at_ms: asst_ts,
    }))
}

async fn handle_admin_event(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AdminEventRequest>,
) -> Result<Json<AdminEventResponse>, StatusCode> {
    use node_proto::common::*;
    use node_proto::events::*;

    let tenant = req.tenant_id.unwrap_or_else(|| "public".into());

    let content_ref = if !req.summary.is_empty() {
        let href = state
            .cas
            .put_bytes("text/plain", req.summary.as_bytes())
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        Some(href)
    } else {
        None
    };

    let event = EventEnvelope {
        event_id: req.event_id.clone(),
        r#type: EventType::CaseCreated as i32,
        node_id: Some(NodeId {
            value: state.node_id.clone(),
        }),
        tenant_id: Some(TenantId { value: tenant }),
        sensitivity: Sensitivity::Public as i32,
        payload: Some(event_envelope::Payload::CaseCreated(CaseCreated {
            case_id: req.event_id.clone(),
            title: req.title,
            summary: req.summary,
            content_ref,
            shareable: false,
        })),
        tags: req.tags,
        ..Default::default()
    };

    let mut log = state.event_log.write().await;
    let stored = log
        .append(event)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    node_storage::projector::apply_event(&conn, &stored)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let event_hash = stored.event_hash.map(|h| h.sha256).unwrap_or_default();

    Ok(Json(AdminEventResponse {
        event_id: req.event_id,
        event_hash,
    }))
}

async fn handle_admin_logs(
    State(state): State<Arc<AppState>>,
    Query(params): Query<LogParams>,
) -> Result<Json<Vec<AuditEntry>>, StatusCode> {
    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut stmt = conn
        .prepare(
            "SELECT event_id, event_type, summary, created_at_ms
             FROM audit_view
             ORDER BY created_at_ms DESC
             LIMIT ?1",
        )
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let entries = stmt
        .query_map([params.n as i64], |row| {
            Ok(AuditEntry {
                event_id: row.get(0)?,
                event_type: row.get(1)?,
                summary: row.get(2)?,
                created_at_ms: row.get(3)?,
            })
        })
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .filter_map(|r| r.ok())
        .collect();

    Ok(Json(entries))
}

async fn handle_admin_sources(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<SourceRow>>, StatusCode> {
    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut stmt = conn
        .prepare(
            "SELECT source_id, display_name, connector_type, status, pii_detected, estimated_size_bytes
             FROM sources_view
             ORDER BY source_id",
        )
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let rows = stmt
        .query_map([], |row| {
            Ok(SourceRow {
                source_id: row.get(0)?,
                display_name: row.get(1)?,
                connector_type: row.get(2)?,
                status: row.get(3)?,
                pii_detected: row.get::<_, i32>(4)? != 0,
                estimated_size_bytes: row.get(5)?,
            })
        })
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .filter_map(|r| r.ok())
        .collect();

    Ok(Json(rows))
}

async fn handle_admin_approve_source(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ApproveSourceRequest>,
) -> Result<Json<ApproveSourceResponse>, StatusCode> {
    use node_proto::common::*;
    use node_proto::events::*;

    let event_id = format!("evt-approve-{}", uuid::Uuid::new_v4());

    let event = EventEnvelope {
        event_id: event_id.clone(),
        r#type: EventType::DataSourceApproved as i32,
        node_id: Some(NodeId {
            value: state.node_id.clone(),
        }),
        tenant_id: Some(TenantId {
            value: "public".into(),
        }),
        sensitivity: Sensitivity::Public as i32,
        payload: Some(event_envelope::Payload::DataSourceApproved(
            DataSourceApproved {
                source_id: req.source_id,
                source_profile_ref: None,
                approved_by: "admin".into(),
                approved_at: None,
                allowed_tables: req.allowed_tables,
                row_limit: req.row_limit,
            },
        )),
        ..Default::default()
    };

    let mut log = state.event_log.write().await;
    let stored = log
        .append(event)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    node_storage::projector::apply_event(&conn, &stored)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(ApproveSourceResponse { event_id }))
}

#[derive(Serialize)]
struct ScanResponse {
    sources_found: usize,
    sources: Vec<ScanSourceInfo>,
}

#[derive(Serialize)]
struct ScanSourceInfo {
    source_id: String,
    display_name: String,
    connector_type: i32,
    path: String,
    estimated_size_bytes: u64,
}

async fn handle_admin_scan(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ScanResponse>, StatusCode> {
    let config = DiscoveryConfig {
        scan_dirs: state.scan_dirs.clone(),
        scan_sqlite: true,
        scan_csv: true,
        scan_json: true,
        scan_images: true,
        scan_documents: true,
    };

    let mut all_sources = Vec::new();
    for dir in &config.scan_dirs {
        let found = scan_directory(dir, &config);
        all_sources.extend(found);
    }

    let mut log = state.event_log.write().await;
    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut result_sources = Vec::new();
    for source in &all_sources {
        let event = build_discovered_event(source, &state.node_id);
        if let Ok(stored) = log.append(event) {
            let _ = node_storage::projector::apply_event(&conn, &stored);
        }
        result_sources.push(ScanSourceInfo {
            source_id: source.source_id.clone(),
            display_name: source.display_name.clone(),
            connector_type: source.connector_type,
            path: source.path.to_string_lossy().into_owned(),
            estimated_size_bytes: source.estimated_size_bytes,
        });
    }

    tracing::info!(dirs = ?state.scan_dirs, found = all_sources.len(), "source scan completed");

    Ok(Json(ScanResponse {
        sources_found: all_sources.len(),
        sources: result_sources,
    }))
}

#[derive(Deserialize)]
struct IngestRequest {
    source_id: String,
}

#[derive(Serialize)]
struct IngestResponse {
    ingest_id: String,
    source_id: String,
    success: bool,
    rows_ingested: u64,
    documents_created: u64,
    bytes_stored: u64,
    duration_ms: u32,
}

async fn handle_admin_ingest(
    State(state): State<Arc<AppState>>,
    Json(req): Json<IngestRequest>,
) -> Result<Json<IngestResponse>, (StatusCode, String)> {
    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}")))?;

    let (connector_type, path_or_uri, status): (i32, String, String) = conn
        .query_row(
            "SELECT connector_type, path_or_uri, status FROM sources_view WHERE source_id = ?1",
            [&req.source_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|_| {
            (
                StatusCode::NOT_FOUND,
                format!("source not found: {}", req.source_id),
            )
        })?;

    if status != "approved" {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("source {} is not approved (status: {})", req.source_id, status),
        ));
    }

    let connector: Box<dyn Connector> = match connector_type {
        1 => Box::new(SQLiteConnector::new("sqlite")),
        2 => Box::new(CsvFolderConnector::new("csv")),
        3 => Box::new(JsonFolderConnector::new("json")),
        7 => Box::new(ImageConnector::new("image")),
        8 => Box::new(DocumentConnector::new("document")),
        other => {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("unsupported connector type: {other}"),
            ))
        }
    };

    let connector_str = match connector_type {
        1 => "sqlite",
        2 => "csv",
        3 => "json",
        7 => "image",
        8 => "document",
        _ => "unknown",
    };

    let source_path = std::path::PathBuf::from(&path_or_uri);

    let tables = connector
        .inspect_schema(&source_path)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("schema inspect failed: {e}")))?;

    let table_names: Vec<String> = tables.iter().map(|t| t.table_name.clone()).collect();

    let ingest_id = format!("ing-{}", uuid::Uuid::new_v4());
    let job = IngestJob {
        ingest_id: ingest_id.clone(),
        source_id: req.source_id.clone(),
        connector_type: connector_str.to_string(),
    };

    let config = IngestConfig::default();
    let node_id = state.node_id.clone();
    let db_path = state.db_path.clone();
    let cas = &state.cas;

    let mut log = state.event_log.write().await;
    let result = node_ingest::run_ingest(
        &job,
        connector.as_ref(),
        &source_path,
        &table_names,
        &config,
        cas,
        &mut log,
        &db_path,
        &node_id,
    )
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("ingest failed: {e}")))?;

    tracing::info!(
        source_id = %req.source_id,
        ingest_id = %ingest_id,
        rows = result.rows_ingested,
        docs = result.documents_created,
        bytes = result.bytes_stored,
        duration_ms = result.duration_ms,
        "ingestion completed"
    );

    Ok(Json(IngestResponse {
        ingest_id: result.ingest_id,
        source_id: result.source_id,
        success: result.success,
        rows_ingested: result.rows_ingested,
        documents_created: result.documents_created,
        bytes_stored: result.bytes_stored,
        duration_ms: result.duration_ms,
    }))
}

async fn handle_admin_train(
    State(state): State<Arc<AppState>>,
    Json(req): Json<TrainRequest>,
) -> Result<Json<TrainResponse>, (StatusCode, String)> {
    use node_proto::common::*;
    use node_proto::events::*;

    let job_id = format!("job-{}", uuid::Uuid::new_v4());

    // 1. Build a dataset manifest from the event log
    let preset = match req.dataset_preset.as_str() {
        "public_shareable_only" => DatasetPreset::PublicShareableOnly,
        "this_tenant_confirmed" => DatasetPreset::ThisTenantConfirmed,
        "all_approved_no_restricted" => DatasetPreset::AllApprovedNoRestricted,
        "numeric_only" => DatasetPreset::NumericOnly,
        other => DatasetPreset::Custom(other.into()),
    };

    let ds_config = DatasetBuildConfig {
        preset,
        source_id: None,
        max_items: 10_000,
        redact_columns: vec![],
    };

    let event_log = state.event_log.read().await;
    let manifest_result = node_datasets::build_dataset(&ds_config, &event_log, &state.cas, &state.node_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("dataset build failed: {e}")))?;
    drop(event_log);

    let dataset_items = manifest_result.total_items;
    let dataset_manifest_id = manifest_result.manifest_id.clone();

    // 2. Record dataset manifest event
    let manifest_event = node_datasets::build_manifest_event(&manifest_result, &ds_config, &state.node_id);
    {
        let mut log = state.event_log.write().await;
        let conn = rusqlite::Connection::open(&state.db_path)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}")))?;
        if let Ok(stored) = log.append(manifest_event) {
            let _ = node_storage::projector::apply_event(&conn, &stored);
        }
    }

    // 3. Record TrainJobStarted event
    let started_event = EventEnvelope {
        event_id: format!("evt-train-start-{}", uuid::Uuid::new_v4()),
        r#type: EventType::TrainJobStarted as i32,
        node_id: Some(NodeId { value: state.node_id.clone() }),
        tenant_id: Some(TenantId { value: "public".into() }),
        sensitivity: Sensitivity::Public as i32,
        payload: Some(event_envelope::Payload::TrainJobStarted(TrainJobStarted {
            job_id: job_id.clone(),
            target: req.target.clone(),
            dataset_manifest_ref: Some(HashRef { sha256: manifest_result.cas_hash.clone() }),
            max_steps: 1000,
            max_minutes: 10,
        })),
        tags: vec![format!("preset:{}", req.dataset_preset)],
        ..Default::default()
    };

    {
        let mut log = state.event_log.write().await;
        let conn = rusqlite::Connection::open(&state.db_path)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}")))?;
        if let Ok(stored) = log.append(started_event) {
            let _ = node_storage::projector::apply_event(&conn, &stored);
        }
    }

    // 4. Create and submit the training job
    let training_manifest = node_trainer::DatasetManifest {
        name: manifest_result.manifest_id.clone(),
        cas_refs: manifest_result.items.iter().map(|i| i.cas_hash.clone()).collect(),
        sample_count: manifest_result.total_items as usize,
    };

    let job = TrainingJob {
        job_id: job_id.clone(),
        model_name: req.target.clone(),
        dataset_manifest: training_manifest,
        max_duration_secs: 600,
        eval_threshold: 0.0,
        status: JobStatus::Queued,
    };

    let job = state.trainer.submit(job);
    if matches!(job.status, JobStatus::Rejected { .. }) {
        let status_msg = match &job.status {
            JobStatus::Rejected { reason } => format!("rejected: {reason}"),
            _ => "rejected".into(),
        };
        return Ok(Json(TrainResponse {
            job_id,
            status: status_msg,
            dataset_items,
            dataset_manifest_id,
            score: None,
            model_version: None,
        }));
    }

    // 5. Run the training job (eval gate + model registration)
    let result = state.trainer.run_job(job).await;

    let (status_str, score, model_version) = match &result.status {
        JobStatus::Completed { score } => {
            let reg = state.model_registry.lock().await;
            let ver = reg.active_version(&req.target).map(|v| v.version.clone());
            ("completed".to_string(), Some(*score), ver)
        }
        JobStatus::Failed { reason } => {
            (format!("failed: {reason}"), None, None)
        }
        other => (format!("{other:?}"), None, None),
    };

    // 6. Record TrainJobCompleted event
    let completed_event = EventEnvelope {
        event_id: format!("evt-train-done-{}", uuid::Uuid::new_v4()),
        r#type: EventType::TrainJobCompleted as i32,
        node_id: Some(NodeId { value: state.node_id.clone() }),
        tenant_id: Some(TenantId { value: "public".into() }),
        sensitivity: Sensitivity::Public as i32,
        payload: Some(event_envelope::Payload::TrainJobCompleted(TrainJobCompleted {
            job_id: job_id.clone(),
            success: score.is_some(),
            notes: status_str.clone(),
            metrics: vec![TrainMetric {
                name: "score".into(),
                value: score.unwrap_or(0.0),
            }],
            model_bundle_ref: Some(HashRef {
                sha256: format!("model-{}", job_id),
            }),
        })),
        ..Default::default()
    };

    {
        let mut log = state.event_log.write().await;
        let conn = rusqlite::Connection::open(&state.db_path)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}")))?;
        if let Ok(stored) = log.append(completed_event) {
            let _ = node_storage::projector::apply_event(&conn, &stored);
        }

        // 7. If training succeeded, emit ModelPromoted event for the projector
        if let Some(ref ver) = model_version {
            let version_num: u32 = ver.trim_start_matches('v')
                .parse()
                .unwrap_or(1);

            let promote_event = EventEnvelope {
                event_id: format!("evt-promote-{}", uuid::Uuid::new_v4()),
                r#type: EventType::ModelPromoted as i32,
                node_id: Some(NodeId { value: state.node_id.clone() }),
                tenant_id: Some(TenantId { value: "public".into() }),
                sensitivity: Sensitivity::Public as i32,
                payload: Some(event_envelope::Payload::ModelPromoted(ModelPromoted {
                    model_id: req.target.clone(),
                    version: version_num,
                    model_bundle_ref: Some(HashRef {
                        sha256: format!("model-{}", job_id),
                    }),
                })),
                ..Default::default()
            };

            if let Ok(stored) = log.append(promote_event) {
                let _ = node_storage::projector::apply_event(&conn, &stored);
            }
        }
    }

    tracing::info!(
        job_id = %job_id,
        status = %status_str,
        dataset_items,
        score = ?score,
        model_version = ?model_version,
        "training pipeline completed"
    );

    Ok(Json(TrainResponse {
        job_id,
        status: status_str,
        dataset_items,
        dataset_manifest_id,
        score,
        model_version,
    }))
}

async fn handle_admin_models(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<ModelRow>>, StatusCode> {
    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut stmt = conn
        .prepare(
            "SELECT model_id, version, promoted, rolled_back
             FROM models_view
             ORDER BY model_id, version",
        )
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let rows = stmt
        .query_map([], |row| {
            Ok(ModelRow {
                model_id: row.get(0)?,
                version: row.get(1)?,
                promoted: row.get::<_, i32>(2)? != 0,
                rolled_back: row.get::<_, i32>(3)? != 0,
            })
        })
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .filter_map(|r| r.ok())
        .collect();

    Ok(Json(rows))
}

async fn handle_admin_rollback_model(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RollbackModelRequest>,
) -> Result<Json<RollbackModelResponse>, StatusCode> {
    use node_proto::common::*;
    use node_proto::events::*;

    let event_id = format!("evt-rollback-{}", uuid::Uuid::new_v4());

    let event = EventEnvelope {
        event_id: event_id.clone(),
        r#type: EventType::ModelRolledBack as i32,
        node_id: Some(NodeId {
            value: state.node_id.clone(),
        }),
        tenant_id: Some(TenantId {
            value: "public".into(),
        }),
        sensitivity: Sensitivity::Public as i32,
        payload: Some(event_envelope::Payload::ModelRolledBack(ModelRolledBack {
            model_id: req.model_id,
            from_version: req.from_version,
            to_version: req.to_version,
            reason: req.reason,
        })),
        ..Default::default()
    };

    let mut log = state.event_log.write().await;
    let stored = log
        .append(event)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    node_storage::projector::apply_event(&conn, &stored)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(RollbackModelResponse { event_id }))
}

async fn handle_admin_datasets(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<DatasetRow>>, StatusCode> {
    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut stmt = conn
        .prepare(
            "SELECT manifest_id, source_id, preset, item_count, total_bytes
             FROM datasets_view
             ORDER BY manifest_id",
        )
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let rows = stmt
        .query_map([], |row| {
            Ok(DatasetRow {
                manifest_id: row.get(0)?,
                source_id: row.get(1)?,
                preset: row.get(2)?,
                item_count: row.get(3)?,
                total_bytes: row.get(4)?,
            })
        })
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .filter_map(|r| r.ok())
        .collect();

    Ok(Json(rows))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use node_ai_mock::MockBackend;
    use node_storage::sqlite_views;
    use tower::ServiceExt;

    fn create_test_state() -> Arc<AppState> {
        let tmp = tempfile::TempDir::new().unwrap();
        let event_log = EventLog::open(tmp.path()).unwrap();
        let cas = CasStore::open(tmp.path()).unwrap();
        let db_path = tmp.path().join("sqlite").join("meshmind.db");
        let _conn = sqlite_views::open_db(&db_path).unwrap();

        // Leak TempDir to keep it alive for the test
        let tmp = Box::leak(Box::new(tmp));
        let _ = tmp;

        let policy = Arc::new(node_policy::PolicyEngine::new(node_policy::PolicyConfig {
            allow_train: true,
            ..Default::default()
        }));
        let model_registry = Arc::new(tokio::sync::Mutex::new(ModelRegistry::new()));
        let trainer = Arc::new(Trainer::new(policy, model_registry.clone()));

        Arc::new(AppState {
            event_log: RwLock::new(event_log),
            cas,
            db_path,
            peer_dir: RwLock::new(PeerDirectory::new()),
            backend: Arc::new(MockBackend::new()),
            transport: None,
            consult_config: ConsultConfig::default(),
            node_id: "test-node-001".into(),
            admin_token: "test-token".into(),
            scan_dirs: vec![],
            trainer,
            model_registry,
        })
    }

    #[tokio::test]
    async fn status_endpoint() {
        let state = create_test_state();
        let app = build_router(state);

        let resp = app
            .oneshot(Request::get("/status").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let status: StatusResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(status.node_id, "test-node-001");
        assert_eq!(status.status, "running");
        assert_eq!(status.backend, "mock");
    }

    #[tokio::test]
    async fn peers_endpoint() {
        let state = create_test_state();
        {
            let mut dir = state.peer_dir.write().await;
            dir.upsert("peer-1", "192.168.1.10", 9000);
        }
        let app = build_router(state);

        let resp = app
            .oneshot(Request::get("/peers").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let peers: Vec<PeerInfo> = serde_json::from_slice(&body).unwrap();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].node_id, "peer-1");
    }

    #[tokio::test]
    async fn search_endpoint_empty() {
        let state = create_test_state();
        let app = build_router(state);

        let resp = app
            .oneshot(Request::get("/search?q=test").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let results: Vec<SearchResult> = serde_json::from_slice(&body).unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn ask_endpoint() {
        let state = create_test_state();
        let app = build_router(state);

        let resp = app
            .oneshot(
                Request::post("/ask")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_string(&AskRequest {
                            question: "hello there".into(),
                            max_tokens: Some(256),
                        })
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let answer: AskResponse = serde_json::from_slice(&body).unwrap();
        assert!(!answer.answer.is_empty());
        assert_eq!(answer.model, "mock-v1");
    }

    #[tokio::test]
    async fn admin_event_endpoint() {
        let state = create_test_state();
        let app = build_router(state);

        let resp = app
            .oneshot(
                Request::post("/admin/event")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_string(&AdminEventRequest {
                            event_id: "evt-test-1".into(),
                            event_type: "case_created".into(),
                            title: "Test Case".into(),
                            summary: "A test case for the API".into(),
                            tenant_id: None,
                            tags: vec!["test".into()],
                        })
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let result: AdminEventResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(result.event_id, "evt-test-1");
        assert!(!result.event_hash.is_empty());
    }

    #[tokio::test]
    async fn admin_logs_endpoint() {
        let state = create_test_state();
        let app = build_router(state);

        let resp = app
            .oneshot(
                Request::get("/admin/logs?n=10")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_admin_sources_empty() {
        let state = create_test_state();
        let app = build_router(state);

        let resp = app
            .oneshot(Request::get("/admin/sources").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let sources: Vec<SourceRow> = serde_json::from_slice(&body).unwrap();
        assert!(sources.is_empty());
    }

    #[tokio::test]
    async fn test_admin_models_empty() {
        let state = create_test_state();
        let app = build_router(state);

        let resp = app
            .oneshot(Request::get("/admin/models").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let models: Vec<ModelRow> = serde_json::from_slice(&body).unwrap();
        assert!(models.is_empty());
    }

    #[tokio::test]
    async fn test_admin_datasets_empty() {
        let state = create_test_state();
        let app = build_router(state);

        let resp = app
            .oneshot(Request::get("/admin/datasets").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let datasets: Vec<DatasetRow> = serde_json::from_slice(&body).unwrap();
        assert!(datasets.is_empty());
    }

    #[tokio::test]
    async fn test_admin_train() {
        let state = create_test_state();
        let app = build_router(state);

        let resp = app
            .oneshot(
                Request::post("/admin/train")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_string(&TrainRequest {
                            target: "router".into(),
                            dataset_preset: "public_shareable_only".into(),
                        })
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let result: TrainResponse = serde_json::from_slice(&body).unwrap();
        assert!(result.job_id.starts_with("job-"));
        assert_eq!(result.status, "completed");
        assert!(result.score.is_some());
        assert!(result.model_version.is_some());
    }
}
