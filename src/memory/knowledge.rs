//! Curated knowledge base.
//!
//! This store is for *human-curated* or “blessed” notes (topic/summary/content/tags/sources).
//! The planner can inject relevant entries to reduce repeated research and keep runs consistent.
//!
//! Why this is separate from episodic artifacts:
//! - Artifacts are auto-generated and can be noisy.
//! - Knowledge entries are meant to be reusable, edited, and versioned.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
/// One knowledge base entry, versioned on each update.
pub struct KnowledgeEntry {
    pub id: String,
    pub topic: String,
    /// Short 1-3 sentence summary — injected into every relevant chat
    pub summary: String,
    /// Full research content — injected when the topic is the primary focus
    pub content: String,
    /// Comma-separated tags for matching (e.g. "dota2,gaming,kez,hero")
    pub tags: String,
    /// JSON array of source URLs
    pub sources: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub version: i64,
}

impl KnowledgeEntry {
    /// Split the comma-separated `tags` field into a normalized list.
    ///
    /// Connection:
    /// - Used by `find_relevant` and by API handlers to decide if an entry is primary to a request.
    pub fn tag_list(&self) -> Vec<String> {
        self.tags
            .split(',')
            .map(|t| t.trim().to_lowercase())
            .filter(|t| !t.is_empty())
            .collect()
    }

    /// How many days old is this entry?
    pub fn age_days(&self) -> i64 {
        (Utc::now() - self.updated_at).num_days()
    }

    /// Returns true when an entry is older than (or equal to) `days`.
    ///
    /// Connection:
    /// - Useful for prompting “refresh this knowledge” workflows.
    pub fn is_stale(&self, days: i64) -> bool {
        self.age_days() >= days
    }
}

pub struct KnowledgeStore {
    pool: SqlitePool,
}

impl KnowledgeStore {
    /// Open (or create) the knowledge database and ensure required tables/indexes exist.
    pub async fn new(db_path: &str) -> Result<Self> {
        let url = format!("sqlite:{db_path}?mode=rwc");
        let pool = SqlitePool::connect(&url).await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS knowledge (
                id         TEXT    PRIMARY KEY,
                topic      TEXT    NOT NULL,
                summary    TEXT    NOT NULL DEFAULT '',
                content    TEXT    NOT NULL DEFAULT '',
                tags       TEXT    NOT NULL DEFAULT '',
                sources    TEXT    NOT NULL DEFAULT '[]',
                created_at TEXT    NOT NULL,
                updated_at TEXT    NOT NULL,
                version    INTEGER NOT NULL DEFAULT 1
            )",
        )
        .execute(&pool)
        .await?;

        // Full-text index on topic + tags for fast keyword search
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_knowledge_topic ON knowledge(topic)")
            .execute(&pool)
            .await?;

