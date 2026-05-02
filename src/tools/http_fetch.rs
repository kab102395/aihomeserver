use async_trait::async_trait;
use reqwest::{header, Client};
use serde_json::{json, Value};

use crate::memory::sources::SourceCacheStore;
use crate::state::{ErrorType, ToolResult};

use super::Tool;

/// Rewrite URLs to more scrapable equivalents.
/// - new Reddit -> old Reddit (the new design is JS-heavy and often returns near-empty HTML)
///
/// Connection:
/// - Used by `HttpFetchTool` before fetching so we get content suitable for LLM parsing.
fn rewrite_url(url: &str) -> String {
    if url.contains("://www.reddit.com") {
        return url.replacen("://www.reddit.com", "://old.reddit.com", 1);
    }
    if url.contains("://reddit.com") {
        return url.replacen("://reddit.com", "://old.reddit.com", 1);
    }
    url.to_string()
}

/// When a page returns 403/429, fall back to an old.reddit.com search using
/// keywords extracted from the URL path as a best-effort alternative.
///
/// Connection:
/// - This reduces “hard failures” in grounded research flows where one URL is blocked.
fn reddit_fallback_url(blocked_url: &str) -> String {
    // Extract path segments as search keywords
    let path = blocked_url
        .split("://")
        .nth(1)
        .unwrap_or(blocked_url)
        .split('/')
        .filter(|s| !s.is_empty() && s.len() > 3)
        .take(4)
        .collect::<Vec<_>>()
        .join(" ");

    // Very small URL encoding (good enough for keywords)
    let query: String = path
        .chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' => c.to_string(),
            ' ' => "+".to_string(),
            _ => String::new(),
        })
        .collect();

    format!("https://old.reddit.com/search?q={query}&sort=top&t=year")
}

/// Strip HTML tags and collapse whitespace so the LLM receives clean readable text.
///
/// Connection:
/// - The grounding contract stores fetched page text as artifacts; stripping makes the
///   text smaller and easier for the LLM/critic to use.
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
                // Add spacing around some block elements
                if [
                    "p", "div", "br", "li", "h1", "h2", "h3", "h4", "h5", "h6", "tr", "td", "th",
                ]
                .iter()
                .any(|t| tag.starts_with(t))
                {
                    out.push('\n');
                }
            }
            _ if in_tag => tag_buf.push(c),
            _ if in_script => {}
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
            if !last_nl {
                result.push('\n');
            }
            last_nl = true;
        } else {
            result.push_str(trimmed);
            result.push('\n');
            last_nl = false;
        }
    }
    result
}

/// HTTP fetch tool used for grounded research.
///
/// Connection:
/// - Typically follows a `web_search`/`parallel_search` step to fetch full page text.
/// - Optionally writes metadata into `SourceCacheStore` for provenance and reuse.
pub struct HttpFetchTool {
    client: Client,
    cache: Option<std::sync::Arc<SourceCacheStore>>,
}

impl HttpFetchTool {
    /// Create a new HTTP fetch tool with an optional source cache.
    pub fn new(cache: Option<std::sync::Arc<SourceCacheStore>>) -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36")
                .gzip(true)
                .build()
                .expect("HTTP client"),
            cache,
        }
    }
}

#[async_trait]
impl Tool for HttpFetchTool {
    /// Canonical tool name used in planner/executor `tool_binding`.
    fn name(&self) -> &str {
        "http_fetch"
    }

    /// Fetch a URL and return cleaned text + metadata as a `ToolResult`.
    async fn execute(&self, params: Value) -> ToolResult {
        let raw_url = match params.get("url").and_then(|v| v.as_str()) {
            Some(u) if !u.trim().is_empty() && u.starts_with("http") => u.to_string(),
            Some(u) => {
                return ToolResult::err(
                    ErrorType::Tool,
                    "invalid_url",
                    &format!(
                        "http_fetch received invalid URL '{u}' — search step likely failed to find any URLs"
                    ),
                )
            }
            None => return ToolResult::err(ErrorType::Tool, "missing_param", "url parameter required"),
        };

        let allow_reddit_fallback = params
            .get("allow_reddit_fallback")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let max_chars: usize = params
            .get("max_chars")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(8000);

        let rewritten_url = rewrite_url(&raw_url);
        let rewritten = rewritten_url != raw_url;

        let is_reddit_url = rewritten_url.contains("://old.reddit.com")
            || rewritten_url.contains("://www.reddit.com")
            || rewritten_url.contains("://reddit.com");

        let mut fallback_used = false;
        let mut fallback_url: Option<String> = None;

        let mut resp = match self.client.get(&rewritten_url).send().await {
            Ok(r) => r,
            Err(e) => return ToolResult::err(ErrorType::Tool, "fetch_error", &e.to_string()),
        };

        // If we're blocked by bot protection and fallback is allowed, attempt a Reddit search page.
        if (resp.status().as_u16() == 403 || resp.status().as_u16() == 429)
            && allow_reddit_fallback
            && !is_reddit_url
        {
            let fb = reddit_fallback_url(&rewritten_url);
            if let Ok(r2) = self.client.get(&fb).send().await {
                fallback_used = true;
                fallback_url = Some(fb);
                resp = r2;
            }
        }

        let status = resp.status().as_u16();
        let final_url = resp.url().to_string();
        let content_type = resp
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        match resp.text().await {
            Ok(body) => {
                let text = strip_html(&body);

                let mut out = String::with_capacity(std::cmp::min(text.len(), max_chars + 32));
                let mut truncated = false;
                for (i, c) in text.chars().enumerate() {
                    if i >= max_chars {
                        truncated = true;
                        break;
                    }
                    out.push(c);
                }
                if truncated {
                    out.push_str("… [truncated]");
                }

                // Persist fetch metadata for cross-run de-dupe/relevance. Best-effort only.
                if let Some(cache) = &self.cache {
                    let _ = cache
                        .record_fetch(&final_url, status, content_type.as_deref(), &out)
                        .await;
                }

                ToolResult::ok(
                    json!({
                        "requested_url": raw_url,
                        "rewritten_url": rewritten_url,
                        "rewritten": rewritten,
                        "allow_reddit_fallback": allow_reddit_fallback,
                        "fallback_used": fallback_used,
                        "fallback_url": fallback_url,
                        "url": final_url,
                        "status": status,
                        "content_type": content_type,
                        "max_chars": max_chars,
                        "body": out
                    }),
                    None,
                )
            }
            Err(e) => ToolResult::err(ErrorType::Tool, "read_error", &e.to_string()),
        }
    }
}
