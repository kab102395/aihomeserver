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
                .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
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

        let url = format!("https://www.bing.com/search?q={encoded}&setlang=en");

        let html = match self.client.get(&url)
            .header("Accept-Language", "en-US,en;q=0.9")
            .header("Accept", "text/html,application/xhtml+xml")
            .send().await
        {
            Ok(r) => match r.text().await {
                Ok(t) => t,
                Err(e) => return ToolResult::err(ErrorType::Tool, "read_error", &e.to_string()),
            },
            Err(e) => return ToolResult::err(ErrorType::Tool, "fetch_error", &e.to_string()),
        };

        let results = extract_bing_results(&html);
        if results.is_empty() {
            return ToolResult::err(ErrorType::Tool, "no_results", "No search results found");
        }
        ToolResult::ok(json!({ "query": query, "results": results }), None)
    }
}

/// Extract up to 5 results from Bing's HTML.
/// Each result is in <li class="b_algo">, title in <h2><a href="...">,
/// snippet in <div class="b_caption"><p>...</p></div>.
fn extract_bing_results(html: &str) -> Vec<serde_json::Value> {
    let mut results = Vec::new();
    let mut pos = 0;

    while results.len() < 5 {
        // Find next result block
        let marker = "class=\"b_algo\"";
        let Some(rel) = html[pos..].find(marker) else { break };
        let block_start = pos + rel;

        // Find end of this block (start of next b_algo or end of results)
        let block_end = html[block_start + marker.len()..]
            .find("class=\"b_algo\"")
            .map(|r| block_start + marker.len() + r)
            .unwrap_or(html.len().min(block_start + 4000));

        let block = &html[block_start..block_end];
        pos = block_start + marker.len();

        // Extract href and title from <h2><a href="...">TITLE</a></h2>
        let (href, title) = extract_h2_link(block);
        if href.is_empty() || title.is_empty() { continue; }

        // Skip Bing's own internal links
        if href.starts_with("https://www.bing.com") { continue; }

        // Extract snippet from b_caption <p>
        let snippet = extract_caption(block);

        results.push(json!({
            "title": title.trim(),
            "url": href,
            "snippet": snippet.trim(),
        }));
    }

    results
}

fn extract_h2_link(block: &str) -> (String, String) {
    let Some(h2_pos) = block.find("<h2>") else { return (String::new(), String::new()) };
    let h2_block = &block[h2_pos..];
    let Some(a_pos) = h2_block.find("<a ") else { return (String::new(), String::new()) };
    let a_block = &h2_block[a_pos..];

    let href = extract_attr(a_block, "href").unwrap_or_default();
    let title = a_block.find('>').map(|i| {
        let rest = &a_block[i + 1..];
        rest.find("</a>").map(|j| strip_tags(&rest[..j])).unwrap_or_default()
    }).unwrap_or_default();

    (href, title)
}

fn extract_caption(block: &str) -> String {
    let Some(cap_pos) = block.find("b_caption") else { return String::new() };
    let after = &block[cap_pos..];
    let Some(p_pos) = after.find("<p") else { return String::new() };
    let p_block = &after[p_pos..];
    p_block.find('>').map(|i| {
        let rest = &p_block[i + 1..];
        rest.find("</p>").map(|j| strip_tags(&rest[..j])).unwrap_or_default()
    }).unwrap_or_default()
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
    out.replace("&amp;", "&")
       .replace("&lt;", "<")
       .replace("&gt;", ">")
       .replace("&quot;", "\"")
       .replace("&#39;", "'")
       .replace("&nbsp;", " ")
       .split_whitespace()
       .collect::<Vec<_>>()
       .join(" ")
}
