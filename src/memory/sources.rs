//! Grounded source cache.
//!
//! This store caches fetched web sources (URL, status, content hash, title) so:
//! - repeated requests don’t refetch the same pages unnecessarily
//! - “facts tables” can reference stable normalized URLs
//! - provenance/audit data can be persisted for later inspection

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use sqlx::SqlitePool;

#[derive(Debug, Clone)]
pub struct SourceRecord {
    pub url: String,
    pub normalized_url: String,
    pub domain: String,
    pub fetched_at: DateTime<Utc>,
    pub status: i64,
    pub content_type: String,
    pub body_sha256: String,
    pub title: String,
}

/// Normalize a URL for caching/deduplication.
///
/// Why this exists:
/// - Web sources often vary by scheme, `www.`, trailing slashes, fragments, etc.
/// - Grounded “facts tables” should reference a stable canonical-ish URL.
///
/// Connection:
/// - Used by `SourceCacheStore` and the tool execution URL picker.
pub fn normalize_url(raw: &str) -> String {
    // Best-effort: if parsing fails, fall back to trimming.
    let trimmed = raw.trim();
    let Ok(mut u) = url::Url::parse(trimmed) else {
        return trimmed.trim_end_matches('/').to_string();
    };

    // Drop fragment, normalize scheme.
    u.set_fragment(None);
    let _ = u.set_scheme("https");

    // Normalize host: lower-case, strip www., unify reddit variants.
    if let Some(host) = u.host_str() {
        let mut h = host.to_lowercase();
        if let Some(rest) = h.strip_prefix("www.") {
            h = rest.to_string();
        }
        // Unify Reddit hosts (www/old) so we don't refetch duplicates.
        if h == "old.reddit.com" || h == "reddit.com" || h == "www.reddit.com" {
            h = "reddit.com".into();
        }
        let _ = u.set_host(Some(&h));
    }

    // Remove default ports.
    if matches!(u.port(), Some(80 | 443)) {
        let _ = u.set_port(None);
    }

    // Strip trailing slashes in path.
    let path = u.path().trim_end_matches('/').to_string();
    u.set_path(&path);

    u.to_string()
}

/// Extract a domain host name from a normalized URL.
///
/// Connection:
/// - Used for grouping/excluding candidate URLs during grounded fetch selection.
pub fn domain_from_url(normalized: &str) -> String {
    url::Url::parse(normalized)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_lowercase()))
        .unwrap_or_default()
}

pub struct SourceCacheStore {
    pool: SqlitePool,
}

impl SourceCacheStore {
    /// Open (or create) the sources database and ensure required tables exist.
    pub async fn new(db_path: &str) -> Result<Self> {
        let url = format!("sqlite:{db_path}?mode=rwc");
        let pool = SqlitePool::connect(&url).await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS sources (
                id             INTEGER PRIMARY KEY AUTOINCREMENT,
                url            TEXT    NOT NULL,
                normalized_url TEXT    NOT NULL,
                domain         TEXT    NOT NULL,
                fetched_at     TEXT    NOT NULL,
                status         INTEGER NOT NULL,
                content_type   TEXT    NOT NULL DEFAULT '',
                body_sha256    TEXT    NOT NULL DEFAULT '',
                title          TEXT    NOT NULL DEFAULT ''
            )",
        )
        .execute(&pool)
        .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_sources_norm ON sources(normalized_url)")
            .execute(&pool)
            .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_sources_fetched ON sources(fetched_at)")
            .execute(&pool)
            .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_sources_domain ON sources(domain)")
            .execute(&pool)
            .await?;

        Ok(Self { pool })
    }

    /// Record a fetch event (URL + status + content hash + title) into the cache.
    ///
    /// Connection:
    /// - `http_fetch` can call this so grounded runs have persisted provenance.
    pub async fn record_fetch(
        &self,
        url: &str,
        status: u16,
        content_type: Option<&str>,
        body_text: &str,
    ) -> Result<()> {
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        let normalized = normalize_url(url);
        let domain = domain_from_url(&normalized);
        let ct = content_type.unwrap_or("").to_string();
        let body_sha256 = sha256_hex(body_text.as_bytes());
        let title = first_nonempty_line(body_text).unwrap_or("").to_string();

        sqlx::query(
            "INSERT INTO sources (url, normalized_url, domain, fetched_at, status, content_type, body_sha256, title)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .bind(url)
        .bind(&normalized)
        .bind(&domain)
        .bind(&now_str)
        .bind(status as i64)
        .bind(ct)
        .bind(body_sha256)
        .bind(title)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// List recently fetched normalized URLs (used to avoid refetching the same sources).
    pub async fn recent_normalized_urls(&self, days: i64, limit: i64) -> Result<Vec<String>> {
        let cutoff = (Utc::now() - Duration::days(days)).to_rfc3339();
        let rows: Vec<(String,)> = sqlx::query_as(
            "SELECT normalized_url
             FROM sources
             WHERE fetched_at >= ?1
             ORDER BY fetched_at DESC
             LIMIT ?2",
        )
        .bind(cutoff)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        // De-dupe while preserving order
        let mut seen = std::collections::HashSet::<String>::new();
        let mut out = Vec::new();
        for (u,) in rows {
            if seen.insert(u.to_string()) {
                out.push(u);
            }
        }
        Ok(out)
    }
}

/// Extract a likely “title” from text by taking the first non-empty line.
fn first_nonempty_line(s: &str) -> Option<&str> {
    for line in s.lines() {
        let t = line.trim();
        if !t.is_empty() {
            return Some(t);
        }
    }
    None
}

/// Hash content for stable deduplication without storing full bodies in the cache table.
fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    let out = hasher.finalize();
    hex::encode(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_unifies_reddit_hosts_and_trailing_slash() {
        let a = normalize_url("https://www.reddit.com/r/DotA2/comments/abc123/test/");
        let b = normalize_url("http://old.reddit.com/r/DotA2/comments/abc123/test");
        assert_eq!(a, b);
        assert!(a.starts_with("https://reddit.com/"));
        assert!(!a.ends_with("//"));
    }
}
