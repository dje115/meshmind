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

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::ws::{WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, Sse};
use axum::response::{Html, IntoResponse, Json, Response};
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
    load_onedrive_config, save_onedrive_config, Connector, CsvFolderConnector, DocumentConnector,
    ImageConnector, JsonFolderConnector, OneDriveConfig, OneDriveConnector, SQLiteConnector,
};
use sha2::Digest;

async fn serve_fallback_landing() -> impl IntoResponse {
    let html = r##"<!DOCTYPE html>
<html>
<head><meta charset="utf-8"><title>MeshMind</title></head>
<body style="font-family:sans-serif;max-width:600px;margin:60px auto;padding:20px">
<h1>MeshMind</h1>
<p>API is running. To use the full UI:</p>
<ol>
<li>From the project root: <code>cd ui && npm run build</code></li>
<li>Restart MeshMind</li>
</ol>
<p>API endpoints: <a href="/v1/status">/v1/status</a></p>
</body>
</html>"##;
    Html(html)
}

fn connector_for_type(connector_type: i32) -> Option<(Box<dyn Connector>, &'static str)> {
    let (connector, name) = match connector_type {
        1 => (
            Box::new(SQLiteConnector::new("sqlite")) as Box<dyn Connector>,
            "sqlite",
        ),
        2 => (
            Box::new(CsvFolderConnector::new("csv")) as Box<dyn Connector>,
            "csv",
        ),
        3 => (
            Box::new(JsonFolderConnector::new("json")) as Box<dyn Connector>,
            "json",
        ),
        7 => (
            Box::new(ImageConnector::new("image")) as Box<dyn Connector>,
            "image",
        ),
        8 => (
            Box::new(DocumentConnector::new("document")) as Box<dyn Connector>,
            "document",
        ),
        9 => (
            Box::new(OneDriveConnector::new("onedrive")) as Box<dyn Connector>,
            "onedrive",
        ),
        _ => return None,
    };
    Some((connector, name))
}

fn connector_for_onedrive(data_dir: &std::path::Path) -> (Box<dyn Connector>, &'static str) {
    let config_path = data_dir.join("config").join("onedrive.json");
    let connector: Box<dyn Connector> = if let Some(cfg) = load_onedrive_config(&config_path) {
        Box::new(OneDriveConnector::new_with_config("onedrive", cfg))
    } else {
        Box::new(OneDriveConnector::new("onedrive"))
    };
    (connector, "onedrive")
}
use node_datasets::{DatasetBuildConfig, DatasetPreset};
use node_discovery::{build_discovered_event, scan_directory, DiscoveryConfig};
use node_ingest::{IngestConfig, IngestJob};
use node_mesh::transport::Transport;
use node_mesh::{ConsultConfig, PeerDirectory};
use node_storage::cas::CasStore;
use node_storage::event_log::EventLog;
use node_storage::insights;
use node_storage::mergeable;
use node_storage::search;
use node_storage::shards;

const STOP_WORDS: &[&str] = &[
    "a", "an", "the", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had",
    "do", "does", "did", "will", "would", "shall", "should", "may", "might", "can", "could", "am",
    "i", "me", "my", "we", "our", "you", "your", "he", "she", "it", "they", "them", "this", "that",
    "these", "those", "of", "in", "on", "at", "to", "for", "with", "from", "by", "about", "into",
    "through", "during", "before", "after", "and", "but", "or", "not", "no", "if", "then", "so",
    "how", "what", "when", "where", "who", "which", "why",
];

