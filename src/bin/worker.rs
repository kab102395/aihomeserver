use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::{
    extract::State,
    http::{header, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD, Engine as _};
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
    #[serde(default)]
    collect_paths: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct BrowserFetchRequest {
    url: String,
    #[serde(default)]
    max_chars: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct WorkspaceSyncRequest {
    #[serde(default)]
    prefix: Option<String>,
    files: Vec<WorkspaceFilePayload>,
}

#[derive(Debug, Deserialize)]
struct WorkspaceFilePayload {
    path: String,
    contents_b64: String,
}

#[derive(Debug, Serialize)]
struct WorkspaceArtifactPayload {
    path: String,
    contents_b64: String,
    size: u64,
    truncated: bool,
}

#[derive(Debug, Serialize)]
struct WorkspaceSyncResponse {
    ok: bool,
    workspace: String,
    files_written: usize,
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

fn is_hidden_or_ignored(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|name| {
            name.starts_with('.')
                || matches!(
                    name,
                    "target" | "node_modules" | "__pycache__" | "workspace" | "data"
                )
        })
        .unwrap_or(false)
}

fn resolve_workspace_path(root: &Path, requested: Option<&str>) -> anyhow::Result<PathBuf> {
    let requested = requested.unwrap_or(".").trim();
    let candidate = if requested.is_empty() || requested == "." {
        root.to_path_buf()
    } else {
        let rel = Path::new(requested);
        if rel.is_absolute() {
            rel.to_path_buf()
        } else {
            root.join(rel)
        }
    };

    let root_canon = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let resolved = std::fs::canonicalize(&candidate).unwrap_or(candidate.clone());
    if !resolved.starts_with(&root_canon) {
        anyhow::bail!("path escapes worker workspace");
    }
    Ok(resolved)
}

fn clear_directory_contents(dir: &Path) -> anyhow::Result<()> {
    if !dir.exists() {
        std::fs::create_dir_all(dir)?;
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            std::fs::remove_dir_all(path)?;
        } else {
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}

fn collect_requested_artifacts(root: &Path, paths: &[String]) -> Vec<WorkspaceArtifactPayload> {
    let mut out = Vec::new();
    for rel in paths {
        let rel_path = Path::new(rel);
        if rel_path.is_absolute()
            || rel_path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            continue;
        }
        let full = root.join(rel_path);
        let bytes = match std::fs::read(&full) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let size = bytes.len() as u64;
        out.push(WorkspaceArtifactPayload {
            path: rel.replace('\\', "/"),
            contents_b64: STANDARD.encode(bytes),
            size,
            truncated: false,
        });
    }
    out
}

async fn health(State(state): State<WorkerState>) -> impl IntoResponse {
    Json(HealthResponse {
        ok: true,
        workspace: (*state.workspace).clone(),
    })
}

async fn sync_workspace(
    State(state): State<WorkerState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<WorkspaceSyncRequest>,
) -> impl IntoResponse {
    if !require_auth(&headers, &state.token) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": "unauthorized" })),
        )
            .into_response();
    }

    let prefix = req
        .prefix
        .as_deref()
        .unwrap_or(".")
        .trim()
        .trim_start_matches("./");
    let target_root =
        match resolve_workspace_path(Path::new(state.workspace.as_ref()), Some(prefix)) {
            Ok(p) => p,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({ "error": e.to_string() })),
                )
                    .into_response();
            }
        };

    if let Err(e) = clear_directory_contents(&target_root) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response();
    }

    let mut written = 0usize;
    for file in req.files {
        let rel = Path::new(&file.path);
        if rel.is_absolute()
            || rel
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
            || is_hidden_or_ignored(rel)
        {
            continue;
        }
        let bytes = match STANDARD.decode(file.contents_b64.as_bytes()) {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };
        let target = target_root.join(rel);
        if let Some(parent) = target.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": e.to_string() })),
                )
                    .into_response();
            }
        }
        if let Err(e) = std::fs::write(&target, bytes) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
        written += 1;
    }

    (
        StatusCode::OK,
        Json(WorkspaceSyncResponse {
            ok: true,
            workspace: (*state.workspace).clone(),
            files_written: written,
        }),
    )
        .into_response()
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
    let cwd = match resolve_workspace_path(Path::new(state.workspace.as_ref()), req.cwd.as_deref())
    {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };
    let cwd_for_output = cwd.to_string_lossy().to_string();
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
        Err(_) => ToolResult::err(
            ErrorType::Timeout,
            "command_timeout",
            &format!("timed out after {timeout_secs}s"),
        ),
        Ok(Err(e)) => ToolResult::err(ErrorType::Env, "spawn_failed", &e.to_string()),
        Ok(Ok(out)) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            let exit_code = out.status.code().unwrap_or(-1);
            let collected_artifacts = collect_requested_artifacts(
                Path::new(state.workspace.as_ref()),
                &req.collect_paths,
            );
            if exit_code == 0 {
                ToolResult::ok(
                    serde_json::json!({
                        "stdout": stdout,
                        "stderr": stderr,
                        "exit_code": exit_code,
                        "command": command_for_output,
                        "cwd": cwd_for_output,
                        "task_id": task_id,
                        "workspace": {
                            "root": (*state.workspace).clone(),
                            "collected_artifacts": collected_artifacts,
                        }
                    }),
                    Some(serde_json::json!({
                        "type": "worker_shell",
                        "cwd": cwd_for_output,
                        "exit_code": exit_code,
                        "collected_artifact_count": req.collect_paths.len(),
                    })),
                )
            } else {
                ToolResult {
                    success: false,
                    error_type: ErrorType::Tool,
                    error_code: Some(format!("exit_{exit_code}")),
                    trace: Some(stderr.clone()),
                    output: Some(serde_json::json!({
                        "stdout": stdout,
                        "stderr": stderr,
                        "exit_code": exit_code,
                        "command": command_for_output,
                        "cwd": cwd_for_output,
                        "task_id": task_id,
                        "workspace": {
                            "root": (*state.workspace).clone(),
                            "collected_artifacts": collected_artifacts,
                        }
                    })),
                    checkpoint: Some(serde_json::json!({
                        "type": "worker_shell",
                        "cwd": cwd_for_output,
                        "exit_code": exit_code,
                        "collected_artifact_count": req.collect_paths.len(),
                    })),
                    observed_state_hash: None,
                    timestamp: chrono::Utc::now(),
                }
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
            return (
                StatusCode::OK,
                Json(ToolResult::err(
                    ErrorType::Tool,
                    "browser_fetch_error",
                    &e.to_string(),
                )),
            )
                .into_response()
        }
    };
    let status = resp.status().as_u16();
    let final_url = resp.url().to_string();
    let html = match resp.text().await {
        Ok(t) => t,
        Err(e) => {
            return (
                StatusCode::OK,
                Json(ToolResult::err(
                    ErrorType::Tool,
                    "browser_read_error",
                    &e.to_string(),
                )),
            )
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
    let token = std::env::var("WORKER_TOKEN")
        .ok()
        .filter(|s| !s.trim().is_empty());
    let state = WorkerState {
        workspace: Arc::new(workspace),
        token,
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/workspace/sync", post(sync_workspace))
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
