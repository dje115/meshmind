use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::sync::RwLock;
use tracing_subscriber::EnvFilter;

use node_ai::InferenceBackend;
use node_ai_mock::MockBackend;
use node_ai_ollama::OllamaBackend;
use node_api::AppState;
use node_crypto::DevCa;
use node_mesh::tcp_transport::{EnvelopeHandler, TcpServer, TcpTransport};
use node_mesh::{ConsultConfig, PeerDirectory};
use node_policy::PolicyEngine;
use node_proto::common::*;
use node_proto::mesh::*;
use node_storage::cas::CasStore;
use node_storage::event_log::EventLog;
use node_storage::sqlite_views;

#[derive(Debug, Clone, serde::Deserialize)]
struct NodeConfig {
    #[serde(default = "default_data_dir")]
    data_dir: PathBuf,
    #[serde(default = "default_listen")]
    listen: String,
    #[serde(default = "default_mesh_port")]
    mesh_port: u16,
    #[serde(default = "default_backend")]
    backend: String,
    #[serde(default)]
    ollama_endpoint: Option<String>,
    #[serde(default)]
    ollama_model: Option<String>,
    #[serde(default = "default_admin_token")]
    admin_token: String,
    #[serde(default = "default_true")]
    enable_mdns: bool,
    #[serde(default = "default_repl_interval")]
    replication_interval_secs: u64,
    #[serde(default)]
    relay_addr: Option<String>,
    #[serde(default)]
    relay_port: Option<u16>,
    #[serde(default)]
    relay_only: bool,
    #[serde(default)]
    public_addr: Option<String>,
}