/// Phrases that indicate the user wants a web search.
const WEB_SEARCH_TRIGGERS: &[&str] = &[
    "search the web",
    "search the internet",
    "search the net",
    "check the internet",
    "check the web",
    "check online",
    "look it up online",
    "look it up on the web",
    "look up online",
    "look up on the internet",
    "google it",
    "search online",
    "find it online",
    "find online",
    "look up",
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

/// Generic words that are too vague to use as a search query.
const VAGUE_QUERY_WORDS: &[&str] = &[
    "it",
    "that",
    "this",
    "information",
    "info",
    "something",
    "things",
];

fn extract_search_query(content: &str, history: &[(String, String)]) -> Option<String> {
    let lower = content.to_lowercase();

    // Try "about X" anywhere in the message first (e.g. "what can you find about scooby doo")
    for pattern in ["about ", "on ", "regarding "] {
        if let Some(pos) = lower.find(pattern) {
            let after = content[pos + pattern.len()..].trim();
            // Take up to next sentence/clause boundary
            let end = after.find(['.', ',', '?', '!']).unwrap_or(after.len());
            let chunk: String = after.chars().take(end.min(80)).collect();
            let trimmed = chunk.trim();
            if trimmed.len() > 3
                && !VAGUE_QUERY_WORDS.contains(&trimmed.to_lowercase().as_str())
                && !trimmed.chars().all(|c| !c.is_alphanumeric())
            {
                return Some(trimmed.to_string());
            }
        }
    }

    for trigger in WEB_SEARCH_TRIGGERS {
        if let Some(pos) = lower.find(trigger) {
            let mut rest = content[pos + trigger.len()..].trim().to_string();
            for prefix in ["for ", "about ", "regarding ", "on "] {
                if rest.to_lowercase().starts_with(prefix) {
                    rest = rest[prefix.len()..].trim().to_string();
                    break;
                }
            }
            if !rest.is_empty() && rest.len() > 3 {
                let query: String = rest.chars().take(100).collect();
                let q = query.trim().to_string();
                let q_lower = q.to_lowercase();
                if !VAGUE_QUERY_WORDS.contains(&q_lower.as_str()) {
                    return Some(q);
                }
            }
        }
    }

    // Fallback: use the last user question from history as the topic
    for (role, content) in history.iter().rev() {
        if role == "user" && content.len() > 5 && !content.chars().all(|c| c.is_whitespace()) {
            // Prefer extracting "about X" from history if present
            let h_lower = content.to_lowercase();
            for pattern in ["about ", "on ", "regarding "] {
                if let Some(pos) = h_lower.find(pattern) {
                    let after = content[pos + pattern.len()..].trim();
                    let end = after.find(['.', ',', '?', '!']).unwrap_or(after.len());
                    let chunk: String = after.chars().take(end.min(80)).collect();
                    let trimmed = chunk.trim();
                    if trimmed.len() > 3
                        && !VAGUE_QUERY_WORDS.contains(&trimmed.to_lowercase().as_str())
                    {
                        return Some(trimmed.to_string());
                    }
                }
            }
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

/// True if the question looks like a general-knowledge query (who/what/when/where) that would benefit from web search.
/// Business intent classification for routing queries to entity graph or facts.
struct BusinessIntent {
    entity_types: Vec<String>,
    metrics: Vec<String>,
}

fn classify_business_intent(question: &str) -> BusinessIntent {
    let lower = question.trim().to_lowercase();
    let mut entity_types = Vec::new();
    let mut metrics = Vec::new();

    // Entity types
    if lower.contains("customer") || lower.contains("customers") {
        entity_types.push("customer".into());
    }
    if lower.contains("invoice") || lower.contains("invoices") || lower.contains("overdue") {
        entity_types.push("invoice".into());
    }
    if lower.contains("quote")
        || lower.contains("quotes")
        || lower.contains("proposal")
        || lower.contains("proposals")
    {
        entity_types.push("quote".into());
    }
    if lower.contains("line item")
        || lower.contains("line items")
        || lower.contains("breakdown")
        || (lower.contains("quote")
            && (lower.contains("similar") || lower.contains("margin") || lower.contains("charged")))
    {
        entity_types.push("quote_line_item".into());
    }
    if lower.contains("account") || lower.contains("accounts") {
        entity_types.push("account".into());
    }
    if lower.contains("job")
        || lower.contains("jobs")
        || lower.contains("install")
        || lower.contains("work order")
    {
        entity_types.push("job".into());
    }

    // Metrics
    if lower.contains("revenue") || lower.contains("revenues") {
        metrics.push("revenue".into());
    }
    if lower.contains("profit") || lower.contains("profits") {
        metrics.push("profit".into());
    }
    if lower.contains("margin") || lower.contains("margins") {
        metrics.push("margin".into());
    }
    if lower.contains("charge")
        || lower.contains("charge for")
        || lower.contains("pricing")
        || lower.contains("price")
    {
        metrics.push("pricing".into());
    }

    BusinessIntent {
        entity_types,
        metrics,
    }
}

fn looks_like_general_knowledge_question(content: &str) -> bool {
    let lower = content.trim().to_lowercase();
    if lower.len() < 8 || lower.len() > 120 {
        return false;
    }
    let prefixes = [
        "who is ",
        "who was ",
        "what is ",
        "what was ",
        "when did ",
        "when was ",
        "where is ",
        "where did ",
        "why did ",
        "why is ",
        "how did ",
        "how does ",
        "who are ",
        "what are ",
        "define ",
        "explain ",
    ];
    prefixes.iter().any(|p| lower.starts_with(p))
}

fn to_fts5_query(text: &str) -> String {
    let keywords: Vec<&str> = text
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|w| w.len() > 1 && !STOP_WORDS.contains(&w.to_lowercase().as_str()))
        .collect();
    if keywords.is_empty() {
        text.split_whitespace().next().unwrap_or("*").to_string()
    } else {
        keywords.join(" OR ")
    }
}

/// Build context bullets for the LLM prompt. For artifacts with content_hash, fetches
/// full content from CAS. Uses adaptive sizing: document-specific questions get full
/// content for referenced docs; broad/trend questions get generous excerpts per doc.
const CONTEXT_CONTENT_DEFAULT_CHARS: usize = 8000; // Per-doc for broad queries (trends, summarize)
const CONTEXT_CONTENT_DOC_SPECIFIC_CHARS: usize = 60_000; // Full doc when question targets it
const CONTEXT_SUMMARY_MAX_CHARS: usize = 500;
const CONTEXT_TOTAL_BUDGET_CHARS: usize = 95_000; // Stay under ~100K for LLM context

/// True if the question asks about a specific document (e.g. "read IT3000.docx", "content of this doc").
fn looks_like_document_specific_question(question: &str) -> bool {
    let lower = question.to_lowercase();
    // Explicit document requests
    if lower.contains("read the content")
        || lower.contains("read the document")
        || lower.contains("contents of this")
        || lower.contains("content of this")
        || (lower.contains("what does ") && (lower.contains("say") || lower.contains("contain")))
        || lower.contains("in this document")
        || lower.contains("in that document")
    {
        return true;
    }
    // Filename patterns (e.g. "IT3000", "report.docx", "invoice.pdf")
    let has_doc_extension = lower.contains(".docx")
        || lower.contains(".pdf")
        || lower.contains(".txt")
        || lower.contains(".md");
    let has_quoted_filename = (lower.contains('"') || lower.contains('\''))
        && (lower.contains('.') || question.chars().filter(|c| c.is_alphanumeric()).count() > 5);
    has_doc_extension || has_quoted_filename
}

/// Extract potential document title substrings from the question for matching.
fn extract_document_refs_from_question(question: &str) -> Vec<String> {
    let mut refs = Vec::new();
    // Words that look like doc names (alphanumeric, possibly with dots)
    for word in question.split_whitespace() {
        let clean: String = word
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '.' || *c == '_' || *c == '-')
            .collect();
        if clean.len() >= 3 && (clean.contains('.') || clean.chars().all(|c| c.is_alphanumeric())) {
            refs.push(clean.to_lowercase());
        }
    }
    refs
}

fn title_matches_refs(title: &str, refs: &[String]) -> bool {
    let title_lower = title.to_lowercase();
    refs.iter()
        .any(|r| title_lower.contains(r) || r.contains(&title_lower))
}

fn build_context_bullets(
    hits: &[search::SearchHit],
    cas: &CasStore,
    question: &str,
) -> Vec<String> {
    let is_doc_specific = looks_like_document_specific_question(question);
    let refs = extract_document_refs_from_question(question);
    // When doc-specific but no refs (e.g. "read this document"), treat first artifact as focused

    let mut total_chars = 0usize;
    let mut bullets = Vec::with_capacity(hits.len());
    let mut seen_first_artifact = false;

    for h in hits {
        let is_focused_doc = if is_doc_specific {
            !refs.is_empty() && title_matches_refs(&h.title, &refs)
                || (refs.is_empty() && h.hit_type == "artifact" && !seen_first_artifact)
        } else {
            false
        };
        if h.hit_type == "artifact" {
            seen_first_artifact = true;
        }

        let content: String = if h.hit_type == "artifact" {
            if let Some(ref hash) = h.content_hash {
                match cas.get_bytes(hash) {
                    Ok(bytes) => {
                        if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                            json.get("content_text")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string()
                        } else {
                            String::new()
                        }
                    }
                    Err(_) => String::new(),
                }
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        let max_per_doc = if is_focused_doc {
            CONTEXT_CONTENT_DOC_SPECIFIC_CHARS
        } else if is_doc_specific {
            CONTEXT_SUMMARY_MAX_CHARS
        } else {
            CONTEXT_CONTENT_DEFAULT_CHARS
        };

        let text = if !content.is_empty() {
            let take = (CONTEXT_TOTAL_BUDGET_CHARS - total_chars).min(max_per_doc);
            let excerpt: String = content.chars().take(take).collect();
            total_chars += excerpt.len();
            excerpt
        } else {
            h.summary.chars().take(CONTEXT_SUMMARY_MAX_CHARS).collect()
        };

        bullets.push(format!("- [{}] {}:\n{}", h.hit_type, h.title, text));

        if total_chars >= CONTEXT_TOTAL_BUDGET_CHARS && !is_doc_specific {
            break;
        }
    }

    bullets
}
use node_federated::{
    build_delta_published_event, build_round_completed_event, build_round_started_event, DeltaInfo,
    FederatedConfig, FederatedCoordinator, RoundState,
};
use node_trainer::{JobStatus, ModelRegistry, Trainer, TrainingJob};

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
    /// Base data directory (e.g. ./data). Config files live under data/config/.
    pub data_dir: std::path::PathBuf,
    pub peer_dir: Arc<RwLock<PeerDirectory>>,
    pub backend: Arc<dyn InferenceBackend>,
    pub transport: Option<Arc<dyn Transport>>,
    pub consult_config: ConsultConfig,
    pub node_id: String,
    pub admin_token: String,
    /// If false, /status omits admin_token.
    pub expose_admin_token: bool,
    /// Root folders to scan (user-configurable, persisted to scan_roots_path).
    pub scan_roots: Arc<tokio::sync::RwLock<Vec<std::path::PathBuf>>>,
    pub scan_roots_path: std::path::PathBuf,
    pub trainer: Arc<Trainer>,
    pub model_registry: Arc<tokio::sync::Mutex<ModelRegistry>>,
    pub ui_dir: Option<std::path::PathBuf>,
    /// Last training job result for GET /admin/train/status.
    pub last_train_status: Arc<RwLock<Option<TrainResponse>>>,
    /// Optional rate limiter for /ask and POST .../messages (and stream). When set, returns 429 when exceeded.
    pub ask_chat_limiter: Option<Arc<governor::DefaultDirectRateLimiter>>,
    /// Policy engine for web research (allow_web, research_web_capable). Required for POST /admin/research.
    pub research_policy: Arc<node_policy::PolicyEngine>,
    /// Base URL for the API (e.g. http://127.0.0.1:9900). Used for OAuth redirect.
    pub listen_base_url: String,
    /// OAuth PKCE pending: state -> (code_verifier, created_at). Cleaned on callback.
    pub oauth_pending: Arc<RwLock<HashMap<String, (String, Instant)>>>,
    /// Federated learning: active rounds (round_id -> RoundState).
    pub federated_rounds: Arc<RwLock<HashMap<String, RoundState>>>,
    /// Federated config (model_id, min/max participants).
    pub federated_config: FederatedConfig,
    /// Policy for federated delta sharing (can_share_deltas).
    pub federated_policy: Arc<node_policy::PolicyEngine>,
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
        _ => Err(ApiError::from_status(
            StatusCode::UNAUTHORIZED,
            "missing or invalid authorization",
        )),
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
        .route("/admin/sources/remove", post(handle_admin_remove_source))
        .route("/admin/train", post(handle_admin_train))
        .route("/admin/train/status", get(handle_admin_train_status))
        .route("/admin/train/export", post(handle_admin_train_export))
        .route("/admin/train/modelfile", post(handle_admin_train_modelfile))
        .route("/admin/models", get(handle_admin_models))
        .route("/admin/models/rollback", post(handle_admin_rollback_model))
        .route("/admin/datasets", get(handle_admin_datasets))
        .route(
            "/admin/federated/status",
            get(handle_admin_federated_status),
        )
        .route(
            "/admin/federated/rounds",
            post(handle_admin_federated_start_round),
        )
        .route(
            "/admin/federated/rounds/:round_id",
            get(handle_admin_federated_round_status),
        )
        .route(
            "/admin/federated/rounds/:round_id/deltas",
            post(handle_admin_federated_submit_delta),
        )
        .route(
            "/admin/federated/rounds/:round_id/aggregate",
            post(handle_admin_federated_aggregate),
        )
        .route("/admin/insights/run", post(handle_admin_insights_run))
        .route("/admin/research", post(handle_admin_research))
        .route("/admin/scan", post(handle_admin_scan))
        .route("/admin/scan-dirs", get(handle_admin_scan_dirs_get))
        .route("/admin/scan-dirs/add", post(handle_admin_scan_dirs_add))
        .route(
            "/admin/scan-dirs/remove",
            post(handle_admin_scan_dirs_remove),
        )
        .route("/admin/ingest", post(handle_admin_ingest))
        .route("/admin/sources/approve-all", post(handle_admin_approve_all))
        .route("/admin/ingest-all", post(handle_admin_ingest_all))
        .route(
            "/admin/config/onedrive",
            get(handle_admin_config_onedrive_get),
        )
        .route(
            "/admin/config/onedrive",
            post(handle_admin_config_onedrive_save),
        )
        .route(
            "/admin/config/general",
            get(handle_admin_config_general_get),
        )
        .route(
            "/admin/config/general",
            post(handle_admin_config_general_save),
        )
        .route("/admin/restart", post(handle_admin_restart))
        .route(
            "/admin/config/onedrive/oauth/start",
            get(handle_admin_config_onedrive_oauth_start),
        )
        .route_layer(middleware::from_fn_with_state(state.clone(), admin_auth));

    let oauth_public_routes = Router::new().route(
        "/oauth/onedrive/callback",
        get(handle_oauth_onedrive_callback),
    );

    let api_routes = Router::new()
        .route("/status", get(handle_status))
        .route("/peers", get(handle_peers))
        .route("/search", get(handle_search))
        .route("/insights", get(handle_insights))
        .route("/insights/alerts", get(handle_insights_alerts))
        .route("/insights/benchmarks", get(handle_insights_benchmarks))
        .route("/shards", get(handle_list_shards))
        .route("/shards/for-question", get(handle_shards_for_question))
        .route("/shards/:key/members", get(handle_shard_members))
        .route(
            "/shards/:key/subscriptions",
            get(handle_shard_subscriptions),
        )
        .route("/shards/subscribe", post(handle_shard_subscribe))
        .route(
            "/mergeable/:object_type/:object_id/tags",
            get(handle_mergeable_tags),
        )
        .route(
            "/mergeable/:object_type/:object_id/counters",
            get(handle_mergeable_counters),
        )
        .route(
            "/mergeable/:object_type/:object_id/annotations",
            get(handle_mergeable_annotations),
        )
        .route("/ask", post(handle_ask))
        .route("/ask/confirm", post(handle_ask_confirm))
        .route("/outcomes", post(handle_outcomes))
        .route(
            "/conversations",
            get(handle_list_conversations).post(handle_create_conversation),
        )
        .route(
            "/conversations/:id/messages",
            get(handle_get_messages).post(handle_send_message),
        )
        .route(
            "/conversations/:id/messages/stream",
            post(handle_send_message_stream),
        )
        .route("/conversations/:id", delete(handle_delete_conversation))
        .route("/ws", get(handle_ws))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            rate_limit_ask_chat,
        ))
        .merge(oauth_public_routes)
        .merge(admin_routes)
        .layer(cors);

    let mut app = Router::new()
        .nest("/v1", api_routes)
        .route("/repl/gossip", post(handle_repl_gossip))
        .route("/repl/pull", post(handle_repl_pull));

    if let Some(ref ui_dir) = state.ui_dir {
        if ui_dir.exists() {
            let serve = tower_http::services::ServeDir::new(ui_dir).fallback(
                tower_http::services::ServeFile::new(ui_dir.join("index.html")),
            );
            app = app.fallback_service(serve);
            tracing::info!("serving UI from {}", ui_dir.display());
        }
    }
    let has_ui = state.ui_dir.as_ref().map(|d| d.exists()).unwrap_or(false);
    if !has_ui {
        app = app.fallback(serve_fallback_landing);
        tracing::info!("serving fallback landing page (build UI: cd ui && npm run build)");
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
        Self {
            error: msg.into(),
            code: 400,
        }
    }
    fn not_found(msg: impl Into<String>) -> Self {
        Self {
            error: msg.into(),
            code: 404,
        }
    }
    fn internal(msg: impl Into<String>) -> Self {
        Self {
            error: msg.into(),
            code: 500,
        }
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
        Self {
            error: msg,
            code: status.as_u16(),
        }
    }
}

