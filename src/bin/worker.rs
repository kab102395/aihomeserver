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
use serde_json::json;

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
struct FilesystemPathRequest {
    path: String,
}

#[derive(Debug, Deserialize)]
struct FilesystemWriteRequest {
    path: String,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    contents_b64: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FilesystemListRequest {
    path: String,
    #[serde(default)]
    depth: Option<u8>,
}

#[derive(Debug, Deserialize)]
struct FilesystemFindRequest {
    path: String,
    pattern: String,
    #[serde(default)]
    max_depth: Option<usize>,
    #[serde(default)]
    max_files: Option<usize>,
    #[serde(default)]
    max_results: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct FilesystemGrepRequest {
    path: String,
    query: String,
    #[serde(default)]
    max_depth: Option<usize>,
    #[serde(default)]
    max_files: Option<usize>,
    #[serde(default)]
    max_results: Option<usize>,
    #[serde(default)]
    max_bytes_per_file: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct FilesystemRenameRequest {
    from: String,
    to: String,
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

#[derive(Debug, Serialize)]
struct WorkerFilesystemCapabilities {
    read: bool,
    write: bool,
    list: bool,
    find: bool,
    grep: bool,
    delete: bool,
    mkdir: bool,
    rename: bool,
}

#[derive(Debug, Serialize)]
struct WorkerBrowserAutomationCapabilities {
    installed: bool,
    playwright: bool,
    chromium: bool,
}

#[derive(Debug, Serialize)]
struct WorkerCapabilitiesResponse {
    ok: bool,
    workspace: String,
    shell: bool,
    browser_fetch: bool,
    filesystem: WorkerFilesystemCapabilities,
    browser_automation: WorkerBrowserAutomationCapabilities,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case", tag = "type")]
enum WorkerFsNode {
    Dir {
        name: String,
        path: String,
        children: Vec<WorkerFsNode>,
    },
    File {
        name: String,
        path: String,
        size: u64,
        ext: String,
    },
}

enum AuthStatus {
    Allowed,
    NoHeader,
    Mismatch,
}

fn check_auth(headers: &axum::http::HeaderMap, token: &Option<String>) -> AuthStatus {
    match token {
        None => AuthStatus::Allowed,
        Some(expected) => match headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
        {
            None => AuthStatus::NoHeader,
            Some(s) if s == format!("Bearer {expected}") => AuthStatus::Allowed,
            Some(_) => AuthStatus::Mismatch,
        },
    }
}

fn reject_unauthorized(reason: &str) -> axum::response::Response {
    eprintln!("[worker] auth rejected: {reason}");
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({ "error": "unauthorized" })),
    )
        .into_response()
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

fn sanitize_relative_path(path: &str) -> anyhow::Result<PathBuf> {
    let trimmed = path.trim();
    if trimmed.is_empty() || trimmed == "." {
        return Ok(PathBuf::from("."));
    }

    let candidate = Path::new(trimmed);
    if candidate.is_absolute() {
        anyhow::bail!("absolute paths are not allowed");
    }
    if candidate.components().any(|c| {
        matches!(
            c,
            std::path::Component::ParentDir
                | std::path::Component::Prefix(_)
                | std::path::Component::RootDir
        )
    }) {
        anyhow::bail!("path escapes worker workspace");
    }
    Ok(candidate.to_path_buf())
}

fn hash_bytes(data: &[u8]) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    data.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn is_probably_text(bytes: &[u8]) -> bool {
    !bytes.iter().any(|b| *b == 0)
}

fn file_kind_and_mime(path: &str, bytes: &[u8]) -> (&'static str, &'static str, bool) {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" => ("image", "image/png", false),
        "jpg" | "jpeg" => ("image", "image/jpeg", false),
        "gif" => ("image", "image/gif", false),
        "webp" => ("image", "image/webp", false),
        "svg" => ("image", "image/svg+xml", true),
        "bmp" => ("image", "image/bmp", false),
        "ico" => ("image", "image/x-icon", false),
        "txt" | "md" | "rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "json" | "toml" | "yaml"
        | "yml" | "html" | "css" | "sql" | "sh" | "ps1" | "log" | "xml" | "csv" => {
            ("text", "text/plain; charset=utf-8", true)
        }
        _ => {
            let is_text = is_probably_text(bytes);
            if is_text {
                ("text", "text/plain; charset=utf-8", true)
            } else {
                ("binary", "application/octet-stream", false)
            }
        }
    }
}

fn rel_path_string(base: &Path, full: &Path) -> String {
    full.strip_prefix(base)
        .unwrap_or(full)
        .to_string_lossy()
        .replace('\\', "/")
}

fn walk_files(root: &Path, max_depth: usize, max_files: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), 0)];

