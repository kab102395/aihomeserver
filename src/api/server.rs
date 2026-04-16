use axum::{
    extract::{Path, State},
    http::{StatusCode, header},
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use super::ui::CHAT_HTML;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

use crate::{
    memory::{
        conversation::ConversationStore,
        episodic::{EpisodicMemory, TaskRecord},
    },
    orchestrator::Orchestrator,
    state::SystemState,
};

// ── In-memory task status store ───────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum TaskStatusPayload {
    Running,
    Done { response: RunResponse },
    Failed { error: String },
}

pub type TaskStore = Arc<RwLock<HashMap<Uuid, TaskStatusPayload>>>;

// ── App state ─────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct AppState {
    pub orchestrator: Arc<Orchestrator>,
    pub memory: Arc<Mutex<EpisodicMemory>>,
    pub conversations: Arc<ConversationStore>,
    pub task_store: TaskStore,
}

// ── Request / response types ──────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct RunRequest {
    pub request: String,
    pub session_id: Option<Uuid>,
    pub max_steps: Option<usize>,
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
    pub event_log: Vec<crate::state::LogEvent>,
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(chat_ui))
        // Task execution
        .route("/run", post(run_task))
        .route("/task/:id/status", get(get_task_status))
        .route("/task/:id", get(get_task))
        .route("/history", get(get_history))
        // Session management
        .route("/sessions", get(get_sessions))
        .route("/sessions/archived", get(get_archived_sessions))
        .route("/session/:id", get(get_session_turns))
        .route("/session/:id/archive", post(archive_session))
        .route("/session/:id/unarchive", post(unarchive_session))
        .route("/session/:id", delete(delete_session))
        // Liveness
        .route("/health", get(health))
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

    // Build initial state
    let mut initial = SystemState::new(req.request.clone());
    initial.session_id = Some(session_id);
    initial.conversation_history = history;
    if let Some(max) = req.max_steps {
        initial.max_steps = max;
    }

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
    tokio::spawn(async move {
        let start = std::time::Instant::now();
        let result = app2.orchestrator.run(initial).await;

        match result {
            Ok(final_state) => {
                let duration_ms = start.elapsed().as_millis() as i64;
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

                let response = RunResponse {
                    task_id: final_state.task_id.to_string(),
                    session_id: session_id.to_string(),
                    success: final_state.termination_met,
                    artifacts: serde_json::to_value(&final_state.artifacts).unwrap_or_default(),
                    steps_taken: final_state.current_step,
                    failure_count: final_state.failure_count,
                    repair_cycles: final_state.repair_cycle,
                    event_log: final_state.event_log,
                };

                let mut store = app2.task_store.write().await;
                store.insert(task_id, TaskStatusPayload::Done { response });
            }
            Err(e) => {
                tracing::error!("Orchestrator error: {e}");
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

    // Delete linked task records first
    if let Ok(mem) = app.memory.try_lock() {
        let _ = mem.delete_by_session(&session_id).await;
    }

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

// ── Helpers ───────────────────────────────────────────────────────────────────

fn extract_answer(artifacts: &HashMap<String, serde_json::Value>) -> String {
    if let Some(v) = artifacts.get("answer") {
        if let Some(s) = v.as_str() {
            if s.len() > 10 {
                return s.to_string();
            }
        }
    }
    for (k, v) in artifacts.iter() {
        if !k.ends_with("_result") && !k.starts_with("repair_") {
            if let Some(s) = v.as_str() {
                if s.len() > 10 {
                    return s.to_string();
                }
            }
        }
    }
    for (k, v) in artifacts.iter() {
        if k.ends_with("_result") {
            if let Some(obj) = v.as_object() {
                if let Some(stdout) = obj.get("stdout").and_then(|s| s.as_str()) {
                    if !stdout.trim().is_empty() {
                        return stdout.trim().to_string();
                    }
                }
            }
        }
    }
    "Task completed.".to_string()
}
