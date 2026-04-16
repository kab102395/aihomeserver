use anyhow::{anyhow, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};

/// Talks to Ollama's /api/chat endpoint (OpenAI-style messages).
/// Ollama must be running on localhost:11434 with the models pulled.
#[derive(Debug, Clone)]
pub struct OllamaClient {
    client: Client,
    pub base_url: String,
    /// Fast model: planner, executor, repair (e.g. "qwen2.5:14b")
    pub fast_model: String,
    /// Deep critic model: high-risk validation only (e.g. "qwen2.5:32b")
    pub critic_model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: "system".into(), content: content.into() }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: "user".into(), content: content.into() }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: "assistant".into(), content: content.into() }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ModelRole {
    Fast,
    Critic,
}

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<Message>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    format: Option<String>,
}

#[derive(Deserialize)]
struct ChatResponse {
    message: Message,
}

impl OllamaClient {
    pub fn new(base_url: &str, fast_model: &str, critic_model: &str) -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(300))
                .build()
                .expect("Failed to build HTTP client"),
            base_url: base_url.to_string(),
            fast_model: fast_model.to_string(),
            critic_model: critic_model.to_string(),
        }
    }

    pub fn model_name(&self, role: ModelRole) -> &str {
        match role {
            ModelRole::Fast => &self.fast_model,
            ModelRole::Critic => &self.critic_model,
        }
    }

    /// Send a chat request. Set `json_mode = true` to instruct Ollama to
    /// return a JSON object (uses Ollama's `format: "json"` field).
    pub async fn chat(
        &self,
        messages: Vec<Message>,
        role: ModelRole,
        json_mode: bool,
    ) -> Result<String> {
        let model = self.model_name(role).to_string();
        tracing::debug!("LLM call: model={model} json={json_mode} msgs={}", messages.len());

        let req = ChatRequest {
            model,
            messages,
            stream: false,
            format: if json_mode { Some("json".to_string()) } else { None },
        };

        let resp = self
            .client
            .post(format!("{}/api/chat", self.base_url))
            .json(&req)
            .send()
            .await?
            .error_for_status()?
            .json::<ChatResponse>()
            .await?;

        Ok(resp.message.content)
    }

    /// Chat and parse the response as JSON into type T.
    /// Automatically sets json_mode = true.
    pub async fn complete_json<T>(&self, messages: Vec<Message>, role: ModelRole) -> Result<T>
    where
        T: serde::de::DeserializeOwned,
    {
        let raw = self.chat(messages, role, true).await?;
        serde_json::from_str(&raw)
            .map_err(|e| anyhow!("JSON parse error: {e}\nRaw response:\n{raw}"))
    }
}
