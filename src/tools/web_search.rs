use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use super::Tool;
use crate::{
    config::ServerConfig,
    state::{ErrorType, ToolResult},
};

/// In-process cache for search results to reduce repeated queries that trigger rate limits.
///
/// Note:
/// - This is intentionally simple (no persistence). It improves reliability within a single run
///   and across nearby requests without complicating deployments.
static SEARCH_CACHE: std::sync::OnceLock<tokio::sync::RwLock<std::collections::HashMap<String, (Instant, Value)>>> =
    std::sync::OnceLock::new();

fn search_cache() -> &'static tokio::sync::RwLock<std::collections::HashMap<String, (Instant, Value)>> {
    SEARCH_CACHE.get_or_init(|| tokio::sync::RwLock::new(std::collections::HashMap::new()))
}

/// In-process SerpAPI budget to avoid burning limited monthly credits.
///
/// This is not intended as a billing-grade limiter; it just prevents runaway usage
/// inside a single run / hot server process.
static SERPAPI_CALLS: std::sync::OnceLock<std::sync::atomic::AtomicU32> = std::sync::OnceLock::new();

fn serpapi_calls() -> &'static std::sync::atomic::AtomicU32 {
    SERPAPI_CALLS.get_or_init(|| std::sync::atomic::AtomicU32::new(0))
}

/// Web search tool.
///
/// Connection:
/// - Used for grounded research flows when the planner/executor decide a request is time-sensitive.
/// - `parallel_search` typically runs several searches concurrently using this tool.
pub struct WebSearchTool {
    client: reqwest::Client,
    config: Arc<RwLock<ServerConfig>>,
}

impl WebSearchTool {
    /// Create a new search tool using the shared runtime config (for search URL settings).
    pub fn new(config: Arc<RwLock<ServerConfig>>) -> Self {
        Self::new_cloned_config(config)
    }

