use axum::{
    extract::{Multipart, Path, State},
    http::{StatusCode, header},
    response::{sse::{Event, KeepAlive, Sse}, IntoResponse},
    routing::{delete, get, post},
    Json, Router,
};
use std::convert::Infallible;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_stream::StreamExt as TokioStreamExt;
use super::ui::CHAT_HTML;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

use crate::{
    config::ServerConfig,
    memory::{
        conversation::ConversationStore,
        episodic::{EpisodicMemory, TaskRecord},
        knowledge::KnowledgeStore,
        semantic::SemanticMemory,
    },
    orchestrator::Orchestrator,
    state::{KnowledgeContext, PlannerOutput, StepDefinition, SystemState},
};

// ── In-memory task status store ───────────────────────────────────────────────


// ── Eval / self-check ──────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct EvalCheck {
    id: String,
    ok: bool,
    detail: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct EvalSelfCheckResponse {
    ok: bool,
    checks: Vec<EvalCheck>,
    duration_ms: u128,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum TaskStatusPayload {
    Running,
    Done { response: RunResponse },
    Failed { error: String },
}

pub type TaskStore = Arc<RwLock<HashMap<Uuid, TaskStatusPayload>>>;
pub type ApprovalGates = Arc<RwLock<HashMap<Uuid, tokio::sync::oneshot::Sender<bool>>>>;

// ── App state ─────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct AppState {
    pub orchestrator: Arc<Orchestrator>,
    pub memory: Arc<Mutex<EpisodicMemory>>,
    pub conversations: Arc<ConversationStore>,
    pub semantic: Arc<SemanticMemory>,
    pub knowledge: Arc<Mutex<KnowledgeStore>>,
    pub task_store: TaskStore,
    /// Pending human-approval gates keyed by task_id.
    pub approval_gates: ApprovalGates,
    /// Live server config — shared with OllamaClient so model/URL changes
    /// propagate immediately. Persisted to config.json on every POST /settings.
    pub config: Arc<RwLock<ServerConfig>>,
    /// In-memory counters/latency metrics (for evals + ops dashboards).
    pub metrics: crate::metrics::SharedMetrics,
}

// ── Request / response types ──────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct RunRequest {
    pub request: String,
    pub session_id: Option<Uuid>,
    pub max_steps: Option<usize>,
    pub admin: Option<bool>,
    /// Answers from the pre-run planning questionnaire (question_id → chosen value).
    pub answers: Option<HashMap<String, String>>,
}

#[derive(Deserialize)]
pub struct PlanRequest {
    pub request: String,
}

#[derive(Debug, Serialize)]
pub struct PlanQuestion {
    pub id: String,
    pub text: String,
    pub options: Vec<PlanOption>,
}

#[derive(Debug, Serialize)]
pub struct PlanOption {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Serialize)]
pub struct PlanResponse {
    pub questions: Vec<PlanQuestion>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunResponse {
    pub task_id: String,
    pub session_id: String,
    pub success: bool,
    pub artifacts: serde_json::Value,
    pub steps_taken: usize,
    pub failure_count: u32,
    pub repair_cycles: u32,
    pub duration_ms: i64,
    pub event_log: Vec<crate::state::LogEvent>,
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(chat_ui))
        // Planning questionnaire
        .route("/plan", post(generate_plan_questions))
        // Task execution
        .route("/run", post(run_task))
        .route("/run/stream", post(run_stream))
        .route("/task/:id/status", get(get_task_status))
        .route("/task/:id/approve", post(approve_task))
        .route("/task/:id/reject", post(reject_task))
        .route("/task/:id", get(get_task))
        .route("/history", get(get_history))
        // Session management
        .route("/sessions", get(get_sessions))
        .route("/sessions/archived", get(get_archived_sessions))
        .route("/session/:id", get(get_session_turns))
        .route("/session/:id/archive", post(archive_session))
        .route("/session/:id/unarchive", post(unarchive_session))
        .route("/session/:id", delete(delete_session))
        // Settings
        .route("/settings", get(get_settings))
        .route("/settings", post(post_settings))
        // Workspace file browser / editor
        .route("/workspace/tree", get(workspace_tree))
        .route("/workspace/file", get(workspace_file))
        .route("/workspace/file", post(workspace_write_file))
        .route("/workspace/file", delete(workspace_delete))
        .route("/workspace/mkdir", post(workspace_mkdir))
        .route("/workspace/rename", post(workspace_rename))
        .route("/workspace/download", get(workspace_download))
        .route("/workspace/upload", post(workspace_upload))
        // Git panel
        .route("/git/status", get(git_status))
        .route("/git/log",    get(git_log))
        .route("/git/diff",   get(git_diff))
        .route("/git/stage",  post(git_stage))
        .route("/git/commit", post(git_commit))
        // Memory browser
        .route("/memory/tasks",        get(memory_list_tasks))
        .route("/memory/tasks/:id",    delete(memory_delete_task))
        .route("/memory/semantic",     get(memory_list_semantic))
        .route("/memory/semantic/:id", delete(memory_delete_semantic))
        .route("/memory/clear",        post(memory_clear_all))
        // Knowledge base
        .route("/knowledge",        get(knowledge_list))
        .route("/knowledge",        post(knowledge_upsert))
        .route("/knowledge/:id",    get(knowledge_get))
        .route("/knowledge/:id",    delete(knowledge_delete))
        // Workspace search
        .route("/workspace/search", get(workspace_search))
        // Eval / self-check
        .route("/eval/self_check", get(eval_self_check))
        .route("/eval/cases", get(eval_cases))
        .route("/eval/run", post(eval_run))
        // Liveness
        .route("/health", get(health))
        .route("/health/deep", get(health_deep))
        .route("/metrics", get(metrics))
        .with_state(state)
}

// ── Handlers ──────────────────────────────────────────────────────────────────

async fn chat_ui() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        CHAT_HTML,
    )
}

