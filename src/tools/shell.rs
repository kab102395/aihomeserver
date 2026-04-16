use async_trait::async_trait;
use serde_json::{json, Value};

use crate::state::{ErrorType, ToolResult};
use super::Tool;

pub struct ShellTool;

#[async_trait]
impl Tool for ShellTool {
    fn name(&self) -> &str { "shell" }

    async fn execute(&self, params: Value) -> ToolResult {
        let command = match params["command"].as_str() {
            Some(c) if !c.is_empty() => c.to_string(),
            _ => return ToolResult::err(ErrorType::Tool, "missing_command", "params.command is required"),
        };

        let timeout_secs = params["timeout_secs"].as_u64().unwrap_or(30);
        let working_dir = params["cwd"].as_str().unwrap_or(".");

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            tokio::process::Command::new("cmd")
                .args(["/C", &command])
                .current_dir(working_dir)
                .output(),
        )
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
                });

                if exit_code == 0 {
                    ToolResult::ok(
                        output,
                        Some(json!({
                            "type": "shell_exec",
                            "command": command,
                            "exit_code": exit_code,
                        })),
                    )
                } else {
                    ToolResult {
                        success: false,
                        error_type: ErrorType::Tool,
                        error_code: Some(format!("exit_{exit_code}")),
                        trace: Some(stderr),
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