    /// Create a new instance sharing the same config Arc — used by ParallelSearchTool
    /// to spawn multiple concurrent search tasks without cloning the whole struct.
    pub fn new_cloned_config(config: Arc<RwLock<ServerConfig>>) -> Self {
        Self {
            config,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(20))
                .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36")
                .gzip(true)
                .build()
                .expect("HTTP client"),
        }
    }

    /// Expose the config Arc so ParallelSearchTool can clone it.
    pub fn config(&self) -> Arc<RwLock<ServerConfig>> {
        Arc::clone(&self.config)
    }

    /// Run one or more API-backed searches (You.com / Brave / SerpAPI) and merge results.
    ///
    /// Returns `None` when no API keys are configured.
    async fn api_search(&self, query: &str) -> Option<Value> {
        let tavily_key = std::env::var("TAVILY_API_KEY")
            .ok()
            .filter(|s| !s.trim().is_empty());
        let you_key = std::env::var("YOU_API_KEY").ok().filter(|s| !s.trim().is_empty());
        let brave_key = std::env::var("BRAVE_SEARCH_API_KEY")
            .ok()
            .filter(|s| !s.trim().is_empty());
        let serpapi_key = std::env::var("SERPAPI_API_KEY")
            .ok()
            .filter(|s| !s.trim().is_empty());

        if tavily_key.is_none() && you_key.is_none() && brave_key.is_none() && serpapi_key.is_none() {
            return None;
        }

        // Priority: Tavily/You/Brave first (reliable + higher quotas), SerpAPI last (low free quota).
        // SerpAPI is only used as a fallback unless SERPAPI_PREFERRED=1.
        let serpapi_preferred = std::env::var("SERPAPI_PREFERRED")
            .ok()
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let serpapi_allow_fallback = std::env::var("SERPAPI_ALLOW_FALLBACK")
            .ok()
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(true);
        let serpapi_max_calls: u32 = std::env::var("SERPAPI_MAX_CALLS_PER_RUN")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(3);

        let mut handles = Vec::new();
        let mut provider_debug: Vec<Value> = Vec::new();
        if let Some(k) = tavily_key {
            let q = query.to_string();
            let client = self.client.clone();
            provider_debug.push(json!({"provider":"tavily","planned":true}));
            handles.push(tokio::spawn(async move {
                ("tavily", tavily_search_api(&client, &q, &k).await)
            }));
        }
        if let Some(k) = you_key {
            let q = query.to_string();
            let client = self.client.clone();
            provider_debug.push(json!({"provider":"you","planned":true}));
            handles.push(tokio::spawn(async move {
                ("you", you_search_api(&client, &q, &k).await)
            }));
        }
        if let Some(k) = brave_key {
            let q = query.to_string();
            let client = self.client.clone();
            provider_debug.push(json!({"provider":"brave","planned":true}));
            handles.push(tokio::spawn(async move {
                ("brave", brave_search_api(&client, &q, &k).await)
            }));
        }

        let mut all: Vec<Value> = Vec::new();
        let mut sources: Vec<String> = Vec::new();
        for h in handles {
            if let Ok((name, res)) = h.await {
                if let Some(mut v) = res {
                    sources.push(name.to_string());
                    all.append(&mut v);
                    provider_debug.push(json!({"provider":name,"used":true,"result_count":v.len()}));
                } else {
                    provider_debug.push(json!({"provider":name,"used":false,"result_count":0}));
                }
            }
        }

        // If the primary providers returned nothing and SerpAPI is configured, optionally use it as fallback.
        if all.is_empty() && serpapi_key.is_some() && (serpapi_preferred || serpapi_allow_fallback) {
            let used = serpapi_calls().load(std::sync::atomic::Ordering::Relaxed);
            if used < serpapi_max_calls {
                let _ = serpapi_calls().fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let k = serpapi_key.unwrap();
                let mut v = serpapi_search_api(&self.client, query, &k).await.unwrap_or_default();
                if !v.is_empty() {
                    sources.push("serpapi".into());
                    provider_debug.push(json!({"provider":"serpapi","used":true,"result_count":v.len()}));
                    all.append(&mut v);
                } else {
                    provider_debug.push(json!({"provider":"serpapi","used":false,"result_count":0,"note":"fallback_returned_empty"}));
                }
            } else {
                provider_debug.push(json!({"provider":"serpapi","used":false,"result_count":0,"note":"skipped_budget_exhausted","max_calls_per_run":serpapi_max_calls}));
            }
        } else if serpapi_key.is_some() && !serpapi_preferred {
            // Make it explicit that SerpAPI was intentionally not used.
            provider_debug.push(json!({"provider":"serpapi","used":false,"note":"skipped_not_needed_or_disabled"}));
        }

        if all.is_empty() {
            return None;
        }

        // Deduplicate by URL and cap.
        let mut seen = std::collections::HashSet::<String>::new();
        let mut deduped = Vec::new();
        for r in all {
            let url = r.get("url").and_then(|u| u.as_str()).unwrap_or("").to_string();
            if url.is_empty() {
                continue;
            }
            if seen.insert(url) {
                deduped.push(r);
            }
            if deduped.len() >= 8 {
                break;
            }
        }

        Some(json!({
            "query": query,
            "results": deduped,
            "source": sources.join("+"),
            "provider_debug": provider_debug
        }))
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    /// Canonical tool name used in planner/executor `tool_binding`.
    fn name(&self) -> &str {
        "web_search"
    }

    /// Execute one web search query and return a normalized result list.
    async fn execute(&self, params: Value) -> ToolResult {
        let query = match params.get("query").and_then(|v| v.as_str()) {
            Some(q) if !q.trim().is_empty() => q.to_string(),
            _ => {
                return ToolResult::err(
                    ErrorType::Tool,
                    "missing_param",
                    "query parameter required",
                )
            }
        };

        // Serve from cache when available (helps avoid SearXNG engine bans / scraping blocks).
        let cache_key = query.trim().to_lowercase();
        let ttl = Duration::from_secs(6 * 60 * 60);
        if let Some(cached) = {
            let cache = search_cache().read().await;
            cache
                .get(&cache_key)
                .and_then(|(t, v)| (t.elapsed() <= ttl).then_some(v.clone()))
        } {
            return ToolResult::ok(cached, None);
        }

        // If API-backed search providers are configured, use them first (most reliable).
        // These are designed for programmatic usage and avoid HTML parsing volatility.
        if let Some(out) = self.api_search(&query).await {
            let mut cache = search_cache().write().await;
            cache.insert(cache_key.clone(), (Instant::now(), out.clone()));
            return ToolResult::ok(out, None);
        }

        // If a custom search engine is configured (e.g. SearXNG), try it first.
        // Fall through to DDG scraping if SearXNG returns nothing.
        let custom_url = self.config.read().await.search_url.clone();
        if !custom_url.is_empty() {
            let result = self.searxng_search(&query, &custom_url).await;
            if result.success {
                if let Some(output) = &result.output {
                    let mut cache = search_cache().write().await;
                    cache.insert(cache_key.clone(), (Instant::now(), output.clone()));
                }
                return result;
            }
            // SearXNG failed or returned 0 results — fall back to DDG below
        }

        let encoded = url_encode(&query);

        // DuckDuckGo HTML endpoint — designed for programmatic use, no JS required
        let url = format!("https://html.duckduckgo.com/html/?q={encoded}&kl=us-en");

        let html = match self
            .client
            .get(&url)
            .header(
                "Accept",
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            )
            .header("Accept-Language", "en-US,en;q=0.5")
            .header("DNT", "1")
            .send()
            .await
        {
            Ok(r) => match r.text().await {
                Ok(t) => t,
                Err(e) => return ToolResult::err(ErrorType::Tool, "read_error", &e.to_string()),
            },
            Err(e) => return ToolResult::err(ErrorType::Tool, "fetch_error", &e.to_string()),
        };

        let results = extract_ddg_results(&html);
        if !results.is_empty() {
            let out = json!({ "query": query, "results": results, "source": "ddg" });
            {
                let mut cache = search_cache().write().await;
                cache.insert(cache_key.clone(), (Instant::now(), out.clone()));
            }
            return ToolResult::ok(
                out,
                None,
            );
        }

        // DDG HTML returned nothing — try DDG lite
        let lite_url = format!("https://lite.duckduckgo.com/lite/?q={encoded}");
        let lite_html = match self.client.get(&lite_url).send().await {
            Ok(r) => r.text().await.unwrap_or_default(),
            Err(_) => String::new(),
        };
        let lite_results = extract_ddg_lite_results(&lite_html);
        if !lite_results.is_empty() {
            let out = json!({ "query": query, "results": lite_results, "source": "ddg_lite" });
            {
                let mut cache = search_cache().write().await;
                cache.insert(cache_key.clone(), (Instant::now(), out.clone()));
            }
            return ToolResult::ok(
                out,
                None,
            );
        }

        // DDG lite also empty — try Bing as last resort
        let bing_url = format!("https://www.bing.com/search?q={encoded}&count=6");
        let bing_html = match self
            .client
            .get(&bing_url)
            .header("Accept", "text/html")
            .header("Accept-Language", "en-US,en;q=0.9")
            .send()
            .await
        {
            Ok(r) => r.text().await.unwrap_or_default(),
            Err(_) => String::new(),
        };
        let bing_results = extract_bing_results(&bing_html);
        if !bing_results.is_empty() {
            let out = json!({ "query": query, "results": bing_results, "source": "bing" });
            {
                let mut cache = search_cache().write().await;
                cache.insert(cache_key.clone(), (Instant::now(), out.clone()));
            }
            return ToolResult::ok(
                out,
                None,
            );
        }

        // All standard backends failed — try Reddit search (highly reliable, great for game content)
        if let Some(reddit_results) = self.reddit_search(&query, &encoded).await {
            if !reddit_results.is_empty() {
                let out = json!({ "query": query, "results": reddit_results, "source": "reddit" });
                {
                    let mut cache = search_cache().write().await;
                    cache.insert(cache_key.clone(), (Instant::now(), out.clone()));
                }
                return ToolResult::ok(
                    out,
                    None,
                );
            }
        }

        // Final fallback: Wikipedia search API (always works, good for game mechanics/heroes)
        if let Some(wiki_results) = self.wikipedia_search(&query, &encoded).await {
            if !wiki_results.is_empty() {
                let out = json!({ "query": query, "results": wiki_results, "source": "wikipedia" });
                {
                    let mut cache = search_cache().write().await;
                    cache.insert(cache_key.clone(), (Instant::now(), out.clone()));
                }
                return ToolResult::ok(
                    out,
                    None,
                );
            }
        }

        ToolResult::err(
            ErrorType::Tool,
            "no_results",
            "All search backends failed (SearXNG, DDG, DDG-lite, Bing, Reddit, Wikipedia). \
             Check Docker networking — the container may not have outbound internet access.",
        )
    }
}

