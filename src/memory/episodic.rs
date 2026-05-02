//! Episodic memory (run history).
//!
//! Stores one record per run in SQLite: the original request, plan JSON, artifacts JSON,
//! timings, critic scores, and whether the run succeeded.
//!
//! Why “episodic”:
//! - It enables replay/debugging (“what tools ran, what outputs existed, why did it fail?”).
//! - It’s also useful for product analytics (latency, repair rates, failure counts).

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

/// A completed task record persisted to SQLite.
///
/// Most fields are persisted as-is; artifacts are stored as JSON text to avoid migrations
/// as the artifact schema evolves.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRecord {
    pub task_id: Uuid,
    pub session_id: Option<Uuid>,
    pub user_request: String,
    pub plan_json: Option<String>,
    pub artifacts_json: String,
    pub critic_scores: Vec<f32>,
    pub failure_count: u32,
    pub repair_cycles: u32,
    pub duration_ms: i64,
    pub success: bool,
    pub created_at: DateTime<Utc>,
}

/// SQLite-backed store for `TaskRecord`s.
pub struct EpisodicMemory {
    pool: SqlitePool,
}

impl EpisodicMemory {
    /// Open (or create) the tasks database and ensure required tables exist.
    pub async fn new(db_path: &str) -> Result<Self> {
        let url = format!("sqlite:{db_path}?mode=rwc");
        let pool = SqlitePool::connect(&url).await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS tasks (
                task_id        TEXT    PRIMARY KEY,
                session_id     TEXT,
                user_request   TEXT    NOT NULL,
                plan_json      TEXT,
                artifacts_json TEXT    NOT NULL DEFAULT '{}',
                critic_scores  TEXT    NOT NULL DEFAULT '[]',
                failure_count  INTEGER NOT NULL DEFAULT 0,
                repair_cycles  INTEGER NOT NULL DEFAULT 0,
                duration_ms    INTEGER NOT NULL DEFAULT 0,
                success        INTEGER NOT NULL DEFAULT 0,
                created_at     TEXT    NOT NULL
            )",
        )
        .execute(&pool)
        .await?;

        // Migration: add session_id to existing tables that predate this column
        let _ = sqlx::query("ALTER TABLE tasks ADD COLUMN session_id TEXT")
            .execute(&pool)
            .await;

        Ok(Self { pool })
    }

    /// Persist a `TaskRecord` (upsert by `task_id`).
    pub async fn save(&self, record: &TaskRecord) -> Result<()> {
        let critic_scores = serde_json::to_string(&record.critic_scores)?;
        sqlx::query(
            "INSERT OR REPLACE INTO tasks
             (task_id, session_id, user_request, plan_json, artifacts_json, critic_scores,
              failure_count, repair_cycles, duration_ms, success, created_at)
             VALUES (?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(record.task_id.to_string())
        .bind(record.session_id.map(|u| u.to_string()))
        .bind(&record.user_request)
        .bind(&record.plan_json)
        .bind(&record.artifacts_json)
        .bind(&critic_scores)
        .bind(record.failure_count as i64)
        .bind(record.repair_cycles as i64)
        .bind(record.duration_ms)
        .bind(record.success as i32)
        .bind(record.created_at.to_rfc3339())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Fetch most recent `n` task records.
    pub async fn recent(&self, n: i64) -> Result<Vec<TaskRecord>> {
        let rows = sqlx::query("SELECT * FROM tasks ORDER BY created_at DESC LIMIT ?")
            .bind(n)
            .fetch_all(&self.pool)
            .await?;

        rows.iter().map(|row| parse_row(row)).collect()
    }

    /// Fetch a single task record by id.
    pub async fn get_by_id(&self, task_id: &str) -> Result<Option<TaskRecord>> {
        let row = sqlx::query("SELECT * FROM tasks WHERE task_id = ?")
            .bind(task_id)
            .fetch_optional(&self.pool)
            .await?;

        row.map(|row| parse_row(&row)).transpose()
    }

    /// Hard-delete all tasks linked to a session. Returns number of rows deleted.
    pub async fn delete_by_session(&self, session_id: &Uuid) -> Result<u64> {
        let result = sqlx::query("DELETE FROM tasks WHERE session_id = ?")
            .bind(session_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    /// Hard-delete a single task record by task_id.
    pub async fn delete_task(&self, task_id: &Uuid) -> Result<()> {
        sqlx::query("DELETE FROM tasks WHERE task_id = ?")
            .bind(task_id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Hard-delete every task record.
    pub async fn clear_all(&self) -> Result<()> {
        sqlx::query("DELETE FROM tasks").execute(&self.pool).await?;
        Ok(())
    }

    /// Find the most recent successful task matching a keyword in the request.
    pub async fn find_similar(&self, keyword: &str, limit: i64) -> Result<Vec<TaskRecord>> {
        let pattern = format!("%{keyword}%");
        let rows = sqlx::query(
            "SELECT * FROM tasks WHERE user_request LIKE ? AND success = 1
             ORDER BY created_at DESC LIMIT ?",
        )
        .bind(&pattern)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        rows.iter().map(|row| parse_row(row)).collect()
    }
}

/// Convert a raw SQL row into a typed `TaskRecord`.
///
/// Why this exists:
/// - Keeps SQLx row parsing in one place so schema changes are easier to manage.
fn parse_row(row: &sqlx::sqlite::SqliteRow) -> Result<TaskRecord> {
    use sqlx::Row;
    let task_id: String = row.get("task_id");
    let created_at: String = row.get("created_at");
    let critic_scores_json: String = row.get("critic_scores");
    let session_id_str: Option<String> = row.try_get("session_id").ok().flatten();

    Ok(TaskRecord {
        task_id: task_id.parse()?,
        session_id: session_id_str.and_then(|s| s.parse().ok()),
        user_request: row.get("user_request"),
        plan_json: row.get("plan_json"),
        artifacts_json: row.get("artifacts_json"),
        critic_scores: serde_json::from_str(&critic_scores_json)?,
        failure_count: row.get::<i64, _>("failure_count") as u32,
        repair_cycles: row.get::<i64, _>("repair_cycles") as u32,
        duration_ms: row.get("duration_ms"),
        success: row.get::<i64, _>("success") != 0,
        created_at: DateTime::parse_from_rfc3339(&created_at)?.with_timezone(&Utc),
    })
}