    while let Some((dir, depth)) = stack.pop() {
        if depth > max_depth {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if out.len() >= max_files {
                return out;
            }
            let path = entry.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                stack.push((path, depth + 1));
            } else if meta.is_file() {
                out.push(path);
            }
        }
    }

    out
}

fn build_fs_tree(dir: &Path, base: &Path, depth: u8) -> Vec<WorkerFsNode> {
    if depth == 0 {
        return vec![];
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return vec![];
    };

    let mut nodes: Vec<WorkerFsNode> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_string_lossy().into_owned();
            if is_hidden_or_ignored(&path) {
                return None;
            }

            let rel = rel_path_string(base, &path);
            if path.is_dir() {
                Some(WorkerFsNode::Dir {
                    name,
                    path: rel,
                    children: build_fs_tree(&path, base, depth - 1),
                })
            } else {
                let size = path.metadata().map(|m| m.len()).unwrap_or(0);
                let ext = path
                    .extension()
                    .map(|e| e.to_string_lossy().into_owned())
                    .unwrap_or_default();
                Some(WorkerFsNode::File {
                    name,
                    path: rel,
                    size,
                    ext,
                })
            }
        })
        .collect();

    nodes.sort_by(|a, b| {
        let a_is_dir = matches!(a, WorkerFsNode::Dir { .. });
        let b_is_dir = matches!(b, WorkerFsNode::Dir { .. });
        b_is_dir.cmp(&a_is_dir).then_with(|| {
            let na = match a {
                WorkerFsNode::Dir { name, .. } | WorkerFsNode::File { name, .. } => name,
            };
            let nb = match b {
                WorkerFsNode::Dir { name, .. } | WorkerFsNode::File { name, .. } => name,
            };
            na.to_lowercase().cmp(&nb.to_lowercase())
        })
    });
    nodes
}

fn python_module_installed(module: &str) -> bool {
    std::process::Command::new("python3")
        .args([
            "-c",
            &format!(
                "import importlib.util, sys; sys.exit(0 if importlib.util.find_spec({module:?}) else 1)"
            ),
        ])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn playwright_chromium_installed() -> bool {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(shared) = std::env::var("PLAYWRIGHT_BROWSERS_PATH") {
        candidates.push(PathBuf::from(shared));
    }
    let home = std::env::var("HOME").unwrap_or_default();
    if !home.is_empty() {
        candidates.push(Path::new(&home).join(".cache").join("ms-playwright"));
    }
    candidates.push(PathBuf::from("/var/lib/aihomeserver/ms-playwright"));

    candidates.into_iter().any(|cache_dir| {
        let Ok(entries) = std::fs::read_dir(cache_dir) else {
            return false;
        };
        entries.flatten().any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .to_lowercase()
                .contains("chromium")
        })
    })
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

async fn capabilities(State(state): State<WorkerState>) -> impl IntoResponse {
    let playwright = python_module_installed("playwright");
    let chromium = playwright_chromium_installed();
    Json(WorkerCapabilitiesResponse {
        ok: true,
        workspace: (*state.workspace).clone(),
        shell: true,
        browser_fetch: true,
        filesystem: WorkerFilesystemCapabilities {
            read: true,
            write: true,
            list: true,
            find: true,
            grep: true,
            delete: true,
            mkdir: true,
            rename: true,
        },
        browser_automation: WorkerBrowserAutomationCapabilities {
            installed: playwright && chromium,
            playwright,
            chromium,
        },
    })
}

