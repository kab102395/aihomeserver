use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::{memory::knowledge::KnowledgeStore, state::{ErrorType, ToolResult}};
use super::Tool;

/// Saves a synthesized knowledge entry to the persistent knowledge base.
/// Called by the AI after researching a topic — stores it so future chats
/// can skip re-searching and use the stored knowledge directly.
pub struct SaveKnowledgeTool {
    store: Arc<Mutex<KnowledgeStore>>,
}

impl SaveKnowledgeTool {
    pub fn new(store: Arc<Mutex<KnowledgeStore>>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for SaveKnowledgeTool {
    fn name(&self) -> &str { "save_knowledge" }

    async fn execute(&self, params: Value) -> ToolResult {
        let topic = match params.get("topic").and_then(|v| v.as_str()) {
            Some(t) if !t.trim().is_empty() => t.trim().to_string(),
            _ => return ToolResult::err(ErrorType::Tool, "missing_param", "topic required"),
        };
        let content = match params.get("content").and_then(|v| v.as_str()) {
            Some(c) if !c.trim().is_empty() => c.trim().to_string(),
            _ => return ToolResult::err(ErrorType::Tool, "missing_param", "content required"),
        };

        // Summary defaults to first 300 chars of content if not provided
        let summary = params.get("summary")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| content.chars().take(300).collect::<String>());

        // Tags: accept array ["tag1","tag2"] or comma string "tag1,tag2"
        let tags = if let Some(arr) = params.get("tags").and_then(|v| v.as_array()) {
            arr.iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(",")
        } else {
            params.get("tags")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        };

        // Sources: accept array or string
        let sources = if let Some(arr) = params.get("sources").and_then(|v| v.as_array()) {
            serde_json::to_string(arr).unwrap_or_else(|_| "[]".to_string())
        } else {
            params.get("sources")
                .and_then(|v| v.as_str())
                .unwrap_or("[]")
                .to_string()
        };

        let store = self.store.lock().await;
        match store.upsert(&topic, &summary, &content, &tags, &sources).await {
            Ok(entry) => ToolResult::ok(json!({
                "saved": true,
                "id":      entry.id,
                "topic":   entry.topic,
                "version": entry.version,
                "message": format!("Knowledge about '{}' saved (v{})", entry.topic, entry.version),
            }), None),
            Err(e) => ToolResult::err(ErrorType::Tool, "db_error", &e.to_string()),
        }
    }
}