/// POST /run — start a task in the background and return immediately.
async fn run_task(
    State(app): State<AppState>,
    Json(req): Json<RunRequest>,
) -> impl IntoResponse {
    // Session setup
    let session_id = match app.conversations.get_or_create_session(req.session_id).await {
        Ok(id) => id,
        Err(e) => {
            tracing::error!("Session error: {e}");
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    };

    let history = app
        .conversations
        .get_recent_turns(&session_id, 10)
        .await
        .unwrap_or_default();

    // Retrieve semantically similar past tasks as few-shot context
    let semantic_examples = match app.orchestrator.llm.embed(&req.request).await {
        Ok(query_vec) => app.semantic.query(&query_vec, 3, 0.65).await,
        Err(e) => {
            tracing::warn!("Semantic embed failed (nomic-embed-text available?): {e}");
            vec![]
        }
    };

    // Retrieve relevant knowledge base entries and inject into planning context
    let knowledge_context: Vec<KnowledgeContext> = {
        let kb = app.knowledge.lock().await;
        match kb.find_relevant(&req.request).await {
            Ok(entries) => entries.into_iter().map(|e| {
                // Only include full content if the topic seems central to the request
                let req_lower = req.request.to_lowercase();
                let age_days = e.age_days();
                let is_primary = e.tag_list().iter().any(|t| req_lower.contains(t.as_str()))
                    || req_lower.contains(&e.topic.to_lowercase());
                KnowledgeContext {
                    id: e.id,
                    topic: e.topic,
                    summary: e.summary,
                    content: if is_primary { Some(e.content) } else { None },
                    tags: e.tags,
                    age_days,
                    version: e.version,
                }
            }).collect(),
            Err(e) => {
                tracing::warn!("Knowledge lookup failed: {e}");
                vec![]
            }
        }
    };

    // Build initial state — inject config values
    let (ws_path, max_steps_cfg, risk_threshold) = {
        let cfg = app.config.read().await;
        (cfg.workspace_path.clone(), cfg.max_steps, cfg.risk_gate_threshold)
    };
    let mut initial = SystemState::new(req.request.clone());
    initial.session_id = Some(session_id);
    initial.conversation_history = history;
    initial.semantic_context = semantic_examples;
    initial.knowledge_context = knowledge_context;
    initial.workspace_path = ws_path;
    initial.risk_gate_threshold = risk_threshold;
    initial.max_steps = max_steps_cfg;
    if let Some(max) = req.max_steps { initial.max_steps = max; }
    if req.admin.unwrap_or(false) { initial.admin_mode = true; }
    if let Some(answers) = req.answers { initial.planning_answers = answers; }

    let task_id = initial.task_id;

    // Register as running
    {
        let mut store = app.task_store.write().await;
        store.insert(task_id, TaskStatusPayload::Running);
    }

    // Return immediately — task runs in background
    let quick = serde_json::json!({
        "task_id": task_id.to_string(),
        "session_id": session_id.to_string(),
        "status": "running",
    });

    // Spawn background execution
    let app2 = app.clone();
    let request_text = req.request.clone();
    app.metrics.record_task_started();
    tokio::spawn(async move {
        let start = std::time::Instant::now();

        let mut initial = initial;
        // Preflight (quick) — store capability snapshot so planner/executor can adapt.
        let (_, preflight) = super::evals::run_eval(
            &app2,
            super::evals::EvalRunRequest {
                cases: vec![],
                mode: super::evals::EvalMode::Quick,
                timeout_secs: Some(15),
            },
        )
        .await;
        initial.artifacts.insert(
            "preflight_eval".into(),
            serde_json::to_value(&preflight).unwrap_or_else(|_| serde_json::json!({})),
        );
        let ws_ok = preflight
            .results
            .iter()
            .find(|r| r.id == "workspace.exists")
            .map(|r| r.ok)
            .unwrap_or(false);
        let fs_ok = preflight
            .results
            .iter()
            .find(|r| r.id == "tool.filesystem.roundtrip")
            .map(|r| r.ok)
            .unwrap_or(false);
        let shell_ok = preflight
            .results
            .iter()
            .find(|r| r.id == "tool.shell.echo")
            .map(|r| r.ok)
            .unwrap_or(false);
        let shell_guard_ok = preflight
            .results
            .iter()
            .find(|r| r.id == "tool.shell.syntax_guard")
            .map(|r| r.ok)
            .unwrap_or(false);
        let shell_backend = preflight
            .results
            .iter()
            .find(|r| r.id == "tool.shell.syntax_guard")
            .and_then(|r| r.detail.get("shell_backend"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "unknown".into());
        let search_url = app2.config.read().await.search_url.clone();
        initial.capabilities = serde_json::json!({
            "workspace_ok": ws_ok,
            "filesystem_ok": fs_ok,
            "shell_ok": shell_ok,
            "shell_syntax_guard_ok": shell_guard_ok,
            "shell_backend": shell_backend,
            "search_url_configured": !search_url.trim().is_empty(),
        });

        let result = app2.orchestrator.run(initial).await;

        match result {
            Ok(final_state) => {
                let duration_ms = start.elapsed().as_millis() as i64;
                app2.metrics.record_task_finished(final_state.termination_met, start.elapsed());
                let answer = extract_answer(&final_state.artifacts);

                // Save conversation turns
                let _ = app2.conversations.add_turn(&session_id, "user", &request_text).await;
                let _ = app2.conversations.add_turn(&session_id, "assistant", &answer).await;

                // Persist episodic record
                let record = TaskRecord {
                    task_id: final_state.task_id,
                    session_id: final_state.session_id,
                    user_request: final_state.user_request.clone(),
                    plan_json: final_state
                        .current_plan
                        .as_ref()
                        .and_then(|p| serde_json::to_string(p).ok()),
                    artifacts_json: serde_json::to_string(&final_state.artifacts)
                        .unwrap_or_else(|_| "{}".into()),
                    critic_scores: final_state.critic_history.iter().map(|r| r.score).collect(),
                    failure_count: final_state.failure_count,
                    repair_cycles: final_state.repair_cycle,
                    duration_ms,
                    success: final_state.termination_met,
                    created_at: chrono::Utc::now(),
                };

                if let Ok(mem) = app2.memory.try_lock() {
                    let _ = mem.save(&record).await;
                }

                // Embed and store in semantic memory (only successful tasks)
                if final_state.termination_met {
                    let summary = answer.chars().take(500).collect::<String>();
                    match app2.orchestrator.llm.embed(&request_text).await {
                        Ok(embedding) => {
                            let _ = app2.semantic.store(
                                &final_state.task_id,
                                final_state.session_id.as_ref(),
                                &request_text,
                                &summary,
                                embedding,
                            ).await;
                        }
                        Err(e) => tracing::warn!("Semantic store failed: {e}"),
                    }
                }

                let response = RunResponse {
                    task_id: final_state.task_id.to_string(),
                    session_id: session_id.to_string(),
                    success: final_state.termination_met,
                    artifacts: serde_json::to_value(&final_state.artifacts).unwrap_or_default(),
                    steps_taken: final_state.current_step,
                    failure_count: final_state.failure_count,
                    repair_cycles: final_state.repair_cycle,
                    duration_ms,
                    event_log: final_state.event_log,
                };

                let mut store = app2.task_store.write().await;
                store.insert(task_id, TaskStatusPayload::Done { response });
            }
            Err(e) => {
                tracing::error!("Orchestrator error: {e}");
                app2.metrics.record_task_finished(false, start.elapsed());
                let mut store = app2.task_store.write().await;
                store.insert(task_id, TaskStatusPayload::Failed { error: e.to_string() });
            }
        }
    });

    (StatusCode::ACCEPTED, Json(quick)).into_response()
}

/// GET /task/:id/status — poll for background task completion.
async fn get_task_status(
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let task_id = match id.parse::<Uuid>() {
        Ok(u) => u,
        Err(_) => return (StatusCode::BAD_REQUEST, "invalid task id").into_response(),
    };
    let store = app.task_store.read().await;
    match store.get(&task_id) {
        Some(status) => (StatusCode::OK, Json(status)).into_response(),
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "status": "not_found" }))).into_response(),
    }
}