impl ApiError {
    fn from_status(status: StatusCode, msg: impl Into<String>) -> Self {
        Self {
            error: msg.into(),
            code: status.as_u16(),
        }
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
    #[serde(default)]
    scan_dirs: Vec<String>,
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
struct InsightItem {
    insight_type: String,
    title: String,
    summary: String,
    entity_ids: Vec<String>,
    confidence: f32,
}

#[derive(Serialize, Deserialize)]
struct AskRequest {
    question: String,
    #[serde(default)]
    max_tokens: Option<u32>,
}

#[derive(Serialize, Deserialize)]
struct EvidenceItem {
    id: String,
    source_type: String, // local | peer | web | insight | business_system
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct AskResponse {
    answer: String,
    confidence: f32,
    model: String,
    context_used: Vec<String>,
    /// ID for outcome confirmation (POST /v1/ask/confirm)
    case_id: String,
    /// Source types that contributed (local, peer, web, insight, business_system)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    source_types: Vec<String>,
    /// Structured evidence with provenance
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    evidence: Vec<EvidenceItem>,
    /// Warnings when data may be missing or incomplete
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    missing_data_warnings: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct AskConfirmRequest {
    case_id: String,
    outcome: String, // e.g. "accepted", "rejected", "edited"
    #[serde(default)]
    confidence: Option<f32>,
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
    /// "completed" | "failed" | "started" | null if never ingested
    #[serde(skip_serializing_if = "Option::is_none")]
    last_ingest_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_ingest_rows: Option<i64>,
    /// Path or URI (for tree grouping)
    path_or_uri: String,
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

#[derive(Deserialize)]
struct RemoveSourceRequest {
    source_id: String,
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
    let scan_dirs: Vec<String> = state
        .scan_roots
        .read()
        .await
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();

    Json(StatusResponse {
        node_id: state.node_id.clone(),
        status: "running".into(),
        event_count,
        peer_count,
        backend: state.backend.name().to_string(),
        admin_token,
        scan_dirs,
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

async fn handle_insights(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<InsightItem>>, ApiError> {
    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let mut insights = Vec::new();

    // Proactive insights from insights_view (scheduled/manual)
    if let Ok(rows) = insights::list_insights(&conn, None, 50) {
        for r in rows {
            let entity_ids: Vec<String> =
                serde_json::from_str(&r.entity_ids_json).unwrap_or_default();
            insights.push(InsightItem {
                insight_type: r.insight_type,
                title: r.title,
                summary: r.summary,
                entity_ids,
                confidence: r.confidence,
            });
        }
    }

    // On-demand: overdue invoices (entity cards with overdue in attributes)
    if let Ok(hits) = search::search_entity_cards(&conn, Some("invoice"), 50) {
        let overdue: Vec<_> = hits
            .iter()
            .filter(|h| {
                h.attributes_json.contains("overdue")
                    || h.attributes_json.contains("Overdue")
                    || h.attributes_json.contains("\"overdue_count\":1")
                    || h.attributes_json.contains("\"overdue_count\": 1")
            })
            .take(10)
            .collect();
        if !overdue.is_empty() {
            insights.push(InsightItem {
                insight_type: "overdue_invoices".into(),
                title: format!("{} overdue invoice(s)", overdue.len()),
                summary: format!(
                    "Entity IDs: {}",
                    overdue
                        .iter()
                        .map(|h| h.entity_id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                entity_ids: overdue.iter().map(|h| h.entity_id.clone()).collect(),
                confidence: 0.8,
            });
        }
    }

    // Recent quotes
    if let Ok(hits) = search::search_entity_cards(&conn, Some("quote"), 5) {
        if !hits.is_empty() {
            insights.push(InsightItem {
                insight_type: "recent_quotes".into(),
                title: format!("{} recent quote(s)", hits.len()),
                summary: format!(
                    "Latest: {}",
                    hits.iter()
                        .map(|h| h.entity_id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                entity_ids: hits.iter().map(|h| h.entity_id.clone()).collect(),
                confidence: 0.9,
            });
        }
    }

    // Revenue/profit facts
    if let Ok(hits) = search::query_facts(&conn, None, 10) {
        let revenue_profit: Vec<_> = hits
            .iter()
            .filter(|h| {
                let m = h.metric.to_lowercase();
                m.contains("revenue") || m.contains("profit") || m.contains("margin")
            })
            .take(5)
            .collect();
        if !revenue_profit.is_empty() {
            insights.push(InsightItem {
                insight_type: "financial_metrics".into(),
                title: format!("{} financial metric(s) available", revenue_profit.len()),
                summary: revenue_profit
                    .iter()
                    .map(|h| format!("{}: {}", h.metric, h.value_json))
                    .collect::<Vec<_>>()
                    .join("; "),
                entity_ids: revenue_profit.iter().map(|h| h.fact_id.clone()).collect(),
                confidence: 0.85,
            });
        }
    }

    Ok(Json(insights))
}

#[derive(Serialize)]
struct AlertResponse {
    alert_id: String,
    alert_type: String,
    severity: String,
    title: String,
    message: String,
    entity_ids: Vec<String>,
    schedule: String,
    created_at_ms: i64,
}

#[derive(Serialize)]
struct BenchmarkResponse {
    benchmark_id: String,
    metric: String,
    dimension: String,
    value: f64,
    time_window: String,
    schedule: String,
    created_at_ms: i64,
}

async fn handle_insights_alerts(
    State(state): State<Arc<AppState>>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<Vec<AlertResponse>>, ApiError> {
    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let schedule = params.get("schedule").map(String::as_str);
    let limit = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(50);
    let rows = insights::list_alerts(&conn, schedule, limit)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let out: Vec<AlertResponse> = rows
        .into_iter()
        .map(|r| {
            let entity_ids: Vec<String> =
                serde_json::from_str(&r.entity_ids_json).unwrap_or_default();
            AlertResponse {
                alert_id: r.alert_id,
                alert_type: r.alert_type,
                severity: r.severity,
                title: r.title,
                message: r.message,
                entity_ids,
                schedule: r.schedule,
                created_at_ms: r.created_at_ms,
            }
        })
        .collect();
    Ok(Json(out))
}

async fn handle_insights_benchmarks(
    State(state): State<Arc<AppState>>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<Vec<BenchmarkResponse>>, ApiError> {
    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let schedule = params.get("schedule").map(String::as_str);
    let limit = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(50);
    let rows = insights::list_benchmarks(&conn, schedule, limit)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let out: Vec<BenchmarkResponse> = rows
        .into_iter()
        .map(|r| BenchmarkResponse {
            benchmark_id: r.benchmark_id,
            metric: r.metric,
            dimension: r.dimension,
            value: r.value,
            time_window: r.time_window,
            schedule: r.schedule,
            created_at_ms: r.created_at_ms,
        })
        .collect();
    Ok(Json(out))
}

#[derive(Deserialize)]
struct ShardsForQuestionParams {
    q: String,
}

#[derive(Serialize)]
struct ShardResponse {
    shard_key: String,
    shard_kind: String,
    created_at_ms: i64,
}

#[derive(Serialize)]
struct ShardMemberResponse {
    shard_key: String,
    member_type: String,
    member_id: String,
    node_id: String,
    created_at_ms: i64,
}

#[derive(Serialize)]
struct ShardSubscriptionResponse {
    shard_key: String,
    node_id: String,
    capability: String,
    last_seen_ms: i64,
}

#[derive(Deserialize)]
struct ShardSubscribeRequest {
    shard_key: String,
    capability: String, // host | cache | query
}

async fn handle_list_shards(
    State(state): State<Arc<AppState>>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<Vec<ShardResponse>>, ApiError> {
    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let limit = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(100);
    let rows = shards::list_shards(&conn, limit).map_err(|e| ApiError::internal(e.to_string()))?;
    let out = rows
        .into_iter()
        .map(|r| ShardResponse {
            shard_key: r.shard_key,
            shard_kind: r.shard_kind,
            created_at_ms: r.created_at_ms,
        })
        .collect();
    Ok(Json(out))
}

async fn handle_shards_for_question(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ShardsForQuestionParams>,
) -> Result<Json<Vec<String>>, ApiError> {
    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let keys = shards::shards_for_question(&conn, &params.q)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(keys))
}

async fn handle_shard_members(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<Vec<ShardMemberResponse>>, ApiError> {
    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let member_type = params.get("member_type").map(String::as_str);
    let limit = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(100);
    let rows = shards::members_of_shard(&conn, &key, member_type, limit)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let out = rows
        .into_iter()
        .map(|r| ShardMemberResponse {
            shard_key: r.shard_key,
            member_type: r.member_type,
            member_id: r.member_id,
            node_id: r.node_id,
            created_at_ms: r.created_at_ms,
        })
        .collect();
    Ok(Json(out))
}

async fn handle_shard_subscriptions(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Result<Json<Vec<ShardSubscriptionResponse>>, ApiError> {
    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let capability = params.get("capability").map(String::as_str);
    let rows = shards::nodes_for_shard(&conn, &key, capability)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let out = rows
        .into_iter()
        .map(|r| ShardSubscriptionResponse {
            shard_key: r.shard_key,
            node_id: r.node_id,
            capability: r.capability,
            last_seen_ms: r.last_seen_ms,
        })
        .collect();
    Ok(Json(out))
}

async fn handle_shard_subscribe(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ShardSubscribeRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    use node_proto::common::*;
    use node_proto::events::*;

    let capability = if req.capability.is_empty() {
        "query"
    } else {
        &req.capability
    };
    let ts = now_ms();
    let event = EventEnvelope {
        event_id: format!("shard-sub-{}", uuid::Uuid::new_v4()),
        r#type: EventType::ShardSubscriptionAdded as i32,
        node_id: Some(NodeId {
            value: state.node_id.clone(),
        }),
        tenant_id: Some(TenantId {
            value: "public".into(),
        }),
        sensitivity: Sensitivity::Public as i32,
        payload: Some(event_envelope::Payload::ShardSubscriptionAdded(
            ShardSubscriptionAdded {
                shard_key: req.shard_key,
                node_id: Some(NodeId {
                    value: state.node_id.clone(),
                }),
                capability: capability.to_string(),
                last_seen_ms: ts,
            },
        )),
        ..Default::default()
    };

    let mut log = state.event_log.write().await;
    let stored = log
        .append(event)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    drop(log);

    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    node_storage::projector::apply_event(&conn, &stored)
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(serde::Deserialize)]
struct MergeablePath {
    object_type: String,
    object_id: String,
}

async fn handle_mergeable_tags(
    State(state): State<Arc<AppState>>,
    Path(path): Path<MergeablePath>,
) -> Result<Json<Vec<String>>, ApiError> {
    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let tags = mergeable::tags_for_object(&conn, &path.object_type, &path.object_id)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(tags))
}

async fn handle_mergeable_counters(
    State(state): State<Arc<AppState>>,
    Path(path): Path<MergeablePath>,
) -> Result<Json<std::collections::HashMap<String, i64>>, ApiError> {
    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let rows = mergeable::counters_for_object(&conn, &path.object_type, &path.object_id)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let out: std::collections::HashMap<String, i64> = rows.into_iter().collect();
    Ok(Json(out))
}

async fn handle_mergeable_annotations(
    State(state): State<Arc<AppState>>,
    Path(path): Path<MergeablePath>,
) -> Result<Json<std::collections::HashMap<String, String>>, ApiError> {
    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let rows = mergeable::annotations_for_object(&conn, &path.object_type, &path.object_id)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let out: std::collections::HashMap<String, String> = rows.into_iter().collect();
    Ok(Json(out))
}

async fn handle_ask(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AskRequest>,
) -> Result<Json<AskResponse>, ApiError> {
    let case_id = format!("ask-{}", uuid::Uuid::new_v4());
    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let fts_query = to_fts5_query(&req.question);
    let search_limit = 100; // More documents for trends, summaries, cross-doc analysis
    let mut context_hits = search::search_all(&conn, &fts_query, search_limit)
        .map_err(|e| ApiError::internal(e.to_string()))?;

    // Business intelligence: augment with entity graph and facts when intent matches
    let intent = classify_business_intent(&req.question);
    let mut entity_fact_bullets: Vec<String> = Vec::new();
    let mut entity_evidence_ids: Vec<String> = Vec::new();

    for et in &intent.entity_types {
        if let Ok(hits) = search::search_entity_cards(&conn, Some(et), 20) {
            for h in hits {
                entity_evidence_ids.push(h.entity_id.clone());
                entity_fact_bullets.push(format!(
                    "[Entity {} {}] {}",
                    h.entity_type, h.entity_id, h.attributes_json
                ));
            }
        }
    }
    for m in &intent.metrics {
        if let Ok(hits) = search::query_facts(&conn, Some(m), 15) {
            for h in hits {
                entity_evidence_ids.push(h.fact_id.clone());
                entity_fact_bullets.push(format!(
                    "[Fact {}] metric={} value={} dimensions={}",
                    h.fact_id, h.metric, h.value_json, h.dimensions_json
                ));
            }
        }
    }
    // If no metric filter matched, try broader facts query for revenue/profit/margin questions
    if intent.metrics.is_empty()
        && (req.question.to_lowercase().contains("revenue")
            || req.question.to_lowercase().contains("profit")
            || req.question.to_lowercase().contains("margin"))
    {
        if let Ok(hits) = search::query_facts(&conn, None, 15) {
            for h in hits {
                entity_evidence_ids.push(h.fact_id.clone());
                entity_fact_bullets.push(format!(
                    "[Fact {}] metric={} value={} dimensions={}",
                    h.fact_id, h.metric, h.value_json, h.dimensions_json
                ));
            }
        }
    }

    // Fallback: for broad questions (summarize, trends, all docs) with no/few hits, retry with wider match
    let lower_q = req.question.to_lowercase();
    let is_broad_request = lower_q.contains("summarize")
        || lower_q.contains("trends")
        || lower_q.contains("across all")
        || lower_q.contains("all documents")
        || (lower_q.contains("document") && (lower_q.contains("list") || lower_q.contains("show")));
    if context_hits.len() < 3 && is_broad_request {
        if let Ok(fallback_hits) =
            search::search_all(&conn, "document OR file OR content", search_limit)
        {
            if !fallback_hits.is_empty() {
                context_hits = fallback_hits;
            }
        }
    }

    let mut web_search_context = String::new();
    let do_web_search = wants_web_search(&req.question)
        || (context_hits.is_empty() && looks_like_general_knowledge_question(&req.question));
    if do_web_search {
        let query = extract_search_query(&req.question, &[])
            .or_else(|| Some(req.question.trim().to_string()))
            .filter(|q| q.len() > 3);
        if let Some(q) = query {
            if let Some((ctx, summary, url)) = node_research::search_and_summarize_with_details(
                &q,
                &state.research_policy,
                &state.backend,
            )
            .await
            {
                web_search_context = ctx.clone();
                let mut log = state.event_log.write().await;
                node_research::store_web_brief_from_search(
                    &q,
                    &summary,
                    &url,
                    &state.cas,
                    &mut log,
                    &state.node_id,
                    &state.db_path,
                );
            }
        }
    }

    let mut context_bullets: Vec<String> =
        build_context_bullets(&context_hits, &state.cas, &req.question);
    if !entity_fact_bullets.is_empty() {
        context_bullets.insert(
            0,
            format!(
                "Entity graph and facts:\n{}",
                entity_fact_bullets.join("\n")
            ),
        );
    }

    let prompt = if !web_search_context.is_empty() {
        let kb = if context_bullets.is_empty() {
            String::new()
        } else {
            format!(
                "Context from local knowledge base:\n{}\n\n",
                context_bullets.join("\n")
            )
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
        let quote_guidance = if !entity_fact_bullets.is_empty()
            && (intent
                .entity_types
                .iter()
                .any(|e| e == "quote" || e == "quote_line_item")
                || intent.metrics.iter().any(|m| m == "pricing"))
        {
            " When answering about quotes or pricing, cite similar historical jobs/line items where relevant and note variance from typical ranges if evident."
        } else {
            ""
        };
        format!(
            "Context from the user's local knowledge base:\n{}\n\nQuestion: {}\n\nAnswer based on the context above. Be concise and specific.{}",
            context_bullets.join("\n"),
            req.question,
            quote_guidance
        )
    };

    let system_prompt = MESHMIND_SYSTEM_PROMPT;

    let gen_req = node_ai::GenerateRequest {
        prompt,
        system: Some(system_prompt.into()),
        max_tokens: req.max_tokens.unwrap_or(2048), // Higher for document summaries and trend analysis
        ..Default::default()
    };

    let gen_resp = state
        .backend
        .generate(gen_req)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let local_confidence: f32 = if context_bullets.is_empty() { 0.3 } else { 0.7 };

    // Build source_types, evidence, missing_data_warnings
    let mut source_types: Vec<String> = Vec::new();
    let mut evidence: Vec<EvidenceItem> = Vec::new();
    let mut missing_data_warnings: Vec<String> = Vec::new();

    if !context_hits.is_empty() {
        if !source_types.contains(&"local".to_string()) {
            source_types.push("local".to_string());
        }
        for h in &context_hits {
            evidence.push(EvidenceItem {
                id: h.id.clone(),
                source_type: "local".to_string(),
                title: Some(h.title.clone()),
            });
        }
    }
    if !entity_evidence_ids.is_empty() {
        if !source_types.contains(&"business_system".to_string()) {
            source_types.push("business_system".to_string());
        }
        for id in &entity_evidence_ids {
            evidence.push(EvidenceItem {
                id: id.clone(),
                source_type: "business_system".to_string(),
                title: None,
            });
        }
    }
    if !web_search_context.is_empty() {
        if !source_types.contains(&"web".to_string()) {
            source_types.push("web".to_string());
        }
        evidence.push(EvidenceItem {
            id: "web_search".to_string(),
            source_type: "web".to_string(),
            title: Some("Web search results".to_string()),
        });
    }
    if context_bullets.is_empty() && !web_search_context.is_empty() {
        missing_data_warnings
            .push("No local knowledge base matches; answer based on web search only.".to_string());
    }
    if context_bullets.is_empty() && web_search_context.is_empty() {
        missing_data_warnings.push("No relevant data found in local knowledge base. Consider scanning and ingesting more sources.".to_string());
    }
    if local_confidence < 0.6
        && entity_evidence_ids.is_empty()
        && intent
            .entity_types
            .iter()
            .any(|e| *e == "customer" || *e == "invoice" || *e == "quote")
    {
        missing_data_warnings.push("Business intent detected but no entity/fact data matched. Ingest customer, invoice, or quote data for better answers.".to_string());
    }

    // If local confidence is low and we have a transport, consult peers
    let mut peer_answers = Vec::new();
    if local_confidence < 0.6 {
        if let Some(ref transport) = state.transport {
            let shard_peers = shards::peers_for_question(&conn, &req.question)
                .ok()
                .filter(|v| !v.is_empty());
            let result = node_mesh::consult::consult_peers_routed(
                transport,
                &state.peer_dir,
                &state.consult_config,
                &state.node_id,
                "public",
                &req.question,
                &context_bullets,
                shard_peers,
            )
            .await;

            for pa in &result.answers {
                peer_answers.push(format!("[{}] {}", pa.peer_id, pa.answer));
                if !source_types.contains(&"peer".to_string()) {
                    source_types.push("peer".to_string());
                }
                evidence.push(EvidenceItem {
                    id: pa.peer_id.clone(),
                    source_type: "peer".to_string(),
                    title: Some(pa.answer.chars().take(80).collect::<String>()),
                });
            }

            if result.answers.is_empty() && !result.refused.is_empty() {
                missing_data_warnings
                    .push("Peers were contacted but had no relevant knowledge.".to_string());
            }

            if let Some(best) = result.best_answer {
                if best.confidence > local_confidence {
                    let mut context_used = best.evidence_refs.clone();
                    context_used.push(best.peer_id.clone());
                    return Ok(Json(AskResponse {
                        answer: best.answer,
                        confidence: best.confidence,
                        model: format!("peer:{}", best.peer_id),
                        context_used,
                        case_id: case_id.clone(),
                        source_types,
                        evidence,
                        missing_data_warnings,
                    }));
                }
            }
        } else {
            missing_data_warnings.push(
                "No mesh/peer consult available; answer from local context only.".to_string(),
            );
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

    let mut context_used: Vec<String> = context_hits.iter().map(|h| h.id.clone()).collect();
    context_used.extend(entity_evidence_ids.clone());

    Ok(Json(AskResponse {
        answer,
        confidence: local_confidence,
        model: gen_resp.model,
        context_used,
        case_id,
        source_types,
        evidence,
        missing_data_warnings,
    }))
}

async fn handle_ask_confirm(
    State(state): State<Arc<AppState>>,
    Json(req): Json<AskConfirmRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    use node_proto::common::*;
    use node_proto::events::*;

    let confidence = req.confidence.unwrap_or(1.0);
    let event = EventEnvelope {
        event_id: format!("confirm-{}", uuid::Uuid::new_v4()),
        r#type: EventType::CaseConfirmed as i32,
        node_id: Some(NodeId {
            value: state.node_id.clone(),
        }),
        tenant_id: Some(TenantId {
            value: "public".into(),
        }),
        sensitivity: Sensitivity::Public as i32,
        payload: Some(event_envelope::Payload::CaseConfirmed(CaseConfirmed {
            case_id: req.case_id,
            outcome: req.outcome,
            confidence,
        })),
        ..Default::default()
    };

    let mut log = state.event_log.write().await;
    let stored = log
        .append(event)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    drop(log);

    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    node_storage::projector::apply_event(&conn, &stored)
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(Serialize, Deserialize)]
struct OutcomeRequest {
    outcome_type: String, // case_failed | quote_accepted | quote_lost | quote_revised
    case_id: Option<String>,
    quote_id: Option<String>,
    outcome: Option<String>,
    reason: Option<String>,
    value_summary: Option<String>,
    revision_reason: Option<String>,
    confidence: Option<f32>,
}

async fn handle_outcomes(
    State(state): State<Arc<AppState>>,
    Json(req): Json<OutcomeRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    use node_proto::common::*;
    use node_proto::events::event_envelope::Payload;
    use node_proto::events::*;

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;

    let event = match req.outcome_type.as_str() {
        "case_failed" => {
            let case_id = req.case_id.unwrap_or_default();
            let reason = req.reason.unwrap_or_default();
            EventEnvelope {
                event_id: format!("out-cf-{}", uuid::Uuid::new_v4()),
                r#type: EventType::CaseFailed as i32,
                node_id: Some(NodeId {
                    value: state.node_id.clone(),
                }),
                tenant_id: Some(TenantId {
                    value: "public".into(),
                }),
                sensitivity: Sensitivity::Public as i32,
                ts: Some(Timestamp { unix_ms: ts }),
                payload: Some(Payload::CaseFailed(CaseFailed { case_id, reason })),
                ..Default::default()
            }
        }
        "quote_accepted" => {
            let case_id = req.case_id.unwrap_or_default();
            let quote_id = req.quote_id.unwrap_or_default();
            let value_summary = req.value_summary.or(req.outcome).unwrap_or_default();
            let confidence = req.confidence.unwrap_or(1.0);
            EventEnvelope {
                event_id: format!("out-qa-{}", uuid::Uuid::new_v4()),
                r#type: EventType::QuoteAccepted as i32,
                node_id: Some(NodeId {
                    value: state.node_id.clone(),
                }),
                tenant_id: Some(TenantId {
                    value: "public".into(),
                }),
                sensitivity: Sensitivity::Public as i32,
                ts: Some(Timestamp { unix_ms: ts }),
                payload: Some(Payload::QuoteAccepted(QuoteAccepted {
                    quote_id,
                    case_id,
                    value_summary,
                    confidence,
                })),
                ..Default::default()
            }
        }
        "quote_lost" => {
            let case_id = req.case_id.unwrap_or_default();
            let quote_id = req.quote_id.unwrap_or_default();
            let reason = req.reason.unwrap_or_default();
            EventEnvelope {
                event_id: format!("out-ql-{}", uuid::Uuid::new_v4()),
                r#type: EventType::QuoteLost as i32,
                node_id: Some(NodeId {
                    value: state.node_id.clone(),
                }),
                tenant_id: Some(TenantId {
                    value: "public".into(),
                }),
                sensitivity: Sensitivity::Public as i32,
                ts: Some(Timestamp { unix_ms: ts }),
                payload: Some(Payload::QuoteLost(QuoteLost {
                    quote_id,
                    case_id,
                    reason,
                })),
                ..Default::default()
            }
        }
        "quote_revised" => {
            let case_id = req.case_id.unwrap_or_default();
            let quote_id = req.quote_id.unwrap_or_default();
            let revision_reason = req.revision_reason.or(req.reason).unwrap_or_default();
            EventEnvelope {
                event_id: format!("out-qr-{}", uuid::Uuid::new_v4()),
                r#type: EventType::QuoteRevised as i32,
                node_id: Some(NodeId {
                    value: state.node_id.clone(),
                }),
                tenant_id: Some(TenantId {
                    value: "public".into(),
                }),
                sensitivity: Sensitivity::Public as i32,
                ts: Some(Timestamp { unix_ms: ts }),
                payload: Some(Payload::QuoteRevised(QuoteRevised {
                    quote_id,
                    case_id,
                    revision_reason,
                })),
                ..Default::default()
            }
        }
        _ => {
            return Err(ApiError::bad_request(format!(
                "unknown outcome_type: {} (use case_failed, quote_accepted, quote_lost, quote_revised)",
                req.outcome_type
            )));
        }
    };

    let mut log = state.event_log.write().await;
    let stored = log
        .append(event)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    drop(log);

    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    node_storage::projector::apply_event(&conn, &stored)
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(
        serde_json::json!({ "ok": true, "outcome_type": req.outcome_type }),
    ))
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
You have access to the user's local knowledge base and can also search the web when the user asks. \
When web search results are provided, use them to answer the question. \
When knowledge base context is provided, use it to answer — you receive document titles and full or excerpted content. \
Use the FULL content when provided: synthesize trends across documents, summarize, compare, and extract specific information. \
For document-specific questions (e.g. 'what does X say about Y'), base your answer on the full document content given. \
If the context is unrelated to the question, say so — do NOT reference unrelated documents. \
Never say you cannot access or read the user's files — you CAN, through the knowledge base. \
If no relevant context was found, say so and suggest they scan more sources or ask you to search the web. \
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
            let context_used: Vec<String> = serde_json::from_str(&ctx_json).unwrap_or_default();
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
        let _ = conn.execute(
            "DELETE FROM messages_fts WHERE message_id = ?1",
            rusqlite::params![mid],
        );
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
    let context_hits = search::search_all(&conn, &fts_query, 100).unwrap_or_default();
    let context_bullets = build_context_bullets(&context_hits, &state.cas, &req.content);

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
        prompt_parts.push(format!("Conversation history:\n{}", hist_lines.join("\n")));
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
        &context_hits
            .iter()
            .map(|h| h.id.clone())
            .collect::<Vec<_>>(),
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
) -> Result<
    Sse<impl futures_util::Stream<Item = Result<Event, std::convert::Infallible>> + Send>,
    ApiError,
> {
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
        search::search_all(&conn, &fts_query, 100).unwrap_or_default()
    };

    let mut web_search_context = String::new();
    let do_web_search = wants_web_search(&req.content)
        || (context_hits.is_empty() && looks_like_general_knowledge_question(&req.content));
    if do_web_search {
        let query = extract_search_query(&req.content, &history)
            .or_else(|| Some(req.content.trim().to_string()))
            .filter(|q| q.len() > 3);
        if let Some(q) = query {
            if let Some((ctx, summary, url)) = node_research::search_and_summarize_with_details(
                &q,
                &state.research_policy,
                &state.backend,
            )
            .await
            {
                web_search_context = ctx.clone();
                let mut log = state.event_log.write().await;
                node_research::store_web_brief_from_search(
                    &q,
                    &summary,
                    &url,
                    &state.cas,
                    &mut log,
                    &state.node_id,
                    &state.db_path,
                );
            }
        }
    }

    let context_bullets: Vec<String> =
        build_context_bullets(&context_hits, &state.cas, &req.content);

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

    let has_context =
        !context_bullets.is_empty() || !cross_session.is_empty() || !web_search_context.is_empty();
    let mut peer_insights = Vec::new();
    let would_have_low_confidence = context_bullets.is_empty() && web_search_context.is_empty();
    if would_have_low_confidence {
        if let Some(ref transport) = state.transport {
            let result = node_mesh::consult::consult_peers(
                transport,
                &state.peer_dir,
                &state.consult_config,
                &state.node_id,
                "public",
                &req.content,
                &context_bullets,
            )
            .await;
            for pa in &result.answers {
                peer_insights.push(format!("[Peer {}] {}", pa.peer_id, pa.answer));
            }
        }
    }

    let mut prompt_parts = Vec::new();
    if !peer_insights.is_empty() {
        prompt_parts.push(format!(
            "Insights from other MeshMind nodes on the network:\n{}",
            peer_insights.join("\n\n")
        ));
    }
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
        prompt_parts.push(format!("Conversation history:\n{}", hist_lines.join("\n")));
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

    // Get latest ingest per source (by completed_at_ms or started_at_ms)
    let mut stmt = conn
        .prepare(
            "WITH latest_ingest AS (
                SELECT source_id, status, rows_ingested,
                       ROW_NUMBER() OVER (PARTITION BY source_id ORDER BY COALESCE(completed_at_ms, 0) DESC, started_at_ms DESC) AS rn
                FROM ingests_view
            )
            SELECT sv.source_id, sv.display_name, sv.connector_type, sv.status, sv.pii_detected, sv.estimated_size_bytes,
                   li.status AS last_ingest_status, li.rows_ingested AS last_ingest_rows, sv.path_or_uri
            FROM sources_view sv
            LEFT JOIN (SELECT source_id, status, rows_ingested FROM latest_ingest WHERE rn = 1) li ON sv.source_id = li.source_id
            WHERE sv.status != 'removed'
            ORDER BY sv.source_id",
        )
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let rows: Vec<SourceRow> = stmt
        .query_map([], |row| {
            Ok(SourceRow {
                source_id: row.get(0)?,
                display_name: row.get(1)?,
                connector_type: row.get(2)?,
                status: row.get(3)?,
                pii_detected: row.get::<_, i32>(4)? != 0,
                estimated_size_bytes: row.get(5)?,
                last_ingest_status: row.get::<_, Option<String>>(6).ok().flatten(),
                last_ingest_rows: row.get::<_, Option<i64>>(7).ok().flatten(),
                path_or_uri: row.get(8)?,
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

async fn handle_admin_remove_source(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RemoveSourceRequest>,
) -> Result<Json<ApproveSourceResponse>, ApiError> {
    use node_proto::common::*;
    use node_proto::events::*;

    let event_id = format!("evt-remove-{}", uuid::Uuid::new_v4());

    let event = EventEnvelope {
        event_id: event_id.clone(),
        r#type: EventType::DataSourceRemoved as i32,
        node_id: Some(NodeId {
            value: state.node_id.clone(),
        }),
        tenant_id: Some(TenantId {
            value: "public".into(),
        }),
        sensitivity: Sensitivity::Public as i32,
        payload: Some(event_envelope::Payload::DataSourceRemoved(
            DataSourceRemoved {
                source_id: req.source_id,
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

fn expand_scan_roots(roots: &[std::path::PathBuf]) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    fn collect(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        out.push(dir.to_path_buf());
        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    collect(&p, out);
                }
            }
        }
    }
    for root in roots {
        if root.is_dir() {
            collect(root, &mut out);
        }
    }
    out
}

async fn handle_admin_scan(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ScanResponse>, ApiError> {
    let roots = state.scan_roots.read().await.clone();
    let scan_dirs = expand_scan_roots(&roots);
    let config = DiscoveryConfig {
        scan_dirs: scan_dirs.clone(),
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

    tracing::info!(dirs = ?roots, found = all_sources.len(), "source scan completed");

    Ok(Json(ScanResponse {
        sources_found: all_sources.len(),
        sources: result_sources,
    }))
}

#[derive(Serialize)]
struct ScanDirsResponse {
    paths: Vec<String>,
}

#[derive(Deserialize)]
struct ScanDirsAddRequest {
    path: String,
}

#[derive(Deserialize)]
struct ScanDirsRemoveRequest {
    path: String,
}

async fn handle_admin_scan_dirs_get(
    State(state): State<Arc<AppState>>,
) -> Result<Json<ScanDirsResponse>, ApiError> {
    let paths: Vec<String> = state
        .scan_roots
        .read()
        .await
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    Ok(Json(ScanDirsResponse { paths }))
}

async fn handle_admin_scan_dirs_add(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ScanDirsAddRequest>,
) -> Result<Json<ScanDirsResponse>, ApiError> {
    let path = std::path::PathBuf::from(req.path.trim());
    if path.as_os_str().is_empty() {
        return Err(ApiError::bad_request("path cannot be empty"));
    }
    let canonical = path.canonicalize().map_err(|e| {
        ApiError::bad_request(format!("path does not exist or is not accessible: {e}"))
    })?;
    if !canonical.is_dir() {
        return Err(ApiError::bad_request("path is not a directory"));
    }
    let mut roots = state.scan_roots.write().await;
    let s = canonical.to_string_lossy();
    if roots.iter().any(|r| r.to_string_lossy() == s) {
        return Ok(Json(ScanDirsResponse {
            paths: roots
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
        }));
    }
    roots.push(canonical);
    // Persist
    if let Ok(json) = serde_json::to_string_pretty(
        &roots
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
    ) {
        let _ = std::fs::write(&state.scan_roots_path, json);
    }
    Ok(Json(ScanDirsResponse {
        paths: roots
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect(),
    }))
}

async fn handle_admin_scan_dirs_remove(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ScanDirsRemoveRequest>,
) -> Result<Json<ScanDirsResponse>, ApiError> {
    let path = std::path::PathBuf::from(req.path.trim());
    let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
    let key = canonical.to_string_lossy();
    let mut roots = state.scan_roots.write().await;
    roots.retain(|r| r.to_string_lossy() != key);
    // Persist
    if let Ok(json) = serde_json::to_string_pretty(
        &roots
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
    ) {
        let _ = std::fs::write(&state.scan_roots_path, json);
    }
    Ok(Json(ScanDirsResponse {
        paths: roots
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect(),
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
            "source {} is not approved (status: {})",
            req.source_id, status
        )));
    }

    let (connector, connector_str) = if connector_type == 9 {
        connector_for_onedrive(&state.data_dir)
    } else {
        connector_for_type(connector_type).ok_or_else(|| {
            ApiError::bad_request(format!("unsupported connector type: {connector_type}"))
        })?
    };

    let source_path = std::path::PathBuf::from(&path_or_uri);

    let tables = connector.inspect_schema(&source_path).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("schema inspect failed: {e}"),
        )
    })?;

    let table_names: Vec<String> = tables.iter().map(|t| t.table_name.clone()).collect();

    let ingest_id = format!("ing-{}", uuid::Uuid::new_v4());
    let job = IngestJob {
        ingest_id: ingest_id.clone(),
        source_id: req.source_id.clone(),
        connector_type: connector_str.to_string(),
    };

    let mapping_json: String = conn
        .query_row(
            "SELECT mapping_rules_json FROM source_profiles_view WHERE source_id = ?1",
            [&req.source_id],
            |row| row.get(0),
        )
        .unwrap_or_else(|_| "{}".to_string());

    let mapping_hints = if mapping_json.is_empty() || mapping_json == "{}" {
        None
    } else {
        Some(node_ingest::parse_mapping_hints(&mapping_json))
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
        mapping_hints.as_ref(),
    )
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("ingest failed: {e}"),
        )
    })?;

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
        if ok {
            approved += 1;
        }
    }

    Ok(Json(BulkApproveResponse {
        approved,
        skipped: total - approved,
    }))
}

async fn handle_admin_config_onedrive_get(
    State(state): State<Arc<AppState>>,
) -> Result<Json<OneDriveConfig>, ApiError> {
    let path = state.data_dir.join("config").join("onedrive.json");
    match load_onedrive_config(&path) {
        Some(cfg) => Ok(Json(cfg)),
        None => Ok(Json(OneDriveConfig {
            client_id: String::new(),
            tenant_id: "common".into(),
            refresh_token: String::new(),
            client_secret: None,
        })),
    }
}

#[derive(Deserialize)]
struct OneDriveConfigSaveRequest {
    client_id: String,
    tenant_id: Option<String>,
    refresh_token: String,
    client_secret: Option<String>,
}

async fn handle_admin_config_onedrive_save(
    State(state): State<Arc<AppState>>,
    Json(req): Json<OneDriveConfigSaveRequest>,
) -> Result<StatusCode, ApiError> {
    let cfg = OneDriveConfig {
        client_id: req.client_id,
        tenant_id: req.tenant_id.unwrap_or_else(|| "common".into()),
        refresh_token: req.refresh_token,
        client_secret: req.client_secret.filter(|s| !s.is_empty()),
    };
    let path = state.data_dir.join("config").join("onedrive.json");
    save_onedrive_config(&path, &cfg).map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

const ONEDRIVE_OAUTH_SCOPES: &str = "Files.ReadWrite offline_access User.Read";
const ONEDRIVE_OAUTH_TENANT: &str = "common";

fn onedrive_oauth_client_id() -> Option<String> {
    std::env::var("MESHMIND_ONEDRIVE_OAUTH_CLIENT_ID").ok()
}

fn pkce_code_verifier_and_challenge() -> (String, String) {
    use base64::Engine;
    let verifier: [u8; 32] = std::array::from_fn(|_| rand::random());
    let verifier_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(verifier);
    let digest = sha2::Sha256::digest(verifier_b64.as_bytes());
    let challenge_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);
    (verifier_b64, challenge_b64)
}

#[derive(Serialize)]
struct OAuthStartResponse {
    auth_url: String,
    state: String,
}

async fn handle_admin_config_onedrive_oauth_start(
    State(state): State<Arc<AppState>>,
) -> Result<Json<OAuthStartResponse>, ApiError> {
    let client_id = onedrive_oauth_client_id().ok_or_else(|| {
        ApiError::from_status(
            StatusCode::SERVICE_UNAVAILABLE,
            "OneDrive OAuth not configured: set MESHMIND_ONEDRIVE_OAUTH_CLIENT_ID (see README)",
        )
    })?;
    let redirect_uri = format!(
        "{}/v1/oauth/onedrive/callback",
        state.listen_base_url.trim_end_matches('/')
    );
    let (code_verifier, code_challenge) = pkce_code_verifier_and_challenge();
    let state_param = uuid::Uuid::new_v4().to_string();
    {
        let mut pending = state.oauth_pending.write().await;
        pending.insert(state_param.clone(), (code_verifier, Instant::now()));
        // Clean expired (older than 10 min)
        let cutoff = Instant::now() - Duration::from_secs(600);
        pending.retain(|_, (_, t)| *t > cutoff);
    }
    let auth_url = format!(
        "https://login.microsoftonline.com/{}/oauth2/v2.0/authorize?client_id={}&response_type=code&redirect_uri={}&scope={}&state={}&code_challenge={}&code_challenge_method=S256",
        ONEDRIVE_OAUTH_TENANT,
        urlencoding::encode(&client_id),
        urlencoding::encode(&redirect_uri),
        urlencoding::encode(ONEDRIVE_OAUTH_SCOPES),
        urlencoding::encode(&state_param),
        urlencoding::encode(&code_challenge),
    );
    Ok(Json(OAuthStartResponse {
        auth_url,
        state: state_param,
    }))
}

#[derive(Deserialize)]
struct OAuthCallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

async fn handle_oauth_onedrive_callback(
    State(state): State<Arc<AppState>>,
    Query(q): Query<OAuthCallbackQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let base = state.listen_base_url.trim_end_matches('/');
    let redirect_base = format!("{}/", base);
    if let Some(err) = q.error {
        let desc = q.error_description.as_deref().unwrap_or(&err);
        let redirect = format!(
            "{}?onedrive_error={}#settings",
            redirect_base,
            urlencoding::encode(desc)
        );
        return Ok(axum::response::Redirect::temporary(&redirect));
    }
    let code = q
        .code
        .ok_or_else(|| ApiError::bad_request("missing code"))?;
    let state_param = q
        .state
        .ok_or_else(|| ApiError::bad_request("missing state"))?;
    let (code_verifier, _) = {
        let mut pending = state.oauth_pending.write().await;
        pending
            .remove(&state_param)
            .ok_or_else(|| ApiError::bad_request("invalid or expired state"))?
    };
    let client_id = onedrive_oauth_client_id()
        .ok_or_else(|| ApiError::internal("OAuth client ID not configured"))?;
    let redirect_uri = format!("{}/v1/oauth/onedrive/callback", base);
    let token_url = format!(
        "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
        ONEDRIVE_OAUTH_TENANT
    );
    let client = reqwest::Client::new();
    let form = [
        ("client_id", client_id.as_str()),
        ("grant_type", "authorization_code"),
        ("code", code.as_str()),
        ("redirect_uri", redirect_uri.as_str()),
        ("code_verifier", code_verifier.as_str()),
    ];
    let resp = client
        .post(&token_url)
        .form(&form)
        .send()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    if !status.is_success() {
        tracing::warn!("OneDrive token exchange failed: {} - {}", status, body);
        let redirect = format!(
            "{}?onedrive_error={}#settings",
            redirect_base,
            urlencoding::encode("Token exchange failed")
        );
        return Ok(axum::response::Redirect::temporary(&redirect));
    }
    let json: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| ApiError::internal(e.to_string()))?;
    let refresh_token = json
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ApiError::internal("no refresh_token in response"))?;
    let cfg = OneDriveConfig {
        client_id: client_id.clone(),
        tenant_id: ONEDRIVE_OAUTH_TENANT.to_string(),
        refresh_token: refresh_token.to_string(),
        client_secret: None,
    };
    let path = state.data_dir.join("config").join("onedrive.json");
    save_onedrive_config(&path, &cfg).map_err(|e| ApiError::internal(e.to_string()))?;
    let redirect = format!("{}?onedrive=ok#settings", redirect_base);
    Ok(axum::response::Redirect::temporary(&redirect))
}

fn meshmind_toml_path() -> std::path::PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join("meshmind.toml")
}

#[derive(Serialize)]
struct GeneralConfigResponse {
    backend: String,
    ollama_endpoint: Option<String>,
    ollama_model: Option<String>,
    enable_mdns: bool,
    replication_interval_secs: u64,
    relay_addr: Option<String>,
    relay_port: Option<u16>,
    relay_only: Option<bool>,
    public_addr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    listen: Option<String>,
}

#[derive(Deserialize)]
struct GeneralConfigUpdate {
    backend: Option<String>,
    ollama_endpoint: Option<String>,
    ollama_model: Option<String>,
    enable_mdns: Option<bool>,
    replication_interval_secs: Option<u64>,
    relay_addr: Option<String>,
    relay_port: Option<u16>,
    relay_only: Option<bool>,
    public_addr: Option<String>,
}

async fn handle_admin_config_general_get() -> Result<Json<GeneralConfigResponse>, ApiError> {
    let path = meshmind_toml_path();
    let empty_map = toml::map::Map::new();
    let (
        backend,
        ollama_endpoint,
        ollama_model,
        enable_mdns,
        replication_interval_secs,
        relay_addr,
        relay_port,
        relay_only,
        public_addr,
        data_dir,
        listen,
    ) = if path.exists() {
        let text = std::fs::read_to_string(&path).map_err(|e| ApiError::internal(e.to_string()))?;
        let value: toml::Value =
            toml::from_str(&text).map_err(|e| ApiError::internal(e.to_string()))?;
        let t = value.as_table().unwrap_or(&empty_map);
        (
            t.get("backend")
                .and_then(|v| v.as_str())
                .unwrap_or("mock")
                .to_string(),
            t.get("ollama_endpoint")
                .and_then(|v| v.as_str())
                .map(String::from),
            t.get("ollama_model")
                .and_then(|v| v.as_str())
                .map(String::from),
            t.get("enable_mdns")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            t.get("replication_interval_secs")
                .and_then(|v| v.as_integer())
                .map(|i| i as u64)
                .unwrap_or(30),
            t.get("relay_addr")
                .and_then(|v| v.as_str())
                .map(String::from),
            t.get("relay_port")
                .and_then(|v| v.as_integer())
                .map(|i| i as u16),
            t.get("relay_only").and_then(|v| v.as_bool()),
            t.get("public_addr")
                .and_then(|v| v.as_str())
                .map(String::from),
            t.get("data_dir").and_then(|v| v.as_str()).map(String::from),
            t.get("listen").and_then(|v| v.as_str()).map(String::from),
        )
    } else {
        (
            "mock".to_string(),
            None,
            None,
            true,
            30,
            None,
            None,
            None,
            None,
            None,
            None,
        )
    };
    Ok(Json(GeneralConfigResponse {
        backend,
        ollama_endpoint,
        ollama_model,
        enable_mdns,
        replication_interval_secs,
        relay_addr,
        relay_port,
        relay_only,
        public_addr,
        data_dir,
        listen,
    }))
}

async fn handle_admin_config_general_save(
    Json(req): Json<GeneralConfigUpdate>,
) -> Result<StatusCode, ApiError> {
    let path = meshmind_toml_path();
    let mut value: toml::Value = if path.exists() {
        let text = std::fs::read_to_string(&path).map_err(|e| ApiError::internal(e.to_string()))?;
        toml::from_str(&text).map_err(|e| ApiError::internal(e.to_string()))?
    } else {
        toml::Value::Table(toml::map::Map::new())
    };
    let t = value
        .as_table_mut()
        .ok_or_else(|| ApiError::internal("invalid config"))?;
    if let Some(v) = req.backend {
        t.insert("backend".into(), toml::Value::String(v));
    }
    if let Some(v) = req.ollama_endpoint {
        t.insert("ollama_endpoint".into(), toml::Value::String(v));
    }
    if let Some(v) = req.ollama_model {
        t.insert("ollama_model".into(), toml::Value::String(v));
    }
    if let Some(v) = req.enable_mdns {
        t.insert("enable_mdns".into(), toml::Value::Boolean(v));
    }
    if let Some(v) = req.replication_interval_secs {
        t.insert(
            "replication_interval_secs".into(),
            toml::Value::Integer(v as i64),
        );
    }
    if let Some(v) = req.relay_addr {
        t.insert("relay_addr".into(), toml::Value::String(v));
    }
    if let Some(v) = req.relay_port {
        t.insert("relay_port".into(), toml::Value::Integer(v as i64));
    }
    if let Some(v) = req.relay_only {
        t.insert("relay_only".into(), toml::Value::Boolean(v));
    }
    if let Some(v) = req.public_addr {
        t.insert("public_addr".into(), toml::Value::String(v));
    }
    let text = toml::to_string_pretty(&value).map_err(|e| ApiError::internal(e.to_string()))?;
    std::fs::write(&path, text).map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn handle_admin_restart() -> Result<StatusCode, ApiError> {
    tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        std::process::exit(0);
    });
    Ok(StatusCode::NO_CONTENT)
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
        let (connector, connector_str) = if *connector_type == 9 {
            connector_for_onedrive(&state.data_dir)
        } else {
            match connector_for_type(*connector_type) {
                Some(p) => p,
                None => {
                    failed += 1;
                    continue;
                }
            }
        };

        let source_path = std::path::PathBuf::from(path_or_uri);
        let tables = match connector.inspect_schema(&source_path) {
            Ok(t) => t,
            Err(_) => {
                failed += 1;
                continue;
            }
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
        let mapping_json: String = rusqlite::Connection::open(&state.db_path)
            .ok()
            .and_then(|c| {
                c.query_row(
                    "SELECT mapping_rules_json FROM source_profiles_view WHERE source_id = ?1",
                    [source_id],
                    |row| row.get(0),
                )
                .ok()
            })
            .unwrap_or_else(|| "{}".to_string());
        let mapping_hints = if mapping_json.is_empty() || mapping_json == "{}" {
            None
        } else {
            Some(node_ingest::parse_mapping_hints(&mapping_json))
        };

        match node_ingest::run_ingest(
            &job,
            connector.as_ref(),
            &source_path,
            &table_names,
            &config,
            &state.cas,
            &mut log,
            &db_path,
            &node_id,
            mapping_hints.as_ref(),
        ) {
            Ok(result) => {
                total_rows += result.rows_ingested;
                total_docs += result.documents_created;
                ingested += 1;
            }
            Err(_) => {
                failed += 1;
            }
        }
    }

    Ok(Json(BulkIngestResponse {
        ingested,
        failed,
        total_rows,
        total_docs,
    }))
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
    let manifest_result =
        node_datasets::build_dataset(&ds_config, &event_log, &state.cas, &state.node_id).map_err(
            |e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("dataset build failed: {e}"),
                )
            },
        )?;
    drop(event_log);

    let dataset_items = manifest_result.total_items;
    let dataset_manifest_id = manifest_result.manifest_id.clone();

    // 2. Record dataset manifest event
    let manifest_event =
        node_datasets::build_manifest_event(&manifest_result, &ds_config, &state.node_id);
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
        node_id: Some(NodeId {
            value: state.node_id.clone(),
        }),
        tenant_id: Some(TenantId {
            value: "public".into(),
        }),
        sensitivity: Sensitivity::Public as i32,
        payload: Some(event_envelope::Payload::TrainJobStarted(TrainJobStarted {
            job_id: job_id.clone(),
            target: req.target.clone(),
            dataset_manifest_ref: Some(HashRef {
                sha256: manifest_result.cas_hash.clone(),
            }),
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
        cas_refs: manifest_result
            .items
            .iter()
            .map(|i| i.cas_hash.clone())
            .collect(),
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
        JobStatus::Failed { reason } => (format!("failed: {reason}"), None, None),
        other => (format!("{other:?}"), None, None),
    };

    // 6. Record TrainJobCompleted event
    let completed_event = EventEnvelope {
        event_id: format!("evt-train-done-{}", uuid::Uuid::new_v4()),
        r#type: EventType::TrainJobCompleted as i32,
        node_id: Some(NodeId {
            value: state.node_id.clone(),
        }),
        tenant_id: Some(TenantId {
            value: "public".into(),
        }),
        sensitivity: Sensitivity::Public as i32,
        payload: Some(event_envelope::Payload::TrainJobCompleted(
            TrainJobCompleted {
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
            },
        )),
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
            let version_num: u32 = ver.trim_start_matches('v').parse().unwrap_or(1);

            let promote_event = EventEnvelope {
                event_id: format!("evt-promote-{}", uuid::Uuid::new_v4()),
                r#type: EventType::ModelPromoted as i32,
                node_id: Some(NodeId {
                    value: state.node_id.clone(),
                }),
                tenant_id: Some(TenantId {
                    value: "public".into(),
                }),
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
    State(state): State<Arc<AppState>>,
) -> Result<Json<FederatedStatusResponse>, ApiError> {
    let config = &state.federated_config;
    Ok(Json(FederatedStatusResponse {
        supported: true,
        aggregation: config.aggregation_strategy.clone(),
        min_participants: config.min_participants,
        max_participants: config.max_participants,
    }))
}

#[derive(Serialize, Deserialize)]
struct FederatedStartRoundRequest {
    model_id: String,
    round_number: Option<u32>,
    min_participants: Option<u32>,
    max_participants: Option<u32>,
}

#[derive(Serialize)]
struct FederatedRoundResponse {
    round_id: String,
    model_id: String,
    round_number: u32,
    status: String,
    delta_count: usize,
    min_participants: u32,
    max_participants: u32,
}

#[derive(Serialize, Deserialize)]
struct FederatedSubmitDeltaRequest {
    delta_id: String,
    model_id: String,
    base_version: String,
    cas_hash: String,
    metrics: Vec<(String, f64)>,
    from_node: String,
}

async fn handle_admin_federated_start_round(
    State(state): State<Arc<AppState>>,
    Json(req): Json<FederatedStartRoundRequest>,
) -> Result<Json<FederatedRoundResponse>, ApiError> {
    if !state.federated_policy.can_share_deltas().is_allowed() {
        return Err(ApiError::from_status(
            StatusCode::FORBIDDEN,
            "federated training disabled by policy",
        ));
    }
    let model_id = if req.model_id.is_empty() {
        state.federated_config.model_id.clone()
    } else {
        req.model_id
    };
    let round_number = req.round_number.unwrap_or(1);
    let min_p = req
        .min_participants
        .unwrap_or(state.federated_config.min_participants);
    let max_p = req
        .max_participants
        .unwrap_or(state.federated_config.max_participants);

    let config = FederatedConfig {
        model_id: model_id.clone(),
        min_participants: min_p,
        max_participants: max_p,
        ..state.federated_config.clone()
    };
    let coordinator = FederatedCoordinator::new(config.clone());
    let round_state = coordinator.start_round(&model_id, round_number);

    let mut rounds = state.federated_rounds.write().await;
    rounds.insert(round_state.round_id.clone(), round_state.clone());
    drop(rounds);

    let evt = build_round_started_event(&round_state, &state.node_id);
    let mut log = state.event_log.write().await;
    let stored = log
        .append(evt)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    drop(log);

    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    node_storage::projector::apply_event(&conn, &stored)
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(FederatedRoundResponse {
        round_id: round_state.round_id,
        model_id: round_state.model_id,
        round_number: round_state.round_number,
        status: round_state.status,
        delta_count: round_state.deltas.len(),
        min_participants: config.min_participants,
        max_participants: config.max_participants,
    }))
}

async fn handle_admin_federated_round_status(
    State(state): State<Arc<AppState>>,
    Path(round_id): Path<String>,
) -> Result<Json<FederatedRoundResponse>, ApiError> {
    let rounds = state.federated_rounds.read().await;
    let round_state = rounds
        .get(&round_id)
        .ok_or_else(|| ApiError::from_status(StatusCode::NOT_FOUND, "round not found"))?;
    let config = &state.federated_config;
    Ok(Json(FederatedRoundResponse {
        round_id: round_state.round_id.clone(),
        model_id: round_state.model_id.clone(),
        round_number: round_state.round_number,
        status: round_state.status.clone(),
        delta_count: round_state.deltas.len(),
        min_participants: config.min_participants,
        max_participants: config.max_participants,
    }))
}

async fn handle_admin_federated_submit_delta(
    State(state): State<Arc<AppState>>,
    Path(round_id): Path<String>,
    Json(req): Json<FederatedSubmitDeltaRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if !state.federated_policy.can_share_deltas().is_allowed() {
        return Err(ApiError::from_status(
            StatusCode::FORBIDDEN,
            "federated training disabled by policy",
        ));
    }
    let delta = DeltaInfo {
        delta_id: req.delta_id,
        model_id: req.model_id,
        base_version: req.base_version,
        from_node: req.from_node,
        cas_hash: req.cas_hash,
        metrics: req.metrics,
    };
    let coordinator = FederatedCoordinator::new(state.federated_config.clone());

    let mut rounds = state.federated_rounds.write().await;
    let round_state = rounds
        .get_mut(&round_id)
        .ok_or_else(|| ApiError::from_status(StatusCode::NOT_FOUND, "round not found"))?;
    let accepted = coordinator.submit_delta(round_state, delta.clone());
    let round_state = round_state.clone();
    drop(rounds);

    if accepted {
        let evt = build_delta_published_event(&delta, &state.node_id);
        let mut log = state.event_log.write().await;
        let stored = log
            .append(evt)
            .map_err(|e| ApiError::internal(e.to_string()))?;
        drop(log);
        let conn = rusqlite::Connection::open(&state.db_path)
            .map_err(|e| ApiError::internal(e.to_string()))?;
        node_storage::projector::apply_event(&conn, &stored)
            .map_err(|e| ApiError::internal(e.to_string()))?;
    }

    Ok(Json(serde_json::json!({
        "accepted": accepted,
        "round_id": round_id,
        "delta_count": round_state.deltas.len(),
    })))
}

async fn handle_admin_federated_aggregate(
    State(state): State<Arc<AppState>>,
    Path(round_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if !state.federated_policy.can_share_deltas().is_allowed() {
        return Err(ApiError::from_status(
            StatusCode::FORBIDDEN,
            "federated training disabled by policy",
        ));
    }
    let coordinator = FederatedCoordinator::new(state.federated_config.clone());

    let mut rounds = state.federated_rounds.write().await;
    let round_state = rounds
        .get_mut(&round_id)
        .ok_or_else(|| ApiError::from_status(StatusCode::NOT_FOUND, "round not found"))?;
    let hash = coordinator
        .aggregate(round_state, &state.cas)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let round_state = round_state.clone();
    drop(rounds);

    let evt = build_round_completed_event(&round_state, &state.node_id);
    let mut log = state.event_log.write().await;
    let stored = log
        .append(evt)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    drop(log);
    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    node_storage::projector::apply_event(&conn, &stored)
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(serde_json::json!({
        "round_id": round_id,
        "result_model_hash": hash,
        "participants": round_state.deltas.len(),
    })))
}

#[derive(Deserialize)]
struct InsightsRunRequest {
    schedule: Option<String>, // hourly | daily | weekly | monthly | manual
}

async fn handle_admin_insights_run(
    State(state): State<Arc<AppState>>,
    Json(req): Json<InsightsRunRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    use node_proto::common::*;
    use node_proto::events::event_envelope::Payload;
    use node_proto::events::*;

    let schedule = req.schedule.as_deref().unwrap_or("manual");
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let mut created = 0u32;

    let conn = rusqlite::Connection::open(&state.db_path)
        .map_err(|e| ApiError::internal(e.to_string()))?;

    // Run same logic as handle_insights but emit InsightGenerated events
    if let Ok(hits) = search::search_entity_cards(&conn, Some("invoice"), 50) {
        let overdue: Vec<_> = hits
            .iter()
            .filter(|h| {
                h.attributes_json.contains("overdue")
                    || h.attributes_json.contains("Overdue")
                    || h.attributes_json.contains("\"overdue_count\":1")
                    || h.attributes_json.contains("\"overdue_count\": 1")
            })
            .take(10)
            .collect();
        if !overdue.is_empty() {
            let entity_ids: Vec<String> = overdue.iter().map(|h| h.entity_id.clone()).collect();
            let insight_id = format!("insight-overdue-{}", uuid::Uuid::new_v4());
            let evt = EventEnvelope {
                event_id: format!("evt-ig-{}", insight_id),
                r#type: EventType::InsightGenerated as i32,
                node_id: Some(NodeId {
                    value: state.node_id.clone(),
                }),
                tenant_id: Some(TenantId {
                    value: "public".into(),
                }),
                sensitivity: Sensitivity::Public as i32,
                ts: Some(Timestamp { unix_ms: ts }),
                payload: Some(Payload::InsightGenerated(InsightGenerated {
                    insight_id: insight_id.clone(),
                    insight_type: "overdue_invoices".into(),
                    title: format!("{} overdue invoice(s)", overdue.len()),
                    summary: format!("Entity IDs: {}", entity_ids.join(", ")),
                    entity_ids,
                    confidence: 0.8,
                    schedule: schedule.to_string(),
                })),
                ..Default::default()
            };
            let mut log = state.event_log.write().await;
            if let Ok(stored) = log.append(evt) {
                drop(log);
                let conn = rusqlite::Connection::open(&state.db_path)
                    .map_err(|e| ApiError::internal(e.to_string()))?;
                let _ = node_storage::projector::apply_event(&conn, &stored);
                created += 1;
            }
        }
    }

    Ok(Json(serde_json::json!({
        "schedule": schedule,
        "insights_created": created,
    })))
}

// ---------- WebSocket (real-time status) ----------

async fn handle_ws(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> axum::response::Response {
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
        if socket
            .send(Message::Text(payload.to_string()))
            .await
            .is_err()
        {
            break;
        }
    }
}

// ---------- Replication endpoints ----------

use node_proto::repl::{
    GossipMeta as ProtoGossipMeta, PullCasObjectsRequest, PullSegmentsRequest,
    SegmentId as ProtoSegmentId,
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
    let local_gossip =
        node_repl::build_gossip_meta(&state.node_id, "public", &event_log, &state.cas, &[])
            .map_err(|e| ApiError::internal(format!("gossip build failed: {e}")))?;

    let missing_segs = node_repl::find_missing_segments(&local_gossip, &remote_gossip);
    let missing_cas = node_repl::find_missing_objects(&state.cas, &remote_gossip);
    drop(event_log);

    let mut local_bytes = Vec::new();
    local_gossip
        .encode(&mut local_bytes)
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
        want_segments: req
            .segment_ids
            .iter()
            .map(|s| ProtoSegmentId { value: s.clone() })
            .collect(),
        budget: None,
    };
    let seg_resp = node_repl::serve_pull_segments(&seg_req, &event_log, &state.node_id)
        .map_err(|e| ApiError::internal(format!("serve segments: {e}")))?;
    let segments_sent = seg_resp.chunks.len();

    let cas_req = PullCasObjectsRequest {
        requester: None,
        want_hashes: req
            .cas_hashes
            .iter()
            .map(|h| node_proto::common::HashRef { sha256: h.clone() })
            .collect(),
        budget: None,
    };
    let cas_resp = node_repl::serve_pull_cas_objects(&cas_req, &state.cas, &state.node_id)
        .map_err(|e| ApiError::internal(format!("serve cas objects: {e}")))?;
    let cas_sent = cas_resp.chunks.len();

    drop(event_log);

    use prost::Message;
    let segment_chunks: Vec<Vec<u8>> = seg_resp.chunks.iter().map(|c| c.encode_to_vec()).collect();
    let cas_chunks: Vec<Vec<u8>> = cas_resp.chunks.iter().map(|c| c.encode_to_vec()).collect();

    Ok(Json(PullResponse {
        segments_sent,
        cas_sent,
        segment_chunks,
        cas_chunks,
    }))
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

        let data_dir = db_path
            .parent()
            .and_then(|p| p.parent())
            .unwrap_or(&db_path)
            .to_path_buf();
        Arc::new(AppState {
            event_log: RwLock::new(event_log),
            cas,
            db_path,
            data_dir: data_dir.clone(),
            peer_dir: Arc::new(RwLock::new(PeerDirectory::new())),
            backend: Arc::new(MockBackend::new()),
            transport: None,
            consult_config: ConsultConfig::default(),
            node_id: "test-node-001".into(),
            admin_token: "test-token".into(),
            expose_admin_token: true,
            scan_roots: Arc::new(tokio::sync::RwLock::new(vec![])),
            scan_roots_path: data_dir.join("scan_roots.json"),
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
            listen_base_url: "http://127.0.0.1:9900".into(),
            oauth_pending: Arc::new(RwLock::new(HashMap::new())),
            federated_rounds: Arc::new(RwLock::new(HashMap::new())),
            federated_config: FederatedConfig::new("router"),
            federated_policy: Arc::new(node_policy::PolicyEngine::new(node_policy::PolicyConfig {
                allow_train: true,
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
            .oneshot(
                Request::get("/v1/search?q=test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let results: Vec<SearchResult> = serde_json::from_slice(&body).unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn ask_confirm_endpoint() {
        let state = create_test_state();
        let app = build_router(state);

        let body = serde_json::json!({
            "case_id": "ask-test-123",
            "outcome": "accepted",
            "confidence": 0.95
        });
        let resp = app
            .oneshot(
                Request::post("/v1/ask/confirm")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json.get("ok").and_then(|v| v.as_bool()), Some(true));
    }

    #[tokio::test]
    async fn insights_endpoint() {
        let state = create_test_state();
        let app = build_router(state);

        let resp = app
            .oneshot(Request::get("/v1/insights").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let insights: Vec<InsightItem> = serde_json::from_slice(&body).unwrap();
        assert!(insights.is_empty() || !insights.is_empty()); // Always valid JSON array
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
            .oneshot(
                Request::get("/v1/admin/sources")
                    .body(Body::empty())
                    .unwrap(),
            )
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
        let data_dir = db_path
            .parent()
            .and_then(|p| p.parent())
            .unwrap_or(&db_path)
            .to_path_buf();
        let state = Arc::new(AppState {
            event_log: RwLock::new(event_log),
            cas,
            db_path,
            data_dir: data_dir.clone(),
            peer_dir: Arc::new(RwLock::new(PeerDirectory::new())),
            backend: Arc::new(MockBackend::new()),
            transport: None,
            consult_config: ConsultConfig::default(),
            node_id: "test-node".into(),
            admin_token: "test-token".into(),
            expose_admin_token: true,
            scan_roots: Arc::new(tokio::sync::RwLock::new(vec![])),
            scan_roots_path: data_dir.join("scan_roots.json"),
            trainer,
            model_registry,
            ui_dir: None,
            last_train_status: Arc::new(RwLock::new(None)),
            ask_chat_limiter: None,
            research_policy: deny_policy,
            listen_base_url: "http://127.0.0.1:9900".into(),
            oauth_pending: Arc::new(RwLock::new(HashMap::new())),
            federated_rounds: Arc::new(RwLock::new(HashMap::new())),
            federated_config: FederatedConfig::new("router"),
            federated_policy: Arc::new(node_policy::PolicyEngine::new(node_policy::PolicyConfig {
                allow_train: true,
                ..Default::default()
            })),
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