async fn sync_workspace(
    State(state): State<WorkerState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<WorkspaceSyncRequest>,
) -> impl IntoResponse {
    match check_auth(&headers, &state.token) {
        AuthStatus::Allowed => {}
        AuthStatus::NoHeader => return reject_unauthorized("no Authorization header"),
        AuthStatus::Mismatch => return reject_unauthorized("token mismatch"),
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
    match check_auth(&headers, &state.token) {
        AuthStatus::Allowed => {}
        AuthStatus::NoHeader => return reject_unauthorized("no Authorization header"),
        AuthStatus::Mismatch => return reject_unauthorized("token mismatch"),
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
    match check_auth(&headers, &state.token) {
        AuthStatus::Allowed => {}
        AuthStatus::NoHeader => return reject_unauthorized("no Authorization header"),
        AuthStatus::Mismatch => return reject_unauthorized("token mismatch"),
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
        json!({
            "url": req.url,
            "final_url": final_url,
            "status": status,
            "title": title,
            "text": text,
            "links": links,
            "mode": "worker_fetch"
        }),
        Some(json!({
            "type": "browser_fetch",
            "url": final_url,
            "status": status
        })),
    );
    (StatusCode::OK, Json(result)).into_response()
}

async fn filesystem_read(
    State(state): State<WorkerState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<FilesystemPathRequest>,
) -> impl IntoResponse {
    match check_auth(&headers, &state.token) {
        AuthStatus::Allowed => {}
        AuthStatus::NoHeader => return reject_unauthorized("no Authorization header"),
        AuthStatus::Mismatch => return reject_unauthorized("token mismatch"),
    }

    let rel = match sanitize_relative_path(&req.path) {
        Ok(rel) => rel,
        Err(e) => {
            return (StatusCode::OK, Json(ToolResult::err(ErrorType::Permission, "invalid_path", &e.to_string()))).into_response();
        }
    };
    let full = std::path::Path::new(state.workspace.as_str()).join(&rel);
    let bytes = match std::fs::read(&full) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return (StatusCode::OK, Json(ToolResult::err(ErrorType::Tool, "file_not_found", &req.path))).into_response();
        }
        Err(e) => {
            return (StatusCode::OK, Json(ToolResult::err(ErrorType::Env, "read_failed", &e.to_string()))).into_response();
        }
    };
    let (kind, mime, is_text) = file_kind_and_mime(&req.path, &bytes);

    let result = ToolResult::ok(
        json!({
            "path": req.path,
            "name": full.file_name().and_then(|n| n.to_str()).unwrap_or("file"),
            "content": if is_text { String::from_utf8_lossy(&bytes).to_string() } else { String::new() },
            "contents_b64": STANDARD.encode(&bytes),
            "size": bytes.len(),
            "truncated": false,
            "is_text": is_text,
            "kind": kind,
            "mime": mime,
            "hash": hash_bytes(&bytes),
        }),
        Some(json!({
            "type": "filesystem_read",
            "path": req.path,
        })),
    );
    (StatusCode::OK, Json(result)).into_response()
}

async fn filesystem_write(
    State(state): State<WorkerState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<FilesystemWriteRequest>,
) -> impl IntoResponse {
    match check_auth(&headers, &state.token) {
        AuthStatus::Allowed => {}
        AuthStatus::NoHeader => return reject_unauthorized("no Authorization header"),
        AuthStatus::Mismatch => return reject_unauthorized("token mismatch"),
    }

    let rel = match sanitize_relative_path(&req.path) {
        Ok(rel) => rel,
        Err(e) => {
            return (StatusCode::OK, Json(ToolResult::err(ErrorType::Permission, "invalid_path", &e.to_string()))).into_response();
        }
    };
    let full = std::path::Path::new(state.workspace.as_str()).join(&rel);
    let bytes = if let Some(contents_b64) = req.contents_b64 {
        match STANDARD.decode(contents_b64.as_bytes()) {
            Ok(bytes) => bytes,
            Err(e) => {
                return (StatusCode::OK, Json(ToolResult::err(ErrorType::Tool, "invalid_base64", &e.to_string()))).into_response();
            }
        }
    } else {
        req.content.unwrap_or_default().into_bytes()
    };

    if let Some(parent) = full.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return (StatusCode::OK, Json(ToolResult::err(ErrorType::Env, "mkdir_failed", &e.to_string()))).into_response();
        }
    }
    if let Err(e) = std::fs::write(&full, &bytes) {
        return (StatusCode::OK, Json(ToolResult::err(ErrorType::Env, "write_failed", &e.to_string()))).into_response();
    }

    let result = ToolResult::ok(
        json!({
            "path": req.path,
            "bytes_written": bytes.len(),
            "hash": hash_bytes(&bytes),
        }),
        Some(json!({
            "type": "filesystem_write",
            "path": req.path,
        })),
    );
    (StatusCode::OK, Json(result)).into_response()
}

