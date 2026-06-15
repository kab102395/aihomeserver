//! Remote worker client and DTOs.
//!
//! The coordinator uses this module to route shell/browser requests to a separate execution
//! environment when `worker_url` is configured.

use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::state::ToolResult;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerShellRequest {
    pub command: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub collect_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerBrowserRequest {
    pub url: String,
    #[serde(default)]
    pub max_chars: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceFilePayload {
    pub path: String,
    pub contents_b64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceSyncRequest {
    #[serde(default)]
    pub prefix: Option<String>,
    pub files: Vec<WorkspaceFilePayload>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceArtifactPayload {
    pub path: String,
    pub contents_b64: String,
    pub size: u64,
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceSyncResponse {
    pub ok: bool,
    #[serde(default)]
    pub workspace: Option<String>,
    pub files_written: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerHealth {
    pub ok: bool,
    #[serde(default)]
    pub workspace: Option<String>,
}

#[derive(Clone)]
pub struct WorkerClient {
    base_url: String,
    token: Option<String>,
    client: reqwest::Client,
}

impl WorkerClient {
    pub fn new(base_url: impl Into<String>, token: impl Into<String>) -> Result<Self> {
        let base_url = base_url.into();
        let token = token.into();
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()?;
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            token: if token.trim().is_empty() {
                None
            } else {
                Some(token)
            },
            client,
        })
    }

    pub fn is_enabled(&self) -> bool {
        !self.base_url.is_empty()
    }

    async fn post_json<T: Serialize, R: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        body: &T,
    ) -> Result<R> {
        if !self.is_enabled() {
            return Err(anyhow!("worker not configured"));
        }

        let mut req = self
            .client
            .post(format!(
                "{}/{}",
                self.base_url,
                path.trim_start_matches('/')
            ))
            .header(CONTENT_TYPE, "application/json")
            .json(body);
        if let Some(token) = &self.token {
            req = req.header(AUTHORIZATION, format!("Bearer {token}"));
        }

        let resp = req.send().await?.error_for_status()?;
        Ok(resp.json::<R>().await?)
    }

    pub async fn health(&self) -> Result<WorkerHealth> {
        if !self.is_enabled() {
            return Err(anyhow!("worker not configured"));
        }
        let mut req = self.client.get(format!("{}/health", self.base_url));
        if let Some(token) = &self.token {
            req = req.header(AUTHORIZATION, format!("Bearer {token}"));
        }
        let resp = req.send().await?.error_for_status()?;
        Ok(resp.json::<WorkerHealth>().await?)
    }

    pub async fn shell(&self, request: &WorkerShellRequest) -> Result<ToolResult> {
        self.post_json("shell", request).await
    }

    pub async fn browser_fetch(&self, request: &WorkerBrowserRequest) -> Result<ToolResult> {
        self.post_json("browser/fetch", request).await
    }

    pub async fn sync_workspace(
        &self,
        request: &WorkspaceSyncRequest,
    ) -> Result<WorkspaceSyncResponse> {
        self.post_json("workspace/sync", request).await
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

/// Collect a workspace subtree for sync to the remote worker.
///
/// Files are encoded as base64 so text and binary artifacts travel through the same path.
pub fn collect_workspace_files(root: &Path) -> Result<Vec<WorkspaceFilePayload>> {
    fn walk(dir: &Path, base: &Path, out: &mut Vec<WorkspaceFilePayload>) -> Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if is_hidden_or_ignored(&path) {
                continue;
            }
            let meta = entry.metadata()?;
            if meta.is_dir() {
                walk(&path, base, out)?;
                continue;
            }
            if !meta.is_file() {
                continue;
            }
            let bytes = std::fs::read(&path)?;
            let rel = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            out.push(WorkspaceFilePayload {
                path: rel,
                contents_b64: STANDARD.encode(bytes),
            });
        }
        Ok(())
    }

    let mut out = Vec::new();
    if root.exists() {
        walk(root, root, &mut out)?;
    }
    Ok(out)
}

/// Write workspace artifacts returned by the worker back into the local workspace.
pub fn write_workspace_artifacts(
    root: &Path,
    artifacts: &[WorkspaceArtifactPayload],
) -> Result<()> {
    for artifact in artifacts {
        let rel = PathBuf::from(&artifact.path);
        if rel.is_absolute()
            || rel
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            continue;
        }
        let target = root.join(&rel);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = STANDARD
            .decode(&artifact.contents_b64)
            .map_err(|e| anyhow!("failed to decode artifact {}: {e}", artifact.path))?;
        std::fs::write(target, bytes)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!("aihomeserver-{name}-{unique}"))
    }

    #[test]
    fn collect_workspace_files_skips_hidden_and_build_dirs() {
        let root = temp_dir("collect");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::create_dir_all(root.join("target")).unwrap();
        std::fs::write(root.join("src/main.rs"), "fn main() {}").unwrap();
        std::fs::write(root.join(".git/config"), "[core]").unwrap();
        std::fs::write(root.join("target/out.bin"), b"bin").unwrap();

        let files = collect_workspace_files(&root).unwrap();
        let paths: Vec<String> = files.into_iter().map(|f| f.path).collect();
        assert_eq!(paths, vec!["src/main.rs".to_string()]);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn write_workspace_artifacts_round_trips_contents() {
        let root = temp_dir("artifacts");
        std::fs::create_dir_all(&root).unwrap();
        let artifacts = vec![WorkspaceArtifactPayload {
            path: "dist/app.txt".into(),
            contents_b64: STANDARD.encode("hello world"),
            size: 11,
            truncated: false,
        }];

        write_workspace_artifacts(&root, &artifacts).unwrap();
        let text = std::fs::read_to_string(root.join("dist/app.txt")).unwrap();
        assert_eq!(text, "hello world");

        let _ = std::fs::remove_dir_all(&root);
    }
}
