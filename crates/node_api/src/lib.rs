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
//! - POST /admin/train/export
//! - POST /admin/train/modelfile
//! - GET  /admin/logs
//! - GET  /admin/sources
//! - POST /admin/sources/approve
//! - GET  /admin/models
//! - POST /admin/models/rollback
//! - GET  /admin/datasets

use std::sync::Arc;

use axum::extract::ws::{WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, Sse};
use axum::response::{Json, Response};
use axum::routing::{delete, get, post};
use axum::Router;
use futures_util::StreamExt;
use governor::{Quota, RateLimiter};
use nonzero_ext::nonzero;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tower_http::cors::CorsLayer;

use node_ai::InferenceBackend;
use node_connectors::{
    Connector, CsvFolderConnector, DocumentConnector, ImageConnector, JsonFolderConnector,
    SQLiteConnector,
};

fn connector_for_type(connector_type: i32) -> Option<(Box<dyn Connector>, &'static str)> {
    let (connector, name) = match connector_type {
        1 => (Box::new(SQLiteConnector::new("sqlite")) as Box<dyn Connector>, "sqlite"),
        2 => (Box::new(CsvFolderConnector::new("csv")) as Box<dyn Connector>, "csv"),
        3 => (Box::new(JsonFolderConnector::new("json")) as Box<dyn Connector>, "json"),
        7 => (Box::new(ImageConnector::new("image")) as Box<dyn Connector>, "image"),
        8 => (Box::new(DocumentConnector::new("document")) as Box<dyn Connector>, "document"),
        _ => return None,
    };
    Some((connector, name))
}
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

/// Phrases that indicate the user wants a web search.
const WEB_SEARCH_TRIGGERS: &[&str] = &[
    "search the web",
    "search the internet",
    "look it up online",
    "look it up on the web",
    "google it",
    "search online",
    "find it online",
    "look up online",
    "search for it",
];

/// Short phrases that indicate a follow-up (continue previous topic).
const FOLLOW_UP_PHRASES: &[&str] = &[
    "yes please",
    "yes, please",
    "tell me more",
    "elaborate",
    "go on",
    "continue",
    "more details",
    "more info",
    "expand on that",
    "can you elaborate",
    "what else",
];

fn wants_web_search(content: &str) -> bool {
    let lower = content.to_lowercase();
    WEB_SEARCH_TRIGGERS.iter().any(|t| lower.contains(t))
}

fn extract_search_query(content: &str, history: &[(String, String)]) -> Option<String> {
    let lower = content.to_lowercase();
    for trigger in WEB_SEARCH_TRIGGERS {
        if let Some(pos) = lower.find(trigger) {
            let mut rest = content[pos + trigger.len()..].trim().to_string();
            for prefix in ["for ", "about ", "regarding "] {
                if rest.to_lowercase().starts_with(prefix) {
                    rest = rest[prefix.len()..].trim().to_string();
                    break;
                }
            }
            if !rest.is_empty() && rest.len() > 2 {
                let query: String = rest.chars().take(100).collect();
                return Some(query.trim().to_string());
            }
        }
    }
    // Fallback: use the last user question from history as the topic
    for (role, content) in history.iter().rev() {
        if role == "user" && content.len() > 5 && !content.chars().all(|c| c.is_whitespace()) {
            let q: String = content.chars().take(80).collect();
            return Some(q.trim().to_string());
        }
    }
    None
}

fn is_follow_up_message(content: &str, history: &[(String, String)]) -> bool {
    let trimmed = content.trim().to_lowercase();
    if trimmed.len() > 40 {
        return false;
    }
    let is_short_phrase = FOLLOW_UP_PHRASES
        .iter()
        .any(|p| trimmed == *p || trimmed.starts_with(&format!("{p} ")) || trimmed.ends_with(p));
    if is_short_phrase {
        return true;
    }
    // Very short messages after a multi-turn conversation are likely follow-ups
    trimmed.len() <= 15 && history.len() >= 4
}

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
use node_federated::{FederatedConfig, FederatedCoordinator};

/// Returns a default rate limiter for /ask and chat (120/min, burst 10). Use in production.
pub fn default_ask_chat_limiter() -> Arc<governor::DefaultDirectRateLimiter> {
    let quota = Quota::per_minute(nonzero!(120u32)).allow_burst(nonzero!(10u32));
    Arc::new(RateLimiter::direct(quota))
}

/// Shared application state for all API handlers.
pub struct AppState {
    pub event_log: RwLock<EventLog>,
    pub cas: CasStore,
    pub db_path: std::path::PathBuf,
    pub peer_dir: Arc<RwLock<PeerDirectory>>,
    pub backend: Arc<dyn InferenceBackend>,
    pub transport: Option<Arc<dyn Transport>>,
    pub consult_config: ConsultConfig,
    pub node_id: String,
    pub admin_token: String,
    /// If false, /status omits admin_token.
    pub expose_admin_token: bool,
    pub scan_dirs: Vec<std::path::PathBuf>,
    pub trainer: Arc<Trainer>,
    pub model_registry: Arc<tokio::sync::Mutex<ModelRegistry>>,
    pub ui_dir: Option<std::path::PathBuf>,
    /// Last training job result for GET /admin/train/status.
    pub last_train_status: Arc<RwLock<Option<TrainResponse>>>,
    /// Optional rate limiter for /ask and POST .../messages (and stream). When set, returns 429 when exceeded.
    pub ask_chat_limiter: Option<Arc<governor::DefaultDirectRateLimiter>>,
    /// Policy engine for web research (allow_web, research_web_capable). Required for POST /admin/research.
    pub research_policy: Arc<node_policy::PolicyEngine>,
}

