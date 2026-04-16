use async_trait::async_trait;
use serde_json::{json, Value};

use crate::state::{ErrorType, ToolResult};
use super::Tool;

pub struct WebSearchTool {
    client: reqwest::Client,
}

impl WebSearchTool {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .user_agent("Mozilla/5.0 (compatible; aihomeserver/0.1)")
                .build()
                .expect("HTTP client"),
        }
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str { "web_search" }

    async fn execute(&self, params: Value) -> ToolResult {
        let query = match params.get("query").and_then(|v| v.as_str()) {
            Some(q) => q.to_string(),
            None => return ToolResult::err(ErrorType::Tool, "missing_param", "query parameter required"),
        };

        let encoded: String = query.chars().map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            ' ' => "+".to_string(),
            _ => format!("%{:02X}", c as u32),
        }).collect();

        let url = format!("https://lite.duckduckgo.com/lite/?q={encoded}");

        let html = match self.client.get(&url)
            .header("Accept-Language", "en-US,en;q=0.9")
            .send().await
        {
            Ok(r) => match r.text().await {
                Ok(t) => t,
                Err(e) => return ToolResult::err(ErrorType::Tool, "read_error", &e.to_string()),
            },
            Err(e) => return ToolResult::err(ErrorType::Tool, "fetch_error", &e.to_string()),
        };

        let results = extract_results(&html);
        ToolResult::ok(json!({ "query": query, "results": results }), None)
    }
}

/// Extract search results from DuckDuckGo lite HTML.
/// Returns up to 5 results as [{title, url, snippet}].
fn extract_results(html: &str) -> Vec<serde_json::Value> {
    let mut results: Vec<serde_json::Value> = Vec::new();
    let mut pos = 0;

    while results.len() < 5 {
        // Find next result link
        let tag = "class=\"result-link\"";
        let Some(start) = html[pos..].find(tag) else { break };
        let abs = pos + start;

        // Look backward for the opening <a to get href
        let tag_open = html[..abs].rfind('<').unwrap_or(0);
        let tag_src_end = (abs + tag.len() + 100).min(html.len());
        let tag_src = &html[tag_open..tag_src_end];

        let href = extract_attr(tag_src, "href").unwrap_or_default();

        // Title = text between > and </a>
        let after_tag = &html[abs..];
        let title = after_tag.find('>').map(|i| {
            let rest = &after_tag[i + 1..];
            rest.find("</a>").map(|j| strip_tags(&rest[..j])).unwrap_or_default()
        }).unwrap_or_default();

        // Snippet follows in the next result-snippet td
        let snippet_tag = "class=\"result-snippet\"";
        let snippet = html[abs..].find(snippet_tag).map(|si| {
            let after = &html[abs + si..];
            after.find('>').map(|i| {
                let rest = &after[i + 1..];
                rest.find("</td>").map(|j| strip_tags(&rest[..j])).unwrap_or_default()
            }).unwrap_or_default()
        }).unwrap_or_default();

        pos = abs + tag.len();

        if !href.is_empty() && !title.trim().is_empty() {
            results.push(json!({
                "title": title.trim(),
                "url": href,
                "snippet": snippet.trim(),
            }));
        }
    }

    results
}

fn extract_attr(tag: &str, attr: &str) -> Option<String> {
    let needle = format!("{}=\"", attr);
    let start = tag.find(&needle)? + needle.len();
    let end = tag[start..].find('"')?;
    Some(tag[start..start + end].to_string())
}

fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    // Decode basic HTML entities
    out.replace("&amp;", "&")
       .replace("&lt;", "<")
       .replace("&gt;", ">")
       .replace("&quot;", "\"")
       .replace("&#39;", "'")
       .replace("&nbsp;", " ")
}
