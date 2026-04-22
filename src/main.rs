mod api;
mod config;
mod grounding;
mod llm;
mod memory;
mod metrics;
mod nodes;
mod orchestrator;
mod state;
mod tools;

use anyhow::Result;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tracing::info;
use tracing_subscriber::EnvFilter;

use std::collections::HashMap;

use crate::{
    api::server::{AppState, ApprovalGates, TaskStore, router},
    config::ServerConfig,
    llm::ollama::OllamaClient,
    memory::{
        conversation::ConversationStore,
        episodic::EpisodicMemory,
        semantic::SemanticMemory,
    },
    orchestrator::Orchestrator,
    metrics::RuntimeMetrics,
    tools::{
        filesystem::FilesystemTool,
        git::GitTool,
        http_fetch::HttpFetchTool,
        parallel_search::ParallelSearchTool,
        save_knowledge::SaveKnowledgeTool,
        shell::ShellTool,
        web_search::WebSearchTool,
        ToolRegistry,
    },
};

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
    if let Ok(url) = std::env::var("OLLAMA_URL")  { cfg.ollama_url  = url; }
    if let Ok(url) = std::env::var("SEARCH_URL")  { cfg.search_url  = url; }
    if let Ok(ws)  = std::env::var("WORKSPACE")   { cfg.workspace_path = ws; }

    cfg.ensure_workspace().ok();
    info!("Data dir:  {}", data_dir.display());
    info!("Workspace: {}", cfg.workspace_path);
    info!("Models:    fast={} critic={}", cfg.fast_model, cfg.critic_model);
    if !cfg.search_url.is_empty() {
        info!("Search:    {}", cfg.search_url);
    }

    let config = Arc::new(RwLock::new(cfg.clone()));

    // ── Inference layer ───────────────────────────────────────────────────────
    let llm = OllamaClient::new(Arc::clone(&config));

    // ── Tool layer ────────────────────────────────────────────────────────────
    let metrics = Arc::new(RuntimeMetrics::new());

    let mut tools = ToolRegistry::new();
    tools.set_metrics(Arc::clone(&metrics));
    tools.register(FilesystemTool::new(&cfg.workspace_path)?);
    tools.register(ShellTool);
    tools.register(GitTool::new(&cfg.workspace_path));
    tools.register(HttpFetchTool::new());
    tools.register(WebSearchTool::new(Arc::clone(&config)));
    tools.register(ParallelSearchTool::new(Arc::clone(&config)));

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

    info!("Registered tools: {:?}", tools.list());

    // ── Orchestrator ──────────────────────────────────────────────────────────
    let orchestrator = Arc::new(Orchestrator::new(llm, tools));

    let db_path = data_dir.join("episodic.db").to_string_lossy().into_owned();

    // ── Episodic memory (SQLite) ──────────────────────────────────────────────
    let memory = Arc::new(Mutex::new(
        EpisodicMemory::new(&db_path).await?,
    ));

    // ── Conversation memory ───────────────────────────────────────────────────
    let conversations = Arc::new(ConversationStore::new(&db_path).await?);

    // ── Semantic memory ───────────────────────────────────────────────────────
    let semantic = Arc::new(SemanticMemory::new(&db_path).await?);

    // ── In-memory task/approval stores ───────────────────────────────────────
    let task_store: TaskStore = Arc::new(tokio::sync::RwLock::new(HashMap::new()));
    let approval_gates: ApprovalGates = Arc::new(tokio::sync::RwLock::new(HashMap::new()));

    info!("Memory ready (episodic.db)");

    // ── HTTP API ──────────────────────────────────────────────────────────────
    let app_state = AppState {
        orchestrator,
        memory,
        conversations,
        semantic,
        knowledge: knowledge_store,
        task_store,
        approval_gates,
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
