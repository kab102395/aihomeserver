use std::sync::Arc;

use axum::{
    extract::State,
    http::{header, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ErrorType {
    None,
    Llm,
    Tool,
    Env,
    Timeout,
    Permission,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ToolResult {
    success: bool,
    error_type: ErrorType,
    error_code: Option<String>,
    trace: Option<String>,
    output: Option<serde_json::Value>,
    checkpoint: Option<serde_json::Value>,
    observed_state_hash: Option<String>,
    timestamp: chrono::DateTime<chrono::Utc>,
}

impl ToolResult {
    fn ok(output: serde_json::Value, checkpoint: Option<serde_json::Value>) -> Self {
        Self {
            success: true,
            error_type: ErrorType::None,
            error_code: None,
            trace: None,
            output: Some(output),
            checkpoint,
            observed_state_hash: None,
            timestamp: chrono::Utc::now(),
        }
    }

    fn err(error_type: ErrorType, code: &str, trace: &str) -> Self {
        Self {
            success: false,
            error_type,
            error_code: Some(code.to_string()),
            trace: Some(trace.to_string()),
            output: None,
            checkpoint: None,
            observed_state_hash: None,
            timestamp: chrono::Utc::now(),
        }
    }
}

#[derive(Clone)]
struct WorkerState {
    workspace: Arc<String>,
    token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ShellRequest {
    command: String,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    timeout_secs: Option<u64>,
    #[serde(default)]
    task_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BrowserFetchRequest {
    url: String,
    #[serde(default)]
    max_chars: Option<usize>,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    ok: bool,
    workspace: String,
}

fn require_auth(headers: &axum::http::HeaderMap, token: &Option<String>) -> bool {
    match token {
        None => true,
        Some(expected) => headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .map(|s| s == format!("Bearer {expected}"))
            .unwrap_or(false),
    }
}

async fn health(State(state): State<WorkerState>) -> impl IntoResponse {
    Json(HealthResponse {
        ok: true,
        workspace: (*state.workspace).clone(),
    })
}

async fn shell(
    State(state): State<WorkerState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<ShellRequest>,
) -> impl IntoResponse {
    if !require_auth(&headers, &state.token) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "unauthorized" })),
        )
            .into_response();
    }

    let task_id = req.task_id.clone();
    let cwd = req.cwd.unwrap_or_else(|| (*state.workspace).clone());
    let cwd_for_output = cwd.clone();
    let command = req.command.clone();
    let command_for_output = command.clone();
    let timeout_secs = req.timeout_secs.unwrap_or(30);

    let out = tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), async move {
        #[cfg(windows)]
        let mut cmd = {
            let mut c = tokio::process::Command::new("powershell");
            c.args(["-NoProfile", "-NonInteractive", "-Command", &command]);
            c
        };

        #[cfg(not(windows))]
        let mut cmd = {
            let mut c = tokio::process::Command::new("sh");
            c.args(["-lc", &command]);
            c
        };

        cmd.current_dir(&cwd).output().await
    })
    .await;

    let result = match out {
        Err(_) => ToolResult::err(ErrorType::Timeout, "command_timeout", &format!("timed out after {timeout_secs}s")),
        Ok(Err(e)) => ToolResult::err(ErrorType::Env, "spawn_failed", &e.to_string()),
        Ok(Ok(out)) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            let exit_code = out.status.code().unwrap_or(-1);
            if exit_code == 0 {
                ToolResult::ok(
                    serde_json::json!({
                        "stdout": stdout,
                        "stderr": stderr,
                        "exit_code": exit_code,
                        "command": command_for_output,
                        "cwd": cwd_for_output,
                        "task_id": task_id,
                    }),
                    Some(serde_json::json!({
                        "type": "worker_shell",
                        "cwd": cwd_for_output,
                        "exit_code": exit_code
                    })),
                )
            } else {
                ToolResult::err(ErrorType::Tool, &format!("exit_{exit_code}"), &stderr)
            }
        }
    };

    (StatusCode::OK, Json(result)).into_response()
}

async fn browser_fetch(
    State(state): State<WorkerState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<BrowserFetchRequest>,
) -> impl IntoResponse {
    if !require_auth(&headers, &state.token) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "unauthorized" })),
        )
            .into_response();
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36")
        .build()
        .expect("browser client");

    let max_chars = req.max_chars.unwrap_or(12000);
    let resp = match client.get(&req.url).send().await {
        Ok(r) => r,
        Err(e) => {
            return (StatusCode::OK, Json(ToolResult::err(ErrorType::Tool, "browser_fetch_error", &e.to_string())))
                .into_response()
        }
    };
    let status = resp.status().as_u16();
    let final_url = resp.url().to_string();
    let html = match resp.text().await {
        Ok(t) => t,
        Err(e) => {
            return (StatusCode::OK, Json(ToolResult::err(ErrorType::Tool, "browser_read_error", &e.to_string())))
                .into_response()
        }
    };

    let doc = scraper::Html::parse_document(&html);
    let title = scraper::Selector::parse("title")
        .ok()
        .and_then(|sel| doc.select(&sel).next())
        .map(|n| n.text().collect::<Vec<_>>().join(" "))
        .unwrap_or_default();
    let body = scraper::Selector::parse("body")
        .ok()
        .and_then(|sel| doc.select(&sel).next())
        .map(|n| n.text().collect::<Vec<_>>().join(" "))
        .unwrap_or_else(|| html.clone());
    let mut text = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() > max_chars {
        text = text.chars().take(max_chars).collect();
    }
    let mut links = Vec::new();
    if let Ok(sel) = scraper::Selector::parse("a[href]") {
        for a in doc.select(&sel).take(40) {
            if let Some(href) = a.value().attr("href") {
                links.push(href.to_string());
            }
        }
    }

    let result = ToolResult::ok(
        serde_json::json!({
            "url": req.url,
            "final_url": final_url,
            "status": status,
            "title": title,
            "text": text,
            "links": links,
            "mode": "worker_fetch"
        }),
        Some(serde_json::json!({
            "type": "browser_fetch",
            "url": final_url,
            "status": status
        })),
    );
    (StatusCode::OK, Json(result)).into_response()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let workspace = std::env::var("WORKER_WORKSPACE").unwrap_or_else(|_| "./workspace".into());
    std::fs::create_dir_all(&workspace)?;
    let token = std::env::var("WORKER_TOKEN").ok().filter(|s| !s.trim().is_empty());
    let state = WorkerState {
        workspace: Arc::new(workspace),
        token,
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/shell", post(shell))
        .route("/browser/fetch", post(browser_fetch))
        .with_state(state);

    let port = std::env::var("WORKER_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(3031);
    let addr = format!("0.0.0.0:{port}");
    axum::serve(tokio::net::TcpListener::bind(&addr).await?, app).await?;
    Ok(())
}