        Ok(Self { pool })
    }

    /// Save a new entry or update an existing one if the topic already exists.
    pub async fn upsert(
        &self,
        topic: &str,
        summary: &str,
        content: &str,
        tags: &str,
        sources: &str,
    ) -> Result<KnowledgeEntry> {
        let now = Utc::now();
        let now_str = now.to_rfc3339();

        // Check if topic already exists
        let existing: Option<(String,)> =
            sqlx::query_as("SELECT id FROM knowledge WHERE LOWER(topic) = LOWER(?1) LIMIT 1")
                .bind(topic)
                .fetch_optional(&self.pool)
                .await?;

        let id = if let Some((existing_id,)) = existing {
            // Update existing entry
            sqlx::query(
                "UPDATE knowledge SET summary=?1, content=?2, tags=?3, sources=?4,
                 updated_at=?5, version=version+1 WHERE id=?6",
            )
            .bind(summary)
            .bind(content)
            .bind(tags)
            .bind(sources)
            .bind(&now_str)
            .bind(&existing_id)
            .execute(&self.pool)
            .await?;
            existing_id
        } else {
            // Insert new entry
            let new_id = Uuid::new_v4().to_string();
            sqlx::query(
                "INSERT INTO knowledge (id, topic, summary, content, tags, sources, created_at, updated_at, version)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, 1)"
            )
            .bind(&new_id).bind(topic).bind(summary).bind(content)
            .bind(tags).bind(sources).bind(&now_str)
            .execute(&self.pool).await?;
            new_id
        };

        Ok(KnowledgeEntry {
            id,
            topic: topic.to_string(),
            summary: summary.to_string(),
            content: content.to_string(),
            tags: tags.to_string(),
            sources: sources.to_string(),
            created_at: now,
            updated_at: now,
            version: 1,
        })
    }

    /// Find entries relevant to a user request via keyword matching on topic + tags.
    pub async fn find_relevant(&self, text: &str) -> Result<Vec<KnowledgeEntry>> {
        let all = self.list().await?;
        let words: Vec<String> = text
            .to_lowercase()
            .split_whitespace()
            .filter(|w| w.len() > 3)
            .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_string())
            .filter(|w| !w.is_empty())
            .collect();

        let mut scored: Vec<(u32, KnowledgeEntry)> = all
            .into_iter()
            .filter_map(|entry| {
                let topic_lower = entry.topic.to_lowercase();
                let tags_lower = entry.tags.to_lowercase();
                let mut score: u32 = 0;
                for word in &words {
                    if topic_lower.contains(word.as_str()) {
                        score += 3;
                    }
                    if tags_lower.contains(word.as_str()) {
                        score += 1;
                    }
                }
                if score > 0 {
                    Some((score, entry))
                } else {
                    None
                }
            })
            .collect();

        scored.sort_by(|a, b| b.0.cmp(&a.0));
        Ok(scored.into_iter().take(5).map(|(_, e)| e).collect())
    }

    /// List knowledge entries (most recently updated first).
    pub async fn list(&self) -> Result<Vec<KnowledgeEntry>> {
        let rows: Vec<(
            String,
            String,
            String,
            String,
            String,
            String,
            String,
            String,
            i64,
        )> = sqlx::query_as(
            "SELECT id, topic, summary, content, tags, sources, created_at, updated_at, version
                 FROM knowledge ORDER BY updated_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .filter_map(
                |(
                    id,
                    topic,
                    summary,
                    content,
                    tags,
                    sources,
                    created_str,
                    updated_str,
                    version,
                )| {
                    let created_at = DateTime::parse_from_rfc3339(&created_str)
                        .ok()?
                        .with_timezone(&Utc);
                    let updated_at = DateTime::parse_from_rfc3339(&updated_str)
                        .ok()?
                        .with_timezone(&Utc);
                    Some(KnowledgeEntry {
                        id,
                        topic,
                        summary,
                        content,
                        tags,
                        sources,
                        created_at,
                        updated_at,
                        version,
                    })
                },
            )
            .collect())
    }

    /// Fetch a single entry by id.
    pub async fn get(&self, id: &str) -> Result<Option<KnowledgeEntry>> {
        let row: Option<(
            String,
            String,
            String,
            String,
            String,
            String,
            String,
            String,
            i64,
        )> = sqlx::query_as(
            "SELECT id, topic, summary, content, tags, sources, created_at, updated_at, version
                 FROM knowledge WHERE id = ?1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.and_then(
            |(id, topic, summary, content, tags, sources, created_str, updated_str, version)| {
                let created_at = DateTime::parse_from_rfc3339(&created_str)
                    .ok()?
                    .with_timezone(&Utc);
                let updated_at = DateTime::parse_from_rfc3339(&updated_str)
                    .ok()?
                    .with_timezone(&Utc);
                Some(KnowledgeEntry {
                    id,
                    topic,
                    summary,
                    content,
                    tags,
                    sources,
                    created_at,
                    updated_at,
                    version,
                })
            },
        ))
    }

    /// Hard-delete a knowledge entry by id.
    pub async fn delete(&self, id: &str) -> Result<()> {
        sqlx::query("DELETE FROM knowledge WHERE id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
