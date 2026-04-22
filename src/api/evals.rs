use async_trait::async_trait;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Instant;
use uuid::Uuid;

use super::server::AppState;

#[derive(Debug, Clone, Serialize)]
pub struct EvalCaseInfo {
    pub id: String,
    pub description: String,
    pub tags: Vec<String>,
    pub quick: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EvalRunRequest {
    /// Optional explicit list of case IDs. If omitted, uses `mode`.
    #[serde(default)]
    pub cases: Vec<String>,
    /// quick: fast, mostly-offline checks
    /// full: includes network and LLM checks
    #[serde(default)]
    pub mode: EvalMode,
    /// Per-case timeout. Applies only to cases that explicitly support it.
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvalMode {
    Quick,
    Full,
}

impl Default for EvalMode {
    fn default() -> Self {
        EvalMode::Quick
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct EvalRunResponse {
    pub ok: bool,
    pub duration_ms: u128,
    pub results: Vec<EvalCaseResult>,
    pub summary: EvalSummary,
}

// (helpers intentionally omitted; keep EvalRunResponse as a pure transport type)

#[derive(Debug, Clone, Serialize)]
pub struct EvalSummary {
    pub passed: u32,
    pub failed: u32,
    pub skipped: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct EvalCaseResult {
    pub id: String,
    pub ok: bool,
    pub skipped: bool,
    pub duration_ms: u128,
    pub detail: serde_json::Value,
}

#[async_trait]
trait EvalCase: Send + Sync {
    fn info(&self) -> EvalCaseInfo;
    async fn run(&self, app: &AppState, timeout_secs: Option<u64>) -> EvalCaseResult;
}

fn all_cases() -> Vec<Box<dyn EvalCase>> {
    vec![
        Box::new(WorkspaceExistsCase),
        Box::new(FilesystemRoundtripCase),
        Box::new(FilesystemFindGrepCase),
        Box::new(ShellEchoCase),
        Box::new(ShellSyntaxGuardCase),
        Box::new(HttpFetchExampleCase),
        Box::new(WebSearchCase),
        Box::new(ParallelSearchCase),
        Box::new(GroundingContractCase),
        Box::new(LlmChatJsonCase),
        Box::new(LlmEmbedCase),
    ]
}

pub fn list_case_infos() -> Vec<EvalCaseInfo> {
    all_cases().into_iter().map(|c| c.info()).collect()
}

pub async fn run_eval(app: &AppState, req: EvalRunRequest) -> (StatusCode, EvalRunResponse) {
    let start = Instant::now();

    let cases = all_cases();
    let selected: Vec<String> = if !req.cases.is_empty() {
        req.cases
    } else {
        match req.mode {
            EvalMode::Quick => cases
                .iter()
                .map(|c| c.info())
                .filter(|i| i.quick)
                .map(|i| i.id)
                .collect(),
            EvalMode::Full => cases.iter().map(|c| c.info().id).collect(),
        }
    };

    let mut results: Vec<EvalCaseResult> = Vec::new();
    for id in selected {
        if let Some(case) = cases.iter().find(|c| c.info().id == id) {
            results.push(case.run(app, req.timeout_secs).await);
        } else {
            results.push(EvalCaseResult {
                id,
                ok: false,
                skipped: false,
                duration_ms: 0,
                detail: json!({ "error": "unknown_case" }),
            });
        }
    }

    let summary = {
        let mut passed = 0u32;
        let mut failed = 0u32;
        let mut skipped = 0u32;
        for r in &results {
            if r.skipped {
                skipped += 1;
            } else if r.ok {
                passed += 1;
            } else {
                failed += 1;
            }
        }
        EvalSummary { passed, failed, skipped }
    };

    let ok = summary.failed == 0;
    let resp = EvalRunResponse {
        ok,
        duration_ms: start.elapsed().as_millis(),
        results,
        summary,
    };

    app.metrics.record_eval_run(ok, start.elapsed());

    let code = if ok { StatusCode::OK } else { StatusCode::SERVICE_UNAVAILABLE };
    (code, resp)
}

// ── Cases ──────────────────────────────────────────────────────────────────────

struct WorkspaceExistsCase;

#[async_trait]
impl EvalCase for WorkspaceExistsCase {
    fn info(&self) -> EvalCaseInfo {
        EvalCaseInfo {
            id: "workspace.exists".into(),
            description: "Configured workspace path exists".into(),
            tags: vec!["workspace".into()],
            quick: true,
        }
    }

    async fn run(&self, app: &AppState, _timeout_secs: Option<u64>) -> EvalCaseResult {
        let t0 = Instant::now();
        let ws = app.config.read().await.workspace_path.clone();
        let ok = std::path::Path::new(&ws).is_dir();
        EvalCaseResult {
            id: self.info().id,
            ok,
            skipped: false,
            duration_ms: t0.elapsed().as_millis(),
            detail: json!({ "workspace_path": ws }),
        }
    }
}

struct FilesystemRoundtripCase;

#[async_trait]
impl EvalCase for FilesystemRoundtripCase {
    fn info(&self) -> EvalCaseInfo {
        EvalCaseInfo {
            id: "tool.filesystem.roundtrip".into(),
            description: "Filesystem tool write/read/delete".into(),
            tags: vec!["tool".into(), "filesystem".into()],
            quick: true,
        }
    }

    async fn run(&self, app: &AppState, _timeout_secs: Option<u64>) -> EvalCaseResult {
        let t0 = Instant::now();
        let tools = &app.orchestrator.tools;

        let rel = format!(".eval/roundtrip_{}.txt", Uuid::new_v4());
        let write = tools
            .execute(
                "filesystem",
                json!({
                    "action": "write",
                    "path": rel,
                    "content": "eval roundtrip\n",
                    "overwrite": true
                }),
            )
            .await;
        let read = tools
            .execute("filesystem", json!({ "action": "read", "path": rel }))
            .await;
        let delete = tools
            .execute("filesystem", json!({ "action": "delete", "path": rel }))
            .await;

        let read_ok = read.success
            && read
                .output
                .as_ref()
                .and_then(|o| o.get("content"))
                .and_then(|v| v.as_str())
                .map(|s| s.contains("eval roundtrip"))
                .unwrap_or(false);

        let ok = write.success && read_ok && delete.success;

        EvalCaseResult {
            id: self.info().id,
            ok,
            skipped: false,
            duration_ms: t0.elapsed().as_millis(),
            detail: json!({
                "path": rel,
                "write": {"success": write.success, "error_code": write.error_code, "trace": write.trace, "output": write.output},
                "read":  {"success": read.success,  "error_code": read.error_code,  "trace": read.trace,  "output": read.output},
                "delete":{"success": delete.success,"error_code": delete.error_code,"trace": delete.trace,"output": delete.output},
            }),
        }
    }
}

struct FilesystemFindGrepCase;

#[async_trait]
impl EvalCase for FilesystemFindGrepCase {
    fn info(&self) -> EvalCaseInfo {
        EvalCaseInfo {
            id: "tool.filesystem.find_grep".into(),
            description: "Filesystem tool find + grep".into(),
            tags: vec!["tool".into(), "filesystem".into()],
            quick: true,
        }
    }

    async fn run(&self, app: &AppState, _timeout_secs: Option<u64>) -> EvalCaseResult {
        let t0 = Instant::now();
        let tools = &app.orchestrator.tools;

        let find = tools
            .execute(
                "filesystem",
                json!({ "action": "find", "path": ".", "pattern": "Cargo.toml" }),
            )
            .await;
        let grep = tools
            .execute(
                "filesystem",
                json!({ "action": "grep", "path": "src", "query": "ToolResult" }),
            )
            .await;

        let ok = find.success && grep.success;

        EvalCaseResult {
            id: self.info().id,
            ok,
            skipped: false,
            duration_ms: t0.elapsed().as_millis(),
            detail: json!({
                "find": {"success": find.success, "error_code": find.error_code, "trace": find.trace, "output": find.output},
                "grep": {"success": grep.success, "error_code": grep.error_code, "trace": grep.trace, "output": grep.output},
            }),
        }
    }
}

struct ShellEchoCase;

#[async_trait]
impl EvalCase for ShellEchoCase {
    fn info(&self) -> EvalCaseInfo {
        EvalCaseInfo {
            id: "tool.shell.echo".into(),
            description: "Shell tool executes a simple command".into(),
            tags: vec!["tool".into(), "shell".into()],
            quick: true,
        }
    }

    async fn run(&self, app: &AppState, timeout_secs: Option<u64>) -> EvalCaseResult {
        let t0 = Instant::now();
        let tools = &app.orchestrator.tools;
        let ws = app.config.read().await.workspace_path.clone();

        let shell = tools
            .execute(
                "shell",
                json!({
                    "command": "echo EVAL_SHELL",
                    "timeout_secs": timeout_secs.unwrap_or(10),
                    "cwd": ws
                }),
            )
            .await;

        let ok = shell.success
            && shell
                .output
                .as_ref()
                .and_then(|o| o.get("stdout"))
                .and_then(|v| v.as_str())
                .map(|s| s.contains("EVAL_SHELL"))
                .unwrap_or(false);

        EvalCaseResult {
            id: self.info().id,
            ok,
            skipped: false,
            duration_ms: t0.elapsed().as_millis(),
            detail: json!({
                "success": shell.success,
                "error_code": shell.error_code,
                "trace": shell.trace,
                "output": shell.output
            }),
        }
    }
}

struct ShellSyntaxGuardCase;

#[async_trait]
impl EvalCase for ShellSyntaxGuardCase {
    fn info(&self) -> EvalCaseInfo {
        EvalCaseInfo {
            id: "tool.shell.syntax_guard".into(),
            description: "Shell tool blocks OS/shell mismatched syntax".into(),
            tags: vec!["tool".into(), "shell".into(), "safety".into()],
            quick: true,
        }
    }

    async fn run(&self, app: &AppState, timeout_secs: Option<u64>) -> EvalCaseResult {
        let t0 = Instant::now();
        let tools = &app.orchestrator.tools;
        let ws = app.config.read().await.workspace_path.clone();

        // First run an echo to discover backend (recorded in output).
        let probe = tools
            .execute(
                "shell",
                json!({"command":"echo PROBE","timeout_secs": timeout_secs.unwrap_or(10), "cwd": ws}),
            )
            .await;
        let backend = probe
            .output
            .as_ref()
            .and_then(|o| o.get("shell_backend"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let mismatch_cmd = if backend == "sh" {
            "Select-Object -First 1"
        } else {
            "echo hi | head -n 1"
        };

        let mismatch = tools
            .execute(
                "shell",
                json!({"command": mismatch_cmd, "timeout_secs": timeout_secs.unwrap_or(10), "cwd": ws}),
            )
            .await;

        let ok = !mismatch.success && matches!(mismatch.error_code.as_deref(), Some("shell_syntax_mismatch"));

        EvalCaseResult {
            id: self.info().id,
            ok,
            skipped: false,
            duration_ms: t0.elapsed().as_millis(),
            detail: json!({
                "shell_backend": backend,
                "probe": {"success": probe.success, "output": probe.output, "error_code": probe.error_code, "trace": probe.trace},
                "mismatch": {"success": mismatch.success, "error_code": mismatch.error_code, "trace": mismatch.trace, "output": mismatch.output},
            }),
        }
    }
}

struct HttpFetchExampleCase;

#[async_trait]
impl EvalCase for HttpFetchExampleCase {
    fn info(&self) -> EvalCaseInfo {
        EvalCaseInfo {
            id: "tool.http_fetch.example".into(),
            description: "http_fetch can retrieve a small deterministic URL".into(),
            tags: vec!["tool".into(), "network".into()],
            quick: false,
        }
    }

    async fn run(&self, app: &AppState, timeout_secs: Option<u64>) -> EvalCaseResult {
        let t0 = Instant::now();
        let tools = &app.orchestrator.tools;

        let fetch = tools
            .execute(
                "http_fetch",
                json!({
                    "url": "https://example.com",
                    "max_chars": 2000,
                    "timeout_secs": timeout_secs.unwrap_or(20),
                    "allow_reddit_fallback": false
                }),
            )
            .await;

        let fetch_ok = fetch.success
            && fetch
                .output
                .as_ref()
                .and_then(|o| o.get("status"))
                .and_then(|v| v.as_u64())
                .map(|s| s >= 200 && s < 500)
                .unwrap_or(false);

        EvalCaseResult {
            id: self.info().id,
            ok: fetch_ok,
            skipped: false,
            duration_ms: t0.elapsed().as_millis(),
            detail: json!({
                "success": fetch.success,
                "error_code": fetch.error_code,
                "trace": fetch.trace,
                "output": fetch.output
            }),
        }
    }
}

struct WebSearchCase;

#[async_trait]
impl EvalCase for WebSearchCase {
    fn info(&self) -> EvalCaseInfo {
        EvalCaseInfo {
            id: "tool.web_search.basic".into(),
            description: "web_search returns results when SEARCH_URL is configured".into(),
            tags: vec!["tool".into(), "network".into(), "search".into()],
            quick: false,
        }
    }

    async fn run(&self, app: &AppState, timeout_secs: Option<u64>) -> EvalCaseResult {
        let t0 = Instant::now();
        let search_url = app.config.read().await.search_url.clone();
        if search_url.trim().is_empty() {
            return EvalCaseResult {
                id: self.info().id,
                ok: true,
                skipped: true,
                duration_ms: t0.elapsed().as_millis(),
                detail: json!({ "skipped": "SEARCH_URL not configured" }),
            };
        }

        let tools = &app.orchestrator.tools;
        let res = tools
            .execute(
                "web_search",
                json!({
                    "query": "example.com",
                    "limit": 3,
                    "timeout_secs": timeout_secs.unwrap_or(20)
                }),
            )
            .await;

        let ok = res.success
            && res
                .output
                .as_ref()
                .and_then(|o| o.get("results"))
                .and_then(|v| v.as_array())
                .map(|a| !a.is_empty())
                .unwrap_or(false);

        EvalCaseResult {
            id: self.info().id,
            ok,
            skipped: false,
            duration_ms: t0.elapsed().as_millis(),
            detail: json!({
                "search_url": search_url,
                "success": res.success,
                "error_code": res.error_code,
                "trace": res.trace,
                "output": res.output
            }),
        }
    }
}

struct ParallelSearchCase;

#[async_trait]
impl EvalCase for ParallelSearchCase {
    fn info(&self) -> EvalCaseInfo {
        EvalCaseInfo {
            id: "tool.parallel_search.basic".into(),
            description: "parallel_search returns results when SEARCH_URL is configured".into(),
            tags: vec!["tool".into(), "network".into(), "search".into()],
            quick: false,
        }
    }

    async fn run(&self, app: &AppState, timeout_secs: Option<u64>) -> EvalCaseResult {
        let t0 = Instant::now();
        let search_url = app.config.read().await.search_url.clone();
        if search_url.trim().is_empty() {
            return EvalCaseResult {
                id: self.info().id,
                ok: true,
                skipped: true,
                duration_ms: t0.elapsed().as_millis(),
                detail: json!({ "skipped": "SEARCH_URL not configured" }),
            };
        }

        let tools = &app.orchestrator.tools;
        let res = tools
            .execute(
                "parallel_search",
                json!({
                    "queries": ["example.com", "rust tool registry"],
                    "limit": 2,
                    "timeout_secs": timeout_secs.unwrap_or(20)
                }),
            )
            .await;

        let ok = res.success;

        EvalCaseResult {
            id: self.info().id,
            ok,
            skipped: false,
            duration_ms: t0.elapsed().as_millis(),
            detail: json!({
                "search_url": search_url,
                "success": res.success,
                "error_code": res.error_code,
                "trace": res.trace,
                "output": res.output
            }),
        }
    }
}

struct GroundingContractCase;

#[async_trait]
impl EvalCase for GroundingContractCase {
    fn info(&self) -> EvalCaseInfo {
        EvalCaseInfo {
            id: "grounding.contract".into(),
            description: "Planner post-processing enforces search + facts + requires_facts".into(),
            tags: vec!["planner".into(), "grounding".into()],
            quick: true,
        }
    }

    async fn run(&self, _app: &AppState, _timeout_secs: Option<u64>) -> EvalCaseResult {
        let t0 = Instant::now();

        let mut plan = crate::state::PlannerOutput {
            steps: vec![crate::state::StepDefinition {
                step_id: "1".into(),
                action: "Write patch-specific code".into(),
                tool_binding: None,
                output_format: None,
                requires_facts: false,
                input_params: json!({}),
                output_key: Some("answer".into()),
                expected_output: None,
            }],
            tools_required: vec![],
            risk_score: 1,
            expected_outputs: vec!["answer".into()],
            completion_criteria: vec!["done".into()],
            dependencies: serde_json::Value::Null,
        };

        crate::grounding::enforce_grounding_contract(
            "Research Kez, Lina, Puck, Brewmaster, Techies, and Zeus on Dota 2 patch 7.41b",
            &json!({ "search_url_configured": true }),
            &mut plan,
        );

        let has_search = plan
            .steps
            .iter()
            .any(|s| s.tool_binding.as_deref() == Some("parallel_search"));
        let has_facts = plan.steps.iter().any(|s| {
            s.output_key.as_deref() == Some("facts")
                && s.tool_binding.is_none()
                && s.output_format.as_deref() == Some("json")
        });
        let answer_requires_facts = plan
            .steps
            .iter()
            .find(|s| s.output_key.as_deref() == Some("answer"))
            .map(|s| s.requires_facts)
            .unwrap_or(false);

        let ok = has_search && has_facts && answer_requires_facts;

        EvalCaseResult {
            id: self.info().id,
            ok,
            skipped: false,
            duration_ms: t0.elapsed().as_millis(),
            detail: json!({
                "has_search": has_search,
                "has_facts": has_facts,
                "answer_requires_facts": answer_requires_facts
            }),
        }
    }
}

struct LlmChatJsonCase;

#[async_trait]
impl EvalCase for LlmChatJsonCase {
    fn info(&self) -> EvalCaseInfo {
        EvalCaseInfo {
            id: "llm.chat.json".into(),
            description: "LLM responds to a tiny JSON-mode request".into(),
            tags: vec!["llm".into()],
            quick: false,
        }
    }

    async fn run(&self, app: &AppState, _timeout_secs: Option<u64>) -> EvalCaseResult {
        let t0 = Instant::now();
        let llm = &app.orchestrator.llm;

        let messages = vec![
            crate::llm::ollama::Message::system(
                "Output ONLY valid JSON: {\"pong\":true}. No prose.",
            ),
            crate::llm::ollama::Message::user("ping"),
        ];

        let resp = llm.chat(messages, crate::llm::ollama::ModelRole::Fast, true, false).await;
        let ok = resp
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| v.get("pong").and_then(|b| b.as_bool()))
            .unwrap_or(false);

        EvalCaseResult {
            id: self.info().id,
            ok,
            skipped: false,
            duration_ms: t0.elapsed().as_millis(),
            detail: json!({ "ok": ok }),
        }
    }
}

struct LlmEmbedCase;

#[async_trait]
impl EvalCase for LlmEmbedCase {
    fn info(&self) -> EvalCaseInfo {
        EvalCaseInfo {
            id: "llm.embed".into(),
            description: "Embedding endpoint works (nomic-embed-text)".into(),
            tags: vec!["llm".into(), "embedding".into()],
            quick: false,
        }
    }

    async fn run(&self, app: &AppState, _timeout_secs: Option<u64>) -> EvalCaseResult {
        let t0 = Instant::now();
        let llm = &app.orchestrator.llm;

        let resp = llm.embed("eval").await;
        let ok = resp.as_ref().map(|v| v.len() >= 16).unwrap_or(false);

        EvalCaseResult {
            id: self.info().id,
            ok,
            skipped: false,
            duration_ms: t0.elapsed().as_millis(),
            detail: match resp {
                Ok(v) => json!({ "dims": v.len() }),
                Err(e) => json!({ "error": e.to_string() }),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_list_contains_quick_checks() {
        let ids: Vec<String> = list_case_infos().into_iter().map(|c| c.id).collect();
        assert!(ids.contains(&"workspace.exists".to_string()));
        assert!(ids.contains(&"grounding.contract".to_string()));
    }
}
