use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use crate::{
    state::{ErrorType, ToolResult},
    worker::{
        write_workspace_artifacts, WorkerClient, WorkerShellRequest,
    },
};

use super::Tool;

/// Default working directory for shell commands.
/// Falls back to the workspace subfolder next to the binary, then the binary's
/// own directory, then ".".
fn default_cwd() -> String {
    // Prefer ./workspace relative to the executable
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let ws = dir.join("workspace");
            if ws.is_dir() {
                return ws.to_string_lossy().into_owned();
            }
            return dir.to_string_lossy().into_owned();
        }
    }
    // Fall back to process cwd
    std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| ".".into())
}

fn resolve_local_cwd(workspace_root: &Path, requested: Option<&str>) -> PathBuf {
    let requested = requested.unwrap_or(".");
    let candidate = if requested.trim().is_empty() || requested.trim() == "." {
        workspace_root.to_path_buf()
    } else {
        let rel = Path::new(requested);
        if rel.is_absolute() {
            if rel.starts_with(workspace_root) {
                rel.to_path_buf()
            } else {
                workspace_root.to_path_buf()
            }
        } else {
            workspace_root.join(rel)
        }
    };

    if candidate.is_dir() {
        candidate
    } else if workspace_root.is_dir() {
        workspace_root.to_path_buf()
    } else {
        PathBuf::from(".")
    }
}

fn resolve_remote_cwd(requested: Option<&str>) -> String {
    let requested = requested.unwrap_or(".").trim();
    if requested.is_empty() || requested == "." {
        return ".".into();
    }

    let normalized = requested.replace('\\', "/");
    if normalized == "/workspace" {
        return ".".into();
    }
    if let Some(stripped) = normalized.strip_prefix("/workspace/") {
        return if stripped.trim().is_empty() {
            ".".into()
        } else {
            stripped.trim_matches('/').to_string()
        };
    }

    let rel = Path::new(&normalized);
    if rel.is_absolute() {
        ".".into()
    } else {
        normalized.trim_start_matches("./").trim_matches('/').to_string()
    }
}

#[cfg(not(windows))]
/// Heuristic: detect PowerShell-ish commands when running on POSIX.
///
/// Connection:
/// - The executor generates shell commands from LLM output.
/// - If the model emits PowerShell cmdlets on Linux, we fail fast so the Repair node
///   can regenerate a correct command instead of executing nonsense.
fn looks_like_powershell(command: &str) -> bool {
    let c = command.to_lowercase();
    [
        "get-content",
        "set-content",
        "select-object",
        "get-childitem",
        "out-file",
        "format-table",
        "where-object",
        "foreach-object",
        "convertto-json",
        "$env:",
    ]
    .iter()
    .any(|needle| c.contains(needle))
}

#[cfg(windows)]
/// Heuristic: detect POSIX-ish commands when running on Windows PowerShell.
///
/// Connection:
/// - Used to fail fast with a clear error so Repair can switch syntax.
fn looks_like_posix_shell(command: &str) -> bool {
    let c = command.to_lowercase();
    c.contains(" | head ")
        || c.contains(" | tail ")
        || c.contains("grep ")
        || c.contains("rg ")
        || c.contains("cat ")
        || c.contains("ls ")
        || c.contains("&& ")
        || c.contains("$(")
        || c.contains('`')
}

/// Tool implementation for running shell commands on the server host.
///
/// Safety note:
/// - The orchestration layer can require human approval for high-risk plans.
/// - This tool adds an extra guardrail by rejecting obviously wrong shell syntax for the OS.
pub struct ShellTool {
    worker: Option<WorkerClient>,
    execution_mode: String,
}

impl ShellTool {
    pub fn new(worker: Option<WorkerClient>, execution_mode: impl Into<String>) -> Self {
        Self {
            worker,
            execution_mode: execution_mode.into(),
        }
    }
}

#[async_trait]
impl Tool for ShellTool {
    /// Canonical tool name used in planner/executor `tool_binding`.
    fn name(&self) -> &str {
        "shell"
    }

    /// Execute a command in the server’s shell (PowerShell on Windows, `sh -lc` on POSIX).
    async fn execute(&self, params: Value) -> ToolResult {
        let workspace_root = params
            .get("workspace_root")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(default_cwd()));
        let cwd_requested = params.get("cwd").and_then(|v| v.as_str());

