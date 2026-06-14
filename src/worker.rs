//! Remote worker client and DTOs.
//!
//! The coordinator uses this module to route shell/browser requests to a separate execution
//! environment when `worker_url` is configured.

use anyhow::{anyhow, Result};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};

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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerBrowserRequest {
    pub url: String,
    #[serde(default)]
    pub max_chars: Option<usize>,
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
            token: if token.trim().is_empty() { None } else { Some(token) },
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
            .post(format!("{}/{}", self.base_url, path.trim_start_matches('/')))
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
}
