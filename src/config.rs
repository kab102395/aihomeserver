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

    /// GPU layers to offload to VRAM. 999 = all layers (max GPU use). 0 = CPU only.
    #[serde(default = "default_num_gpu")]
    pub num_gpu: i32,

    /// Context window in tokens. Larger = more research data fits in one call.
    /// 8192 is a good default; 16384 for deep research on large models.
    #[serde(default = "default_num_ctx")]
    pub num_ctx: u32,

    /// Batch size: tokens processed in parallel during prompt eval.
    /// Higher = GPU works harder before streaming starts. 512 is a good default.
    #[serde(default = "default_num_batch")]
    pub num_batch: u32,

    /// CPU inference threads. 0 = Ollama auto-selects (recommended).
    #[serde(default)]
    pub num_thread: u32,
}

fn default_num_gpu() -> i32 { 999 }
fn default_num_ctx() -> u32 { 8192 }
fn default_num_batch() -> u32 { 1024 }

impl Default for ServerConfig {
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
            num_gpu: 999,
            num_ctx: 8192,
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
