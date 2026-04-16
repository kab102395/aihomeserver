use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::state::ConversationTurn;

pub struct ConversationStore {
    pool: SqlitePool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionSummary {
    pub session_id: String,
    pub created_at: DateTime<Utc>,
    pub last_active: DateTime<Utc>,
    /// First user message in the session — used as the sidebar label.
    pub first_message: String,
    pub archived: bool,
}

impl ConversationStore {
    pub async fn new(db_path: &str) -> Result<Self> {
        let url = format!("sqlite:{db_path}?mode=rwc");
        let pool = SqlitePool::connect(&url).await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS sessions (
                session_id  TEXT    PRIMARY KEY,
                created_at  TEXT    NOT NULL,
                last_active TEXT    NOT NULL,
                archived    INTEGER NOT NULL DEFAULT 0
            )",
        )
        .execute(&pool)
        .await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS conversation_turns (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id  TEXT    NOT NULL,
                role        TEXT    NOT NULL,
                content     TEXT    NOT NULL,
                timestamp   TEXT    NOT NULL
            )",
        )
        .execute(&pool)
        .await?;

        // Migrations for existing databases
        let _ = sqlx::query("ALTER TABLE sessions ADD COLUMN archived INTEGER NOT NULL DEFAULT 0")
            .execute(&pool)
            .await;

        Ok(Self { pool })
    }

    /// Returns existing session unchanged, or creates a new one.
    pub async fn get_or_create_session(&self, session_id: Option<Uuid>) -> Result<Uuid> {
        let id = session_id.unwrap_or_else(Uuid::new_v4);
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT OR IGNORE INTO sessions (session_id, created_at, last_active, archived)
             VALUES (?,?,?,0)",
        )
        .bind(id.to_string())
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }

    /// Fetch the most recent `limit` turns for a session, oldest first.
    pub async fn get_recent_turns(
        &self,
        session_id: &Uuid,
        limit: i64,
    ) -> Result<Vec<ConversationTurn>> {
        use sqlx::Row;

        let rows = sqlx::query(
            "SELECT role, content, timestamp FROM conversation_turns
             WHERE session_id = ? ORDER BY id DESC LIMIT ?",
        )
        .bind(session_id.to_string())
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        let mut turns: Vec<ConversationTurn> = rows
            .iter()
            .map(|row| {
                let ts: String = row.get("timestamp");
                ConversationTurn {
                    role: row.get("role"),
                    content: row.get("content"),
                    timestamp: DateTime::parse_from_rfc3339(&ts)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                }
            })
            .collect();

        turns.reverse(); // return oldest-first for prompt injection
        Ok(turns)
    }

    /// Append a single turn and bump `last_active` on the session.
    pub async fn add_turn(&self, session_id: &Uuid, role: &str, content: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO conversation_turns (session_id, role, content, timestamp)
             VALUES (?,?,?,?)",
        )
        .bind(session_id.to_string())
        .bind(role)
        .bind(content)
        .bind(&now)
        .execute(&self.pool)
        .await?;

        sqlx::query("UPDATE sessions SET last_active = ? WHERE session_id = ?")
            .bind(&now)
            .bind(session_id.to_string())
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// List active (non-archived) sessions ordered by last_active DESC.
    pub async fn list_sessions(&self, limit: i64) -> Result<Vec<SessionSummary>> {
        self.list_sessions_filtered(limit, false).await
    }

    /// List archived sessions ordered by last_active DESC.
    pub async fn list_archived_sessions(&self, limit: i64) -> Result<Vec<SessionSummary>> {
        self.list_sessions_filtered(limit, true).await
    }

    async fn list_sessions_filtered(
        &self,
        limit: i64,
        archived: bool,
    ) -> Result<Vec<SessionSummary>> {
        use sqlx::Row;

        let rows = sqlx::query(
            "SELECT s.session_id, s.created_at, s.last_active, s.archived,
                (SELECT content FROM conversation_turns
                 WHERE session_id = s.session_id AND role = 'user'
                 ORDER BY id ASC LIMIT 1) as first_message
             FROM sessions s
             WHERE s.archived = ?
             ORDER BY s.last_active DESC LIMIT ?",
        )
        .bind(archived as i32)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        rows.iter()
            .map(|row| {
                let created_at_str: String = row.get("created_at");
                let last_active_str: String = row.get("last_active");
                let first_message: Option<String> =
                    row.try_get("first_message").ok().flatten();
                Ok(SessionSummary {
                    session_id: row.get("session_id"),
                    created_at: DateTime::parse_from_rfc3339(&created_at_str)?
                        .with_timezone(&Utc),
                    last_active: DateTime::parse_from_rfc3339(&last_active_str)?
                        .with_timezone(&Utc),
                    first_message: first_message.unwrap_or_default(),
                    archived: row.get::<i64, _>("archived") != 0,
                })
            })
            .collect()
    }

    /// Load all turns for a session, oldest first (for UI replay).
    pub async fn get_all_turns(&self, session_id: &Uuid) -> Result<Vec<ConversationTurn>> {
        use sqlx::Row;

        let rows = sqlx::query(
            "SELECT role, content, timestamp FROM conversation_turns
             WHERE session_id = ? ORDER BY id ASC",
        )
        .bind(session_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        rows.iter()
            .map(|row| {
                let ts: String = row.get("timestamp");
                Ok(ConversationTurn {
                    role: row.get("role"),
                    content: row.get("content"),
                    timestamp: DateTime::parse_from_rfc3339(&ts)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                })
            })
            .collect()
    }

    /// Archive a session — hidden from main list but not deleted.
    pub async fn archive_session(&self, session_id: &Uuid) -> Result<()> {
        sqlx::query("UPDATE sessions SET archived = 1 WHERE session_id = ?")
            .bind(session_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Restore an archived session back to the active list.
    pub async fn unarchive_session(&self, session_id: &Uuid) -> Result<()> {
        sqlx::query("UPDATE sessions SET archived = 0 WHERE session_id = ?")
            .bind(session_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Hard-delete a session and ALL its turns. No recovery.
    /// Caller is responsible for also deleting linked task records.
    pub async fn delete_session(&self, session_id: &Uuid) -> Result<()> {
        // Delete turns first (no FK cascade in SQLite by default)
        sqlx::query("DELETE FROM conversation_turns WHERE session_id = ?")
            .bind(session_id.to_string())
            .execute(&self.pool)
            .await?;

        sqlx::query("DELETE FROM sessions WHERE session_id = ?")
            .bind(session_id.to_string())
            .execute(&self.pool)
            .await?;

        Ok(())
    }
}