async fn admin_auth(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    match token {
        Some(t) if t == state.admin_token => Ok(next.run(request).await),
        _ => Err(ApiError::from_status(StatusCode::UNAUTHORIZED, "missing or invalid authorization")),
    }
}

async fn rate_limit_ask_chat(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let path = request.uri().path();
    let is_limited = request.method() == axum::http::Method::POST
        && (path == "/ask" || path.ends_with("/messages") || path.ends_with("/messages/stream"));
    if is_limited {
        if let Some(ref limiter) = state.ask_chat_limiter {
            if limiter.check().is_err() {
                return Err(ApiError::from_status(
                    StatusCode::TOO_MANY_REQUESTS,
                    "rate limit exceeded for ask/chat",
                ));
            }
        }
    }
    Ok(next.run(request).await)
}

pub fn build_router(state: Arc<AppState>) -> Router {
    let localhost_origins = [
        "http://127.0.0.1:9900".parse().unwrap(),
        "http://localhost:9900".parse().unwrap(),
        "http://127.0.0.1:1420".parse().unwrap(),
        "http://localhost:1420".parse().unwrap(),
        "http://127.0.0.1:5173".parse().unwrap(),
        "http://localhost:5173".parse().unwrap(),
    ];
    let cors = CorsLayer::new()
        .allow_origin(localhost_origins)
        .allow_methods(tower_http::cors::Any)
        .allow_headers(tower_http::cors::Any);

    let admin_routes = Router::new()
        .route("/admin/event", post(handle_admin_event))
        .route("/admin/logs", get(handle_admin_logs))
        .route("/admin/sources", get(handle_admin_sources))
        .route("/admin/sources/approve", post(handle_admin_approve_source))
        .route("/admin/train", post(handle_admin_train))
        .route("/admin/train/status", get(handle_admin_train_status))
        .route("/admin/train/export", post(handle_admin_train_export))
        .route("/admin/train/modelfile", post(handle_admin_train_modelfile))
        .route("/admin/models", get(handle_admin_models))
        .route("/admin/models/rollback", post(handle_admin_rollback_model))
        .route("/admin/datasets", get(handle_admin_datasets))
        .route("/admin/federated/status", get(handle_admin_federated_status))
        .route("/admin/research", post(handle_admin_research))
        .route("/admin/scan", post(handle_admin_scan))
        .route("/admin/ingest", post(handle_admin_ingest))
        .route("/admin/sources/approve-all", post(handle_admin_approve_all))
        .route("/admin/ingest-all", post(handle_admin_ingest_all))
        .route_layer(middleware::from_fn_with_state(state.clone(), admin_auth));

    let api_routes = Router::new()
        .route("/status", get(handle_status))
        .route("/peers", get(handle_peers))
        .route("/search", get(handle_search))
        .route("/ask", post(handle_ask))
        .route("/conversations", get(handle_list_conversations).post(handle_create_conversation))
        .route("/conversations/:id/messages", get(handle_get_messages).post(handle_send_message))
        .route("/conversations/:id/messages/stream", post(handle_send_message_stream))
        .route("/conversations/:id", delete(handle_delete_conversation))
        .route("/ws", get(handle_ws))
        .route_layer(middleware::from_fn_with_state(state.clone(), rate_limit_ask_chat))
        .merge(admin_routes)
        .layer(cors);

    let mut app = Router::new()
        .nest("/v1", api_routes)
        .route("/repl/gossip", post(handle_repl_gossip))
        .route("/repl/pull", post(handle_repl_pull));

    if let Some(ref ui_dir) = state.ui_dir {
        if ui_dir.exists() {
            let serve = tower_http::services::ServeDir::new(ui_dir)
                .fallback(tower_http::services::ServeFile::new(ui_dir.join("index.html")));
            app = app.fallback_service(serve);
            tracing::info!("serving UI from {}", ui_dir.display());
        }
    }

    app.with_state(state)
}

// ---------- Error type ----------

#[derive(Debug, Serialize)]
struct ApiError {
    error: String,
    code: u16,
}

#[allow(dead_code)]
impl ApiError {
    fn bad_request(msg: impl Into<String>) -> Self {
        Self { error: msg.into(), code: 400 }
    }
    fn not_found(msg: impl Into<String>) -> Self {
        Self { error: msg.into(), code: 404 }
    }
    fn internal(msg: impl Into<String>) -> Self {
        Self { error: msg.into(), code: 500 }
    }
}

impl axum::response::IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        (status, Json(self)).into_response()
    }
}

impl From<(StatusCode, String)> for ApiError {
    fn from((status, msg): (StatusCode, String)) -> Self {
        Self { error: msg, code: status.as_u16() }
    }
}

impl ApiError {
    fn from_status(status: StatusCode, msg: impl Into<String>) -> Self {
        Self { error: msg.into(), code: status.as_u16() }
    }
}

// ---------- Data types ----------

#[derive(Serialize, Deserialize)]
struct StatusResponse {
    node_id: String,
    status: String,
    event_count: u64,
    peer_count: usize,
    backend: String,
    admin_token: String,
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

#[derive(Clone, Serialize, Deserialize)]
pub struct TrainResponse {
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
    let admin_token = if state.expose_admin_token {
        state.admin_token.clone()
    } else {
        String::new()
    };

