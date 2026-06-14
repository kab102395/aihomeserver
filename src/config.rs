//! Runtime configuration.
//!
//! `ServerConfig` is persisted to `config.json` and can be updated at runtime via `POST /settings`.
//! The config is stored behind an `Arc<RwLock<...>>` so handlers and background runs can read it
//! concurrently while updates remain atomic.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Persisted server configuration.
/// Saved as `config.json` next to the binary and reloaded at startup.
/// Changes via POST /settings take effect immediately without a restart.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Absolute (or relative) path used as the default working directory for
    /// all shell commands and as the root for the filesystem tool.
    pub workspace_path: String,
    /// Ollama base URL (default: http://localhost:11434)
    pub ollama_url: String,
    /// Fast model — planner, executor, repair, plan-questions
    pub fast_model: String,
    /// Critic model — high-risk task validation
    pub critic_model: String,
    /// Hard cap on orchestrator steps per task (1–50)
    pub max_steps: usize,
    /// Risk score (0–10) at which the human-approval gate triggers.
    /// Default 8: only truly destructive tasks require approval.
    pub risk_gate_threshold: u8,
    /// Optional custom search engine URL (e.g. SearXNG JSON endpoint).
    /// When set, overrides the built-in DuckDuckGo scraping.
    /// Example: http://searxng:8080  or  http://localhost:8888
    /// Leave empty to use DDG scraping (default).
    #[serde(default)]
    pub search_url: String,

    /// Optional remote worker base URL. When set, shell/browser execution routes to it.
    #[serde(default)]
    pub worker_url: String,

    /// Shared bearer token used by the coordinator when calling the worker.
    #[serde(default)]
    pub worker_token: String,

    /// Optional API key for protecting the public HTTP API.
    #[serde(default)]
    pub api_key: String,

    /// Execution preference: "auto" (default), "remote", or "local".
    #[serde(default = "default_execution_mode")]
    pub execution_mode: String,

    /// Auto-save assistant outputs into the Knowledge Base.
    /// Values:
    /// - "off" (default)
    /// - "research" (only when the plan uses research tools)
    /// - "always" (for most substantial answers)
    #[serde(default = "default_auto_kb_mode")]
    pub auto_kb_mode: String,

    /// Minimum character count before auto-saving to KB (when auto_kb_mode != "off").
    #[serde(default = "default_auto_kb_min_chars")]
    pub auto_kb_min_chars: u32,

    /// GPU layers to offload to VRAM. 999 = all layers (max GPU use). 0 = CPU only.
    #[serde(default = "default_num_gpu")]
    pub num_gpu: i32,

    /// Context window in tokens. Larger = more research data fits in one call.
    /// 8192 is a good default; 16384 for deep research on large models.
    #[serde(default = "default_num_ctx")]
    pub num_ctx: u32,

    /// Max tokens the model is allowed to generate for a single response.
    /// Ollama option: `num_predict`.
    /// If outputs feel "shallow"/cut off, increase this.
    #[serde(default = "default_num_predict")]
    pub num_predict: u32,

    /// Batch size: tokens processed in parallel during prompt eval.
    /// Higher = GPU works harder before streaming starts. 512 is a good default.
    #[serde(default = "default_num_batch")]
    pub num_batch: u32,

    /// CPU inference threads. 0 = Ollama auto-selects (recommended).
    #[serde(default)]
    pub num_thread: u32,
}

/// Default: offload all layers to GPU when possible.
fn default_num_gpu() -> i32 {
    999
}
/// Default context window used for LLM calls.
fn default_num_ctx() -> u32 {
    8192
}
/// Default generation token cap for LLM calls.
fn default_num_predict() -> u32 {
    2048
}
/// Default batch size used by Ollama during prompt evaluation.
fn default_num_batch() -> u32 {
    1024
}
fn default_auto_kb_mode() -> String {
    // Fast + high signal default: only save when the run already used research tools.
    "research".into()
}
fn default_auto_kb_min_chars() -> u32 {
    1800
}
fn default_execution_mode() -> String {
    "auto".into()
}

impl Default for ServerConfig {
    /// Default config values for a local developer deployment.
    fn default() -> Self {
        // Resolve workspace relative to the binary's directory so it's
        // predictable regardless of where the binary is invoked from.
        let workspace = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("workspace")))
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "./workspace".into());

        Self {
            workspace_path: workspace,
            ollama_url: "http://localhost:11434".into(),
            fast_model: "qwen3:14b".into(),
            critic_model: "qwen3:14b".into(),
            max_steps: 20,
            risk_gate_threshold: 8,
            search_url: String::new(),
            worker_url: String::new(),
            worker_token: String::new(),
            api_key: String::new(),
            execution_mode: default_execution_mode(),
            auto_kb_mode: default_auto_kb_mode(),
            auto_kb_min_chars: default_auto_kb_min_chars(),
            num_gpu: 999,
            num_ctx: 8192,
            num_predict: 2048,
            num_batch: 1024,
            num_thread: 0,
        }
    }
}

impl ServerConfig {
    /// Load from `path`. Returns `Default` if the file doesn't exist yet.
    pub async fn load(path: &Path) -> Self {
        match tokio::fs::read_to_string(path).await {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                tracing::warn!("config.json parse error ({e}), using defaults");
                Self::default()
            }),
            Err(_) => {
                tracing::info!("No config.json found, using defaults");
                Self::default()
            }
        }
    }

    /// Persist to `path`, creating the file if needed.
    pub async fn save(&self, path: &Path) -> Result<()> {
        let text = serde_json::to_string_pretty(self)?;
        tokio::fs::write(path, text).await?;
        Ok(())
    }

    /// Ensure the workspace directory exists on disk.
    pub fn ensure_workspace(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.workspace_path)
    }
}
