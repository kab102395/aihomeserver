use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

/// A completed task record persisted to SQLite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRecord {
    pub task_id: Uuid,
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

pub struct EpisodicMemory {
    pool: SqlitePool,
}

impl EpisodicMemory {
    pub async fn new(db_path: &str) -> Result<Self> {
        let url = format!("sqlite:{db_path}?mode=rwc");
        let pool = SqlitePool::connect(&url).await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS tasks (
                task_id      TEXT    PRIMARY KEY,
                user_request TEXT    NOT NULL,
                plan_json    TEXT,
                artifacts_json TEXT NOT NULL DEFAULT '{}',
                critic_scores  TEXT NOT NULL DEFAULT '[]',
                failure_count  INTEGER NOT NULL DEFAULT 0,
                repair_cycles  INTEGER NOT NULL DEFAULT 0,
                duration_ms    INTEGER NOT NULL DEFAULT 0,
                success        INTEGER NOT NULL DEFAULT 0,
                created_at     TEXT    NOT NULL
            )",
        )
        .execute(&pool)
        .await?;

        Ok(Self { pool })
    }

    pub async fn save(&self, record: &TaskRecord) -> Result<()> {
        let critic_scores = serde_json::to_string(&record.critic_scores)?;
        sqlx::query(
            "INSERT OR REPLACE INTO tasks
             (task_id, user_request, plan_json, artifacts_json, critic_scores,
              failure_count, repair_cycles, duration_ms, success, created_at)
             VALUES (?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(record.task_id.to_string())
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

    pub async fn recent(&self, n: i64) -> Result<Vec<TaskRecord>> {
        use sqlx::Row;

        let rows = sqlx::query(
            "SELECT * FROM tasks ORDER BY created_at DESC LIMIT ?",
        )
        .bind(n)
        .fetch_all(&self.pool)
        .await?;

        rows.iter()
            .map(|row| {
                let task_id: String = row.get("task_id");
                let created_at: String = row.get("created_at");
                let critic_scores_json: String = row.get("critic_scores");

                Ok(TaskRecord {
                    task_id: task_id.parse()?,
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
            })
            .collect()
    }

    /// Find the most recent successful task matching a keyword in the request.
    pub async fn find_similar(&self, keyword: &str, limit: i64) -> Result<Vec<TaskRecord>> {
        use sqlx::Row;

        let pattern = format!("%{keyword}%");
        let rows = sqlx::query(
            "SELECT * FROM tasks WHERE user_request LIKE ? AND success = 1
             ORDER BY created_at DESC LIMIT ?",
        )
        .bind(&pattern)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        rows.iter()
            .map(|row| {
                let task_id: String = row.get("task_id");
                let created_at: String = row.get("created_at");
                let critic_scores_json: String = row.get("critic_scores");

                Ok(TaskRecord {
                    task_id: task_id.parse()?,
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
            })
            .collect()
    }
}
