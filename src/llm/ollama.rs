use anyhow::{anyhow, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::config::ServerConfig;

/// Talks to Ollama's /api/chat endpoint (OpenAI-style messages).
/// Model names and the base URL are read from the shared ServerConfig on every
/// call so that changes via POST /settings take effect immediately.
#[derive(Debug, Clone)]
pub struct OllamaClient {
    client: Client,
    pub config: Arc<RwLock<ServerConfig>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
    /// Populated in qwen3 thinking-mode responses — the chain-of-thought trace.
    /// Always None when sending; only present in model replies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: "system".into(), content: content.into(), thinking: None }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: "user".into(), content: content.into(), thinking: None }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: "assistant".into(), content: content.into(), thinking: None }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ModelRole {
    Fast,
    Critic,
}

#[derive(Serialize)]
struct OllamaOptions {
    /// GPU layers — 999 means put everything on the GPU, nothing on CPU.
    num_gpu: i32,
    /// Context window in tokens.
    num_ctx: u32,
    /// Batch size: how many tokens Ollama processes in parallel during prompt eval.
    /// Higher = GPU works harder during the "thinking" phase before streaming starts.
    /// 512 is a solid default; can go to 1024+ with enough VRAM.
    num_batch: u32,
    /// CPU threads — 0 lets Ollama auto-detect.
    #[serde(skip_serializing_if = "Option::is_none")]
    num_thread: Option<u32>,
}

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<Message>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    format: Option<String>,
    options: OllamaOptions,
    /// Keep the model loaded in VRAM for this many seconds after the request.
    /// -1 = keep forever (until Ollama is restarted or memory is needed).
    /// Without this, Ollama may unload between requests and waste time reloading.
    keep_alive: i32,
    /// Enable qwen3 chain-of-thought thinking mode.
    /// When true the model reasons silently before producing its answer.
    /// Only set when we actually want thinking — omit entirely otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    think: Option<bool>,
}

#[derive(Deserialize)]
struct ChatResponse {
    message: Message,
}

#[derive(Deserialize)]
struct StreamChunk {
    message: Message,
    done: bool,
}

impl OllamaClient {
    pub fn new(config: Arc<RwLock<ServerConfig>>) -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(300))
                .build()
                .expect("Failed to build HTTP client"),
            config,
        }
    }

    async fn model_for(&self, role: ModelRole) -> String {
        let cfg = self.config.read().await;
        match role {
            ModelRole::Fast   => cfg.fast_model.clone(),
            ModelRole::Critic => cfg.critic_model.clone(),
        }
    }

    async fn base_url(&self) -> String {
        self.config.read().await.ollama_url.clone()
    }

    async fn options(&self) -> OllamaOptions {
        let cfg = self.config.read().await;
        OllamaOptions {
            num_gpu:    cfg.num_gpu,
            num_ctx:    cfg.num_ctx,
            num_batch:  cfg.num_batch,
            num_thread: if cfg.num_thread == 0 { None } else { Some(cfg.num_thread) },
        }
    }

    /// Send a chat request.
    /// `json_mode = true` → Ollama returns a JSON object (`format: "json"`).
    /// `think = true`     → qwen3 chain-of-thought reasoning before answering.
    pub async fn chat(
        &self,
        messages: Vec<Message>,
        role: ModelRole,
        json_mode: bool,
        think: bool,
    ) -> Result<String> {
        let model    = self.model_for(role).await;
        let base_url = self.base_url().await;
        tracing::debug!("LLM call: model={model} json={json_mode} think={think} msgs={}", messages.len());

        let options  = self.options().await;
        let req = ChatRequest {
            model,
            messages,
            stream: false,
            format: if json_mode { Some("json".to_string()) } else { None },
            options,
            keep_alive: -1,
            think: if think { Some(true) } else { None },
        };

        let resp = self
            .client
            .post(format!("{base_url}/api/chat"))
            .json(&req)
            .send()
            .await?
            .error_for_status()?
            .json::<ChatResponse>()
            .await?;

        Ok(resp.message.content)
    }

    /// Stream tokens from Ollama. Sends each content token to `token_tx`.
    /// When `think = true` (qwen3 thinking mode), chain-of-thought tokens are
    /// sent to `thinking_tx` if provided — kept separate so the UI can render
    /// them as a collapsible block rather than mixing them into the answer.
    /// Returns the full accumulated answer text when done.
    pub async fn chat_stream(
        &self,
        messages: Vec<Message>,
        role: ModelRole,
        token_tx: &tokio::sync::mpsc::UnboundedSender<String>,
        think: bool,
        thinking_tx: Option<&tokio::sync::mpsc::UnboundedSender<String>>,
    ) -> Result<String> {
        use futures::StreamExt;

        let model    = self.model_for(role).await;
        let base_url = self.base_url().await;
        let options  = self.options().await;
        let req = ChatRequest {
            model,
            messages,
            stream: true,
            format: None,
            options,
            keep_alive: -1,
            think: if think { Some(true) } else { None },
        };

        let response = self.client
            .post(format!("{base_url}/api/chat"))
            .json(&req)
            .send().await?
            .error_for_status()?;

        let mut byte_stream = response.bytes_stream();
        let mut line_buf    = String::new();
        let mut accumulated = String::new();

        while let Some(chunk) = byte_stream.next().await {
            let chunk = chunk?;
            line_buf.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(pos) = line_buf.find('\n') {
                let line = line_buf[..pos].trim().to_string();
                line_buf = line_buf[pos + 1..].to_string();
                if line.is_empty() { continue; }
                if let Ok(parsed) = serde_json::from_str::<StreamChunk>(&line) {
                    // Thinking tokens (qwen3 CoT) — forward to thinking channel if present
                    if let Some(thinking) = &parsed.message.thinking {
                        if !thinking.is_empty() {
                            if let Some(tx) = thinking_tx {
                                let _ = tx.send(thinking.clone());
                            }
                        }
                    }
                    // Regular answer tokens
                    let token = parsed.message.content.clone();
                    if !token.is_empty() {
                        accumulated.push_str(&token);
                        let _ = token_tx.send(token);
                    }
                    if parsed.done { break; }
                }
            }
        }
        Ok(accumulated)
    }

    /// Chat and parse the response as JSON into type T.
    /// `think = true` enables qwen3 chain-of-thought before producing JSON —
    /// use this for complex decisions (e.g. planning) where reasoning quality matters.
    /// The thinking trace is discarded; only the clean JSON content is returned.
    pub async fn complete_json<T>(&self, messages: Vec<Message>, role: ModelRole, think: bool) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        let raw = self.chat(messages, role, true, think).await?;
        serde_json::from_str(&raw)
            .map_err(|e| anyhow!("JSON parse error: {e}\nRaw response:\n{raw}"))
    }

    /// Generate a text embedding using nomic-embed-text.
    /// Returns a 768-dimensional vector. Fails gracefully — callers should
    /// log and continue rather than hard-failing if embeddings are unavailable.
    pub async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        #[derive(Serialize)]
        struct EmbedRequest<'a> {
            model: &'static str,
            prompt: &'a str,
        }
        #[derive(Deserialize)]
        struct EmbedResponse {
            embedding: Vec<f32>,
        }

        let base_url = self.base_url().await;
        let resp = self
            .client
            .post(format!("{base_url}/api/embeddings"))
            .json(&EmbedRequest { model: "nomic-embed-text", prompt: text })
            .send()
            .await?
            .error_for_status()?
            .json::<EmbedResponse>()
            .await?;

        Ok(resp.embedding)
    }
}
