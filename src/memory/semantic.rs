use anyhow::Result;
use chrono::Utc;
use sqlx::SqlitePool;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::state::SemanticExample;

/// A past task stored with its embedding for similarity search.
#[derive(Debug, Clone)]
struct EmbeddingRecord {
    task_id: String,
    session_id: Option<String>,
    user_request: String,
    answer_summary: String,
    embedding: Vec<f32>,
}

/// Stores task embeddings in SQLite and performs in-memory cosine similarity search.
///
/// At startup the full embedding table is loaded into memory. For a personal
/// home server (<10k tasks) this is fast and requires no external index.
pub struct SemanticMemory {
    pool: SqlitePool,
    /// In-memory cache — avoids round-tripping SQLite on every query.
    cache: RwLock<Vec<EmbeddingRecord>>,
}

impl SemanticMemory {
    pub async fn new(db_path: &str) -> Result<Self> {
        let url = format!("sqlite:{db_path}?mode=rwc");
        let pool = SqlitePool::connect(&url).await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS embeddings (
                id             INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id        TEXT    NOT NULL,
                session_id     TEXT,
                user_request   TEXT    NOT NULL,
                answer_summary TEXT    NOT NULL,
                embedding      BLOB    NOT NULL,
                created_at     TEXT    NOT NULL
            )",
        )
        .execute(&pool)
        .await?;

        // Migration: add session_id if not present
        let _ = sqlx::query("ALTER TABLE embeddings ADD COLUMN session_id TEXT")
            .execute(&pool)
            .await;

        // Load existing embeddings into the in-memory cache
        let cache = load_all(&pool).await?;
        tracing::info!("Semantic memory: loaded {} embeddings", cache.len());

        Ok(Self {
            pool,
            cache: RwLock::new(cache),
        })
    }

    /// Embed and store a completed task. Silently skips if text is empty.
    pub async fn store(
        &self,
        task_id: &Uuid,
        session_id: Option<&Uuid>,
        user_request: &str,
        answer_summary: &str,
        embedding: Vec<f32>,
    ) -> Result<()> {
        if user_request.is_empty() || embedding.is_empty() {
            return Ok(());
        }

        let blob = f32_to_bytes(&embedding);
        let now = Utc::now().to_rfc3339();

        sqlx::query(
            "INSERT OR REPLACE INTO embeddings
             (task_id, session_id, user_request, answer_summary, embedding, created_at)
             VALUES (?,?,?,?,?,?)",
        )
        .bind(task_id.to_string())
        .bind(session_id.map(|u| u.to_string()))
        .bind(user_request)
        .bind(answer_summary)
        .bind(&blob)
        .bind(&now)
        .execute(&self.pool)
        .await?;

        // Update in-memory cache
        let mut cache = self.cache.write().await;
        // Remove any existing entry for this task (idempotent)
        cache.retain(|r| r.task_id != task_id.to_string());
        cache.push(EmbeddingRecord {
            task_id: task_id.to_string(),
            session_id: session_id.map(|u| u.to_string()),
            user_request: user_request.to_string(),
            answer_summary: answer_summary.to_string(),
            embedding,
        });

        Ok(())
    }

    /// Return the top-k most similar past tasks to the query embedding.
    /// Only returns entries with similarity above `min_score`.
    pub async fn query(
        &self,
        query_embedding: &[f32],
        k: usize,
        min_score: f32,
    ) -> Vec<SemanticExample> {
        let cache = self.cache.read().await;
        if cache.is_empty() {
            return vec![];
        }

        let mut scored: Vec<(f32, &EmbeddingRecord)> = cache
            .iter()
            .map(|r| (cosine_similarity(query_embedding, &r.embedding), r))
            .filter(|(score, _)| *score >= min_score)
            .collect();

        // Sort descending by similarity
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        scored
            .into_iter()
            .take(k)
            .map(|(similarity, r)| SemanticExample {
                user_request: r.user_request.clone(),
                answer_summary: r.answer_summary.clone(),
                similarity,
            })
            .collect()
    }

    /// Hard-delete all embeddings linked to a session.
    pub async fn delete_by_session(&self, session_id: &Uuid) -> Result<()> {
        let sid = session_id.to_string();
        sqlx::query("DELETE FROM embeddings WHERE session_id = ?")
            .bind(&sid)
            .execute(&self.pool)
            .await?;

        let mut cache = self.cache.write().await;
        cache.retain(|r| r.session_id.as_deref() != Some(&sid));
        Ok(())
    }

    /// Hard-delete a single task's embedding.
    pub async fn delete_by_task(&self, task_id: &Uuid) -> Result<()> {
        let tid = task_id.to_string();
        sqlx::query("DELETE FROM embeddings WHERE task_id = ?")
            .bind(&tid)
            .execute(&self.pool)
            .await?;

        let mut cache = self.cache.write().await;
        cache.retain(|r| r.task_id != tid);
        Ok(())
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

async fn load_all(pool: &SqlitePool) -> Result<Vec<EmbeddingRecord>> {
    use sqlx::Row;
    let rows = sqlx::query(
        "SELECT task_id, session_id, user_request, answer_summary, embedding FROM embeddings",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .filter_map(|row| {
            let blob: Vec<u8> = row.get("embedding");
            let embedding = bytes_to_f32(&blob);
            if embedding.is_empty() {
                return None;
            }
            Some(EmbeddingRecord {
                task_id: row.get("task_id"),
                session_id: row.try_get("session_id").ok().flatten(),
                user_request: row.get("user_request"),
                answer_summary: row.get("answer_summary"),
                embedding,
            })
        })
        .collect())
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot / (norm_a * norm_b)
    }
}

fn f32_to_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn bytes_to_f32(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
        .collect()
}
