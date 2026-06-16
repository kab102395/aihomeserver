//! Application entry point.
//!
//! This binary wires together:
//! - HTTP server (`axum`) with embedded UIs (`/` and `/learn`)
//! - the agent runtime (`Orchestrator` + `nodes/*`)
//! - tools (`tools/*`) as the only side-effect surface area
//! - persistence (SQLite-backed `memory/*`)
//! - runtime config (`config.rs`) and metrics (`metrics.rs`)
//!
//! If you’re explaining the system, this is the best “start here” file because it
//! shows dependency injection: what gets constructed and how it’s shared.

mod api;
mod coder;
mod config;
mod grounding;
mod llm;
mod memory;
mod metrics;
mod nodes;
mod orchestrator;
mod state;
mod worker;
mod tools;

use anyhow::Result;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tracing::info;
use tracing_subscriber::EnvFilter;

use std::collections::HashMap;

use crate::{
    api::server::{router, AppState, ApprovalGates, TaskStore},
    config::ServerConfig,
    llm::ollama::OllamaClient,
    memory::{
        conversation::ConversationStore, episodic::EpisodicMemory, semantic::SemanticMemory,
        sources::SourceCacheStore,
    },
    metrics::RuntimeMetrics,
    orchestrator::Orchestrator,
    tools::{
        browser::BrowserTool, filesystem::FilesystemTool, git::GitTool, http_fetch::HttpFetchTool,
        parallel_search::ParallelSearchTool, save_knowledge::SaveKnowledgeTool, shell::ShellTool,
        web_search::WebSearchTool, ToolRegistry,
    },
    worker::WorkerClient,
};

fn in_container() -> bool {
    if std::path::Path::new("/.dockerenv").exists() {
        return true;
    }
    if let Ok(cgroup) = std::fs::read_to_string("/proc/1/cgroup") {
        let c = cgroup.to_lowercase();
        return c.contains("docker") || c.contains("containerd") || c.contains("kubepods");
    }
    false
}

async fn searxng_healthy(base_url: &str) -> bool {
    let url = format!(
        "{}/search?q=aihomeserver%20healthcheck&format=json&categories=general&language=en",
        base_url.trim_end_matches('/')
    );
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .user_agent("aihomeserver/healthcheck")
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };

    let resp = match client.get(url).send().await {
        Ok(r) => r,
        Err(_) => return false,
    };
    if !resp.status().is_success() {
        return false;
    }
    let v: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(_) => return false,
    };
    v.get("results").and_then(|x| x.as_array()).is_some()
}