// ── Reddit + Wikipedia direct search ─────────────────────────────────────────

impl WebSearchTool {
    /// Search Reddit via the JSON API — very reliable from server IPs.
    /// Returns top posts with titles, URLs, and self-text snippets.
    /// Best-effort Reddit-only search used as a fallback when the primary engine is blocked.
    async fn reddit_search(&self, _query: &str, encoded: &str) -> Option<Vec<Value>> {
        // Search across all of Reddit, sorted by relevance, past year
        let url =
            format!("https://www.reddit.com/search.json?q={encoded}&sort=top&t=year&limit=8",);
        let resp = self
            .client
            .get(&url)
            .header("Accept", "application/json")
            .send()
            .await
            .ok()?;
        let body: Value = resp.json().await.ok()?;
        let posts = body.get("data")?.get("children")?.as_array()?;

        let results: Vec<Value> = posts
            .iter()
            .filter_map(|p| {
                let d = p.get("data")?;
                let title = d.get("title")?.as_str().unwrap_or("").to_string();
                let permalink = d.get("permalink")?.as_str().unwrap_or("");
                let selftext = d.get("selftext")?.as_str().unwrap_or("");
                let url = format!("https://old.reddit.com{permalink}");
                let snippet = selftext.chars().take(300).collect::<String>();
                if title.is_empty() {
                    return None;
                }
                Some(json!({ "title": title, "url": url, "snippet": snippet }))
            })
            .collect();

        Some(results)
    }