fn default_data_dir() -> PathBuf {
    PathBuf::from("./data")
}
fn default_listen() -> String {
    "127.0.0.1:9900".into()
}
fn default_mesh_port() -> u16 {
    9901
}
fn default_backend() -> String {
    "mock".into()
}
fn default_admin_token() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn default_true() -> bool {
    true
}
fn default_repl_interval() -> u64 {
    30
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            data_dir: default_data_dir(),
            listen: default_listen(),
            mesh_port: default_mesh_port(),
            backend: default_backend(),
            ollama_endpoint: None,
            ollama_model: None,
            admin_token: default_admin_token(),
            enable_mdns: true,
            replication_interval_secs: 30,
            relay_addr: None,
            relay_port: None,
            relay_only: false,
            public_addr: None,
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

fn build_envelope_handler(node_id: String, backend: Arc<dyn InferenceBackend>) -> EnvelopeHandler {
    Arc::new(move |env: Envelope| -> Option<Envelope> {
        match env.body {
            Some(envelope::Body::Ping(ping)) => Some(Envelope {
                msg_id: format!("{}-pong", env.msg_id),
                r#type: MsgType::Pong as i32,
                from_node_id: Some(NodeId {
                    value: node_id.clone(),
                }),
                body: Some(envelope::Body::Pong(Pong { nonce: ping.nonce })),
                ..Default::default()
            }),
            Some(envelope::Body::Hello(_)) => Some(Envelope {
                msg_id: format!("{}-hello-ack", env.msg_id),
                r#type: MsgType::Hello as i32,
                from_node_id: Some(NodeId {
                    value: node_id.clone(),
                }),
                body: Some(envelope::Body::Hello(Hello {
                    capabilities: vec!["inference".into(), "storage".into()],
                    version: env!("CARGO_PKG_VERSION").into(),
                })),
                ..Default::default()
            }),
            Some(envelope::Body::Ask(ask)) => {
                if env.ttl_hops == 0 {
                    return Some(Envelope {
                        msg_id: format!("{}-refuse", env.msg_id),
                        r#type: MsgType::Refuse as i32,
                        from_node_id: Some(NodeId {
                            value: node_id.clone(),
                        }),
                        body: Some(envelope::Body::Refuse(Refuse {
                            code: "TTL_EXPIRED".into(),
                            message: "ttl_hops exhausted".into(),
                        })),
                        ..Default::default()
                    });
                }

                let be = backend.clone();
                let nid = node_id.clone();
                let question = ask.question.clone();

                let rt = tokio::runtime::Handle::try_current();
                if let Ok(handle) = rt {
                    let result = std::thread::spawn(move || {
                        handle.block_on(async {
                            let req = node_ai::GenerateRequest {
                                prompt: format!("Answer concisely: {question}"),
                                system: Some("You are a helpful AI assistant.".into()),
                                max_tokens: 256,
                                ..Default::default()
                            };
                            be.generate(req).await.ok()
                        })
                    })
                    .join()
                    .ok()
                    .flatten();

                    if let Some(resp) = result {
                        Some(Envelope {
                            msg_id: format!("{}-answer", env.msg_id),
                            r#type: MsgType::Answer as i32,
                            from_node_id: Some(NodeId { value: nid }),
                            body: Some(envelope::Body::Answer(Answer {
                                answer: resp.text,
                                confidence: 0.6,
                                evidence_refs: vec![],
                                warnings: vec![],
                            })),
                            ..Default::default()
                        })
                    } else {
                        Some(Envelope {
                            msg_id: format!("{}-refuse", env.msg_id),
                            r#type: MsgType::Refuse as i32,
                            from_node_id: Some(NodeId { value: nid }),
                            body: Some(envelope::Body::Refuse(Refuse {
                                code: "INFERENCE_FAILED".into(),
                                message: "could not generate answer".into(),
                            })),
                            ..Default::default()
                        })
                    }
                } else {
                    None
                }
            }
            _ => None,
        }
    })
}

async fn replication_loop(state: Arc<AppState>, policy: Arc<PolicyEngine>, interval: Duration) {
    let mut ticker = tokio::time::interval(interval);
    loop {
        ticker.tick().await;

        let peers: Vec<(String, String, u16)> = {
            let dir = state.peer_dir.read().await;
            dir.reachable_peers()
                .iter()
                .map(|p| (p.node_id.clone(), p.address.clone(), p.port))
                .collect()
        };

        if peers.is_empty() {
            continue;
        }

        let transport = match &state.transport {
            Some(t) => t.clone(),
            None => continue,
        };

        for (peer_id, address, port) in &peers {
            let ping = Envelope {
                msg_id: uuid::Uuid::new_v4().to_string(),
                r#type: MsgType::Ping as i32,
                from_node_id: Some(NodeId {
                    value: state.node_id.clone(),
                }),
                body: Some(envelope::Body::Ping(Ping { nonce: 1 })),
                ..Default::default()
            };

            match transport.request(address, *port, &ping).await {
                Ok(resp) => {
                    if resp.r#type == MsgType::Pong as i32 {
                        tracing::debug!("peer {peer_id} alive (pong received)");
                        let mut dir = state.peer_dir.write().await;
                        if let Some(peer) = dir.get_mut(peer_id) {
                            peer.mark_alive();
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("peer {peer_id} unreachable: {e}");
                    let mut dir = state.peer_dir.write().await;
                    if let Some(peer) = dir.get_mut(peer_id) {
                        peer.mark_suspect();
                    }
                }
            }
        }

        // Pull-based replication with first alive peer
        let event_log = state.event_log.read().await;
        let local_gossip =
            node_repl::build_gossip_meta(&state.node_id, "public", &event_log, &state.cas, &[]);
        drop(event_log);

        let local_gossip = match local_gossip {
            Ok(g) => g,
            Err(e) => {
                tracing::warn!("failed to build gossip: {e}");
                continue;
            }
        };

        for (peer_id, address, port) in &peers {
            let gossip_ask = Envelope {
                msg_id: uuid::Uuid::new_v4().to_string(),
                r#type: MsgType::Hello as i32,
                from_node_id: Some(NodeId {
                    value: state.node_id.clone(),
                }),
                body: Some(envelope::Body::Hello(Hello {
                    capabilities: vec!["replication".into()],
                    version: format!("events:{}", local_gossip.segments.len()),
                })),
                ..Default::default()
            };

            match transport.request(address, *port, &gossip_ask).await {
                Ok(_resp) => {
                    tracing::debug!("replication handshake with {peer_id} complete");
                }
                Err(e) => {
                    tracing::debug!("replication handshake with {peer_id} failed: {e}");
                }
            }
        }

        let _ = &policy;
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
    tracing::info!("data_dir   = {:?}", config.data_dir);
    tracing::info!("listen     = {}", config.listen);
    tracing::info!("mesh_port  = {}", config.mesh_port);
    tracing::info!("backend    = {}", config.backend);

    std::fs::create_dir_all(&config.data_dir)?;

    let mut event_log = EventLog::open(&config.data_dir).context("open event log")?;
    let cas = CasStore::open(&config.data_dir).context("open CAS")?;
    let db_path = config.data_dir.join("sqlite").join("meshmind.db");
    let _conn = sqlite_views::open_db(&db_path).context("open SQLite")?;

    let backend = create_backend(&config);

    // Generate node identity (dev CA for now)
    let ca = DevCa::generate().context("generate dev CA")?;
    let identity = ca
        .generate_node_cert("meshmind-node")
        .context("generate node cert")?;
    let node_id = identity.node_id.clone();

    // Load seed data
    let seed_dir = PathBuf::from("seed/public");
    if seed_dir.exists() {
        match node_app::load_seed_data(&seed_dir, &mut event_log, &cas, &db_path, &node_id) {
            Ok(n) if n > 0 => tracing::info!("loaded {n} seed items"),
            Ok(_) => {}
            Err(e) => tracing::warn!("seed loading failed: {e}"),
        }
    }

    tracing::info!("node_id    = {}", node_id);
    tracing::info!("admin_token= {}", config.admin_token);

    // Build TCP+mTLS transport for peer communication
    let transport: Arc<dyn node_mesh::transport::Transport> =
        Arc::new(TcpTransport::new(&identity, &ca.cert_pem).context("create TCP transport")?);

    let peer_dir = Arc::new(RwLock::new(PeerDirectory::new()));

    let data_path = std::path::Path::new(&config.data_dir).to_path_buf();
    let mut scan_dirs = vec![
        data_path.clone(),
        std::path::PathBuf::from("seed"),
        std::path::PathBuf::from("seed/public"),
        std::path::PathBuf::from("seed/public/cases"),
        std::path::PathBuf::from("seed/public/runbooks"),
    ];

    if let Some(home) = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
    {
        let home = std::path::PathBuf::from(home);
        for subdir in &["Documents", "Pictures", "Desktop", "Downloads"] {
            let p = home.join(subdir);
            if p.is_dir() {
                scan_dirs.push(p);
            }
        }
    }

    let state = Arc::new(AppState {
        event_log: RwLock::new(event_log),
        cas,
        db_path,
        peer_dir: RwLock::new(PeerDirectory::new()),
        backend: backend.clone(),
        transport: Some(transport),
        consult_config: ConsultConfig::default(),
        node_id: node_id.clone(),
        admin_token: config.admin_token,
        scan_dirs,
    });

    // Start TCP mesh server
    let mesh_server = TcpServer::bind(&identity, &ca.cert_pem, "0.0.0.0", config.mesh_port)
        .await
        .context("bind mesh server")?;
    let actual_mesh_port = mesh_server.local_port().unwrap_or(config.mesh_port);
    tracing::info!("mesh server listening on 0.0.0.0:{actual_mesh_port}");

    let handler = build_envelope_handler(node_id.clone(), backend);
    tokio::spawn(async move {
        if let Err(e) = mesh_server.serve(handler).await {
            tracing::error!("mesh server error: {e}");
        }
    });

    // Start mDNS discovery
    if config.enable_mdns {
        match mdns_sd::ServiceDaemon::new() {
            Ok(daemon) => {
                if let Err(e) = node_mesh::register_service(&daemon, &node_id, actual_mesh_port) {
                    tracing::warn!("mDNS register failed: {e}");
                }

                match node_mesh::start_discovery(&daemon, peer_dir.clone(), node_id.clone()) {
                    Ok(handle) => {
                        tracing::info!("mDNS discovery started");
                        tokio::spawn(async move {
                            handle.await.ok();
                        });
                    }
                    Err(e) => tracing::warn!("mDNS browse failed: {e}"),
                }
            }
            Err(e) => tracing::warn!("mDNS daemon failed: {e}"),
        }
    }

    // Start relay transport (Internet Mode) if configured
    if let (Some(relay_addr), Some(relay_port)) = (&config.relay_addr, config.relay_port) {
        tracing::info!("relay mode enabled: {}:{}", relay_addr, relay_port);
        match node_mesh::RelayTransport::new(&identity, &ca.cert_pem, relay_addr, relay_port) {
            Ok(relay_transport) => {
                let relay = Arc::new(relay_transport);
                let public_addr = config
                    .public_addr
                    .clone()
                    .unwrap_or_default();

                let relay_reg = relay.clone();
                tokio::spawn(async move {
                    match relay_reg
                        .register(
                            vec!["inference".into(), "storage".into()],
                            &public_addr,
                            config.relay_only,
                        )
                        .await
                    {
                        Ok(resp) if resp.success => {
                            tracing::info!("registered with relay server (token={})", resp.relay_token);
                        }
                        Ok(resp) => {
                            tracing::warn!("relay registration failed: {}", resp.error);
                        }
                        Err(e) => {
                            tracing::warn!("relay registration error: {e}");
                        }
                    }
                });

                // Start relay heartbeat loop
                let relay_hb = relay.clone();
                tokio::spawn(async move {
                    let mut interval = tokio::time::interval(Duration::from_secs(60));
                    loop {
                        interval.tick().await;
                        match relay_hb.heartbeat().await {
                            Ok(resp) if resp.alive => {
                                tracing::debug!("relay heartbeat ok ({} peers)", resp.connected_peers);
                            }
                            Ok(_) => {
                                tracing::warn!("relay heartbeat: not alive");
                            }
                            Err(e) => {
                                tracing::debug!("relay heartbeat failed: {e}");
                            }
                        }
                    }
                });

                // Start WAN discovery loop
                let relay_disc = relay.clone();
                let wan_peer_dir = peer_dir.clone();
                let wan_node_id = node_id.clone();
                tokio::spawn(async move {
                    let mut interval = tokio::time::interval(Duration::from_secs(30));
                    loop {
                        interval.tick().await;
                        match relay_disc.discover("public", 30).await {
                            Ok(resp) => {
                                let mut dir = wan_peer_dir.write().await;
                                for peer in &resp.peers {
                                    let pid = peer.node_id.as_ref().map(|n| n.value.as_str()).unwrap_or("");
                                    if pid.is_empty() || pid == wan_node_id {
                                        continue;
                                    }
                                    let addr = if peer.public_addr.is_empty() {
                                        "relay".to_string()
                                    } else {
                                        peer.public_addr.clone()
                                    };
                                    let is_new = dir.upsert(pid, &addr, peer.mesh_port as u16);
                                    if is_new {
                                        tracing::info!("WAN: discovered peer {pid} at {addr}");
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::debug!("WAN discovery failed: {e}");
                            }
                        }
                    }
                });
            }
            Err(e) => {
                tracing::warn!("failed to create relay transport: {e}");
            }
        }
    }

    // Start replication loop
    let policy = Arc::new(PolicyEngine::with_defaults());
    let repl_state = state.clone();
    let repl_interval = Duration::from_secs(config.replication_interval_secs);
    tokio::spawn(async move {
        replication_loop(repl_state, policy, repl_interval).await;
    });

    // Start HTTP API
    let app = node_api::build_router(state);
    let listener = tokio::net::TcpListener::bind(&config.listen)
        .await
        .with_context(|| format!("bind to {}", config.listen))?;
    tracing::info!("HTTP API listening on {}", config.listen);

    axum::serve(listener, app).await.context("serve")?;

    Ok(())
}
