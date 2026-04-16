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
    api::server::{AppState, TaskStore, router},
    llm::ollama::OllamaClient,
    memory::{conversation::ConversationStore, episodic::EpisodicMemory},
    orchestrator::Orchestrator,
    tools::{
        filesystem::FilesystemTool,
        git::GitTool,
        shell::ShellTool,
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

    info!("Registered tools: {:?}", tools.list());

    // ── Orchestrator ──────────────────────────────────────────────────────────
    let orchestrator = Arc::new(Orchestrator::new(llm, tools));

    // ── Episodic memory (SQLite) ──────────────────────────────────────────────
    let memory = Arc::new(Mutex::new(
        EpisodicMemory::new("episodic.db").await?,
    ));

    // ── Conversation memory (same db file, separate tables) ──────────────────
    let conversations = Arc::new(ConversationStore::new("episodic.db").await?);

    // ── In-memory task status store ───────────────────────────────────────────
    let task_store: TaskStore = Arc::new(tokio::sync::RwLock::new(HashMap::new()));

    info!("Memory ready (episodic.db)");

    // ── HTTP API ──────────────────────────────────────────────────────────────
    // POST /run                    — start task in background, returns task_id immediately
    // GET  /task/:id/status        — poll for task completion
    // GET  /task/:id               — load a completed task record
    // GET  /sessions               — active sessions for sidebar
    // GET  /sessions/archived      — archived sessions
    // GET  /session/:id            — load all turns
    // POST /session/:id/archive    — archive a session
    // POST /session/:id/unarchive  — restore from archive
    // DELETE /session/:id          — hard delete (session + turns + tasks)
    // GET  /health                 — liveness check
    let app_state = AppState { orchestrator, memory, conversations, task_store };
    let app = router(app_state);

    let addr = "0.0.0.0:8080";
    info!("Listening on http://{addr}");
    info!("Example: curl -X POST http://localhost:8080/run -H 'Content-Type: application/json' -d '{{\"request\":\"write hello world to hello.txt\"}}'");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