/// Program entry point.
///
/// Connection:
/// - Loads config, initializes stores/tools/orchestrator, and starts the Axum server.
#[tokio::main]
async fn main() -> Result<()> {
    // Logging: RUST_LOG=aihomeserver=debug,info
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("aihomeserver=debug,info")),
        )
        .init();

    info!("Starting aihomeserver");

    // ── Data directory (supports DATA_DIR env var for Docker) ────────────────
    // Native: files sit next to the binary.
    // Docker: mount a volume at /data and set DATA_DIR=/data.
    let data_dir = if let Ok(d) = std::env::var("DATA_DIR") {
        std::path::PathBuf::from(d)
    } else {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| std::path::PathBuf::from("."))
    };
    std::fs::create_dir_all(&data_dir).ok();

    // ── Load config ───────────────────────────────────────────────────────────
    let config_path = data_dir.join("config.json");

    let mut cfg = ServerConfig::load(&config_path).await;

    // Env-var overrides — useful in Docker Compose without touching config.json
    if let Ok(url) = std::env::var("OLLAMA_URL") {
        cfg.ollama_url = url;
    }
    if let Ok(url) = std::env::var("SEARCH_URL") {
        cfg.search_url = url;
    }
    if let Ok(url) = std::env::var("WORKER_URL") {
        cfg.worker_url = url;
    }
    if let Ok(token) = std::env::var("WORKER_TOKEN") {
        cfg.worker_token = token;
    }
    if let Ok(key) = std::env::var("API_KEY") {
        cfg.api_key = key;
    }
    if let Ok(mode) = std::env::var("EXECUTION_MODE") {
        cfg.execution_mode = mode;
    }
    if let Ok(ws) = std::env::var("WORKSPACE") {
        cfg.workspace_path = ws;
    }

    // If no search URL is configured, prefer a local SearXNG if it's reachable.
    // This makes grounded research far more reliable than scraping DDG/Bing.
    if cfg.search_url.trim().is_empty() {
        let candidates: &[&str] = if in_container() {
            &["http://searxng:8080", "http://localhost:8080"]
        } else {
            &[
                "http://localhost:8080",
                "http://127.0.0.1:8080",
                "http://localhost:8888",
                "http://127.0.0.1:8888",
            ]
        };

        for c in candidates {
            if searxng_healthy(c).await {
                cfg.search_url = c.to_string();
                info!("Auto-detected SearXNG: {}", cfg.search_url);
                break;
            }
        }
    }

    cfg.ensure_workspace().ok();
    info!("Data dir:  {}", data_dir.display());
    info!("Workspace: {}", cfg.workspace_path);
    info!(
        "Models:    fast={} critic={}",
        cfg.fast_model, cfg.critic_model
    );
    if !cfg.search_url.is_empty() {
        info!("Search:    {}", cfg.search_url);
    }

    // Repo root for the `/learn` interview-prep site.
    // Default: current working directory; override with REPO_ROOT if needed.
    let repo_root = std::env::var("REPO_ROOT")
        .ok()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
        });
    info!("Repo root: {}", repo_root.display());

    let config = Arc::new(RwLock::new(cfg.clone()));

    // ── Inference layer ───────────────────────────────────────────────────────
    let llm = OllamaClient::new(Arc::clone(&config));

    // ── Tool layer ────────────────────────────────────────────────────────────
    let metrics = Arc::new(RuntimeMetrics::new());
    let worker_client = if cfg.worker_url.trim().is_empty() {
        None
    } else {
        let token_desc = if cfg.worker_token.trim().is_empty() {
            "none (open access)".to_string()
        } else {
            let fp = &cfg.worker_token[..cfg.worker_token.len().min(8)];
            format!("configured (len={}, fp={fp}...)", cfg.worker_token.len())
        };
        info!("Worker URL: {} | token: {}", cfg.worker_url, token_desc);

        match WorkerClient::new(cfg.worker_url.clone(), cfg.worker_token.clone()) {
            Ok(client) => {
                match client.health().await {
                    Ok(health) => {
                        info!(
                            "Worker health: ok={} workspace={}",
                            health.ok,
                            health.workspace.unwrap_or_default()
                        );
                        // Only probe authenticated execution when a token is configured.
                        // An open-access worker skips this; a token-protected worker must
                        // accept the probe or coordinator startup will log the mismatch.
                        if !cfg.worker_token.trim().is_empty() {
                            let probe = crate::worker::WorkerShellRequest {
                                command: "echo aihomeserver-auth-probe".into(),
                                cwd: Some(".".into()),
                                timeout_secs: Some(10),
                                task_id: None,
                                collect_paths: vec![],
                            };
                            match client.shell(&probe).await {
                                Ok(result) if result.success => {
                                    info!("Worker auth probe: ok");
                                }
                                Ok(result) => {
                                    info!(
                                        "Worker auth probe failed: error_type={:?} trace={:?}",
                                        result.error_type, result.trace
                                    );
                                }
                                Err(e) => {
                                    info!("Worker auth probe error: {e}");
                                }
                            }
                        }
                    }
                    Err(e) => {
                        info!("Worker health check failed: {e}");
                    }
                }
                Some(client)
            }
            Err(e) => {
                info!("Worker client disabled: {e}");
                None
            }
        }
    };

    let mut tools = ToolRegistry::new();
    tools.set_metrics(Arc::clone(&metrics));
    tools.register(FilesystemTool::new(&cfg.workspace_path)?);
    tools.register(ShellTool::new(worker_client.clone(), cfg.execution_mode.clone()));
    tools.register(GitTool::new(&cfg.workspace_path));
    let sources_db_path = data_dir.join("sources.db").to_string_lossy().into_owned();
    let sources = Arc::new(SourceCacheStore::new(&sources_db_path).await?);
    tools.register(HttpFetchTool::new(Some(Arc::clone(&sources))));
    tools.register(WebSearchTool::new(Arc::clone(&config)));
    tools.register(ParallelSearchTool::new(Arc::clone(&config)));
    tools.register(BrowserTool::new(worker_client));

    // ── Knowledge base ────────────────────────────────────────────────────────
    let knowledge_db_path = data_dir.join("knowledge.db").to_string_lossy().into_owned();
    let knowledge_store = Arc::new(tokio::sync::Mutex::new(
        memory::knowledge::KnowledgeStore::new(&knowledge_db_path).await?,
    ));
    tools.register(SaveKnowledgeTool::new(Arc::clone(&knowledge_store)));

    // Aliases for alternate names LLMs tend to use
    tools.alias("run_command", "shell");
    tools.alias("bash", "shell");
    tools.alias("execute_command", "shell");
    tools.alias("fetch", "http_fetch");
    tools.alias("search", "web_search");
    tools.alias("multi_search", "parallel_search");
    tools.alias("batch_search", "parallel_search");
    tools.alias("open_browser", "browser");
    tools.alias("browse", "browser");

    info!("Registered tools: {:?}", tools.list());

    // ── Orchestrator ──────────────────────────────────────────────────────────
    let orchestrator = Arc::new(Orchestrator::new(llm, tools));

    let db_path = data_dir.join("episodic.db").to_string_lossy().into_owned();

    // ── Episodic memory (SQLite) ──────────────────────────────────────────────
    let memory = Arc::new(Mutex::new(EpisodicMemory::new(&db_path).await?));

    // ── Conversation memory ───────────────────────────────────────────────────
    let conversations = Arc::new(ConversationStore::new(&db_path).await?);

    // ── Semantic memory ───────────────────────────────────────────────────────
    let semantic = Arc::new(SemanticMemory::new(&db_path).await?);

    // ── In-memory task/approval stores ───────────────────────────────────────
    let task_store: TaskStore = Arc::new(tokio::sync::RwLock::new(HashMap::new()));
    let approval_gates: ApprovalGates = Arc::new(tokio::sync::RwLock::new(HashMap::new()));
    let cancel_store: crate::api::server::CancelStore =
        Arc::new(tokio::sync::RwLock::new(HashMap::new()));

    info!("Memory ready (episodic.db)");

    // ── HTTP API ──────────────────────────────────────────────────────────────
    // Everything the handlers and background runs need is bundled here once and shared
    // through Axum state instead of being recreated per request.
    let app_state = AppState {
        orchestrator,
        memory,
        conversations,
        semantic,
        knowledge: knowledge_store,
        sources,
        task_store,
        repo_root,
        approval_gates,
        cancel_store,
        config,
        metrics,
    };
    let app = router(app_state);

    let port = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(3000);
    let addr = format!("0.0.0.0:{port}");
    info!("Listening on http://{addr}");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