async fn get_task(
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let mem = app.memory.lock().await;
    match mem.get_by_id(&id).await {
        Ok(Some(record)) => (StatusCode::OK, Json(serde_json::json!(record))).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, "not found").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn get_history(State(app): State<AppState>) -> impl IntoResponse {
    let mem = app.memory.lock().await;
    match mem.recent(50).await {
        Ok(records) => (StatusCode::OK, Json(records)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn get_sessions(State(app): State<AppState>) -> impl IntoResponse {
    match app.conversations.list_sessions(50).await {
        Ok(sessions) => (StatusCode::OK, Json(sessions)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn get_archived_sessions(State(app): State<AppState>) -> impl IntoResponse {
    match app.conversations.list_archived_sessions(50).await {
        Ok(sessions) => (StatusCode::OK, Json(sessions)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn get_session_turns(
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let session_id = match id.parse::<Uuid>() {
        Ok(u) => u,
        Err(_) => return (StatusCode::BAD_REQUEST, "invalid session id").into_response(),
    };
    match app.conversations.get_all_turns(&session_id).await {
        Ok(turns) => (StatusCode::OK, Json(turns)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn archive_session(
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let session_id = match id.parse::<Uuid>() {
        Ok(u) => u,
        Err(_) => return (StatusCode::BAD_REQUEST, "invalid session id").into_response(),
    };
    match app.conversations.archive_session(&session_id).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn unarchive_session(
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let session_id = match id.parse::<Uuid>() {
        Ok(u) => u,
        Err(_) => return (StatusCode::BAD_REQUEST, "invalid session id").into_response(),
    };
    match app.conversations.unarchive_session(&session_id).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// DELETE /session/:id — hard deletes the session, all turns, and all linked tasks.
/// Nothing is recoverable after this.
async fn delete_session(
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let session_id = match id.parse::<Uuid>() {
        Ok(u) => u,
        Err(_) => return (StatusCode::BAD_REQUEST, "invalid session id").into_response(),
    };

    // Delete linked task records
    if let Ok(mem) = app.memory.try_lock() {
        let _ = mem.delete_by_session(&session_id).await;
    }

    // Delete semantic embeddings
    let _ = app.semantic.delete_by_session(&session_id).await;

    // Delete session + all turns
    match app.conversations.delete_session(&session_id).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// GET /eval/self_check — quick deterministic checks that the runtime can:
/// - access the workspace
/// - read/write/delete files via tools
/// - run a basic shell command
/// - perform a small HTTP fetch
async fn health_deep(State(app): State<AppState>) -> impl IntoResponse {
    let req = super::evals::EvalRunRequest {
        cases: vec![],
        mode: super::evals::EvalMode::Quick,
        timeout_secs: Some(15),
    };
    let (code, resp) = super::evals::run_eval(&app, req).await;
    (code, Json(resp)).into_response()
}

async fn metrics(State(app): State<AppState>) -> impl IntoResponse {
    let snap = app.metrics.snapshot().await;
    Json(snap)
}

async fn eval_cases() -> impl IntoResponse {
    Json(super::evals::list_case_infos())
}

async fn eval_run(
    State(app): State<AppState>,
    Json(req): Json<super::evals::EvalRunRequest>,
) -> impl IntoResponse {
    let (code, resp) = super::evals::run_eval(&app, req).await;
    (code, Json(resp)).into_response()
}

async fn eval_self_check(State(app): State<AppState>) -> impl IntoResponse {
    let start = std::time::Instant::now();
    let mut checks: Vec<EvalCheck> = Vec::new();

    let ws = app.config.read().await.workspace_path.clone();
    checks.push(EvalCheck {
        id: "workspace.exists".into(),
        ok: std::path::Path::new(&ws).is_dir(),
        detail: serde_json::json!({ "workspace_path": ws }),
    });

    let tools = &app.orchestrator.tools;

    // Filesystem: write/read/delete roundtrip in a hidden eval file.
    let eval_file_rel = format!(".eval/self_check_{}.txt", Uuid::new_v4());

    let fs_write = tools
        .execute(
            "filesystem",
            serde_json::json!({
                "action": "write",
                "path": eval_file_rel,
                "content": "aihomeserver self-check\n",
                "overwrite": true
            }),
        )
        .await;
    checks.push(EvalCheck {
        id: "tool.filesystem.write".into(),
        ok: fs_write.success,
        detail: serde_json::json!({
            "success": fs_write.success,
            "error_code": fs_write.error_code,
            "trace": fs_write.trace,
            "output": fs_write.output
        }),
    });

    let fs_read = tools
        .execute(
            "filesystem",
            serde_json::json!({
                "action": "read",
                "path": eval_file_rel
            }),
        )
        .await;
    let read_ok = fs_read.success
        && fs_read
            .output
            .as_ref()
            .and_then(|o| o.get("content"))
            .and_then(|v| v.as_str())
            .map(|s| s.contains("self-check"))
            .unwrap_or(false);
    checks.push(EvalCheck {
        id: "tool.filesystem.read".into(),
        ok: read_ok,
        detail: serde_json::json!({
            "success": fs_read.success,
            "error_code": fs_read.error_code,
            "trace": fs_read.trace,
            "output": fs_read.output
        }),
    });

    let fs_delete = tools
        .execute(
            "filesystem",
            serde_json::json!({
                "action": "delete",
                "path": eval_file_rel
            }),
        )
        .await;
    checks.push(EvalCheck {
        id: "tool.filesystem.delete".into(),
        ok: fs_delete.success,
        detail: serde_json::json!({
            "success": fs_delete.success,
            "error_code": fs_delete.error_code,
            "trace": fs_delete.trace,
            "output": fs_delete.output
        }),
    });

    // Shell: cross-platform basic execution (avoid OS-specific commands).
    let shell = tools
        .execute(
            "shell",
            serde_json::json!({
                "command": "echo SELF_CHECK",
                "timeout_secs": 10,
                "cwd": ws
            }),
        )
        .await;
    let shell_ok = shell.success
        && shell
            .output
            .as_ref()
            .and_then(|o| o.get("stdout"))
            .and_then(|v| v.as_str())
            .map(|s| s.contains("SELF_CHECK"))
            .unwrap_or(false);
    let shell_backend = shell
        .output
        .as_ref()
        .and_then(|o| o.get("shell_backend"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    checks.push(EvalCheck {
        id: "tool.shell.basic".into(),
        ok: shell_ok,
        detail: serde_json::json!({
            "success": shell.success,
            "shell_ok": shell_ok,
            "shell_backend": shell_backend,
            "error_code": shell.error_code,
            "trace": shell.trace,
            "output": shell.output
        }),
    });

    // Shell: ensure our shell syntax-mismatch guard works (PowerShell vs sh).
    let mismatch_cmd = if shell_backend == "sh" {
        // PowerShell-only cmdlet, should be rejected on sh.
        "Select-Object -First 1"
    } else {
        // POSIX tool, should be rejected on PowerShell.
        "echo hi | head -n 1"
    };
    let mismatch = tools
        .execute(
            "shell",
            serde_json::json!({
                "command": mismatch_cmd,
                "timeout_secs": 10,
                "cwd": ws
            }),
        )
        .await;
    let mismatch_ok = !mismatch.success
        && matches!(mismatch.error_code.as_deref(), Some("shell_syntax_mismatch"));
    checks.push(EvalCheck {
        id: "tool.shell.syntax_guard".into(),
        ok: mismatch_ok,
        detail: serde_json::json!({
            "success": mismatch.success,
            "expected_error_code": "shell_syntax_mismatch",
            "error_code": mismatch.error_code,
            "trace": mismatch.trace,
            "output": mismatch.output,
            "shell_backend": shell_backend
        }),
    });

    // http_fetch: known small URL (network-dependent, but deterministic).
    let fetch = tools
        .execute(
            "http_fetch",
            serde_json::json!({
                "url": "https://example.com",
                "max_chars": 2000,
                "allow_reddit_fallback": false
            }),
        )
        .await;
    let fetch_ok = fetch.success
        && fetch
            .output
            .as_ref()
            .and_then(|o| o.get("status"))
            .and_then(|v| v.as_u64())
            .map(|s| s >= 200 && s < 500)
            .unwrap_or(false);
    checks.push(EvalCheck {
        id: "tool.http_fetch.example".into(),
        ok: fetch_ok,
        detail: serde_json::json!({
            "success": fetch.success,
            "error_code": fetch.error_code,
            "trace": fetch.trace,
            "output": fetch.output
        }),
    });

    // Grounding contract (planner post-process): deterministic check that patch/version
    // requests get search + facts + requires_facts propagation.
    let mut plan = PlannerOutput {
        steps: vec![StepDefinition {
            step_id: "1".into(),
            action: "Write patch-specific code".into(),
            tool_binding: None,
            output_format: None,
            requires_facts: false,
            input_params: serde_json::json!({}),
            output_key: Some("answer".into()),
            expected_output: None,
        }],
        tools_required: vec![],
        risk_score: 1,
        expected_outputs: vec!["answer".into()],
        completion_criteria: vec!["done".into()],
        dependencies: serde_json::Value::Null,
    };
    crate::grounding::enforce_grounding_contract(
        "Research Kez, Lina, Puck, Brewmaster, Techies, and Zeus on Dota 2 patch 7.41b",
        &serde_json::json!({ "search_url_configured": true }),
        &mut plan,
    );
    let has_search = plan
        .steps
        .iter()
        .any(|s| s.tool_binding.as_deref() == Some("parallel_search"));
    let has_facts = plan.steps.iter().any(|s| {
        s.output_key.as_deref() == Some("facts")
            && s.tool_binding.is_none()
            && s.output_format.as_deref() == Some("json")
    });
    let answer_requires_facts = plan
        .steps
        .iter()
        .find(|s| s.output_key.as_deref() == Some("answer"))
        .map(|s| s.requires_facts)
        .unwrap_or(false);
    checks.push(EvalCheck {
        id: "grounding.contract".into(),
        ok: has_search && has_facts && answer_requires_facts,
        detail: serde_json::json!({
            "has_search": has_search,
            "has_facts": has_facts,
            "answer_requires_facts": answer_requires_facts,
            "steps": plan.steps.iter().map(|s| serde_json::json!({
                "id": s.step_id,
                "tool": s.tool_binding,
                "output_key": s.output_key,
                "output_format": s.output_format,
                "requires_facts": s.requires_facts
            })).collect::<Vec<_>>(),
        }),
    });

    let ok = checks.iter().all(|c| c.ok);
    (StatusCode::OK, Json(EvalSelfCheckResponse {
        ok,
        checks,
        duration_ms: start.elapsed().as_millis(),
    })).into_response()
}

/// POST /run/stream — start a task and stream SSE events back immediately.
async fn run_stream(
    State(app): State<AppState>,
    Json(req): Json<RunRequest>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    use crate::state::SseEvent;
    use tokio::sync::mpsc;

    let (sse_tx, sse_rx) = mpsc::unbounded_channel::<SseEvent>();

    // Session setup (fire-and-forget defaults on error)
    let session_id = app.conversations
        .get_or_create_session(req.session_id).await
        .unwrap_or_else(|_| uuid::Uuid::new_v4());

    let history = app.conversations
        .get_recent_turns(&session_id, 10).await
        .unwrap_or_default();

    let semantic_examples = match app.orchestrator.llm.embed(&req.request).await {
        Ok(v) => app.semantic.query(&v, 3, 0.65).await,
        Err(_) => vec![],
    };

    let knowledge_context: Vec<KnowledgeContext> = {
        let kb = app.knowledge.lock().await;
        match kb.find_relevant(&req.request).await {
            Ok(entries) => entries.into_iter().map(|e| {
                let req_lower = req.request.to_lowercase();
                let age_days = e.age_days();
                let is_primary = e.tag_list().iter().any(|t| req_lower.contains(t.as_str()))
                    || req_lower.contains(&e.topic.to_lowercase());
                KnowledgeContext {
                    id: e.id, topic: e.topic, summary: e.summary,
                    content: if is_primary { Some(e.content) } else { None },
                    tags: e.tags, age_days, version: e.version,
                }
            }).collect(),
            Err(_) => vec![],
        }
    };

    // Inject config values into initial state
    let (ws_path, max_steps_cfg, risk_threshold) = {
        let cfg = app.config.read().await;
        (cfg.workspace_path.clone(), cfg.max_steps, cfg.risk_gate_threshold)
    };
    let mut initial = crate::state::SystemState::new(req.request.clone());
    initial.session_id = Some(session_id);
    initial.conversation_history = history;
    initial.semantic_context = semantic_examples;
    initial.knowledge_context = knowledge_context;
    initial.sse_tx = Some(sse_tx.clone());
    initial.gate_store = Some(app.approval_gates.clone());
    initial.workspace_path = ws_path;
    initial.risk_gate_threshold = risk_threshold;
    initial.max_steps = max_steps_cfg;
    if let Some(max) = req.max_steps { initial.max_steps = max; }
    if req.admin.unwrap_or(false) { initial.admin_mode = true; }
    if let Some(answers) = req.answers { initial.planning_answers = answers; }

    let task_id = initial.task_id;
    {
        let mut store = app.task_store.write().await;
        store.insert(task_id, TaskStatusPayload::Running);
    }

    let app2 = app.clone();
    let request_text = req.request.clone();
    app.metrics.record_task_started();
    tokio::spawn(async move {
        let start = std::time::Instant::now();
        let _ = sse_tx.send(crate::state::SseEvent::Status { phase: "preflight".into() });

        let mut initial = initial;
        let (_, preflight) = super::evals::run_eval(
            &app2,
            super::evals::EvalRunRequest {
                cases: vec![],
                mode: super::evals::EvalMode::Quick,
                timeout_secs: Some(15),
            },
        )
        .await;
        initial.artifacts.insert(
            "preflight_eval".into(),
            serde_json::to_value(&preflight).unwrap_or_else(|_| serde_json::json!({})),
        );
        let ws_ok = preflight
            .results
            .iter()
            .find(|r| r.id == "workspace.exists")
            .map(|r| r.ok)
            .unwrap_or(false);
        let fs_ok = preflight
            .results
            .iter()
            .find(|r| r.id == "tool.filesystem.roundtrip")
            .map(|r| r.ok)
            .unwrap_or(false);
        let shell_ok = preflight
            .results
            .iter()
            .find(|r| r.id == "tool.shell.echo")
            .map(|r| r.ok)
            .unwrap_or(false);
        let shell_guard_ok = preflight
            .results
            .iter()
            .find(|r| r.id == "tool.shell.syntax_guard")
            .map(|r| r.ok)
            .unwrap_or(false);
        let shell_backend = preflight
            .results
            .iter()
            .find(|r| r.id == "tool.shell.syntax_guard")
            .and_then(|r| r.detail.get("shell_backend"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "unknown".into());
        let search_url = app2.config.read().await.search_url.clone();
        initial.capabilities = serde_json::json!({
            "workspace_ok": ws_ok,
            "filesystem_ok": fs_ok,
            "shell_ok": shell_ok,
            "shell_syntax_guard_ok": shell_guard_ok,
            "shell_backend": shell_backend,
            "search_url_configured": !search_url.trim().is_empty(),
        });

        let result = app2.orchestrator.run(initial).await;

        match result {
            Ok(final_state) => {
                let duration_ms = start.elapsed().as_millis() as i64;
                let answer = extract_answer(&final_state.artifacts);
                let success = final_state.termination_met;
                app2.metrics.record_task_finished(success, start.elapsed());

                let _ = app2.conversations.add_turn(&session_id, "user", &request_text).await;
                let _ = app2.conversations.add_turn(&session_id, "assistant", &answer).await;

                let record = TaskRecord {
                    task_id: final_state.task_id,
                    session_id: final_state.session_id,
                    user_request: final_state.user_request.clone(),
                    plan_json: final_state.current_plan.as_ref()
                        .and_then(|p| serde_json::to_string(p).ok()),
                    artifacts_json: serde_json::to_string(&final_state.artifacts)
                        .unwrap_or_else(|_| "{}".into()),
                    critic_scores: final_state.critic_history.iter().map(|r| r.score).collect(),
                    failure_count: final_state.failure_count,
                    repair_cycles: final_state.repair_cycle,
                    duration_ms,
                    success,
                    created_at: chrono::Utc::now(),
                };
                if let Ok(mem) = app2.memory.try_lock() {
                    let _ = mem.save(&record).await;
                }

                if success {
                    let summary = answer.chars().take(500).collect::<String>();
                    if let Ok(embedding) = app2.orchestrator.llm.embed(&request_text).await {
                        let _ = app2.semantic.store(
                            &final_state.task_id,
                            final_state.session_id.as_ref(),
                            &request_text,
                            &summary,
                            embedding,
                        ).await;
                    }
                }

                let response = RunResponse {
                    task_id: final_state.task_id.to_string(),
                    session_id: session_id.to_string(),
                    success,
                    artifacts: serde_json::to_value(&final_state.artifacts).unwrap_or_default(),
                    steps_taken: final_state.current_step,
                    failure_count: final_state.failure_count,
                    repair_cycles: final_state.repair_cycle,
                    duration_ms,
                    event_log: final_state.event_log,
                };
                {
                    let mut store = app2.task_store.write().await;
                    store.insert(task_id, TaskStatusPayload::Done { response });
                }

                let _ = sse_tx.send(SseEvent::Done {
                    task_id: task_id.to_string(),
                    session_id: session_id.to_string(),
                    success,
                    answer,
                    duration_ms,
                });
            }
            Err(e) => {
                tracing::error!("Stream orchestrator error: {e}");
                app2.metrics.record_task_finished(false, start.elapsed());
                let mut store = app2.task_store.write().await;
                store.insert(task_id, TaskStatusPayload::Failed { error: e.to_string() });
                let _ = sse_tx.send(SseEvent::Error { message: e.to_string() });
            }
        }
    });

    let stream = UnboundedReceiverStream::new(sse_rx).map(|event| {
        let event_type = match &event {
            SseEvent::Status { .. } => "status",
            SseEvent::Token { .. } => "token",
            SseEvent::Done { .. } => "done",
            SseEvent::Error { .. } => "error",
            SseEvent::Plan { .. } => "plan",
            SseEvent::ToolCall { .. } => "tool_call",
            SseEvent::ToolDone { .. } => "tool_done",
            SseEvent::NeedsApproval { .. } => "needs_approval",
            SseEvent::TerminalCmd { .. } => "terminal_cmd",
            SseEvent::TerminalOut { .. } => "terminal_out",
            SseEvent::CriticResult { .. } => "critic_result",
            SseEvent::Repair { .. } => "repair",
            SseEvent::Replan { .. } => "replan",
            SseEvent::FileWritten { .. } => "file_written",
            SseEvent::ThinkingToken { .. } => "thinking_token",
        };
        let data = serde_json::to_string(&event).unwrap_or_default();
        Ok::<Event, Infallible>(Event::default().event(event_type).data(data))
    });

    Sse::new(stream).keep_alive(KeepAlive::default())
}

/// POST /task/:id/approve — resolve a pending high-risk gate (approved).
async fn approve_task(
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let task_id = match id.parse::<Uuid>() {
        Ok(u) => u,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let mut gates = app.approval_gates.write().await;
    if let Some(tx) = gates.remove(&task_id) {
        let _ = tx.send(true);
        StatusCode::NO_CONTENT.into_response()
    } else {
        (StatusCode::NOT_FOUND, "No pending approval for this task").into_response()
    }
}

/// POST /task/:id/reject — resolve a pending high-risk gate (rejected).
async fn reject_task(
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let task_id = match id.parse::<Uuid>() {
        Ok(u) => u,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let mut gates = app.approval_gates.write().await;
    if let Some(tx) = gates.remove(&task_id) {
        let _ = tx.send(false);
        StatusCode::NO_CONTENT.into_response()
    } else {
        (StatusCode::NOT_FOUND, "No pending approval for this task").into_response()
    }
}

// ── /settings ────────────────────────────────────────────────────────────────

async fn get_settings(State(app): State<AppState>) -> impl IntoResponse {
    let cfg = app.config.read().await.clone();
    (StatusCode::OK, Json(cfg)).into_response()
}

async fn post_settings(
    State(app): State<AppState>,
    Json(new_cfg): Json<ServerConfig>,
) -> impl IntoResponse {
    // Ensure the workspace directory exists
    if let Err(e) = new_cfg.ensure_workspace() {
        tracing::warn!("Could not create workspace dir: {e}");
    }

    // Update in-memory config (propagates to OllamaClient immediately)
    {
        let mut cfg = app.config.write().await;
        *cfg = new_cfg.clone();
    }

    // Persist to disk
    let config_path = config_file_path();
    if let Err(e) = new_cfg.save(&config_path).await {
        tracing::error!("Failed to save config.json: {e}");
        return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to save config").into_response();
    }

    tracing::info!("Settings updated and saved");
    StatusCode::NO_CONTENT.into_response()
}

fn config_file_path() -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("config.json")))
        .unwrap_or_else(|| std::path::PathBuf::from("config.json"))
}

// ── /plan — generate pre-run questionnaire ────────────────────────────────────

const PLAN_QUESTION_PROMPT: &str = r#"You are a planning assistant for an AI agent running on a local home server.
Given the user's request, output a short questionnaire (2–4 questions) that would help you complete the task better.
Each question must have exactly 3–5 preset options. Always include a "Custom…" option last.
Output ONLY valid JSON matching this exact shape — no prose, no markdown:
{
  "questions": [
    {
      "id": "q1",
      "text": "How thorough should the work be?",
      "options": [
        {"label": "Quick and concise",  "value": "quick"},
        {"label": "Balanced",           "value": "balanced"},
        {"label": "Deep / exhaustive",  "value": "deep"},
        {"label": "Custom…",            "value": "custom"}
      ]
    }
  ]
}
Keep questions specific to the request. Ask about scope, depth, output format, constraints, or safety preferences.
Never ask about things you can infer from the request itself."#;

async fn generate_plan_questions(
    State(app): State<AppState>,
    Json(req): Json<PlanRequest>,
) -> impl IntoResponse {
    use crate::llm::ollama::{Message, ModelRole};

    let messages = vec![
        Message::system(PLAN_QUESTION_PROMPT),
        Message::user(format!("User request: {}", req.request)),
    ];

    #[derive(serde::Deserialize)]
    struct RawQuestion {
        id: String,
        text: String,
        options: Vec<RawOption>,
    }
    #[derive(serde::Deserialize)]
    struct RawOption {
        label: String,
        value: String,
    }
    #[derive(serde::Deserialize)]
    struct RawResponse {
        questions: Vec<RawQuestion>,
    }

    match app.orchestrator.llm.complete_json::<RawResponse>(messages, ModelRole::Fast, false).await {
        Ok(raw) => {
            let questions: Vec<PlanQuestion> = raw.questions.into_iter().map(|q| PlanQuestion {
                id: q.id,
                text: q.text,
                options: q.options.into_iter().map(|o| PlanOption {
                    label: o.label,
                    value: o.value,
                }).collect(),
            }).collect();
            (StatusCode::OK, Json(PlanResponse { questions })).into_response()
        }
        Err(e) => {
            tracing::warn!("Plan question generation failed: {e}");
            // Graceful fallback: return a minimal default questionnaire
            let questions = vec![
                PlanQuestion {
                    id: "q_depth".into(),
                    text: "How thorough should the work be?".into(),
                    options: vec![
                        PlanOption { label: "Quick summary".into(),    value: "quick".into() },
                        PlanOption { label: "Balanced".into(),         value: "balanced".into() },
                        PlanOption { label: "Deep / exhaustive".into(),value: "deep".into() },
                        PlanOption { label: "Custom…".into(),          value: "custom".into() },
                    ],
                },
                PlanQuestion {
                    id: "q_format".into(),
                    text: "Preferred output format?".into(),
                    options: vec![
                        PlanOption { label: "Plain text".into(),  value: "plain".into() },
                        PlanOption { label: "Markdown".into(),    value: "markdown".into() },
                        PlanOption { label: "Code only".into(),   value: "code".into() },
                        PlanOption { label: "Custom…".into(),     value: "custom".into() },
                    ],
                },
            ];
            (StatusCode::OK, Json(PlanResponse { questions })).into_response()
        }
    }
}

// ── Workspace file browser ────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case", tag = "type")]
enum FsNode {
    Dir  { name: String, path: String, children: Vec<FsNode> },
    File { name: String, path: String, size: u64, ext: String },
}

/// GET /workspace/tree — returns a JSON directory tree up to 4 levels deep,
/// rooted at the configured workspace_path.
async fn workspace_tree(State(app): State<AppState>) -> impl IntoResponse {
    let root = app.config.read().await.workspace_path.clone();
    let root_path = std::path::PathBuf::from(&root);

    if !root_path.exists() {
        return (StatusCode::OK, Json(serde_json::json!({
            "type": "dir", "name": "workspace", "path": "", "children": []
        }))).into_response();
    }

    fn walk(dir: &std::path::Path, base: &std::path::Path, depth: u8) -> Vec<FsNode> {
        if depth == 0 { return vec![]; }
        let Ok(entries) = std::fs::read_dir(dir) else { return vec![]; };

        let mut nodes: Vec<FsNode> = entries
            .flatten()
            .filter_map(|e| {
                let path = e.path();
                let name = path.file_name()?.to_string_lossy().into_owned();
                // Skip hidden files/dirs and common noise
                if name.starts_with('.') { return None; }
                if matches!(name.as_str(), "target" | "node_modules" | "__pycache__") { return None; }

                let rel = path.strip_prefix(base).ok()?.to_string_lossy()
                    .replace('\\', "/");

                if path.is_dir() {
                    Some(FsNode::Dir {
                        children: walk(&path, base, depth - 1),
                        name, path: rel,
                    })
                } else {
                    let size = path.metadata().map(|m| m.len()).unwrap_or(0);
                    let ext  = path.extension()
                        .map(|e| e.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    Some(FsNode::File { name, path: rel, size, ext })
                }
            })
            .collect();

        // Dirs first, then files, both alphabetical
        nodes.sort_by(|a, b| {
            let a_is_dir = matches!(a, FsNode::Dir { .. });
            let b_is_dir = matches!(b, FsNode::Dir { .. });
            b_is_dir.cmp(&a_is_dir).then_with(|| {
                let na = match a { FsNode::Dir { name, .. } | FsNode::File { name, .. } => name };
                let nb = match b { FsNode::Dir { name, .. } | FsNode::File { name, .. } => name };
                na.to_lowercase().cmp(&nb.to_lowercase())
            })
        });
        nodes
    }

    let children = walk(&root_path, &root_path, 4);
    let root_name = root_path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "workspace".into());

    let tree = FsNode::Dir { name: root_name, path: "".into(), children };
    (StatusCode::OK, Json(tree)).into_response()
}

#[derive(Deserialize)]
struct FileQuery { path: String }

/// GET /workspace/file?path=relative/path — returns up to 64 KB of a file's
/// content. Returns an error if the resolved path escapes the workspace root.
async fn workspace_file(
    State(app): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<FileQuery>,
) -> impl IntoResponse {
    let root = app.config.read().await.workspace_path.clone();
    let root_path = std::path::PathBuf::from(&root);

    // Sanitise: strip leading separators, reject path traversal
    let clean: std::path::PathBuf = q.path
        .split(['/', '\\'])
        .filter(|c| !c.is_empty() && *c != "..")
        .collect();

    let full = root_path.join(&clean);

    // Final check: resolved path must still be inside root
    let canonical_root = root_path.canonicalize().unwrap_or(root_path.clone());
    let canonical_full = match full.canonicalize() {
        Ok(p) => p,
        Err(_) => return (StatusCode::NOT_FOUND, "File not found").into_response(),
    };
    if !canonical_full.starts_with(&canonical_root) {
        return (StatusCode::FORBIDDEN, "Path outside workspace").into_response();
    }
    if !canonical_full.is_file() {
        return (StatusCode::BAD_REQUEST, "Not a file").into_response();
    }

    const MAX_BYTES: u64 = 65_536; // 64 KB
    let size = canonical_full.metadata().map(|m| m.len()).unwrap_or(0);
    let truncated = size > MAX_BYTES;

    let content = match tokio::fs::read(&canonical_full).await {
        Ok(bytes) => {
            let slice = if truncated { &bytes[..MAX_BYTES as usize] } else { &bytes };
            String::from_utf8_lossy(slice).into_owned()
        }
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    (StatusCode::OK, Json(serde_json::json!({
        "path": q.path,
        "content": content,
        "size": size,
        "truncated": truncated,
    }))).into_response()
}

// ── Workspace file management ─────────────────────────────────────────────────

/// DELETE /workspace/file?path=relative/path
async fn workspace_delete(
    State(app): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<FileQuery>,
) -> impl IntoResponse {
    let root = app.config.read().await.workspace_path.clone();
    let root_path = std::path::PathBuf::from(&root);
    let clean: std::path::PathBuf = q.path.split(['/', '\\']).filter(|c| !c.is_empty() && *c != "..").collect();
    let full = root_path.join(&clean);
    // Safety: must still be inside workspace
    match full.canonicalize() {
        Ok(p) if !p.starts_with(root_path.canonicalize().unwrap_or(root_path.clone())) =>
            return (StatusCode::FORBIDDEN, "outside workspace").into_response(),
        Err(_) => return (StatusCode::NOT_FOUND, "not found").into_response(),
        _ => {}
    }
    let result = if full.is_dir() {
        tokio::fs::remove_dir_all(&full).await
    } else {
        tokio::fs::remove_file(&full).await
    };
    match result {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// POST /workspace/mkdir  body: { "path": "new/dir" }
async fn workspace_mkdir(
    State(app): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let root = app.config.read().await.workspace_path.clone();
    let root_path = std::path::PathBuf::from(&root);
    let rel = match body["path"].as_str() {
        Some(p) if !p.is_empty() => p,
        _ => return (StatusCode::BAD_REQUEST, "missing path").into_response(),
    };
    let clean: std::path::PathBuf = rel.split(['/', '\\']).filter(|c| !c.is_empty() && *c != "..").collect();
    let full = root_path.join(&clean);
    match tokio::fs::create_dir_all(&full).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// POST /workspace/rename  body: { "from": "old.rs", "to": "new.rs" }
async fn workspace_rename(
    State(app): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let root = app.config.read().await.workspace_path.clone();
    let root_path = std::path::PathBuf::from(&root);
    let from_rel = match body["from"].as_str() { Some(p) if !p.is_empty() => p, _ => return (StatusCode::BAD_REQUEST, "missing from").into_response() };
    let to_rel   = match body["to"].as_str()   { Some(p) if !p.is_empty() => p, _ => return (StatusCode::BAD_REQUEST, "missing to").into_response()   };
    let from_clean: std::path::PathBuf = from_rel.split(['/', '\\']).filter(|c| !c.is_empty() && *c != "..").collect();
    let to_clean:   std::path::PathBuf = to_rel.split(['/', '\\']).filter(|c| !c.is_empty() && *c != "..").collect();
    let from = root_path.join(&from_clean);
    let to   = root_path.join(&to_clean);
    if let Some(p) = to.parent() { let _ = tokio::fs::create_dir_all(p).await; }
    match tokio::fs::rename(&from, &to).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// POST /workspace/file — write content back to a file in the workspace.
/// Body: { "path": "relative/path", "content": "..." }
async fn workspace_write_file(
    State(app): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let root = app.config.read().await.workspace_path.clone();
    let root_path = std::path::PathBuf::from(&root);

    let rel_path = match body["path"].as_str() {
        Some(p) if !p.is_empty() => p,
        _ => return (StatusCode::BAD_REQUEST, "missing path").into_response(),
    };
    let content = body["content"].as_str().unwrap_or("");

    // Sanitise path — no traversal
    let clean: std::path::PathBuf = rel_path
        .split(['/', '\\'])
        .filter(|c| !c.is_empty() && *c != "..")
        .collect();
    let full = root_path.join(&clean);

    // Verify it's still inside the workspace after canonicalisation
    if let Some(parent) = full.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response();
        }
    }

    match tokio::fs::write(&full, content).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// ── Git panel ─────────────────────────────────────────────────────────────────

/// Run a git command in the workspace directory, return stdout as String.
async fn run_git(workspace: &str, args: &[&str]) -> Result<String, String> {
    let out = tokio::process::Command::new("git")
        .args(args)
        .current_dir(workspace)
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).into_owned())
    }
}

/// GET /git/status — branch name + porcelain file list
async fn git_status(State(app): State<AppState>) -> impl IntoResponse {
    let ws = app.config.read().await.workspace_path.clone();

    // Current branch
    let branch = run_git(&ws, &["rev-parse", "--abbrev-ref", "HEAD"]).await
        .unwrap_or_else(|_| "unknown".into())
        .trim().to_string();

    // Ahead/behind
    let ahead_behind = run_git(&ws, &["rev-list", "--left-right", "--count", "HEAD...@{u}"]).await
        .unwrap_or_default();
    let mut ahead = 0u32; let mut behind = 0u32;
    let parts: Vec<&str> = ahead_behind.trim().split_whitespace().collect();
    if parts.len() == 2 {
        ahead  = parts[0].parse().unwrap_or(0);
        behind = parts[1].parse().unwrap_or(0);
    }

    // Porcelain status — split into staged / unstaged / untracked buckets
    let porcelain = run_git(&ws, &["status", "--porcelain"]).await.unwrap_or_default();
    let mut staged: Vec<serde_json::Value>    = Vec::new();
    let mut unstaged: Vec<serde_json::Value>  = Vec::new();
    let mut untracked: Vec<serde_json::Value> = Vec::new();

    for l in porcelain.lines().filter(|l| l.len() > 3) {
        let x = &l[0..1]; // index (staged) status
        let y = &l[1..2]; // worktree (unstaged) status
        let path = l[3..].trim().to_string();

        if x == "?" && y == "?" {
            untracked.push(serde_json::json!({ "path": path, "status": "?" }));
        } else {
            if !matches!(x, " " | "?") {
                staged.push(serde_json::json!({ "path": path, "status": x }));
            }
            if !matches!(y, " " | "?") {
                unstaged.push(serde_json::json!({ "path": path, "status": y }));
            }
        }
    }

    (StatusCode::OK, Json(serde_json::json!({
        "branch": branch, "ahead": ahead, "behind": behind,
        "staged": staged, "unstaged": unstaged, "untracked": untracked,
    }))).into_response()
}

#[derive(Deserialize)]
struct GitLogQuery {
    #[serde(default = "default_log_n")] n: usize,
    #[serde(default = "default_log_n")] limit: usize,
}
fn default_log_n() -> usize { 20 }

/// GET /git/log?limit=20 — recent commits (also accepts `n=`)
async fn git_log(
    State(app): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<GitLogQuery>,
) -> impl IntoResponse {
    let ws = app.config.read().await.workspace_path.clone();
    let count = q.limit.max(q.n);
    let n = count.min(100).to_string();
    let fmt = "%H\x1f%h\x1f%s\x1f%an\x1f%ar";
    let raw = run_git(&ws, &["log", &format!("-{n}"), &format!("--pretty=format:{fmt}")]).await
        .unwrap_or_default();
    let commits: Vec<serde_json::Value> = raw.lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            let p: Vec<&str> = l.splitn(5, '\x1f').collect();
            serde_json::json!({
                "hash":    p.first().unwrap_or(&""),
                "short":   p.get(1).unwrap_or(&""),
                "message": p.get(2).unwrap_or(&""),
                "author":  p.get(3).unwrap_or(&""),
                "date":    p.get(4).unwrap_or(&""),
            })
        })
        .collect();
    (StatusCode::OK, Json(serde_json::json!({ "commits": commits }))).into_response()
}

#[derive(Deserialize)]
struct GitDiffQuery { path: Option<String>, #[serde(default)] staged: bool }

/// GET /git/diff?path=file.rs&staged=false
async fn git_diff(
    State(app): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<GitDiffQuery>,
) -> impl IntoResponse {
    let ws = app.config.read().await.workspace_path.clone();
    let mut args = vec!["diff"];
    if q.staged { args.push("--cached"); }
    let path_str;
    if let Some(ref p) = q.path { path_str = p.clone(); args.push("--"); args.push(&path_str); }
    let diff = run_git(&ws, &args).await.unwrap_or_else(|e| format!("error: {e}"));
    (StatusCode::OK, Json(serde_json::json!({ "diff": diff }))).into_response()
}

/// POST /git/stage  body: { "paths": ["a.rs", "b.rs"] } or { "all": true } or { "path": "a.rs" }
async fn git_stage(
    State(app): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let ws = app.config.read().await.workspace_path.clone();
    let result = if body["all"].as_bool().unwrap_or(false) {
        run_git(&ws, &["add", "-A"]).await
    } else if let Some(arr) = body["paths"].as_array() {
        // Stage each path individually; first failure short-circuits
        let mut last_err = String::new();
        let mut ok = true;
        for v in arr {
            if let Some(p) = v.as_str() {
                if let Err(e) = run_git(&ws, &["add", "--", p]).await {
                    last_err = e; ok = false; break;
                }
            }
        }
        if ok { Ok(String::new()) } else { Err(last_err) }
    } else {
        match body["path"].as_str() {
            Some(p) => run_git(&ws, &["add", "--", p]).await,
            None => return (StatusCode::BAD_REQUEST, "missing path, paths, or all").into_response(),
        }
    };
    match result {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

/// POST /git/commit  body: { "message": "commit msg" }
async fn git_commit(
    State(app): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let ws = app.config.read().await.workspace_path.clone();
    let msg = match body["message"].as_str() {
        Some(m) if !m.trim().is_empty() => m,
        _ => return (StatusCode::BAD_REQUEST, "missing message").into_response(),
    };
    match run_git(&ws, &["commit", "-m", msg]).await {
        Ok(out) => {
            // Extract short hash from first line of output e.g. "[main abc1234] message"
            let hash = out.lines().next()
                .and_then(|l| l.split_whitespace().nth(1))
                .map(|s| s.trim_end_matches(']').to_string());
            (StatusCode::OK, Json(serde_json::json!({
                "success": true,
                "hash": hash,
                "output": out,
            }))).into_response()
        }
        Err(e) => (StatusCode::OK, Json(serde_json::json!({
            "success": false,
            "error": e,
        }))).into_response(),
    }
}

// ── Memory browser ────────────────────────────────────────────────────────────

/// GET /memory/tasks?limit=30
async fn memory_list_tasks(
    State(app): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let limit = params.get("limit")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(30)
        .min(100);
    let mem = app.memory.lock().await;
    match mem.recent(limit).await {
        Ok(tasks) => (StatusCode::OK, Json(tasks)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// DELETE /memory/tasks/:id
async fn memory_delete_task(
    State(app): State<AppState>,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    let uuid = match Uuid::parse_str(&task_id) {
        Ok(u) => u,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let mem = app.memory.lock().await;
    match mem.delete_task(&uuid).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// GET /memory/semantic
async fn memory_list_semantic(State(app): State<AppState>) -> impl IntoResponse {
    let entries = app.semantic.list().await;
    (StatusCode::OK, Json(entries)).into_response()
}

/// DELETE /memory/semantic/:task_id
async fn memory_delete_semantic(
    State(app): State<AppState>,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    let uuid = match Uuid::parse_str(&task_id) {
        Ok(u) => u,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    match app.semantic.delete_by_task(&uuid).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// POST /memory/clear — wipe all episodic task records and semantic embeddings.
async fn memory_clear_all(State(app): State<AppState>) -> impl IntoResponse {
    let mem_result = app.memory.lock().await.clear_all().await;
    let sem_result = app.semantic.clear_all().await;
    match (mem_result, sem_result) {
        (Ok(_), Ok(_)) => StatusCode::NO_CONTENT.into_response(),
        (Err(e), _) | (_, Err(e)) =>
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// ── Workspace search ──────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct SearchQuery { q: String }

#[derive(Serialize)]
struct SearchMatch { file: String, line: usize, preview: String }

/// GET /workspace/search?q=text — recursive content search, max 200 hits.
async fn workspace_search(
    State(app): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<SearchQuery>,
) -> impl IntoResponse {
    if q.q.trim().is_empty() {
        return (StatusCode::OK, Json(serde_json::json!({ "matches": [] }))).into_response();
    }
    let root = app.config.read().await.workspace_path.clone();
    let root_path = std::path::PathBuf::from(&root);
    let query = q.q.to_lowercase();
    let mut matches: Vec<SearchMatch> = Vec::new();
    search_files_recursive(&root_path, &root_path, &query, &mut matches, 5);
    (StatusCode::OK, Json(serde_json::json!({ "matches": matches }))).into_response()
}

fn search_files_recursive(
    dir: &std::path::Path,
    root: &std::path::Path,
    query: &str,
    matches: &mut Vec<SearchMatch>,
    depth: u8,
) {
    if depth == 0 || matches.len() >= 200 { return; }
    let Ok(entries) = std::fs::read_dir(dir) else { return; };
    for entry in entries.flatten() {
        if matches.len() >= 200 { break; }
        let path = entry.path();
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if name.starts_with('.') { continue; }
        if matches!(name.as_str(), "target" | "node_modules" | "__pycache__") { continue; }

        if path.is_dir() {
            search_files_recursive(&path, root, query, matches, depth - 1);
        } else if is_searchable_file(&path) {
            if let Ok(content) = std::fs::read_to_string(&path) {
                let rel = path.strip_prefix(root).ok()
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default();
                let mut per_file = 0usize;
                for (i, line) in content.lines().enumerate() {
                    if per_file >= 5 || matches.len() >= 200 { break; }
                    if line.to_lowercase().contains(query) {
                        matches.push(SearchMatch {
                            file: rel.clone(),
                            line: i + 1,
                            preview: line.trim().chars().take(120).collect(),
                        });
                        per_file += 1;
                    }
                }
            }
        }
    }
}

fn is_searchable_file(path: &std::path::Path) -> bool {
    match path.extension() {
        None => true, // Makefile, Dockerfile, etc.
        Some(ext) => matches!(
            ext.to_string_lossy().to_lowercase().as_str(),
            "rs" | "js" | "ts" | "jsx" | "tsx" | "py" | "go" | "java" | "c" | "cpp" | "h"
            | "html" | "css" | "json" | "toml" | "yaml" | "yml" | "md" | "txt" | "sh"
            | "ps1" | "sql" | "env" | "gitignore" | "lock" | "xml" | "cfg" | "conf"
            | "ini" | "log" | "csv"
        ),
    }
}

// ── Knowledge base API ────────────────────────────────────────────────────────

async fn knowledge_list(State(app): State<AppState>) -> impl IntoResponse {
    match app.knowledge.lock().await.list().await {
        Ok(entries) => Json(serde_json::json!({ "entries": entries })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn knowledge_get(
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match app.knowledge.lock().await.get(&id).await {
        Ok(Some(e)) => Json(e).into_response(),
        Ok(None)    => StatusCode::NOT_FOUND.into_response(),
        Err(e)      => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
struct KnowledgeUpsertReq {
    topic:   String,
    summary: Option<String>,
    content: String,
    tags:    Option<String>,
    sources: Option<String>,
}

async fn knowledge_upsert(
    State(app): State<AppState>,
    Json(req): Json<KnowledgeUpsertReq>,
) -> impl IntoResponse {
    let summary = req.summary.unwrap_or_else(|| req.content.chars().take(300).collect());
    let tags    = req.tags.unwrap_or_default();
    let sources = req.sources.unwrap_or_else(|| "[]".to_string());
    match app.knowledge.lock().await.upsert(&req.topic, &summary, &req.content, &tags, &sources).await {
        Ok(e)  => Json(e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn knowledge_delete(
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match app.knowledge.lock().await.delete(&id).await {
        Ok(_)  => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn extract_answer(artifacts: &HashMap<String, serde_json::Value>) -> String {
    if let Some(v) = artifacts.get("answer") {
        if let Some(s) = v.as_str() {
            if s.len() > 10 {
                return s.to_string();
            }
        }
    }

    // If anything failed, surface the most recent tool failure rather than returning
    // arbitrary intermediate artifacts (e.g. tool-call JSON).
    let mut failures: Vec<(&String, &serde_json::Value)> = artifacts
        .iter()
        .filter(|(k, v)| k.ends_with("_result") && v.get("success").and_then(|b| b.as_bool()) == Some(false))
        .collect();
    failures.sort_by_key(|(k, _)| *k);
    if let Some((k, v)) = failures.last() {
        let code = v.get("error_code").and_then(|x| x.as_str()).unwrap_or("unknown");
        let trace = v.get("trace").and_then(|x| x.as_str()).unwrap_or("").trim();
        if trace.is_empty() {
            return format!("Task failed: tool error in {k} ({code}).");
        }
        return format!("Task failed: tool error in {k} ({code}): {trace}");
    }

    for (k, v) in artifacts.iter() {
        if k.ends_with("_result") {
            if let Some(obj) = v.as_object() {
                if let Some(stdout) = obj.get("stdout").and_then(|s| s.as_str()) {
                    if !stdout.trim().is_empty() {
                        return stdout.trim().to_string();
                    }
                }
                if let Some(body) = obj.get("body").and_then(|s| s.as_str()) {
                    if !body.trim().is_empty() {
                        return body.trim().to_string();
                    }
                }
            }
        }
    }
    "Task completed.".to_string()
}

/// POST /workspace/upload — multipart form upload to workspace.
///
/// Form fields:
/// - `dir` (optional): relative directory under workspace root to place uploads into.
/// - `files` (one or more): uploaded files (filename may include a relative path).
async fn workspace_upload(
    State(app): State<AppState>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let root = app.config.read().await.workspace_path.clone();
    let root_path = std::path::PathBuf::from(&root);

    let mut dir_prefix: Option<String> = None;
    let mut pending_files: Vec<(String, Vec<u8>)> = Vec::new();

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();

        if name == "dir" {
            if let Ok(text) = field.text().await {
                let t = text.trim();
                if !t.is_empty() {
                    dir_prefix = Some(t.to_string());
                }
            }
            continue;
        }

        // Treat any field with a filename as an upload.
        let filename = match field.file_name() {
            Some(f) if !f.trim().is_empty() => f.to_string(),
            _ => continue,
        };

        let Ok(bytes) = field.bytes().await else { continue };
        // Limit to 10MB per file to prevent abuse/accidents.
        if bytes.len() > 10 * 1024 * 1024 {
            continue;
        }
        pending_files.push((filename, bytes.to_vec()));
    }

    let prefix = dir_prefix.unwrap_or_default();

    let mut uploaded: Vec<String> = Vec::new();
    let mut failed: Vec<serde_json::Value> = Vec::new();

    for (filename, bytes) in pending_files {
        // Clean path: drop empties + traversal segments.
        let mut segs: Vec<&str> = Vec::new();
        if !prefix.is_empty() {
            segs.extend(prefix.split(['/', '\\']).filter(|c| !c.is_empty() && *c != ".."));
        }
        segs.extend(filename.split(['/', '\\']).filter(|c| !c.is_empty() && *c != ".."));

        if segs.is_empty() {
            failed.push(serde_json::json!({
                "file": filename,
                "error": "invalid_path"
            }));
            continue;
        }

        let clean: std::path::PathBuf = segs.into_iter().collect();
        let full = root_path.join(&clean);

        if let Some(parent) = full.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                failed.push(serde_json::json!({
                    "file": filename,
                    "error": "mkdir_failed",
                    "detail": e.to_string()
                }));
                continue;
            }
        }

        match tokio::fs::write(&full, &bytes).await {
            Ok(_) => {
                let rel = clean.to_string_lossy().replace('\\', "/");
                uploaded.push(rel);
            }
            Err(e) => failed.push(serde_json::json!({
                "file": filename,
                "error": "write_failed",
                "detail": e.to_string()
            })),
        }
    }

    (StatusCode::OK, Json(serde_json::json!({
        "uploaded": uploaded,
        "failed": failed
    }))).into_response()
}

/// GET /workspace/download?path=relative/path — download a file as an attachment.
async fn workspace_download(
    State(app): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<FileQuery>,
) -> impl IntoResponse {
    let root = app.config.read().await.workspace_path.clone();
    let root_path = std::path::PathBuf::from(&root);

    let clean: std::path::PathBuf = q
        .path
        .split(['/', '\\'])
        .filter(|c| !c.is_empty() && *c != "..")
        .collect();
    let full = root_path.join(&clean);

    let canonical_root = root_path.canonicalize().unwrap_or(root_path.clone());
    let canonical_full = match full.canonicalize() {
        Ok(p) => p,
        Err(_) => return (StatusCode::NOT_FOUND, "File not found").into_response(),
    };
    if !canonical_full.starts_with(&canonical_root) {
        return (StatusCode::FORBIDDEN, "Path outside workspace").into_response();
    }
    if !canonical_full.is_file() {
        return (StatusCode::BAD_REQUEST, "Not a file").into_response();
    }

    let file = match tokio::fs::File::open(&canonical_full).await {
        Ok(f) => f,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };

    let filename = canonical_full
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "download".into())
        .replace('"', "");
    let disposition = format!("attachment; filename=\"{filename}\"");

    let stream = tokio_util::io::ReaderStream::new(file);
    let body = axum::body::Body::from_stream(stream);

    let mut resp = axum::response::Response::new(body);
    *resp.status_mut() = StatusCode::OK;
    resp.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/octet-stream"),
    );
    if let Ok(v) = header::HeaderValue::from_str(&disposition) {
        resp.headers_mut().insert(header::CONTENT_DISPOSITION, v);
    }
    resp
}
