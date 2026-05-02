//! Tool system (capabilities).
//!
//! Tools are the only way the agent can cause side effects (filesystem writes, shell commands,
//! network fetches, git operations, etc.). This is an explicit boundary:
//! - LLM nodes can *request* a tool call by emitting JSON.
//! - Only this layer actually executes the side effect.
//! - Every call returns a structured `ToolResult` that is stored as an artifact for replay.
//!
//! Interview talk track:
//! - “We don’t let the model do arbitrary side effects; it must go through typed tools.”
//! - “We store all tool results so failures are debuggable and reproducible.”

pub mod filesystem;
pub mod git;
pub mod http_fetch;
pub mod parallel_search;
pub mod save_knowledge;
pub mod shell;
pub mod web_search;

use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

use crate::metrics::SharedMetrics;
use crate::state::{ErrorType, ToolResult};

/// Every tool must implement this contract.
///
/// The mandatory output schema (`ToolResult`) is what makes the system auditable:
/// the UI and the critic can reason about tools in a uniform way.
/// The mandatory output schema (success/error_type/error_code/trace/output/checkpoint)
/// is enforced by returning ToolResult from every execution path.
#[async_trait]
pub trait Tool: Send + Sync {
    /// Canonical tool name used in plans (`tool_binding`).
    fn name(&self) -> &str;
    /// Execute the tool with JSON params and return a normalized `ToolResult`.
    async fn execute(&self, params: Value) -> ToolResult;
}

/// Registry of concrete tool implementations, keyed by tool name.
///
/// This is created at startup, shared via `Arc`, and used by the tool_execution node.
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
    metrics: Option<SharedMetrics>,
}

impl ToolRegistry {
    /// Create an empty tool registry.
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            metrics: None,
        }
    }

    /// Attach a shared metrics collector for tool latency/success tracking.
    pub fn set_metrics(&mut self, metrics: SharedMetrics) {
        self.metrics = Some(metrics);
    }

    /// Register a concrete tool under its canonical name (`Tool::name()`).
    pub fn register<T: Tool + 'static>(&mut self, tool: T) {
        let name = tool.name().to_string();
        self.tools.insert(name, Arc::new(tool));
    }

    /// Register a tool under an additional alias name.
    /// Useful when LLMs use alternate names (e.g. "run_command" for "shell").
    pub fn alias(&mut self, alias: &str, canonical: &str) {
        if let Some(tool) = self.tools.get(canonical).cloned() {
            self.tools.insert(alias.to_string(), tool);
        }
    }

    /// Execute a tool by name with JSON params.
    ///
    /// If the tool does not exist, a standardized `ToolResult::err` is returned.
    pub async fn execute(&self, name: &str, params: Value) -> ToolResult {
        let start = std::time::Instant::now();
        let result = match self.tools.get(name) {
            Some(tool) => tool.execute(params).await,
            None => ToolResult::err(
                ErrorType::Tool,
                "tool_not_found",
                &format!("No tool registered: {name}"),
            ),
        };

        if let Some(metrics) = &self.metrics {
            metrics
                .record_tool_call(name, result.success, start.elapsed())
                .await;
        }

        result
    }

    /// List registered tool names.
    ///
    /// Connection:
    /// - Used by evals and UIs to display capabilities.
    pub fn list(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.tools.keys().map(|s| s.as_str()).collect();
        names.sort();
        names
    }

    /// Check whether a tool is registered.
    ///
    /// Connection:
    /// - Used for capabilities snapshots and planner guardrails.
    pub fn has(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }
}
