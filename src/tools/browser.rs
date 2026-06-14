use async_trait::async_trait;
use reqwest::Client;
use scraper::{Html, Selector};
use serde_json::{json, Value};

use crate::{
    state::{ErrorType, ToolResult},
    worker::{WorkerBrowserRequest, WorkerClient},
};

use super::Tool;

fn strip_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn extract_dom(html: &str) -> (String, Vec<String>, String) {
    let doc = Html::parse_document(html);

    let title = Selector::parse("title")
        .ok()
        .and_then(|sel| doc.select(&sel).next())
        .map(|n| strip_whitespace(&n.text().collect::<Vec<_>>().join(" ")))
        .unwrap_or_default();

    let body_text = Selector::parse("body")
        .ok()
        .and_then(|sel| doc.select(&sel).next())
        .map(|n| strip_whitespace(&n.text().collect::<Vec<_>>().join(" ")))
        .unwrap_or_else(|| strip_whitespace(html));

    let mut links = Vec::new();
    if let Ok(sel) = Selector::parse("a[href]") {
        for node in doc.select(&sel).take(40) {
            if let Some(href) = node.value().attr("href") {
                let label = strip_whitespace(&node.text().collect::<Vec<_>>().join(" "));
                links.push(if label.is_empty() {
                    href.to_string()
                } else {
                    format!("{label} -> {href}")
                });
            }
        }
    }

    (title, links, body_text)
}

#[derive(Clone)]
pub struct BrowserTool {
    client: Client,
    worker: Option<WorkerClient>,
}

impl BrowserTool {
    pub fn new(worker: Option<WorkerClient>) -> Self {
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36")
                .build()
                .expect("browser client"),
            worker,
        }
    }

    async fn local_fetch(&self, url: &str, max_chars: usize) -> ToolResult {
        let resp = match self.client.get(url).send().await {
            Ok(r) => r,
            Err(e) => return ToolResult::err(ErrorType::Tool, "browser_fetch_error", &e.to_string()),
        };

        let final_url = resp.url().to_string();
        let status = resp.status().as_u16();
        let html = match resp.text().await {
            Ok(t) => t,
            Err(e) => return ToolResult::err(ErrorType::Tool, "browser_read_error", &e.to_string()),
        };

        let (title, links, body_text) = extract_dom(&html);
        let text = if body_text.chars().count() > max_chars {
            body_text.chars().take(max_chars).collect::<String>()
        } else {
            body_text
        };

        ToolResult::ok(
            json!({
                "url": url,
                "final_url": final_url,
                "status": status,
                "title": title,
                "text": text,
                "links": links,
                "mode": "local_fetch"
            }),
            Some(json!({ "type": "browser_fetch", "url": url, "status": status })),
        )
    }
}

#[async_trait]
impl Tool for BrowserTool {
    fn name(&self) -> &str {
        "browser"
    }

    async fn execute(&self, params: Value) -> ToolResult {
        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("fetch");
        let url = params.get("url").and_then(|v| v.as_str()).unwrap_or("").trim();
        if url.is_empty() {
            return ToolResult::err(ErrorType::Tool, "missing_url", "params.url is required");
        }
        let max_chars = params
            .get("max_chars")
            .and_then(|v| v.as_u64())
            .unwrap_or(12000) as usize;

        match action {
            "fetch" | "open" | "visit" => {
                if let Some(worker) = &self.worker {
                    if worker.is_enabled() {
                        if let Ok(r) = worker
                            .browser_fetch(&WorkerBrowserRequest {
                                url: url.to_string(),
                                max_chars: Some(max_chars),
                            })
                            .await
                        {
                            return r;
                        }
                    }
                }
                self.local_fetch(url, max_chars).await
            }
            _ => ToolResult::err(ErrorType::Tool, "unsupported_browser_action", action),
        }
    }
}
