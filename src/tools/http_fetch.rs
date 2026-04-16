use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};
use crate::state::{ErrorType, ToolResult};
use super::Tool;

pub struct HttpFetchTool {
    client: Client,
}

impl HttpFetchTool {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .user_agent("aihomeserver/0.1")
                .build()
                .expect("HTTP client"),
        }
    }
}

#[async_trait]
impl Tool for HttpFetchTool {
    fn name(&self) -> &str { "http_fetch" }

    async fn execute(&self, params: Value) -> ToolResult {
        let url = match params.get("url").and_then(|v| v.as_str()) {
            Some(u) => u.to_string(),
            None => return ToolResult::err(ErrorType::Tool, "missing_param", "url parameter required"),
        };

        match self.client.get(&url).send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                match resp.text().await {
                    Ok(body) => {
                        // Truncate to 8000 chars to keep prompts reasonable
                        let truncated = if body.len() > 8000 {
                            format!("{}... [truncated {} chars]", &body[..8000], body.len() - 8000)
                        } else {
                            body
                        };
                        ToolResult::ok(json!({ "status": status, "body": truncated, "url": url }), None)
                    }
                    Err(e) => ToolResult::err(ErrorType::Tool, "read_error", &e.to_string()),
                }
            }
            Err(e) => ToolResult::err(ErrorType::Tool, "fetch_error", &e.to_string()),
        }
    }
}
