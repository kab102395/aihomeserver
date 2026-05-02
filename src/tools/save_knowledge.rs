use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::Mutex;

use super::Tool;
use crate::{
    memory::knowledge::KnowledgeStore,
    state::{ErrorType, ToolResult},
};

/// Saves a synthesized knowledge entry to the persistent knowledge base.
/// Called by the AI after researching a topic — stores it so future chats
/// can skip re-searching and use the stored knowledge directly.
pub struct SaveKnowledgeTool {
    store: Arc<Mutex<KnowledgeStore>>,
}

impl SaveKnowledgeTool {
    /// Create a tool that persists knowledge entries into the shared `KnowledgeStore`.
    pub fn new(store: Arc<Mutex<KnowledgeStore>>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for SaveKnowledgeTool {
    /// Canonical tool name used in planner/executor `tool_binding`.
    fn name(&self) -> &str {
        "save_knowledge"
    }

    /// Persist a knowledge entry (topic/summary/content/tags/sources) into SQLite.
    async fn execute(&self, params: Value) -> ToolResult {
        // Bulk mode: allow saving a "textbook" as multiple chapters/entries in one tool call.
        // Schema:
        // { "chapters": [ { "topic": "...", "summary": "...", "content": "...", "tags": "...", "sources": [...] }, ... ] }
        if let Some(chapters) = params.get("chapters").and_then(|v| v.as_array()) {
            if chapters.is_empty() {
                return ToolResult::err(ErrorType::Tool, "missing_param", "chapters array is empty");
            }

            let store = self.store.lock().await;
            let mut saved: Vec<Value> = Vec::new();
            let mut failed: Vec<Value> = Vec::new();

            for ch in chapters.iter().take(40) {
                let topic = ch.get("topic").and_then(|v| v.as_str()).unwrap_or("").trim();
                let content = ch
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                if topic.is_empty() || content.is_empty() {
                    failed.push(json!({
                        "topic": topic,
                        "error": "missing_topic_or_content"
                    }));
                    continue;
                }

                let summary = ch
                    .get("summary")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .map(|s| s.trim().to_string())
                    .unwrap_or_else(|| content.chars().take(300).collect::<String>());

                let tags = ch
                    .get("tags")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                let sources = if let Some(arr) = ch.get("sources").and_then(|v| v.as_array()) {
                    serde_json::to_string(arr).unwrap_or_else(|_| "[]".to_string())
                } else {
                    ch.get("sources")
                        .and_then(|v| v.as_str())
                        .unwrap_or("[]")
                        .to_string()
                };

                match store.upsert(topic, &summary, content, &tags, &sources).await {
                    Ok(entry) => saved.push(json!({
                        "id": entry.id,
                        "topic": entry.topic,
                        "version": entry.version
                    })),
                    Err(e) => failed.push(json!({
                        "topic": topic,
                        "error": "db_error",
                        "detail": e.to_string()
                    })),
                }
            }

            return ToolResult::ok(
                json!({
                    "saved": true,
                    "mode": "chapters",
                    "saved_count": saved.len(),
                    "failed_count": failed.len(),
                    "saved_entries": saved,
                    "failed_entries": failed
                }),
                None,
            );
        }

        let topic = match params.get("topic").and_then(|v| v.as_str()) {
            Some(t) if !t.trim().is_empty() => t.trim().to_string(),
            _ => return ToolResult::err(ErrorType::Tool, "missing_param", "topic required"),
        };
        let content = match params.get("content").and_then(|v| v.as_str()) {
            Some(c) if !c.trim().is_empty() => c.trim().to_string(),
            _ => return ToolResult::err(ErrorType::Tool, "missing_param", "content required"),
        };

        // Auto-save guard: avoid filling the KB with tiny one-off answers.
        let is_auto = params.get("auto").and_then(|v| v.as_bool()).unwrap_or(false);
        if is_auto {
            let min_chars = params
                .get("min_chars")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            if min_chars > 0 && content.chars().count() < min_chars {
                return ToolResult::ok(
                    json!({
                        "saved": false,
                        "skipped": true,
                        "reason": "auto_kb_min_chars",
                        "content_len": content.chars().count(),
                        "min_chars": min_chars
                    }),
                    None,
                );
            }
        }

        // Summary defaults to first 300 chars of content if not provided
        let summary = params
            .get("summary")
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
            params
                .get("tags")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        };

        // Sources: accept array or string
        let sources = if let Some(arr) = params.get("sources").and_then(|v| v.as_array()) {
            serde_json::to_string(arr).unwrap_or_else(|_| "[]".to_string())
        } else {
            params
                .get("sources")
                .and_then(|v| v.as_str())
                .unwrap_or("[]")
                .to_string()
        };

        let store = self.store.lock().await;
        match store
            .upsert(&topic, &summary, &content, &tags, &sources)
            .await
        {
            Ok(entry) => ToolResult::ok(
                json!({
                    "saved": true,
                    "id":      entry.id,
                    "topic":   entry.topic,
                    "version": entry.version,
                    "message": format!("Knowledge about '{}' saved (v{})", entry.topic, entry.version),
                }),
                None,
            ),
            Err(e) => ToolResult::err(ErrorType::Tool, "db_error", &e.to_string()),
        }
    }
}
