use axum::{
    extract::State,
    http::{StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use super::ui::CHAT_HTML;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::{
    memory::episodic::{EpisodicMemory, TaskRecord},
    orchestrator::Orchestrator,
    state::SystemState,
};

#[derive(Clone)]
pub struct AppState {
    pub orchestrator: Arc<Orchestrator>,
    pub memory: Arc<Mutex<EpisodicMemory>>,
}

#[derive(Deserialize)]
pub struct RunRequest {
    /// The natural-language task for the orchestrator to execute.
    pub request: String,
    /// Override default max_steps (20) if needed.
    pub max_steps: Option<usize>,
}

#[derive(Serialize)]
pub struct RunResponse {
    pub task_id: String,
    pub success: bool,
    pub artifacts: serde_json::Value,
    pub steps_taken: usize,
    pub failure_count: u32,
    pub repair_cycles: u32,
    pub event_log: Vec<crate::state::LogEvent>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(chat_ui))
        .route("/run", post(run_task))
        .route("/history", get(get_history))
        .route("/health", get(health))
        .with_state(state)
}

async fn chat_ui() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        CHAT_HTML,
    )
}

async fn run_task(
    State(app): State<AppState>,
    Json(req): Json<RunRequest>,
) -> impl IntoResponse {
    let mut initial = SystemState::new(req.request);
    if let Some(max) = req.max_steps {
        initial.max_steps = max;
    }

    let start = std::time::Instant::now();

    match app.orchestrator.run(initial).await {
        Ok(final_state) => {
            let duration_ms = start.elapsed().as_millis() as i64;

            let record = TaskRecord {
                task_id: final_state.task_id,
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

            // Best-effort memory persist (don't fail the response if this errors)
            if let Ok(mem) = app.memory.try_lock() {
                let _ = mem.save(&record).await;
            }

            let response = RunResponse {
                task_id: final_state.task_id.to_string(),
                success: final_state.termination_met,
                artifacts: serde_json::to_value(&final_state.artifacts).unwrap_or_default(),
                steps_taken: final_state.current_step,
                failure_count: final_state.failure_count,
                repair_cycles: final_state.repair_cycle,
                event_log: final_state.event_log,
            };

            (StatusCode::OK, Json(response)).into_response()
        }
        Err(e) => {
            tracing::error!("Orchestrator error: {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

async fn get_history(State(app): State<AppState>) -> impl IntoResponse {
    let mem = app.memory.lock().await;
    match mem.recent(50).await {
        Ok(records) => (StatusCode::OK, Json(records)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}