async fn filesystem_list(
    State(state): State<WorkerState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<FilesystemListRequest>,
) -> impl IntoResponse {
    match check_auth(&headers, &state.token) {
        AuthStatus::Allowed => {}
        AuthStatus::NoHeader => return reject_unauthorized("no Authorization header"),
        AuthStatus::Mismatch => return reject_unauthorized("token mismatch"),
    }

    let rel = match sanitize_relative_path(&req.path) {
        Ok(rel) => rel,
        Err(e) => {
            return (StatusCode::OK, Json(ToolResult::err(ErrorType::Permission, "invalid_path", &e.to_string()))).into_response();
        }
    };
    let full = std::path::Path::new(state.workspace.as_str()).join(&rel);
    if !full.is_dir() {
        return (StatusCode::OK, Json(ToolResult::err(ErrorType::Tool, "not_a_directory", &req.path))).into_response();
    }

    let root_name = if req.path.trim().is_empty() || req.path.trim() == "." {
        "VM /workspace".to_string()
    } else {
        rel.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "VM /workspace".to_string())
    };
    let tree = WorkerFsNode::Dir {
        name: root_name,
        path: if req.path.trim().is_empty() || req.path.trim() == "." {
            "".into()
        } else {
            req.path.clone()
        },
        children: build_fs_tree(&full, &full, req.depth.unwrap_or(4)),
    };
    let mut value = serde_json::to_value(tree).unwrap_or_else(|_| json!({}));
    if let Some(obj) = value.as_object_mut() {
        obj.insert("source".into(), json!("vm"));
            obj.insert("root_path".into(), json!(state.workspace.as_str()));
    }
    (StatusCode::OK, Json(ToolResult::ok(value, None))).into_response()
}

async fn filesystem_find(
    State(state): State<WorkerState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<FilesystemFindRequest>,
) -> impl IntoResponse {
    match check_auth(&headers, &state.token) {
        AuthStatus::Allowed => {}
        AuthStatus::NoHeader => return reject_unauthorized("no Authorization header"),
        AuthStatus::Mismatch => return reject_unauthorized("token mismatch"),
    }

    let rel = match sanitize_relative_path(&req.path) {
        Ok(rel) => rel,
        Err(e) => {
            return (StatusCode::OK, Json(ToolResult::err(ErrorType::Permission, "invalid_path", &e.to_string()))).into_response();
        }
    };
    let full = std::path::Path::new(state.workspace.as_str()).join(&rel);
    if !full.is_dir() {
        return (StatusCode::OK, Json(ToolResult::err(ErrorType::Tool, "not_a_directory", &req.path))).into_response();
    }

    let pattern = req.pattern.to_lowercase();
    let max_depth = req.max_depth.unwrap_or(6);
    let max_files = req.max_files.unwrap_or(4000);
    let max_results = req.max_results.unwrap_or(200);
    let files = walk_files(&full, max_depth, max_files);

    let mut matches = Vec::new();
    for file in files.iter() {
        let candidate = rel_path_string(&full, file);
        if candidate.to_lowercase().contains(&pattern) {
            matches.push(candidate);
            if matches.len() >= max_results {
                break;
            }
        }
    }
    matches.sort();
    let result = ToolResult::ok(
        json!({
            "root": req.path,
            "pattern": req.pattern,
            "matches": matches,
            "files_scanned": files.len(),
            "max_depth": max_depth,
            "max_files": max_files,
            "max_results": max_results,
        }),
        None,
    );
    (StatusCode::OK, Json(result)).into_response()
}