    Json(StatusResponse {
        node_id: state.node_id.clone(),
        status: "running".into(),
        event_count,
        peer_count,
        backend: state.backend.name().to_string(),
        admin_token,
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
) -> Result<Json<Vec<SearchResult>>, ApiError> {
    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let fts_query = to_fts5_query(&params.q);
    let hits = search::search_all(&conn, &fts_query, params.limit)
        .map_err(|e| ApiError::internal(e.to_string()))?;

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
) -> Result<Json<AskResponse>, ApiError> {
    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let mut web_search_context = String::new();
    if wants_web_search(&req.question) {
        if let Some(query) = extract_search_query(&req.question, &[]) {
            if let Some(ctx) = node_research::search_and_summarize_for_chat(
                &query,
                &state.research_policy,
                &state.backend,
            )
            .await
            {
                web_search_context = ctx;
            }
        }
    }

    let fts_query = to_fts5_query(&req.question);
    let context_hits = search::search_all(&conn, &fts_query, 10)
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let context_bullets: Vec<String> = context_hits
        .iter()
        .map(|h| {
            let preview: String = h.summary.chars().take(300).collect();
            format!("- [{}] {}: {}", h.hit_type, h.title, preview)
        })
        .collect();

    let prompt = if !web_search_context.is_empty() {
        let kb = if context_bullets.is_empty() {
            String::new()
        } else {
            format!("Context from local knowledge base:\n{}\n\n", context_bullets.join("\n"))
        };
        format!(
            "{}{}\nQuestion: {}\n\nAnswer based on the web search results above. {} Be concise and specific.",
            kb,
            web_search_context,
            req.question,
            if kb.is_empty() { "" } else { "Use local context if relevant. " }
        )
    } else if context_bullets.is_empty() {
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
        .map_err(|e| ApiError::internal(e.to_string()))?;

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
) -> Result<Json<Vec<ConversationSummary>>, ApiError> {
    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let mut stmt = conn
        .prepare(
            "SELECT conversation_id, title, created_at_ms, updated_at_ms
             FROM conversations_view ORDER BY updated_at_ms DESC LIMIT 100",
        )
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let convos = stmt
        .query_map([], |row| {
            Ok(ConversationSummary {
                conversation_id: row.get(0)?,
                title: row.get(1)?,
                created_at_ms: row.get(2)?,
                updated_at_ms: row.get(3)?,
            })
        })
        .map_err(|e| ApiError::internal(e.to_string()))?
        .filter_map(|r| r.ok())
        .collect();

    Ok(Json(convos))
}

async fn handle_create_conversation(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ConversationSummary>, ApiError> {
    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let id = uuid::Uuid::new_v4().to_string();
    let ts = now_ms();

    conn.execute(
        "INSERT INTO conversations_view (conversation_id, title, created_at_ms, updated_at_ms)
         VALUES (?1, 'New conversation', ?2, ?2)",
        rusqlite::params![id, ts],
    )
    .map_err(|e| ApiError::internal(e.to_string()))?;

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
) -> Result<Json<Vec<MessageResponse>>, ApiError> {
    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let mut stmt = conn
        .prepare(
            "SELECT message_id, conversation_id, role, content, context_used, model, confidence, created_at_ms
             FROM messages_view WHERE conversation_id = ?1 ORDER BY created_at_ms ASC",
        )
        .map_err(|e| ApiError::internal(e.to_string()))?;

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
        .map_err(|e| ApiError::internal(e.to_string()))?
        .filter_map(|r| r.ok())
        .collect();

    Ok(Json(msgs))
}

async fn handle_delete_conversation(
    State(state): State<Arc<AppState>>,
    Path(conv_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|e| ApiError::internal(e.to_string()))?;

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
) -> Result<Json<MessageResponse>, ApiError> {
    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let ts = now_ms();
    let user_msg_id = uuid::Uuid::new_v4().to_string();

    // 1. Store user message
    conn.execute(
        "INSERT INTO messages_view (message_id, conversation_id, role, content, created_at_ms)
         VALUES (?1, ?2, 'user', ?3, ?4)",
        rusqlite::params![user_msg_id, conv_id, req.content, ts],
    )
    .map_err(|e| ApiError::internal(e.to_string()))?;

    conn.execute(
        "INSERT INTO messages_fts (message_id, content) VALUES (?1, ?2)",
        rusqlite::params![user_msg_id, req.content],
    )
    .map_err(|e| ApiError::internal(e.to_string()))?;

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
        .map_err(|e| ApiError::internal(e.to_string()))?;

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
    .map_err(|e| ApiError::internal(e.to_string()))?;

    conn.execute(
        "INSERT INTO messages_fts (message_id, content) VALUES (?1, ?2)",
        rusqlite::params![asst_msg_id, gen_resp.text],
    )
    .map_err(|e| ApiError::internal(e.to_string()))?;

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

async fn handle_send_message_stream(
    State(state): State<Arc<AppState>>,
    Path(conv_id): Path<String>,
    Json(req): Json<SendMessageRequest>,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, std::convert::Infallible>> + Send>, ApiError> {
    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let ts = now_ms();
    let user_msg_id = uuid::Uuid::new_v4().to_string();

    conn.execute(
        "INSERT INTO messages_view (message_id, conversation_id, role, content, created_at_ms)
         VALUES (?1, ?2, 'user', ?3, ?4)",
        rusqlite::params![user_msg_id, conv_id, req.content, ts],
    )
    .map_err(|e| ApiError::internal(e.to_string()))?;

    conn.execute(
        "INSERT INTO messages_fts (message_id, content) VALUES (?1, ?2)",
        rusqlite::params![user_msg_id, req.content],
    )
    .map_err(|e| ApiError::internal(e.to_string()))?;

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

    let is_follow_up = is_follow_up_message(&req.content, &history);
    let fts_query_text: String = if is_follow_up {
        history
            .iter()
            .rev()
            .find(|(r, _)| r == "user")
            .map(|(_, c)| c.as_str())
            .unwrap_or(&req.content)
            .to_string()
    } else {
        req.content.clone()
    };
    let fts_query = to_fts5_query(&fts_query_text);
    let context_hits = if is_follow_up && fts_query_text.trim().len() < 10 {
        Vec::new()
    } else {
        search::search_all(&conn, &fts_query, 10).unwrap_or_default()
    };

    let mut web_search_context = String::new();
    if wants_web_search(&req.content) {
        if let Some(query) = extract_search_query(&req.content, &history) {
            if let Some(ctx) = node_research::search_and_summarize_for_chat(
                &query,
                &state.research_policy,
                &state.backend,
            )
            .await
            {
                web_search_context = ctx;
            }
        }
    }

    let context_bullets: Vec<String> = context_hits
        .iter()
        .map(|h| {
            let preview: String = h.summary.chars().take(300).collect();
            format!("- [{}] {}: {}", h.hit_type, h.title, preview)
        })
        .collect();

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

    let has_context = !context_bullets.is_empty() || !cross_session.is_empty() || !web_search_context.is_empty();
    let mut prompt_parts = Vec::new();
    if is_follow_up {
        prompt_parts.push(
            "The user is asking a follow-up question. Stay on the topic of your previous answer. \
             Do NOT introduce new topics from the knowledge base. Elaborate on what you already discussed."
                .to_string(),
        );
    }
    if !web_search_context.is_empty() {
        prompt_parts.push(web_search_context);
    }
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
    if !has_context && hist_len <= 1 {
        prompt_parts.push(
            "No matching data was found in the knowledge base. \
             Tell the user you searched but found no relevant results. \
             Suggest they scan and ingest local data sources first."
                .to_string(),
        );
    }
    let prompt = prompt_parts.join("\n\n");

    let gen_req = node_ai::GenerateRequest {
        prompt,
        system: Some(MESHMIND_SYSTEM_PROMPT.into()),
        max_tokens: req.max_tokens.unwrap_or(1024),
        ..Default::default()
    };

    let mut backend_stream = state
        .backend
        .generate_stream(gen_req)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let db_path = state.db_path.clone();
    let context_ids: Vec<String> = context_hits.iter().map(|h| h.id.clone()).collect();
    let backend_name = state.backend.name().to_string();
    let backend_name_for_done = backend_name.clone();
    let confidence: f32 = if context_bullets.is_empty() && cross_session.is_empty() {
        0.3
    } else {
        0.7
    };

    let stream = async_stream::stream! {
        let mut full_text = String::new();
        while let Some(result) = backend_stream.next().await {
            match result {
                Ok(chunk) => {
                    full_text.push_str(&chunk);
                    let data = serde_json::json!({ "token": chunk });
                    yield Ok(Event::default().data(data.to_string()));
                }
                Err(_) => break,
            }
        }
        let asst_msg_id = uuid::Uuid::new_v4().to_string();
        let asst_ts = now_ms();
        let ctx_json = serde_json::to_string(&context_ids).unwrap_or_else(|_| "[]".into());
        let _ = rusqlite::Connection::open(&db_path).and_then(|conn| {
            conn.execute(
                "INSERT INTO messages_view (message_id, conversation_id, role, content, context_used, model, confidence, created_at_ms)
                 VALUES (?1, ?2, 'assistant', ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    asst_msg_id,
                    conv_id,
                    full_text,
                    ctx_json,
                    backend_name,
                    confidence,
                    asst_ts,
                ],
            )?;
            conn.execute(
                "INSERT INTO messages_fts (message_id, content) VALUES (?1, ?2)",
                rusqlite::params![asst_msg_id, full_text],
            )?;
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
            Ok::<(), rusqlite::Error>(())
        });
        let done_data = serde_json::json!({
            "done": true,
            "message_id": asst_msg_id,
            "content": full_text,
            "model": backend_name_for_done,
            "confidence": confidence,
            "context_used": context_ids,
            "created_at_ms": asst_ts,
        });
        yield Ok(Event::default().data(done_data.to_string()));
    };

    Ok(Sse::new(stream))
}

async fn handle_admin_event(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AdminEventRequest>,
) -> Result<Json<AdminEventResponse>, ApiError> {
    use node_proto::common::*;
    use node_proto::events::*;

    let tenant = req.tenant_id.unwrap_or_else(|| "public".into());

    let content_ref = if !req.summary.is_empty() {
        let href = state
            .cas
            .put_bytes("text/plain", req.summary.as_bytes())
            .map_err(|e| ApiError::internal(e.to_string()))?;
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
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    node_storage::projector::apply_event(&conn, &stored)
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let event_hash = stored.event_hash.map(|h| h.sha256).unwrap_or_default();

    Ok(Json(AdminEventResponse {
        event_id: req.event_id,
        event_hash,
    }))
}

async fn handle_admin_logs(
    State(state): State<Arc<AppState>>,
    Query(params): Query<LogParams>,
) -> Result<Json<Vec<AuditEntry>>, ApiError> {
    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let mut stmt = conn
        .prepare(
            "SELECT event_id, event_type, summary, created_at_ms
             FROM audit_view
             ORDER BY created_at_ms DESC
             LIMIT ?1",
        )
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let entries = stmt
        .query_map([params.n as i64], |row| {
            Ok(AuditEntry {
                event_id: row.get(0)?,
                event_type: row.get(1)?,
                summary: row.get(2)?,
                created_at_ms: row.get(3)?,
            })
        })
        .map_err(|e| ApiError::internal(e.to_string()))?
        .filter_map(|r| r.ok())
        .collect();

    Ok(Json(entries))
}

async fn handle_admin_sources(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<SourceRow>>, ApiError> {
    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let mut stmt = conn
        .prepare(
            "SELECT source_id, display_name, connector_type, status, pii_detected, estimated_size_bytes
             FROM sources_view
             ORDER BY source_id",
        )
        .map_err(|e| ApiError::internal(e.to_string()))?;

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
        .map_err(|e| ApiError::internal(e.to_string()))?
        .filter_map(|r| r.ok())
        .collect();

    Ok(Json(rows))
}

async fn handle_admin_approve_source(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ApproveSourceRequest>,
) -> Result<Json<ApproveSourceResponse>, ApiError> {
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
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    node_storage::projector::apply_event(&conn, &stored)
        .map_err(|e| ApiError::internal(e.to_string()))?;

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

#[derive(Deserialize)]
struct ResearchApiRequest {
    url: String,
    question: String,
    #[serde(default)]
    allow_web: bool,
    #[serde(default)]
    redaction_required: bool,
}

#[derive(Serialize)]
struct ResearchApiResponse {
    artifact_id: String,
    question: String,
    summary: String,
    sources: Vec<String>,
    confidence: f32,
    event_id: String,
}

async fn handle_admin_research(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ResearchApiRequest>,
) -> Result<Json<ResearchApiResponse>, ApiError> {
    let research_req = node_research::ResearchRequest {
        url: req.url,
        question: req.question,
        tenant_id: "public".into(),
        allow_web: req.allow_web,
        redaction_required: req.redaction_required,
    };

    let mut log = state.event_log.write().await;
    let result = node_research::research(
        &research_req,
        &state.research_policy,
        &state.backend,
        &state.cas,
        &mut log,
        &state.node_id,
        Some(&state.db_path),
    )
    .await
    .map_err(|e| ApiError::bad_request(e.to_string()))?;

    Ok(Json(ResearchApiResponse {
        artifact_id: result.artifact_id,
        question: result.question,
        summary: result.summary,
        sources: result.sources,
        confidence: result.confidence,
        event_id: result.event_id,
    }))
}

async fn handle_admin_scan(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ScanResponse>, ApiError> {
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
        .map_err(|e| ApiError::internal(e.to_string()))?;

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
) -> Result<Json<IngestResponse>, ApiError> {
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
        return Err(ApiError::bad_request(format!(
            "source {} is not approved (status: {})", req.source_id, status
        )));
    }

    let (connector, connector_str) = connector_for_type(connector_type)
        .ok_or_else(|| ApiError::bad_request(format!("unsupported connector type: {connector_type}")))?;

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

#[derive(Serialize)]
struct BulkApproveResponse {
    approved: usize,
    skipped: usize,
}

async fn handle_admin_approve_all(
    State(state): State<Arc<AppState>>,
) -> Result<Json<BulkApproveResponse>, ApiError> {
    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}")))?;

    let source_ids: Vec<String> = conn
        .prepare("SELECT source_id FROM sources_view WHERE status != 'approved' AND display_name NOT LIKE '%keys%'")
        .and_then(|mut s| {
            let ids = s
                .query_map([], |row| row.get(0))?
                .filter_map(|r| r.ok())
                .collect();
            Ok(ids)
        })
        .unwrap_or_default();

    let total = source_ids.len();
    let mut approved = 0;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;

    for sid in &source_ids {
        let ok = conn
            .execute(
                "UPDATE sources_view SET status = 'approved', approved_at_ms = ?1 WHERE source_id = ?2",
                rusqlite::params![ts, sid],
            )
            .is_ok();
        if ok { approved += 1; }
    }

    Ok(Json(BulkApproveResponse {
        approved,
        skipped: total - approved,
    }))
}

#[derive(Serialize)]
struct BulkIngestResponse {
    ingested: u64,
    failed: u64,
    total_rows: u64,
    total_docs: u64,
}

async fn handle_admin_ingest_all(
    State(state): State<Arc<AppState>>,
) -> Result<Json<BulkIngestResponse>, ApiError> {
    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("db error: {e}")))?;

    let sources: Vec<(String, i32, String)> = conn
        .prepare("SELECT source_id, connector_type, path_or_uri FROM sources_view WHERE status = 'approved'")
        .and_then(|mut s| {
            let rows = s
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
                .filter_map(|r| r.ok())
                .collect();
            Ok(rows)
        })
        .unwrap_or_default();

    drop(conn);

    let mut ingested = 0u64;
    let mut failed = 0u64;
    let mut total_rows = 0u64;
    let mut total_docs = 0u64;

    for (source_id, connector_type, path_or_uri) in &sources {
        let (connector, connector_str) = match connector_for_type(*connector_type) {
            Some(p) => p,
            None => { failed += 1; continue; }
        };

        let source_path = std::path::PathBuf::from(path_or_uri);
        let tables = match connector.inspect_schema(&source_path) {
            Ok(t) => t,
            Err(_) => { failed += 1; continue; }
        };
        let table_names: Vec<String> = tables.iter().map(|t| t.table_name.clone()).collect();

        let ingest_id = format!("ing-{}", uuid::Uuid::new_v4());
        let job = IngestJob {
            ingest_id,
            source_id: source_id.clone(),
            connector_type: connector_str.to_string(),
        };

        let config = IngestConfig::default();
        let node_id = state.node_id.clone();
        let db_path = state.db_path.clone();

        let mut log = state.event_log.write().await;
        match node_ingest::run_ingest(
            &job, connector.as_ref(), &source_path, &table_names,
            &config, &state.cas, &mut log, &db_path, &node_id,
        ) {
            Ok(result) => {
                total_rows += result.rows_ingested;
                total_docs += result.documents_created;
                ingested += 1;
            }
            Err(_) => { failed += 1; }
        }
    }

    Ok(Json(BulkIngestResponse { ingested, failed, total_rows, total_docs }))
}

async fn handle_admin_train(
    State(state): State<Arc<AppState>>,
    Json(req): Json<TrainRequest>,
) -> Result<Json<TrainResponse>, ApiError> {
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
        let resp = TrainResponse {
            job_id,
            status: status_msg,
            dataset_items,
            dataset_manifest_id,
            score: None,
            model_version: None,
        };
        let _ = state.last_train_status.write().await.insert(resp.clone());
        return Ok(Json(resp));
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

    let resp = TrainResponse {
        job_id,
        status: status_str,
        dataset_items,
        dataset_manifest_id,
        score,
        model_version,
    };
    let _ = state.last_train_status.write().await.insert(resp.clone());
    Ok(Json(resp))
}

async fn handle_admin_train_status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<TrainResponse>, ApiError> {
    let status = state.last_train_status.read().await.clone();
    status
        .map(Json)
        .ok_or_else(|| ApiError::not_found("no training job has run yet"))
}

const MESHMIND_TRAINING_SYSTEM: &str = "\
You are MeshMind, a helpful local-first AI assistant running on the user's own machine. \
You have access to a local knowledge base of ingested documents, databases, images, \
and other data sources. Answer questions accurately based on this knowledge. \
Be concise, specific, and helpful.";

async fn handle_admin_train_export(
    State(state): State<Arc<AppState>>,
) -> Result<(HeaderMap, String), ApiError> {
    use node_proto::events::event_envelope::Payload;

    let event_log = state.event_log.read().await;
    let events = event_log
        .replay()
        .map_err(|e| ApiError::internal(format!("replay failed: {e}")))?;
    drop(event_log);

    let mut lines = Vec::new();
    for event in &events {
        let (user_msg, asst_msg) = match &event.payload {
            Some(Payload::CaseCreated(cc)) if !cc.title.is_empty() && !cc.summary.is_empty() => {
                (cc.title.clone(), cc.summary.clone())
            }
            Some(Payload::ArtifactPublished(ap))
                if !ap.title.is_empty() && !ap.summary.is_empty() =>
            {
                (ap.title.clone(), ap.summary.clone())
            }
            _ => continue,
        };

        let example = serde_json::json!({
            "messages": [
                {"role": "system", "content": MESHMIND_TRAINING_SYSTEM},
                {"role": "user", "content": user_msg},
                {"role": "assistant", "content": asst_msg},
            ]
        });
        lines.push(serde_json::to_string(&example).unwrap());
    }

    let content = if lines.is_empty() {
        String::new()
    } else {
        let mut s = lines.join("\n");
        s.push('\n');
        s
    };

    let data_dir = state
        .db_path
        .parent()
        .and_then(|p| p.parent())
        .ok_or_else(|| ApiError::internal("cannot determine data_dir from db_path"))?;
    let training_dir = data_dir.join("training");
    std::fs::create_dir_all(&training_dir)
        .map_err(|e| ApiError::internal(format!("create training dir: {e}")))?;
    std::fs::write(training_dir.join("dataset.jsonl"), &content)
        .map_err(|e| ApiError::internal(format!("write dataset: {e}")))?;

    tracing::info!(
        lines = lines.len(),
        path = %training_dir.join("dataset.jsonl").display(),
        "training dataset exported"
    );

    let mut headers = HeaderMap::new();
    headers.insert("content-type", "text/plain; charset=utf-8".parse().unwrap());
    Ok((headers, content))
}

async fn handle_admin_train_modelfile(
    State(state): State<Arc<AppState>>,
) -> Result<(HeaderMap, String), ApiError> {
    let content = format!(
        "\
FROM llama3.2:3b

SYSTEM \"\"\"
{system}
\"\"\"

PARAMETER temperature 0.7
PARAMETER top_p 0.9
PARAMETER stop \"<|eot_id|>\"

TEMPLATE \"\"\"
{{{{- if .System }}}}
<|start_header_id|>system<|end_header_id|>

{{{{ .System }}}}<|eot_id|>
{{{{- end }}}}
{{{{- range .Messages }}}}
<|start_header_id|>{{{{ .Role }}}}<|end_header_id|>

{{{{ .Content }}}}<|eot_id|>
{{{{- end }}}}
<|start_header_id|>assistant<|end_header_id|>

\"\"\"
",
        system = MESHMIND_TRAINING_SYSTEM,
    );

    let data_dir = state
        .db_path
        .parent()
        .and_then(|p| p.parent())
        .ok_or_else(|| ApiError::internal("cannot determine data_dir from db_path"))?;
    let training_dir = data_dir.join("training");
    std::fs::create_dir_all(&training_dir)
        .map_err(|e| ApiError::internal(format!("create training dir: {e}")))?;
    std::fs::write(training_dir.join("Modelfile"), &content)
        .map_err(|e| ApiError::internal(format!("write Modelfile: {e}")))?;

    tracing::info!(
        path = %training_dir.join("Modelfile").display(),
        "Ollama Modelfile generated"
    );

    let mut headers = HeaderMap::new();
    headers.insert("content-type", "text/plain; charset=utf-8".parse().unwrap());
    Ok((headers, content))
}

async fn handle_admin_models(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<ModelRow>>, ApiError> {
    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let mut stmt = conn
        .prepare(
            "SELECT model_id, version, promoted, rolled_back
             FROM models_view
             ORDER BY model_id, version",
        )
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let rows = stmt
        .query_map([], |row| {
            Ok(ModelRow {
                model_id: row.get(0)?,
                version: row.get(1)?,
                promoted: row.get::<_, i32>(2)? != 0,
                rolled_back: row.get::<_, i32>(3)? != 0,
            })
        })
        .map_err(|e| ApiError::internal(e.to_string()))?
        .filter_map(|r| r.ok())
        .collect();

    Ok(Json(rows))
}

async fn handle_admin_rollback_model(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RollbackModelRequest>,
) -> Result<Json<RollbackModelResponse>, ApiError> {
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
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    node_storage::projector::apply_event(&conn, &stored)
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(RollbackModelResponse { event_id }))
}

async fn handle_admin_datasets(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<DatasetRow>>, ApiError> {
    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let mut stmt = conn
        .prepare(
            "SELECT manifest_id, source_id, preset, item_count, total_bytes
             FROM datasets_view
             ORDER BY manifest_id",
        )
        .map_err(|e| ApiError::internal(e.to_string()))?;

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
        .map_err(|e| ApiError::internal(e.to_string()))?
        .filter_map(|r| r.ok())
        .collect();

    Ok(Json(rows))
}

#[derive(Serialize)]
struct FederatedStatusResponse {
    supported: bool,
    aggregation: String,
    min_participants: u32,
    max_participants: u32,
}

async fn handle_admin_federated_status(
) -> Result<Json<FederatedStatusResponse>, ApiError> {
    let config = FederatedConfig::new("router");
    let _coordinator = FederatedCoordinator::new(config.clone());
    Ok(Json(FederatedStatusResponse {
        supported: true,
        aggregation: config.aggregation_strategy,
        min_participants: config.min_participants,
        max_participants: config.max_participants,
    }))
}

// ---------- WebSocket (real-time status) ----------

async fn handle_ws(ws: WebSocketUpgrade, State(state): State<Arc<AppState>>) -> axum::response::Response {
    ws.on_upgrade(move |socket| handle_ws_socket(socket, state))
}

async fn handle_ws_socket(mut socket: WebSocket, state: Arc<AppState>) {
    use axum::extract::ws::Message;
    use tokio::time::interval;
    let mut ticker = interval(std::time::Duration::from_secs(10));
    loop {
        ticker.tick().await;
        let event_count = state.event_log.read().await.event_count();
        let peer_count = state.peer_dir.read().await.all_peers().len();
        let last_train = state.last_train_status.read().await.clone();
        let payload = serde_json::json!({
            "type": "status",
            "event_count": event_count,
            "peer_count": peer_count,
            "backend": state.backend.name(),
            "last_train": last_train,
        });
        if socket.send(Message::Text(payload.to_string())).await.is_err() {
            break;
        }
    }
}

// ---------- Replication endpoints ----------

use node_proto::repl::{
    GossipMeta as ProtoGossipMeta, SegmentId as ProtoSegmentId, PullSegmentsRequest,
    PullCasObjectsRequest,
};

#[derive(Serialize, Deserialize)]
struct GossipExchangeRequest {
    gossip_bytes: Vec<u8>,
}

#[derive(Serialize)]
struct GossipExchangeResponse {
    gossip_bytes: Vec<u8>,
    missing_segment_ids: Vec<String>,
    missing_cas_hashes: Vec<String>,
}

async fn handle_repl_gossip(
    State(state): State<Arc<AppState>>,
    Json(req): Json<GossipExchangeRequest>,
) -> Result<Json<GossipExchangeResponse>, ApiError> {
    use prost::Message;
    let remote_gossip = ProtoGossipMeta::decode(req.gossip_bytes.as_slice())
        .map_err(|e| ApiError::bad_request(format!("invalid gossip: {e}")))?;

    let event_log = state.event_log.read().await;
    let local_gossip = node_repl::build_gossip_meta(
        &state.node_id, "public", &event_log, &state.cas, &[],
    )
    .map_err(|e| ApiError::internal(format!("gossip build failed: {e}")))?;

    let missing_segs = node_repl::find_missing_segments(&local_gossip, &remote_gossip);
    let missing_cas = node_repl::find_missing_objects(&state.cas, &remote_gossip);
    drop(event_log);

    let mut local_bytes = Vec::new();
    local_gossip.encode(&mut local_bytes)
        .map_err(|e| ApiError::internal(format!("encode gossip: {e}")))?;

    Ok(Json(GossipExchangeResponse {
        gossip_bytes: local_bytes,
        missing_segment_ids: missing_segs.iter().map(|s| s.value.clone()).collect(),
        missing_cas_hashes: missing_cas.iter().map(|h| h.sha256.clone()).collect(),
    }))
}

#[derive(Deserialize)]
struct PullRequest {
    segment_ids: Vec<String>,
    cas_hashes: Vec<String>,
}

#[derive(Serialize)]
struct PullResponse {
    segments_sent: usize,
    cas_sent: usize,
    segment_chunks: Vec<Vec<u8>>,
    cas_chunks: Vec<Vec<u8>>,
}

async fn handle_repl_pull(
    State(state): State<Arc<AppState>>,
    Json(req): Json<PullRequest>,
) -> Result<Json<PullResponse>, ApiError> {
    let event_log = state.event_log.read().await;

    let seg_req = PullSegmentsRequest {
        requester: None,
        tenant_id: None,
        want_segments: req.segment_ids.iter().map(|s| ProtoSegmentId { value: s.clone() }).collect(),
        budget: None,
    };
    let seg_resp = node_repl::serve_pull_segments(&seg_req, &event_log, &state.node_id)
        .map_err(|e| ApiError::internal(format!("serve segments: {e}")))?;
    let segments_sent = seg_resp.chunks.len();

    let cas_req = PullCasObjectsRequest {
        requester: None,
        want_hashes: req.cas_hashes.iter().map(|h| node_proto::common::HashRef { sha256: h.clone() }).collect(),
        budget: None,
    };
    let cas_resp = node_repl::serve_pull_cas_objects(&cas_req, &state.cas, &state.node_id)
        .map_err(|e| ApiError::internal(format!("serve cas objects: {e}")))?;
    let cas_sent = cas_resp.chunks.len();

    drop(event_log);

    use prost::Message;
    let segment_chunks: Vec<Vec<u8>> = seg_resp.chunks.iter().map(|c| c.encode_to_vec()).collect();
    let cas_chunks: Vec<Vec<u8>> = cas_resp.chunks.iter().map(|c| c.encode_to_vec()).collect();

    Ok(Json(PullResponse { segments_sent, cas_sent, segment_chunks, cas_chunks }))
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
            peer_dir: Arc::new(RwLock::new(PeerDirectory::new())),
            backend: Arc::new(MockBackend::new()),
            transport: None,
            consult_config: ConsultConfig::default(),
            node_id: "test-node-001".into(),
            admin_token: "test-token".into(),
            expose_admin_token: true,
            scan_dirs: vec![],
            trainer,
            model_registry,
            ui_dir: None,
            last_train_status: Arc::new(RwLock::new(None)),
            ask_chat_limiter: None,
            research_policy: Arc::new(node_policy::PolicyEngine::new(node_policy::PolicyConfig {
                allow_web: true,
                research_web_capable: true,
                ..Default::default()
            })),
        })
    }

    #[tokio::test]
    async fn status_endpoint() {
        let state = create_test_state();
        let app = build_router(state);

        let resp = app
            .oneshot(Request::get("/v1/status").body(Body::empty()).unwrap())
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
            .oneshot(Request::get("/v1/peers").body(Body::empty()).unwrap())
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
            .oneshot(Request::get("/v1/search?q=test").body(Body::empty()).unwrap())
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
                Request::post("/v1/ask")
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

    const TEST_AUTH: &str = "Bearer test-token";

    #[tokio::test]
    async fn admin_requires_auth() {
        let state = create_test_state();
        let app = build_router(state);

        let resp = app
            .oneshot(Request::get("/v1/admin/sources").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn admin_event_endpoint() {
        let state = create_test_state();
        let app = build_router(state);

        let resp = app
            .oneshot(
                Request::post("/v1/admin/event")
                    .header("content-type", "application/json")
                    .header("authorization", TEST_AUTH)
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
                Request::get("/v1/admin/logs?n=10")
                    .header("authorization", TEST_AUTH)
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
            .oneshot(
                Request::get("/v1/admin/sources")
                    .header("authorization", TEST_AUTH)
                    .body(Body::empty())
                    .unwrap(),
            )
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
            .oneshot(
                Request::get("/v1/admin/models")
                    .header("authorization", TEST_AUTH)
                    .body(Body::empty())
                    .unwrap(),
            )
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
            .oneshot(
                Request::get("/v1/admin/datasets")
                    .header("authorization", TEST_AUTH)
                    .body(Body::empty())
                    .unwrap(),
            )
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
                Request::post("/v1/admin/train")
                    .header("content-type", "application/json")
                    .header("authorization", TEST_AUTH)
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

    #[tokio::test]
    async fn test_admin_research_policy_denied() {
        // Use a policy that denies web research
        let deny_policy = Arc::new(node_policy::PolicyEngine::new(node_policy::PolicyConfig {
            allow_web: false,
            research_web_capable: false,
            ..Default::default()
        }));
        // We need a new AppState - create_test_state returns Arc<AppState> and we can't mutate.
        // Create custom state for this test.
        let tmp = tempfile::TempDir::new().unwrap();
        let event_log = EventLog::open(tmp.path()).unwrap();
        let cas = CasStore::open(tmp.path()).unwrap();
        let db_path = tmp.path().join("sqlite").join("meshmind.db");
        let _conn = sqlite_views::open_db(&db_path).unwrap();
        let policy = Arc::new(node_policy::PolicyEngine::new(node_policy::PolicyConfig {
            allow_train: true,
            ..Default::default()
        }));
        let model_registry = Arc::new(tokio::sync::Mutex::new(ModelRegistry::new()));
        let trainer = Arc::new(Trainer::new(policy, model_registry.clone()));
        let state = Arc::new(AppState {
            event_log: RwLock::new(event_log),
            cas,
            db_path,
            peer_dir: Arc::new(RwLock::new(PeerDirectory::new())),
            backend: Arc::new(MockBackend::new()),
            transport: None,
            consult_config: ConsultConfig::default(),
            node_id: "test-node".into(),
            admin_token: "test-token".into(),
            expose_admin_token: true,
            scan_dirs: vec![],
            trainer,
            model_registry,
            ui_dir: None,
            last_train_status: Arc::new(RwLock::new(None)),
            ask_chat_limiter: None,
            research_policy: deny_policy,
        });

        let app = build_router(state);
        let resp = app
            .oneshot(
                Request::post("/v1/admin/research")
                    .header("content-type", "application/json")
                    .header("authorization", TEST_AUTH)
                    .body(Body::from(
                        serde_json::json!({
                            "url": "https://example.com",
                            "question": "What is this?",
                            "allow_web": false,
                            "redaction_required": false
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let body_str = String::from_utf8_lossy(&body).to_lowercase();
        assert!(
            body_str.contains("denied") || body_str.contains("policy"),
            "expected policy denial message, got: {body_str}"
        );
    }

    #[tokio::test]
    async fn test_admin_train_status_404_when_no_train() {
        let state = create_test_state();
        let app = build_router(state);

        let resp = app
            .oneshot(
                Request::get("/v1/admin/train/status")
                    .header("authorization", TEST_AUTH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
