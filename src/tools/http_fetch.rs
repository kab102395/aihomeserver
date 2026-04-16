use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};
use crate::state::{ErrorType, ToolResult};
use super::Tool;

/// Strip HTML tags and collapse whitespace so the LLM receives clean readable text.
fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut in_script = false;
    let mut tag_buf = String::new();

    let mut chars = html.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '<' => {
                in_tag = true;
                tag_buf.clear();
            }
            '>' if in_tag => {
                let tag = tag_buf.trim().to_lowercase();
                // Skip script/style content entirely
                if tag.starts_with("script") || tag.starts_with("style") {
                    in_script = true;
                } else if tag.starts_with("/script") || tag.starts_with("/style") {
                    in_script = false;
                }
                in_tag = false;
                // Add spacing around block elements
                if ["p","div","br","li","h1","h2","h3","h4","h5","h6","tr","td","th"].iter()
                    .any(|t| tag.starts_with(t))
                {
                    out.push('\n');
                }
            }
            _ if in_tag => tag_buf.push(c),
            _ if in_script => {} // skip script/style text
            _ => out.push(c),
        }
    }

    // Decode common HTML entities
    let out = out
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ");

    // Collapse runs of whitespace/newlines
    let mut result = String::with_capacity(out.len());
    let mut last_nl = false;
    for line in out.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if !last_nl { result.push('\n'); }
            last_nl = true;
        } else {
            result.push_str(trimmed);
            result.push('\n');
            last_nl = false;
        }
    }
    result
}

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
                        // Strip HTML tags → readable text for the LLM
                        let text = strip_html(&body);
                        // Truncate to 6000 chars to keep prompts reasonable
                        let truncated = if text.chars().count() > 6000 {
                            let cut: String = text.chars().take(6000).collect();
                            format!("{cut}… [truncated]")
                        } else {
                            text
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
