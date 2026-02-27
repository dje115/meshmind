use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::RwLock;
use tracing_subscriber::EnvFilter;

use node_ai::InferenceBackend;
use node_ai_mock::MockBackend;
use node_ai_ollama::OllamaBackend;
use node_api::AppState;
use node_mesh::PeerDirectory;
use node_storage::cas::CasStore;
use node_storage::event_log::EventLog;
use node_storage::sqlite_views;

#[derive(Debug, Clone, serde::Deserialize)]
struct NodeConfig {
    #[serde(default = "default_data_dir")]
    data_dir: PathBuf,
    #[serde(default = "default_listen")]
    listen: String,
    #[serde(default = "default_backend")]
    backend: String,
    #[serde(default)]
    ollama_endpoint: Option<String>,
    #[serde(default)]
    ollama_model: Option<String>,
    #[serde(default = "default_admin_token")]
    admin_token: String,
}

fn default_data_dir() -> PathBuf {
    PathBuf::from("./data")
}
fn default_listen() -> String {
    "127.0.0.1:9900".into()
}
fn default_backend() -> String {
    "mock".into()
}
fn default_admin_token() -> String {
    uuid::Uuid::new_v4().to_string()
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            data_dir: default_data_dir(),
            listen: default_listen(),
            backend: default_backend(),
            ollama_endpoint: None,
            ollama_model: None,
            admin_token: default_admin_token(),
        }
    }
}

fn load_config() -> Result<NodeConfig> {
    let config_path = PathBuf::from("meshmind.toml");
    if config_path.exists() {
        let text = std::fs::read_to_string(&config_path).context("read meshmind.toml")?;
        let config: NodeConfig = toml::from_str(&text).context("parse meshmind.toml")?;
        Ok(config)
    } else {
        Ok(NodeConfig::default())
    }
}

fn create_backend(config: &NodeConfig) -> Arc<dyn InferenceBackend> {
    match config.backend.as_str() {
        "ollama" => {
            let endpoint = config
                .ollama_endpoint
                .clone()
                .unwrap_or_else(|| "http://localhost:11434".into());
            let model = config
                .ollama_model
                .clone()
                .unwrap_or_else(|| "llama3.2:3b".into());
            Arc::new(OllamaBackend::new(&endpoint, &model, 30000).expect("create ollama backend"))
        }
        _ => Arc::new(MockBackend::new()),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = load_config()?;

    tracing::info!("MeshMind starting...");
    tracing::info!("data_dir = {:?}", config.data_dir);
    tracing::info!("listen   = {}", config.listen);
    tracing::info!("backend  = {}", config.backend);

    std::fs::create_dir_all(&config.data_dir)?;

    let mut event_log = EventLog::open(&config.data_dir).context("open event log")?;
    let cas = CasStore::open(&config.data_dir).context("open CAS")?;
    let db_path = config.data_dir.join("sqlite").join("meshmind.db");
    let _conn = sqlite_views::open_db(&db_path).context("open SQLite")?;

    let backend = create_backend(&config);
    let node_id = format!("node-{}", &uuid::Uuid::new_v4().to_string()[..8]);

    // Load seed data if available
    let seed_dir = PathBuf::from("seed/public");
    if seed_dir.exists() {
        match node_app::load_seed_data(&seed_dir, &mut event_log, &cas, &db_path, &node_id) {
            Ok(n) if n > 0 => tracing::info!("loaded {n} seed items"),
            Ok(_) => {}
            Err(e) => tracing::warn!("seed loading failed: {e}"),
        }
    }

    tracing::info!("node_id  = {}", node_id);
    tracing::info!("admin_token = {}", config.admin_token);

    let state = Arc::new(AppState {
        event_log: RwLock::new(event_log),
        cas,
        db_path,
        peer_dir: RwLock::new(PeerDirectory::new()),
        backend,
        node_id,
        admin_token: config.admin_token,
    });

    let app = node_api::build_router(state);

    let listener = tokio::net::TcpListener::bind(&config.listen)
        .await
        .with_context(|| format!("bind to {}", config.listen))?;
    tracing::info!("listening on {}", config.listen);

    axum::serve(listener, app).await.context("serve")?;

    Ok(())
}
