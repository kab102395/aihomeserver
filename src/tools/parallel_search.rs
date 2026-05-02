use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{sleep, Duration};

use super::web_search::WebSearchTool;
use super::Tool;
use crate::{
    config::ServerConfig,
    state::{ErrorType, ToolResult},
};

/// Runs multiple web search queries concurrently and returns all results merged.
/// This is far faster than sequential searches for research tasks —
/// 4 queries in parallel takes the same wall-clock time as 1.
pub struct ParallelSearchTool {
    inner: WebSearchTool,
}

impl ParallelSearchTool {
    /// Create a parallel search tool using the shared server config.
    ///
    /// Connection:
    /// - It internally spawns multiple `WebSearchTool` instances that share config.
    pub fn new(config: Arc<RwLock<ServerConfig>>) -> Self {
        Self {
            inner: WebSearchTool::new(config),
        }
    }
}

fn should_retry(tool_result: &ToolResult) -> bool {
    if tool_result.success {
        return false;
    }
    // Only retry on likely-transient failures. Retrying "no_results" tends to just burn time.
    match tool_result.error_code.as_deref() {
        Some("fetch_error") | Some("read_error") | Some("parse_error") => true,
        _ => matches!(tool_result.error_type, ErrorType::Timeout),
    }
}

#[async_trait]
impl Tool for ParallelSearchTool {
    /// Canonical tool name used in planner/executor `tool_binding`.
    fn name(&self) -> &str {
        "parallel_search"
    }

    /// Execute multiple searches concurrently and merge results.
    async fn execute(&self, params: Value) -> ToolResult {
        // Accept either { "queries": ["q1","q2",...] } or { "query": "single" }
        let queries: Vec<String> =
            if let Some(arr) = params.get("queries").and_then(|v| v.as_array()) {
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

        // Cap total query count (planner sometimes generates long lists).
        let queries: Vec<String> = queries.into_iter().take(8).collect();

        // Limit parallelism to reduce rate-limits / bot-detection (DDG/Bing can be sensitive).
        let semaphore = Arc::new(tokio::sync::Semaphore::new(3));

        // Fire all queries concurrently
        let handles: Vec<_> = queries
            .iter()
            .map(|q| {
                let q = q.clone();
                let inner = WebSearchTool::new_cloned_config(self.inner.config());
                let semaphore = Arc::clone(&semaphore);
                tokio::spawn(async move {
                    let _permit = semaphore
                        .acquire_owned()
                        .await
                        .expect("semaphore acquire");

                    // Best-effort retry to smooth over transient networking/rate limit issues.
                    let mut attempt = 0u8;
                    let mut last = inner.execute(json!({ "query": q })).await;
                    while attempt < 1 && should_retry(&last) {
                        attempt += 1;
                        sleep(Duration::from_millis(250 + (attempt as u64 * 350))).await;
                        last = inner.execute(json!({ "query": q })).await;
                    }

                    (q, last)
                })
            })
            .collect();

        let mut all_results: Vec<Value> = Vec::new();
        let mut successful_queries = 0usize;
        let mut failed_queries = 0usize;
        let mut failures: Vec<Value> = Vec::new();

        for handle in handles {
            match handle.await {
                Ok((query, tool_result)) => {
                    if tool_result.success {
                        successful_queries += 1;
                        if let Some(output) = &tool_result.output {
                            if let Some(results) = output.get("results").and_then(|r| r.as_array())
                            {
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
                        failures.push(json!({
                            "query": query,
                            "error_type": tool_result.error_type,
                            "error_code": tool_result.error_code,
                            "trace": tool_result.trace,
                        }));
                    }
                }
                Err(e) => {
                    failed_queries += 1;
                    failures.push(json!({
                        "query": "(join_error)",
                        "error_type": ErrorType::Tool,
                        "error_code": "join_error",
                        "trace": e.to_string(),
                    }));
                }
            }
        }

        if all_results.is_empty() {
            return ToolResult::err(
                ErrorType::Tool,
                "no_results",
                &format!(
                    "All search queries returned no results. failed_queries={failed_queries} details={}",
                    serde_json::to_string(&failures).unwrap_or_else(|_| "[]".into())
                ),
            );
        }

        // Deduplicate by URL — same page shouldn't appear multiple times
        let mut seen_urls = std::collections::HashSet::new();
        let deduped: Vec<Value> = all_results
            .into_iter()
            .filter(|r| {
                let url = r
                    .get("url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                seen_urls.insert(url)
            })
            .collect();

        let total = deduped.len();
        ToolResult::ok(
            json!({
                "results": deduped,
                "total": total,
                "successful_queries": successful_queries,
                "failed_queries": failed_queries,
            }),
            None,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_retry_on_fetch_error() {
        let r = ToolResult::err(ErrorType::Tool, "fetch_error", "network down");
        assert!(should_retry(&r));
    }

    #[test]
    fn should_not_retry_on_no_results() {
        let r = ToolResult::err(ErrorType::Tool, "no_results", "no backends worked");
        assert!(!should_retry(&r));
    }

    #[test]
    fn should_not_retry_on_success() {
        let r = ToolResult::ok(json!({"results":[]}), None);
        assert!(!should_retry(&r));
    }
}
