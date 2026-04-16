use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use crate::state::{ErrorType, ToolResult};
use super::Tool;

pub struct FilesystemTool {
    base_dir: PathBuf,
}

impl FilesystemTool {
    pub fn new(base_dir: impl Into<PathBuf>) -> std::io::Result<Self> {
        let base_dir = base_dir.into();
        std::fs::create_dir_all(&base_dir)?;
        Ok(Self { base_dir })
    }

    fn resolve(&self, path: &str) -> PathBuf {
        // Prevent path traversal: strip leading / and ..
        let clean: PathBuf = path
            .split(['/', '\\'])
            .filter(|c| !c.is_empty() && *c != "..")
            .collect();
        self.base_dir.join(clean)
    }

    fn hash_bytes(data: &[u8]) -> String {
        let mut hasher = DefaultHasher::new();
        data.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }
}

#[async_trait]
impl Tool for FilesystemTool {
    fn name(&self) -> &str { "filesystem" }

    async fn execute(&self, params: Value) -> ToolResult {
        let action = params["action"].as_str().unwrap_or("").to_string();
        let path = match params["path"].as_str() {
            Some(p) if !p.is_empty() => p.to_string(),
            _ => return ToolResult::err(ErrorType::Tool, "missing_path", "params.path is required"),
        };

        match action.as_str() {
            "write" => {
                let content = params["content"].as_str().unwrap_or("");
                let overwrite = params["overwrite"].as_bool().unwrap_or(true);
                let full = self.resolve(&path);

                if let Some(parent) = full.parent() {
                    if let Err(e) = std::fs::create_dir_all(parent) {
                        return ToolResult::err(ErrorType::Env, "mkdir_failed", &e.to_string());
                    }
                }
                if full.exists() && !overwrite {
                    return ToolResult::err(
                        ErrorType::Permission,
                        "file_exists_no_overwrite",
                        &full.display().to_string(),
                    );
                }
                if let Err(e) = std::fs::write(&full, content) {
                    return ToolResult::err(ErrorType::Env, "write_failed", &e.to_string());
                }

                let hash = Self::hash_bytes(content.as_bytes());
                let cp = json!({
                    "type": "filesystem_write",
                    "path": full.display().to_string(),
                    "hash": hash,
                    "bytes": content.len(),
                });
                ToolResult::ok(
                    json!({
                        "path": full.display().to_string(),
                        "bytes_written": content.len(),
                        "hash": hash,
                    }),
                    Some(cp),
                )
            }

            "read" => {
                let full = self.resolve(&path);
                match std::fs::read(&full) {
                    Ok(bytes) => {
                        let content = String::from_utf8_lossy(&bytes).to_string();
                        let hash = Self::hash_bytes(&bytes);
                        ToolResult::ok(
                            json!({
                                "path": full.display().to_string(),
                                "content": content,
                                "hash": hash,
                            }),
                            Some(json!({
                                "type": "filesystem_read",
                                "path": full.display().to_string(),
                                "hash": hash,
                            })),
                        )
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        ToolResult::err(ErrorType::Tool, "file_not_found", &full.display().to_string())
                    }
                    Err(e) => ToolResult::err(ErrorType::Env, "read_failed", &e.to_string()),
                }
            }

            "list" => {
                let full = self.resolve(&path);
                match std::fs::read_dir(&full) {
                    Ok(entries) => {
                        let files: Vec<String> = entries
                            .filter_map(|e| e.ok())
                            .map(|e| e.file_name().to_string_lossy().to_string())
                            .collect();
                        ToolResult::ok(
                            json!({ "path": full.display().to_string(), "entries": files }),
                            None,
                        )
                    }
                    Err(e) => ToolResult::err(ErrorType::Env, "list_failed", &e.to_string()),
                }
            }

            "delete" => {
                let full = self.resolve(&path);
                if !full.exists() {
                    return ToolResult::err(ErrorType::Tool, "file_not_found", &full.display().to_string());
                }
                let result = if full.is_dir() {
                    std::fs::remove_dir_all(&full)
                } else {
                    std::fs::remove_file(&full)
                };
                match result {
                    Ok(_) => ToolResult::ok(
                        json!({ "deleted": full.display().to_string() }),
                        Some(json!({ "type": "filesystem_delete", "path": full.display().to_string() })),
                    ),
                    Err(e) => ToolResult::err(ErrorType::Env, "delete_failed", &e.to_string()),
                }
            }

            _ => ToolResult::err(ErrorType::Tool, "unsupported_action", &action),
        }
    }
}