        let remote_requested = self.execution_mode.trim().eq_ignore_ascii_case("remote")
            || self.execution_mode.trim().eq_ignore_ascii_case("auto");
        if remote_requested {
            if let Some(worker) = &self.worker {
                if worker.is_enabled() {
                    let command = match params["command"].as_str() {
                        Some(c) if !c.is_empty() => c.to_string(),
                        _ => {
                            return ToolResult::err(
                                ErrorType::Tool,
                                "missing_command",
                                "params.command is required",
                            )
                        }
                    };
                    let timeout_secs = params["timeout_secs"].as_u64();
                    let task_id = params["task_id"].as_str().map(|s| s.to_string());
                    let remote_cwd = resolve_remote_cwd(cwd_requested);
                    let collect_paths = params
                        .get("collect_paths")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str())
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty())
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();

                    match worker
                        .shell(&WorkerShellRequest {
                            command,
                            cwd: Some(remote_cwd),
                            timeout_secs,
                            task_id,
                            collect_paths,
                        })
                        .await
                    {
                        Ok(result) => {
                            if let Some(workspace) = result
                                .output
                                .as_ref()
                                .and_then(|o| o.get("workspace"))
                                .and_then(|w| w.get("collected_artifacts"))
                                .and_then(|a| a.as_array())
                            {
                                match serde_json::from_value::<
                                    Vec<crate::worker::WorkspaceArtifactPayload>,
                                >(serde_json::Value::Array(
                                    workspace.clone(),
                                )) {
                                    Ok(artifacts) => {
                                        if let Err(e) =
                                            write_workspace_artifacts(&workspace_root, &artifacts)
                                        {
                                            return ToolResult::err(
                                                ErrorType::Env,
                                                "artifact_sync_failed",
                                                &e.to_string(),
                                            );
                                        }
                                    }
                                    Err(e) => {
                                        return ToolResult::err(
                                            ErrorType::Env,
                                            "artifact_sync_failed",
                                            &e.to_string(),
                                        );
                                    }
                                }
                            }
                            return result;
                        }
                        Err(e) => {
                            if self.execution_mode.trim().eq_ignore_ascii_case("remote") {
                                return ToolResult::err(
                                    ErrorType::Env,
                                    "worker_shell_failed",
                                    &e.to_string(),
                                );
                            }
                        }
                    }
                }
            }
        }

        let command = match params["command"].as_str() {
            Some(c) if !c.is_empty() => c.to_string(),
            _ => {
                return ToolResult::err(
                    ErrorType::Tool,
                    "missing_command",
                    "params.command is required",
                )
            }
        };

        let timeout_secs = params["timeout_secs"].as_u64().unwrap_or(30);

        // Use caller-supplied cwd, otherwise pick the server's workspace/root dir.
        // If the requested cwd doesn't exist (common when a Windows config is
        // mounted into a Linux container), fall back to a safe default.
        let cwd_requested = params["cwd"]
            .as_str()
            .map(|s| s.to_string())
            .unwrap_or_else(default_cwd);
        let cwd = resolve_local_cwd(&workspace_root, Some(&cwd_requested));
        let cwd = cwd.to_string_lossy().to_string();

        // Fail fast on OS/shell mismatch so the repair loop can correct the command.
        #[cfg(not(windows))]
        if looks_like_powershell(&command) {
            return ToolResult::err(
                ErrorType::Tool,
                "shell_syntax_mismatch",
                "Command looks like PowerShell, but this server is running on a POSIX shell (sh -lc). Use POSIX commands (ls/cat/rg/head) or the filesystem tool.",
            );
        }
        #[cfg(windows)]
        if looks_like_posix_shell(&command) {
            return ToolResult::err(
                ErrorType::Tool,
                "shell_syntax_mismatch",
                "Command looks like POSIX shell, but this server is running on Windows PowerShell. Use PowerShell syntax (Get-ChildItem/Get-Content/Select-Object) or the filesystem tool.",
            );
        }

        let shell_backend = if cfg!(windows) { "powershell" } else { "sh" };

        let result = tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), async {
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

        match result {
            Err(_) => ToolResult::err(
                ErrorType::Timeout,
                "command_timeout",
                &format!("Command timed out after {timeout_secs}s: {command}"),
            ),
            Ok(Err(e)) => ToolResult::err(ErrorType::Env, "spawn_failed", &e.to_string()),
            Ok(Ok(out)) => {
                let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                let exit_code = out.status.code().unwrap_or(-1);

                let output = json!({
                    "stdout": stdout,
                    "stderr": stderr,
                    "exit_code": exit_code,
                    "command": command,
                    "cwd": cwd,
                    "cwd_requested": cwd_requested,
                    "shell_backend": shell_backend,
                });

                if exit_code == 0 {
                    ToolResult::ok(
                        output,
                        Some(json!({
                            "type": "shell_exec",
                            "command": command,
                            "exit_code": exit_code,
                            "cwd": cwd,
                            "cwd_requested": cwd_requested,
                            "shell_backend": shell_backend,
                        })),
                    )
                } else {
                    ToolResult {
                        success: false,
                        error_type: ErrorType::Tool,
                        error_code: Some(format!("exit_{exit_code}")),
                        trace: Some(stderr.clone()),
                        output: Some(output),
                        checkpoint: None,
                        observed_state_hash: None,
                        timestamp: chrono::Utc::now(),
                    }
                }
            }
        }
    }
}