    /// Search Wikipedia via the MediaWiki API — always accessible, good for hero info.
    /// Best-effort Wikipedia search used as an additional “high precision” source.
    async fn wikipedia_search(&self, _query: &str, encoded: &str) -> Option<Vec<Value>> {
        let url = format!(
            "https://en.wikipedia.org/w/api.php?action=query&list=search&srsearch={encoded}&format=json&srlimit=5&utf8=1"
        );
        let resp = self.client.get(&url).send().await.ok()?;
        let body: Value = resp.json().await.ok()?;
        let hits = body.get("query")?.get("search")?.as_array()?;

        let results: Vec<Value> = hits
            .iter()
            .filter_map(|h| {
                let title = h.get("title")?.as_str().unwrap_or("").to_string();
                let snippet = h.get("snippet")?.as_str().unwrap_or("");
                let snippet = strip_tags(snippet); // Wikipedia returns HTML in snippets
                let slug = title.replace(' ', "_");
                let url = format!("https://en.wikipedia.org/wiki/{slug}");
                Some(json!({ "title": title, "url": url, "snippet": snippet }))
            })
            .collect();

        Some(results)
    }
}

// ── SearXNG JSON search ───────────────────────────────────────────────────────

impl WebSearchTool {
    /// Call a SearXNG instance's JSON API.
    /// Expects base_url like `http://localhost:8080` or `http://searxng:8080`.
    /// Query a configured SearXNG instance (preferred when available).
    ///
    /// Connection:
    /// - Uses `ServerConfig.search_url` so operators can run searches without scraping DDG/Bing.
    async fn searxng_search(&self, query: &str, base_url: &str) -> ToolResult {
        let encoded = url_encode(query);
        let url = format!(
            "{}/search?q={}&format=json&categories=general&language=en",
            base_url.trim_end_matches('/'),
            encoded,
        );

        // SearXNG's botdetection rejects requests that lack proxy headers.
        // Setting these to 127.0.0.1 tells SearXNG the request comes from localhost
        // (trusted internal source), bypassing the bot check entirely.
        let resp = match self
            .client
            .get(&url)
            .header("X-Forwarded-For", "127.0.0.1")
            .header("X-Real-IP", "127.0.0.1")
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => return ToolResult::err(ErrorType::Tool, "fetch_error", &e.to_string()),
        };

        let status = resp.status().as_u16();
        if !(200..=299).contains(&status) {
            let text = resp.text().await.unwrap_or_default();
            let snippet: String = text.chars().take(400).collect();
            return ToolResult::err(
                ErrorType::Tool,
                "http_status",
                &format!("SearXNG returned HTTP {status}. body_snippet={snippet}"),
            );
        }

        let body: serde_json::Value = match resp.json().await {
            Ok(v) => v,
            Err(e) => return ToolResult::err(ErrorType::Tool, "parse_error", &e.to_string()),
        };

