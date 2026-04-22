use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::{config::ServerConfig, state::{ErrorType, ToolResult}};
use super::Tool;
use super::web_search::WebSearchTool;

/// Runs multiple web search queries concurrently and returns all results merged.
/// This is far faster than sequential searches for research tasks —
/// 4 queries in parallel takes the same wall-clock time as 1.
pub struct ParallelSearchTool {
    inner: WebSearchTool,
}

impl ParallelSearchTool {
    pub fn new(config: Arc<RwLock<ServerConfig>>) -> Self {
        Self { inner: WebSearchTool::new(config) }
    }
}

#[async_trait]
impl Tool for ParallelSearchTool {
    fn name(&self) -> &str { "parallel_search" }

    async fn execute(&self, params: Value) -> ToolResult {
        // Accept either { "queries": ["q1","q2",...] } or { "query": "single" }
        let queries: Vec<String> = if let Some(arr) = params.get("queries").and_then(|v| v.as_array()) {
            arr.iter()
                .filter_map(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.to_string())
                .collect()
        } else if let Some(q) = params.get("query").and_then(|v| v.as_str()) {
            vec![q.to_string()]
        } else {
            return ToolResult::err(ErrorType::Tool, "missing_param", "queries array required");
        };

        if queries.is_empty() {
            return ToolResult::err(ErrorType::Tool, "missing_param", "queries array is empty");
        }

        // Cap at 6 concurrent queries to avoid hammering search engines
        let queries: Vec<String> = queries.into_iter().take(6).collect();

        // Fire all queries concurrently
        let handles: Vec<_> = queries.iter().map(|q| {
            let q = q.clone();
            let inner = WebSearchTool::new_cloned_config(self.inner.config());
            tokio::spawn(async move {
                let result = inner.execute(json!({ "query": q })).await;
                (q, result)
            })
        }).collect();

        let mut all_results: Vec<Value> = Vec::new();
        let mut successful_queries = 0usize;
        let mut failed_queries = 0usize;

        for handle in handles {
            match handle.await {
                Ok((query, tool_result)) => {
                    if tool_result.success {
                        successful_queries += 1;
                        if let Some(output) = &tool_result.output {
                            if let Some(results) = output.get("results").and_then(|r| r.as_array()) {
                                for r in results {
                                    // Tag each result with which query found it
                                    let mut entry = r.clone();
                                    if let Some(obj) = entry.as_object_mut() {
                                        obj.insert("from_query".to_string(), json!(query));
                                    }
                                    all_results.push(entry);
                                }
                            }
                        }
                    } else {
                        failed_queries += 1;
                    }
                }
                Err(_) => { failed_queries += 1; }
            }
        }

        if all_results.is_empty() {
            return ToolResult::err(
                ErrorType::Tool,
                "no_results",
                &format!("All {failed_queries} search queries returned no results"),
            );
        }

        // Deduplicate by URL — same page shouldn't appear multiple times
        let mut seen_urls = std::collections::HashSet::new();
        let deduped: Vec<Value> = all_results.into_iter().filter(|r| {
            let url = r.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string();
            seen_urls.insert(url)
        }).collect();

        let total = deduped.len();
        ToolResult::ok(json!({
            "results": deduped,
            "total": total,
            "successful_queries": successful_queries,
            "failed_queries": failed_queries,
        }), None)
    }
}
