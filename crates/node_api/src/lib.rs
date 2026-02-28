//! Axum localhost API: status, peers, search, ask, admin endpoints.
//!
//! Endpoints:
//! - GET  /status
//! - GET  /peers
//! - GET  /search?q=
//! - POST /ask
//! - POST /admin/event
//! - POST /admin/train
//! - GET  /admin/logs
//! - GET  /admin/sources
//! - POST /admin/sources/approve
//! - GET  /admin/models
//! - POST /admin/models/rollback
//! - GET  /admin/datasets

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::{get, post};
use axum::Router;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use node_ai::InferenceBackend;
use node_mesh::transport::Transport;
use node_mesh::{ConsultConfig, PeerDirectory};
use node_storage::cas::CasStore;
use node_storage::event_log::EventLog;
use node_storage::search;

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
}

pub fn build_router(state: Arc<AppState>) -> Router {
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
    case_id: String,
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

    let hits = search::search_cases(&conn, &params.q, params.limit)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let results = hits
        .into_iter()
        .map(|h| SearchResult {
            case_id: h.case_id,
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

    let context_hits = search::search_cases(&conn, &req.question, 5)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let context_bullets: Vec<String> = context_hits
        .iter()
        .map(|h| format!("- {}: {}", h.title, h.summary))
        .collect();

    let prompt = if context_bullets.is_empty() {
        format!("Question: {}\n\nAnswer concisely.", req.question)
    } else {
        format!(
            "Context from knowledge base:\n{}\n\nQuestion: {}\n\nAnswer based on the context above. Be concise.",
            context_bullets.join("\n"),
            req.question
        )
    };

    let gen_req = node_ai::GenerateRequest {
        prompt,
        system: Some("You are MeshMind, a helpful AI assistant that answers questions using your local knowledge base.".into()),
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
        context_used: context_hits.iter().map(|h| h.case_id.clone()).collect(),
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

async fn handle_admin_train(
    State(state): State<Arc<AppState>>,
    Json(req): Json<TrainRequest>,
) -> Result<Json<TrainResponse>, StatusCode> {
    use node_proto::common::*;
    use node_proto::events::*;

    let job_id = format!("job-{}", uuid::Uuid::new_v4());
    let event_id = format!("evt-train-{}", uuid::Uuid::new_v4());

    let event = EventEnvelope {
        event_id,
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
            target: req.target,
            dataset_manifest_ref: None,
            max_steps: 1000,
            max_minutes: 10,
        })),
        tags: vec![format!("preset:{}", req.dataset_preset)],
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

    Ok(Json(TrainResponse { job_id }))
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
    }
}