async fn filesystem_grep(
    State(state): State<WorkerState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<FilesystemGrepRequest>,
) -> impl IntoResponse {
    match check_auth(&headers, &state.token) {
        AuthStatus::Allowed => {}
        AuthStatus::NoHeader => return reject_unauthorized("no Authorization header"),
        AuthStatus::Mismatch => return reject_unauthorized("token mismatch"),
    }

    let rel = match sanitize_relative_path(&req.path) {
        Ok(rel) => rel,
        Err(e) => {
            return (StatusCode::OK, Json(ToolResult::err(ErrorType::Permission, "invalid_path", &e.to_string()))).into_response();
        }
    };
    let full = std::path::Path::new(state.workspace.as_str()).join(&rel);
    if !full.is_dir() {
        return (StatusCode::OK, Json(ToolResult::err(ErrorType::Tool, "not_a_directory", &req.path))).into_response();
    }

    let query = req.query.to_lowercase();
    let max_depth = req.max_depth.unwrap_or(6);
    let max_files = req.max_files.unwrap_or(1500);
    let max_results = req.max_results.unwrap_or(200);
    let max_bytes_per_file = req.max_bytes_per_file.unwrap_or(200_000);
    let files = walk_files(&full, max_depth, max_files);
    let mut matches = Vec::new();
    let mut files_scanned = 0usize;
    let mut files_skipped = 0usize;

    for file in files.iter() {
        if matches.len() >= max_results {
            break;
        }
        let Ok(bytes) = std::fs::read(file) else {
            files_skipped += 1;
            continue;
        };
        files_scanned += 1;
        if bytes.len() > max_bytes_per_file || !is_probably_text(&bytes) {
            files_skipped += 1;
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        for (idx, line) in text.lines().enumerate() {
            if matches.len() >= max_results {
                break;
            }
            if line.to_lowercase().contains(&query) {
                matches.push(json!({
                    "path": rel_path_string(&full, file),
                    "line": idx + 1,
                    "text": line.trim(),
                }));
            }
        }
    }

    let result = ToolResult::ok(
        json!({
            "root": req.path,
            "query": req.query,
            "matches": matches,
            "files_scanned": files_scanned,
            "files_skipped": files_skipped,
            "max_depth": max_depth,
            "max_files": max_files,
            "max_results": max_results,
            "max_bytes_per_file": max_bytes_per_file,
        }),
        None,
    );
    (StatusCode::OK, Json(result)).into_response()
}

async fn filesystem_delete(
    State(state): State<WorkerState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<FilesystemPathRequest>,
) -> impl IntoResponse {
    match check_auth(&headers, &state.token) {
        AuthStatus::Allowed => {}
        AuthStatus::NoHeader => return reject_unauthorized("no Authorization header"),
        AuthStatus::Mismatch => return reject_unauthorized("token mismatch"),
    }

    let rel = match sanitize_relative_path(&req.path) {
        Ok(rel) => rel,
        Err(e) => {
            return (StatusCode::OK, Json(ToolResult::err(ErrorType::Permission, "invalid_path", &e.to_string()))).into_response();
        }
    };
    let full = std::path::Path::new(state.workspace.as_str()).join(&rel);
    if !full.exists() {
        return (StatusCode::OK, Json(ToolResult::err(ErrorType::Tool, "file_not_found", &req.path))).into_response();
    }

    let delete_result = if full.is_dir() {
        std::fs::remove_dir_all(&full)
    } else {
        std::fs::remove_file(&full)
    };
    match delete_result {
        Ok(_) => (StatusCode::OK, Json(ToolResult::ok(json!({ "deleted": req.path }), Some(json!({ "type": "filesystem_delete", "path": req.path }))))).into_response(),
        Err(e) => (StatusCode::OK, Json(ToolResult::err(ErrorType::Env, "delete_failed", &e.to_string()))).into_response(),
    }
}

async fn filesystem_mkdir(
    State(state): State<WorkerState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<FilesystemPathRequest>,
) -> impl IntoResponse {
    match check_auth(&headers, &state.token) {
        AuthStatus::Allowed => {}
        AuthStatus::NoHeader => return reject_unauthorized("no Authorization header"),
        AuthStatus::Mismatch => return reject_unauthorized("token mismatch"),
    }

    let rel = match sanitize_relative_path(&req.path) {
        Ok(rel) => rel,
        Err(e) => {
            return (StatusCode::OK, Json(ToolResult::err(ErrorType::Permission, "invalid_path", &e.to_string()))).into_response();
        }
    };
    let full = std::path::Path::new(state.workspace.as_str()).join(&rel);
    match std::fs::create_dir_all(&full) {
        Ok(_) => (StatusCode::OK, Json(ToolResult::ok(json!({ "path": req.path, "created": true }), Some(json!({ "type": "filesystem_mkdir", "path": req.path }))))).into_response(),
        Err(e) => (StatusCode::OK, Json(ToolResult::err(ErrorType::Env, "mkdir_failed", &e.to_string()))).into_response(),
    }
}

async fn filesystem_rename(
    State(state): State<WorkerState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<FilesystemRenameRequest>,
) -> impl IntoResponse {
    match check_auth(&headers, &state.token) {
        AuthStatus::Allowed => {}
        AuthStatus::NoHeader => return reject_unauthorized("no Authorization header"),
        AuthStatus::Mismatch => return reject_unauthorized("token mismatch"),
    }

    let from_rel = match sanitize_relative_path(&req.from) {
        Ok(rel) => rel,
        Err(e) => {
            return (StatusCode::OK, Json(ToolResult::err(ErrorType::Permission, "invalid_from_path", &e.to_string()))).into_response();
        }
    };
    let to_rel = match sanitize_relative_path(&req.to) {
        Ok(rel) => rel,
        Err(e) => {
            return (StatusCode::OK, Json(ToolResult::err(ErrorType::Permission, "invalid_to_path", &e.to_string()))).into_response();
        }
    };
    let from = std::path::Path::new(state.workspace.as_str()).join(&from_rel);
    let to = std::path::Path::new(state.workspace.as_str()).join(&to_rel);
    if let Some(parent) = to.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return (StatusCode::OK, Json(ToolResult::err(ErrorType::Env, "mkdir_failed", &e.to_string()))).into_response();
        }
    }
    match std::fs::rename(&from, &to) {
        Ok(_) => (StatusCode::OK, Json(ToolResult::ok(json!({ "from": req.from, "to": req.to }), Some(json!({ "type": "filesystem_rename", "from": req.from, "to": req.to }))))).into_response(),
        Err(e) => (StatusCode::OK, Json(ToolResult::err(ErrorType::Env, "rename_failed", &e.to_string()))).into_response(),
    }
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
        token: token.clone(),
    };

    let auth_desc = match &token {
        None => "none (open access)".to_string(),
        Some(t) => format!("configured (len={}, fp={}...)", t.len(), &t[..t.len().min(8)]),
    };
    eprintln!("[worker] auth: {auth_desc}");

    let app = Router::new()
        .route("/health", get(health))
        .route("/capabilities", get(capabilities))
        .route("/workspace/sync", post(sync_workspace))
        .route("/shell", post(shell))
        .route("/browser/fetch", post(browser_fetch))
        .route("/filesystem/read", post(filesystem_read))
        .route("/filesystem/write", post(filesystem_write))
        .route("/filesystem/list", post(filesystem_list))
        .route("/filesystem/find", post(filesystem_find))
        .route("/filesystem/grep", post(filesystem_grep))
        .route("/filesystem/delete", post(filesystem_delete))
        .route("/filesystem/mkdir", post(filesystem_mkdir))
        .route("/filesystem/rename", post(filesystem_rename))
        .with_state(state);

    let port = std::env::var("WORKER_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(3031);
    let addr = format!("0.0.0.0:{port}");
    axum::serve(tokio::net::TcpListener::bind(&addr).await?, app).await?;
    Ok(())
}
