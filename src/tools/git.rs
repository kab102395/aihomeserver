use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;

use crate::state::{ErrorType, ToolResult};
use super::Tool;

pub struct GitTool {
    repo_path: PathBuf,
}

impl GitTool {
    pub fn new(repo_path: impl Into<PathBuf>) -> Self {
        Self { repo_path: repo_path.into() }
    }

    async fn git(&self, args: &[&str]) -> Result<std::process::Output, std::io::Error> {
        tokio::process::Command::new("git")
            .args(args)
            .current_dir(&self.repo_path)
            .output()
            .await
    }

    fn to_result(out: std::process::Output, action: &str) -> ToolResult {
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        let exit_code = out.status.code().unwrap_or(-1);

        if exit_code == 0 {
            ToolResult::ok(
                json!({ "stdout": stdout, "action": action }),
                Some(json!({ "type": "git", "action": action, "exit_code": exit_code })),
            )
        } else {
            ToolResult::err(
                ErrorType::Tool,
                &format!("git_{action}_failed"),
                &format!("exit {exit_code}: {stderr}"),
            )
        }
    }
}

#[async_trait]
impl Tool for GitTool {
    fn name(&self) -> &str { "git" }

    async fn execute(&self, params: Value) -> ToolResult {
        let action = params["action"].as_str().unwrap_or("").to_string();

        match action.as_str() {
            "status" => match self.git(&["status", "--porcelain"]).await {
                Ok(out) => Self::to_result(out, "status"),
                Err(e) => ToolResult::err(ErrorType::Env, "git_failed", &e.to_string()),
            },

            "diff" => {
                let path = params["path"].as_str();
                let mut args = vec!["diff"];
                if let Some(p) = path {
                    args.push(p);
                }
                match self.git(&args).await {
                    Ok(out) => Self::to_result(out, "diff"),
                    Err(e) => ToolResult::err(ErrorType::Env, "git_failed", &e.to_string()),
                }
            }

            "log" => {
                let n = params["n"].as_u64().unwrap_or(10).to_string();
                let flag = format!("-{n}");
                match self.git(&["log", "--oneline", &flag]).await {
                    Ok(out) => Self::to_result(out, "log"),
                    Err(e) => ToolResult::err(ErrorType::Env, "git_failed", &e.to_string()),
                }
            }

            "add" => {
                let path = params["path"].as_str().unwrap_or(".");
                match self.git(&["add", path]).await {
                    Ok(out) => Self::to_result(out, "add"),
                    Err(e) => ToolResult::err(ErrorType::Env, "git_failed", &e.to_string()),
                }
            }

            "commit" => {
                let message = match params["message"].as_str() {
                    Some(m) => m.to_string(),
                    None => return ToolResult::err(ErrorType::Tool, "missing_message", "params.message required for git commit"),
                };
                match self.git(&["commit", "-m", &message]).await {
                    Ok(out) => {
                        // Extract commit SHA from output
                        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                        let sha = stdout
                            .lines()
                            .next()
                            .and_then(|l| l.split_whitespace().nth(1))
                            .unwrap_or("unknown")
                            .trim_end_matches(']')
                            .to_string();
                        let exit_code = out.status.code().unwrap_or(-1);
                        if exit_code == 0 {
                            ToolResult::ok(
                                json!({ "stdout": stdout, "sha": sha }),
                                Some(json!({ "type": "git_commit", "sha": sha, "message": message })),
                            )
                        } else {
                            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                            ToolResult::err(ErrorType::Tool, "commit_failed", &stderr)
                        }
                    }
                    Err(e) => ToolResult::err(ErrorType::Env, "git_failed", &e.to_string()),
                }
            }

            _ => ToolResult::err(ErrorType::Tool, "unsupported_git_action", &action),
        }
    }
}
