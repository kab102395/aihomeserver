mod api;
mod llm;
mod memory;
mod nodes;
mod orchestrator;
mod state;
mod tools;

use anyhow::Result;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;
use tracing_subscriber::EnvFilter;

use std::collections::HashMap;

use crate::{
    api::server::{AppState, ApprovalGates, TaskStore, router},
    llm::ollama::OllamaClient,
    memory::{
        conversation::ConversationStore,
        episodic::EpisodicMemory,
        semantic::SemanticMemory,
    },
    orchestrator::Orchestrator,
    tools::{
        filesystem::FilesystemTool,
        git::GitTool,
        http_fetch::HttpFetchTool,
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

    // ── Inference layer ───────────────────────────────────────────────────────
    // Ollama must be running: `ollama serve`
    // Models must be pulled:
    //   ollama pull qwen2.5:14b
    //   ollama pull qwen2.5:32b
    let llm = OllamaClient::new(
        "http://localhost:11434",
        "qwen2.5:14b",   // fast: planner, executor, repair
        "qwen2.5:32b",   // deep critic: high-risk validation only
    );

    // ── Tool layer ────────────────────────────────────────────────────────────
    let mut tools = ToolRegistry::new();
    tools.register(FilesystemTool::new("./workspace")?);
    tools.register(ShellTool);
    tools.register(GitTool::new("."));
    tools.register(HttpFetchTool::new());
    tools.register(WebSearchTool::new());

    // Aliases for alternate names LLMs tend to use
    tools.alias("run_command", "shell");
    tools.alias("bash", "shell");
    tools.alias("execute_command", "shell");
    tools.alias("fetch", "http_fetch");
    tools.alias("search", "web_search");

    info!("Registered tools: {:?}", tools.list());

    // ── Orchestrator ──────────────────────────────────────────────────────────
    let orchestrator = Arc::new(Orchestrator::new(llm, tools));

    // ── Episodic memory (SQLite) ──────────────────────────────────────────────
    let memory = Arc::new(Mutex::new(
        EpisodicMemory::new("episodic.db").await?,
    ));

    // ── Conversation memory (same db file, separate tables) ──────────────────
    let conversations = Arc::new(ConversationStore::new("episodic.db").await?);

    // ── Semantic memory (embeddings + cosine similarity) ──────────────────────
    // Requires: ollama pull nomic-embed-text
    // Gracefully disabled if the model is unavailable — tasks still run normally.
    let semantic = Arc::new(SemanticMemory::new("episodic.db").await?);

    // ── In-memory task status store ───────────────────────────────────────────
    let task_store: TaskStore = Arc::new(tokio::sync::RwLock::new(HashMap::new()));

    // ── Human-approval gate store ─────────────────────────────────────────────
    let approval_gates: ApprovalGates = Arc::new(tokio::sync::RwLock::new(HashMap::new()));

    info!("Memory ready (episodic.db)");

    // ── HTTP API ──────────────────────────────────────────────────────────────
    let app_state = AppState { orchestrator, memory, conversations, semantic, task_store, approval_gates };
    let app = router(app_state);

    let addr = "0.0.0.0:8080";
    info!("Listening on http://{addr}");
    info!("Example: curl -X POST http://localhost:8080/run -H 'Content-Type: application/json' -d '{{\"request\":\"write hello world to hello.txt\"}}'");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