        let raw = match body.get("results").and_then(|v| v.as_array()) {
            Some(a) => a,
            None => {
                return ToolResult::err(
                    ErrorType::Tool,
                    "no_results",
                    "SearXNG returned no results array",
                )
            }
        };

        let results: Vec<serde_json::Value> = raw
            .iter()
            .take(6)
            .filter_map(|r| {
                let title = r
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let url = r
                    .get("url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let snippet = r
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if url.is_empty() {
                    return None;
                }
                Some(json!({ "title": title, "url": url, "snippet": snippet }))
            })
            .collect();

        if results.is_empty() {
            return ToolResult::err(ErrorType::Tool, "no_results", "SearXNG returned 0 results");
        }

        ToolResult::ok(
            json!({ "query": query, "results": results, "source": "searxng" }),
            None,
        )
    }
}

// ── DDG HTML parser ───────────────────────────────────────────────────────────

/// Parse `html.duckduckgo.com/html/` response.
/// Results are wrapped in `<div class="result__body">` blocks.
/// Parse DuckDuckGo HTML results into a normalized JSON list.
///
/// Why this exists:
/// - Keeps the search tool dependency-free (no headless browser).
/// - Produces a stable schema for the planner/executor to consume.
fn extract_ddg_results(html: &str) -> Vec<Value> {
    let mut results = Vec::new();
    let mut pos = 0;

    while results.len() < 6 {
        let marker = "result__body";
        let Some(rel) = html[pos..].find(marker) else {
            break;
        };
        let block_start = pos + rel;

        // Bound the block — everything until the next result__body
        let block_end = html[block_start + marker.len()..]
            .find("result__body")
            .map(|r| block_start + marker.len() + r)
            .unwrap_or_else(|| html.len().min(block_start + 3000));

        let block = &html[block_start..block_end];
        pos = block_start + marker.len();

        // Title link: class="result__a"
        let Some(a_pos) = block.find("result__a") else {
            continue;
        };
        let a_tag_start = match block[..a_pos].rfind('<') {
            Some(p) => p,
            None => continue,
        };
        let a_block = &block[a_tag_start..];

        let href = extract_attr(a_block, "href").unwrap_or_default();
        let url = decode_ddg_url(&href);
        if url.is_empty() {
            continue;
        }

        let title = a_block
            .find('>')
            .map(|i| {
                let rest = &a_block[i + 1..];
                rest.find("</a>")
                    .map(|j| strip_tags(&rest[..j]))
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        if title.trim().is_empty() {
            continue;
        }

        // Snippet: class="result__snippet"
        let snippet = block
            .find("result__snippet")
            .and_then(|sp| {
                let after = &block[sp..];
                after.find('>').map(|i| {
                    let rest = &after[i + 1..];
                    let end = rest
                        .find("</a>")
                        .or_else(|| rest.find("</span>"))
                        .unwrap_or_else(|| rest.len().min(400));
                    strip_tags(&rest[..end])
                })
            })
            .unwrap_or_default();

        results.push(json!({
            "title":   title.trim(),
            "url":     url,
            "snippet": snippet.trim(),
        }));
    }
    results
}

/// Parse Bing search results — fallback when DDG is unavailable.
/// Parse Bing HTML results into a normalized JSON list (fallback engine).
fn extract_bing_results(html: &str) -> Vec<Value> {
    let mut results = Vec::new();
    let mut pos = 0;

    while results.len() < 6 {
        // Bing wraps each result in <li class="b_algo">
        let Some(rel) = html[pos..].find("b_algo") else {
            break;
        };
        let block_start = pos + rel;
        let block_end = html[block_start + 6..]
            .find("b_algo")
            .map(|r| block_start + 6 + r)
            .unwrap_or_else(|| html.len().min(block_start + 3000));
        let block = &html[block_start..block_end];
        pos = block_start + 6;

        // Extract <a href="...">title</a>
        let Some(a_pos) = block.find("<a href=\"http") else {
            continue;
        };
        let a_block = &block[a_pos..block.len().min(a_pos + 500)];
        let href = extract_attr(a_block, "href").unwrap_or_default();
        if href.is_empty() || href.contains("bing.com") || href.contains("microsoft.com") {
            continue;
        }
        let title = a_block
            .find('>')
            .map(|i| {
                let rest = &a_block[i + 1..];
                rest.find("</a>")
                    .map(|j| strip_tags(&rest[..j]))
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        if title.trim().is_empty() {
            continue;
        }

        // Snippet is usually in <p> after the link
        let snippet = block
            .find("<p>")
            .and_then(|sp| {
                let after = &block[sp + 3..];
                after.find("</p>").map(|j| strip_tags(&after[..j]))
            })
            .unwrap_or_default();

        results.push(json!({
            "title":   title.trim(),
            "url":     href,
            "snippet": snippet.trim(),
        }));
    }
    results
}

/// Parse `lite.duckduckgo.com/lite/` — even simpler table-based HTML.
/// Each row with a link is a search result.
/// Parse DuckDuckGo “lite” HTML results into a normalized JSON list (fallback format).
fn extract_ddg_lite_results(html: &str) -> Vec<Value> {
    let mut results = Vec::new();
    let mut pos = 0;

    while results.len() < 6 {
        let Some(rel) = html[pos..].find("<a rel=\"nofollow\"") else {
            break;
        };
        let a_start = pos + rel;
        let a_block = &html[a_start..html.len().min(a_start + 1000)];
        pos = a_start + 1;

        let href = extract_attr(a_block, "href").unwrap_or_default();
        if !href.starts_with("http") {
            continue;
        }
        if href.contains("duckduckgo.com") {
            continue;
        }

        let title = a_block
            .find('>')
            .map(|i| {
                let rest = &a_block[i + 1..];
                rest.find("</a>")
                    .map(|j| strip_tags(&rest[..j]))
                    .unwrap_or_default()
            })
            .unwrap_or_default();
        if title.trim().is_empty() {
            continue;
        }

        results.push(json!({ "title": title.trim(), "url": href, "snippet": "" }));
    }
    results
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// DDG wraps URLs as `/l/?uddg=PERCENT_ENCODED_URL&...` — decode to the real URL.
/// Decode DuckDuckGo redirect-style URLs to the underlying destination URL.
fn decode_ddg_url(href: &str) -> String {
    if let Some(after) = href.split("uddg=").nth(1) {
        let end = after.find('&').unwrap_or(after.len());
        return percent_decode(&after[..end]);
    }
    if href.starts_with("http") {
        return href.to_string();
    }
    String::new()
}

/// Minimal percent-decoder used by the HTML scrapers.
fn percent_decode(s: &str) -> String {
    let mut bytes: Vec<u8> = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(hex) = std::str::from_utf8(&b[i + 1..i + 3]) {
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    bytes.push(byte);
                    i += 3;
                    continue;
                }
            }
        }
        if b[i] == b'+' {
            bytes.push(b' ');
        } else {
            bytes.push(b[i]);
        }
        i += 1;
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Minimal URL encoder used to build query strings without pulling in extra dependencies.
fn url_encode(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            ' ' => "+".to_string(),
            _ => {
                let mut buf = [0u8; 4];
                let encoded = c.encode_utf8(&mut buf);
                encoded.bytes().map(|b| format!("%{:02X}", b)).collect()
            }
        })
        .collect()
}

/// Extract an attribute value from a naive HTML tag slice.
///
/// This is not a general-purpose HTML parser; it’s tuned for a couple search page shapes.
fn extract_attr(tag: &str, attr: &str) -> Option<String> {
    // Try both attr="..." and attr='...'
    for quote in ['"', '\''] {
        let needle = format!("{}={}", attr, quote);
        if let Some(start) = tag.find(&needle) {
            let after = &tag[start + needle.len()..];
            let end = after.find(quote)?;
            return Some(after[..end].to_string());
        }
    }
    None
}

/// Strip HTML tags from a snippet.
///
/// Connection:
/// - Search snippets are shown to the user and passed to the LLM; stripping tags reduces noise.
fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_result(title: &str, url: &str, snippet: &str) -> Option<Value> {
    let t = title.trim();
    let u = url.trim();
    if u.is_empty() || !u.starts_with("http") {
        return None;
    }
    Some(json!({
        "title": t,
        "url": u,
        "snippet": snippet.trim()
    }))
}

async fn you_search_api(
    client: &reqwest::Client,
    query: &str,
    api_key: &str,
) -> Option<Vec<Value>> {
    // Docs: https://you.com/docs/api-reference/search/v1-search
    let url = "https://ydc-index.io/v1/search";
    let resp = client
        .get(url)
        .header("X-API-Key", api_key)
        .query(&[
            ("query", query),
            ("count", "6"),
            ("language", "EN"),
            ("country", "US"),
        ])
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }
    let body: serde_json::Value = resp.json().await.ok()?;
    let web = body
        .get("results")
        .and_then(|r| r.get("web"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut out = Vec::new();
    for r in web.into_iter().take(6) {
        let title = r.get("title").and_then(|x| x.as_str()).unwrap_or("");
        let url = r.get("url").and_then(|x| x.as_str()).unwrap_or("");
        let snippet = r
            .get("description")
            .and_then(|x| x.as_str())
            .or_else(|| {
                r.get("snippets")
                    .and_then(|s| s.as_array())
                    .and_then(|a| a.first())
                    .and_then(|x| x.as_str())
            })
            .unwrap_or("");
        if let Some(v) = normalize_result(title, url, snippet) {
            out.push(v);
        }
    }
    (!out.is_empty()).then_some(out)
}

async fn brave_search_api(
    client: &reqwest::Client,
    query: &str,
    api_key: &str,
) -> Option<Vec<Value>> {
    // Docs: https://brave.com/search/api/  (X-Subscription-Token header)
    let url = "https://api.search.brave.com/res/v1/web/search";
    let resp = client
        .get(url)
        .header("Accept", "application/json")
        .header("X-Subscription-Token", api_key)
        .query(&[("q", query), ("count", "6"), ("country", "us"), ("search_lang", "en")])
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }
    let body: serde_json::Value = resp.json().await.ok()?;
    let results = body
        .get("web")
        .and_then(|w| w.get("results"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut out = Vec::new();
    for r in results.into_iter().take(6) {
        let title = r.get("title").and_then(|x| x.as_str()).unwrap_or("");
        let url = r.get("url").and_then(|x| x.as_str()).unwrap_or("");
        let snippet = r
            .get("description")
            .and_then(|x| x.as_str())
            .or_else(|| r.get("snippet").and_then(|x| x.as_str()))
            .unwrap_or("");
        if let Some(v) = normalize_result(title, url, snippet) {
            out.push(v);
        }
    }
    (!out.is_empty()).then_some(out)
}

async fn serpapi_search_api(
    client: &reqwest::Client,
    query: &str,
    api_key: &str,
) -> Option<Vec<Value>> {
    // Docs: https://serpapi.com/search-api
    let url = "https://serpapi.com/search.json";
    let resp = client
        .get(url)
        .query(&[
            ("engine", "google"),
            ("q", query),
            ("num", "6"),
            ("api_key", api_key),
        ])
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }
    let body: serde_json::Value = resp.json().await.ok()?;
    let results = body
        .get("organic_results")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut out = Vec::new();
    for r in results.into_iter().take(6) {
        let title = r.get("title").and_then(|x| x.as_str()).unwrap_or("");
        let url = r
            .get("link")
            .and_then(|x| x.as_str())
            .or_else(|| r.get("url").and_then(|x| x.as_str()))
            .unwrap_or("");
        let snippet = r.get("snippet").and_then(|x| x.as_str()).unwrap_or("");
        if let Some(v) = normalize_result(title, url, snippet) {
            out.push(v);
        }
    }
    (!out.is_empty()).then_some(out)
}

async fn tavily_search_api(
    client: &reqwest::Client,
    query: &str,
    api_key: &str,
) -> Option<Vec<Value>> {
    // Docs: https://docs.tavily.com/api-reference/endpoint/search
    let url = "https://api.tavily.com/search";
    let resp = client
        .post(url)
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {api_key}"))
        .json(&serde_json::json!({
            "query": query,
            "max_results": 6,
            "search_depth": "basic",
            "include_answer": false,
            "include_raw_content": false
        }))
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }
    let body: serde_json::Value = resp.json().await.ok()?;
    let results = body
        .get("results")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut out = Vec::new();
    for r in results.into_iter().take(6) {
        let title = r.get("title").and_then(|x| x.as_str()).unwrap_or("");
        let url = r.get("url").and_then(|x| x.as_str()).unwrap_or("");
        let snippet = r
            .get("content")
            .and_then(|x| x.as_str())
            .or_else(|| r.get("snippet").and_then(|x| x.as_str()))
            .unwrap_or("");
        if let Some(v) = normalize_result(title, url, snippet) {
            out.push(v);
        }
    }
    (!out.is_empty()).then_some(out)
}
