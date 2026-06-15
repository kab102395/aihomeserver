//! Tool execution node.
//!
//! Responsibility:
//! - Take the tool call artifact produced by the executor/repair.
//! - Validate/normalize params.
//! - Invoke the concrete tool implementation via `ToolRegistry`.
//! - Store the resulting `ToolResult` into artifacts (success *and* failure).
//!
//! Why this is its own node:
//! - It is the only place in the runtime where side effects happen.
//! - It provides a single choke point for safety checks and logging.

use anyhow::Result;
use std::sync::Arc;

use crate::{
    state::{ErrorType, FailureTaxonomy, SystemState},
    tools::ToolRegistry,
};

/// Normalize a JSON value into a string array (`Vec<String>`).
///
/// Connection:
/// - Tool calls sometimes accept either a single string or an array; this helper keeps parsing
///   tolerant to minor schema drift from the LLM.
fn get_string_array(v: Option<&serde_json::Value>) -> Vec<String> {
    v.and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|q| q.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

/// Parse tool params out of an executor/repair artifact.
///
/// Connection:
/// - The executor stores the generated tool call under `output_key`.
/// - Tool execution needs only the params object to pass into the actual tool implementation.
fn parse_tool_params_from_artifact(
    artifact: Option<&serde_json::Value>,
) -> Option<serde_json::Value> {
    let artifact = artifact?;

    // Executor/repair may store tool calls as a JSON string or a structured object.
    let v: serde_json::Value = match artifact {
        serde_json::Value::String(s) => serde_json::from_str(s).ok()?,
        serde_json::Value::Object(_) => artifact.clone(),
        _ => return None,
    };

    // Common shape: {"tool":"shell","params":{...}}
    if let Some(params) = v.get("params") {
        return Some(params.clone());
    }

    // Alternate: {"tool":"shell","command":"...", ...}
    if v.is_object() {
        let mut obj = v.as_object()?.clone();
        obj.remove("tool");
        obj.remove("tool_name");
        obj.remove("name");
        return Some(serde_json::Value::Object(obj));
    }

    None
}

/// Execute the tool call for the current step and store the result into artifacts.
pub async fn run(mut state: SystemState, tools: &Arc<ToolRegistry>) -> Result<SystemState> {
    // Tool execution is driven by the current plan step’s `output_key`, which the
    // executor uses to store a tool-call JSON blob. We read that artifact, parse it,
    // and run the named tool with the derived params.
    let plan = match state.current_plan.clone() {
        Some(p) => p,
        None => return Ok(state),
    };

    if state.current_step == 0 || state.current_step > plan.steps.len() {
        return Ok(state);
    }

    let step = plan.steps[state.current_step - 1].clone();

    // No tool binding on this step — executor handled it via LLM only
    let tool_name = match step.tool_binding {
        Some(ref t) => t.clone(),
        None => return Ok(state),
    };

    // The executor placed a tool call JSON in artifacts under output_key
    let output_key = step
        .output_key
        .clone()
        .unwrap_or_else(|| format!("step_{}", state.current_step));

    let executor_tool_call = state.artifacts.get(&output_key);
    let parsed_tool_params = parse_tool_params_from_artifact(executor_tool_call);

    // Prefer executor-generated tool params (dynamic, based on artifacts). If the executor
    // produced *invalid* tool-call JSON, fail fast so Repair can regenerate a valid call
    // instead of accidentally executing stale planner params (often wrong OS/shell syntax).
    let tool_params = {
        let mut p = match parsed_tool_params {
            Some(v) => v,
            None => {
                // If the executor produced something but we can't parse it, treat this as an LLM failure.
                if executor_tool_call.is_some() {
                    let tool_output_key = format!("{output_key}_result");
                    let raw = executor_tool_call
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    let result = crate::state::ToolResult::err(
                        ErrorType::Llm,
                        "invalid_tool_call",
                        &format!("Executor produced invalid tool call JSON for tool '{tool_name}': {raw}"),
                    );
                    state.failure_count += 1;
                    state.apply_tool_result(&result, &tool_output_key);
                    if let Some(tx) = &state.sse_tx {
                        let _ = tx.send(crate::state::SseEvent::ToolDone {
                            step: state.current_step,
                            tool: tool_name.clone(),
                            success: false,
                        });
                    }
                    return Ok(state);
                }

                // Compatibility fallback: if the executor didn't emit a tool call at all,
                // use planner-provided input_params.
                step.input_params.clone()
            }
        };

        // Web search tools: auto-fill missing required params so we fail less often when the
        // tool-call generator produces incomplete JSON.
        if tool_name == "parallel_search" {
            let has_queries = p.get("queries").and_then(|v| v.as_array()).is_some();
            let has_query = p
                .get("query")
                .and_then(|v| v.as_str())
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);
            if !has_queries && !has_query {
                // Prefer planner-provided queries, else fall back to the user request.
                let mut qs = get_string_array(step.input_params.get("queries"));
                if qs.is_empty() {
                    if let Some(q) = step.input_params.get("query").and_then(|v| v.as_str()) {
                        let q = q.trim();
                        if !q.is_empty() {
                            qs.push(q.to_string());
                        }
                    }
                }
                if qs.is_empty() {
                    qs.push(state.user_request.clone());
                }
                if let Some(obj) = p.as_object_mut() {
                    obj.insert(
                        "queries".into(),
                        serde_json::Value::Array(
                            qs.into_iter().map(serde_json::Value::String).collect(),
                        ),
                    );
                }
            }
        }

        if tool_name == "web_search" {
            let has_query = p
                .get("query")
                .and_then(|v| v.as_str())
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);
            if !has_query {
                let q = step
                    .input_params
                    .get("query")
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| state.user_request.clone());
                if let Some(obj) = p.as_object_mut() {
                    obj.insert("query".into(), serde_json::Value::String(q));
                }
            }
        }

        // Shell: inject workspace as cwd if missing
        let is_shell = tool_name == "shell"
            || tool_name == "run_command"
            || tool_name == "bash"
            || tool_name == "execute_command";
        if is_shell && p.get("cwd").and_then(|v| v.as_str()).is_none() {
            if let Some(obj) = p.as_object_mut() {
                obj.insert(
                    "cwd".into(),
                    serde_json::Value::String(state.workspace_path.clone()),
                );
                obj.insert(
                    "task_id".into(),
                    serde_json::Value::String(state.task_id.to_string()),
                );
            }
        }

        if is_shell {
            if let Some(obj) = p.as_object_mut() {
                if obj.get("workspace_root").and_then(|v| v.as_str()).is_none() {
                    obj.insert(
                        "workspace_root".into(),
                        serde_json::Value::String(state.workspace_path.clone()),
                    );
                }

                if obj
                    .get("collect_paths")
                    .and_then(|v| v.as_array())
                    .is_none()
                {
                    if let Some(manifest) = &state.execution_manifest {
                        let paths: Vec<serde_json::Value> = manifest
                            .expected_artifacts
                            .iter()
                            .cloned()
                            .map(serde_json::Value::String)
                            .collect();
                        if !paths.is_empty() {
                            obj.insert("collect_paths".into(), serde_json::Value::Array(paths));
                        }
                    }
                }
            }
        }

        if tool_name == "browser" {
            if let Some(obj) = p.as_object_mut() {
                if obj.get("action").and_then(|v| v.as_str()).is_none() {
                    obj.insert("action".into(), serde_json::Value::String("fetch".into()));
                }
                if obj.get("max_chars").and_then(|v| v.as_u64()).is_none() {
                    obj.insert(
                        "max_chars".into(),
                        serde_json::Value::Number(serde_json::Number::from(12000u64)),
                    );
                }
            }
        }

        // Filesystem: tolerate minor schema drift (models sometimes omit `path` or use `file`/`filename`).
        if tool_name == "filesystem" {
            let has_path = p
                .get("path")
                .and_then(|v| v.as_str())
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);
            let has_action = p
                .get("action")
                .and_then(|v| v.as_str())
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);

            if let Some(obj) = p.as_object_mut() {
                if !has_action {
                    // Prefer planner-provided action.
                    if let Some(a) = step.input_params.get("action").and_then(|v| v.as_str()) {
                        if !a.trim().is_empty() {
                            obj.insert("action".into(), serde_json::Value::String(a.trim().into()));
                        }
                    }
                }

                if !has_path {
                    // Common alternate param names.
                    let mut path = obj
                        .get("file")
                        .and_then(|v| v.as_str())
                        .or_else(|| obj.get("filename").and_then(|v| v.as_str()))
                        .or_else(|| obj.get("filepath").and_then(|v| v.as_str()))
                        .or_else(|| obj.get("file_path").and_then(|v| v.as_str()))
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty());

                    // Planner fallback.
                    if path.is_none() {
                        path = step
                            .input_params
                            .get("path")
                            .and_then(|v| v.as_str())
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty());
                    }

                    // Heuristic: pull a likely path from the step action text.
                    if path.is_none() {
                        let a = step.action.to_lowercase();
                        for cand in ["cargo.toml", "src/main.rs", "src\\main.rs", "readme.md"] {
                            if a.contains(cand) {
                                path = Some(cand.replace('\\', "/"));
                                break;
                            }
                        }
                    }

                    if let Some(pth) = path {
                        obj.insert("path".into(), serde_json::Value::String(pth));
                    }
                }
            }
        }

        // http_fetch: resolve URL from search artifacts if the planner left a placeholder
        // or if the URL is missing/invalid. This avoids the LLM guessing wrong URLs.
        if tool_name == "http_fetch" {
            let url = p
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let needs_resolution =
                url.is_empty() || url == "FROM_SEARCH_RESULTS" || !url.starts_with("http");

            if needs_resolution {
                // Deterministic special-cases: some research questions have stable canonical sources.
                // This avoids the URL picker grabbing irrelevant docs pages (e.g. std::thread::spawn).
                let action_lower = step.action.to_lowercase();
                if action_lower.contains("rust official stable release notes index") {
                    if let Some(obj) = p.as_object_mut() {
                        obj.insert(
                            "url".into(),
                            serde_json::Value::String(
                                "https://doc.rust-lang.org/stable/releases.html".into(),
                            ),
                        );
                    }
                } else if action_lower.contains("rust official blog release announcements") {
                    if let Some(obj) = p.as_object_mut() {
                        obj.insert(
                            "url".into(),
                            serde_json::Value::String(
                                "https://blog.rust-lang.org/releases/".into(),
                            ),
                        );
                    }
                }

                // If we filled a deterministic URL above, skip pick_best_url.
                let url_now = p
                    .get("url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if url_now.starts_with("http") {
                    // continue to defaults (allow_reddit_fallback/max_chars) below
                } else {
                    // Avoid re-fetching the same page across multiple http_fetch steps.
                    let mut exclude: std::collections::HashSet<String> =
                        std::collections::HashSet::new();
                    for (k, v) in &state.artifacts {
                        if !k.ends_with("_result") {
                            continue;
                        }
                        // Tool results are stored as the raw output object.
                        // Older code wrapped outputs under {output:{...}}; support both shapes.
                        let url = v
                            .get("output")
                            .and_then(|o| o.get("url"))
                            .or_else(|| v.get("url"))
                            .and_then(|x| x.as_str());
                        if let Some(u) = url {
                            exclude.insert(crate::memory::sources::normalize_url(u));
                        }
                    }

                    // Cross-run de-dupe: also exclude URLs fetched recently in past tasks.
                    if let Some(arr) = state
                        .capabilities
                        .get("recent_source_urls_normalized")
                        .and_then(|v| v.as_array())
                    {
                        for item in arr.iter().filter_map(|v| v.as_str()) {
                            let t = item.trim();
                            if !t.is_empty() {
                                exclude.insert(t.to_string());
                            }
                        }
                    }

                    let hint_for_pick = if url.is_empty() || url == "FROM_SEARCH_RESULTS" {
                        state.user_request.as_str()
                    } else {
                        url.as_str()
                    };

                    if let Some(best_url) =
                        pick_best_url_from_artifacts(&state.artifacts, hint_for_pick, &exclude)
                    {
                        // Strict gating: if we are resolving a blank/placeholder URL, only accept
                        // URLs that look like "real pages" (not function docs mistakenly matching
                        // a token) when the step action implies an official/source fetch.
                        let action_lower = step.action.to_lowercase();
                        let wants_official = action_lower.contains("official")
                            || action_lower.contains("release notes")
                            || action_lower.contains("changelog");
                        if wants_official {
                            let u = best_url.to_lowercase();
                            let ok = if state.user_request.to_lowercase().contains("rust") {
                                u.contains("doc.rust-lang.org/")
                                    || u.contains("blog.rust-lang.org/")
                            } else {
                                // Generic heuristic: avoid deep stdlib pages / API reference pages when the user asked for release notes.
                                !u.contains("/std/") && !u.contains("/api/")
                            };
                            if !ok {
                                // Force the tool to error rather than fetching nonsense.
                                if let Some(obj) = p.as_object_mut() {
                                    obj.insert(
                                        "url".into(),
                                        serde_json::Value::String(String::new()),
                                    );
                                }
                            } else if let Some(obj) = p.as_object_mut() {
                                obj.insert("url".into(), serde_json::Value::String(best_url));
                            }
                        } else if let Some(obj) = p.as_object_mut() {
                            obj.insert("url".into(), serde_json::Value::String(best_url));
                        }
                    } else {
                        // Final fallback for Rust release-note requests.
                        let hint_lower = hint_for_pick.to_lowercase();
                        if hint_lower.contains("rust")
                            && (hint_lower.contains("release") || hint_lower.contains("changelog"))
                        {
                            if let Some(obj) = p.as_object_mut() {
                                obj.insert(
                                    "url".into(),
                                    serde_json::Value::String(
                                        "https://doc.rust-lang.org/stable/releases.html".into(),
                                    ),
                                );
                            }
                        }
                    }
                }
            }

            // Defaults for research: allow fallback and return more text.
            if let Some(obj) = p.as_object_mut() {
                if obj
                    .get("allow_reddit_fallback")
                    .and_then(|v| v.as_bool())
                    .is_none()
                {
                    obj.insert(
                        "allow_reddit_fallback".into(),
                        serde_json::Value::Bool(true),
                    );
                }
                if obj.get("max_chars").and_then(|v| v.as_u64()).is_none() {
                    obj.insert(
                        "max_chars".into(),
                        serde_json::Value::Number(serde_json::Number::from(18000u64)),
                    );
                }
            }
        }

        // save_knowledge: auto-fill missing topic/content so KB updates reliably even when the
        // tool-call generator forgets required fields.
        if tool_name == "save_knowledge" {
            // If chapters were provided, we don't need to auto-fill topic/content.
            let has_chapters = p.get("chapters").and_then(|v| v.as_array()).is_some();
            let is_auto = p.get("auto").and_then(|v| v.as_bool()).unwrap_or(false);
            let chapters_from = p
                .get("chapters_from")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string());

            let has_topic = p
                .get("topic")
                .and_then(|v| v.as_str())
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);
            let has_content = p
                .get("content")
                .and_then(|v| v.as_str())
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);

            if let Some(obj) = p.as_object_mut() {
                // Resolve chapters from an earlier textbook synthesis artifact.
                if !has_chapters {
                    if let Some(src) = &chapters_from {
                        if src == "kb_textbook" {
                            if let Some(v) = state.artifacts.get("kb_textbook") {
                                let parsed: Option<serde_json::Value> = match v {
                                    serde_json::Value::String(s) => serde_json::from_str(s).ok(),
                                    serde_json::Value::Object(_) => Some(v.clone()),
                                    _ => None,
                                };
                                if let Some(book) = parsed {
                                    if let Some(chaps) =
                                        book.get("chapters").and_then(|c| c.as_array())
                                    {
                                        obj.insert(
                                            "chapters".into(),
                                            serde_json::Value::Array(chaps.clone()),
                                        );
                                    }
                                }
                            }
                        }
                    }
                }

                if !has_chapters && !has_topic {
                    let topic = state
                        .user_request
                        .lines()
                        .next()
                        .unwrap_or("")
                        .trim()
                        .chars()
                        .take(80)
                        .collect::<String>();
                    if !topic.is_empty() {
                        obj.insert("topic".into(), serde_json::Value::String(topic));
                    }
                }

                if !has_chapters && !has_content {
                    // Prefer a synthesized answer artifact if present.
                    let mut content: Option<String> = state
                        .artifacts
                        .get("answer")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());

                    // Otherwise, pick the first "large" string artifact that isn't a tool result.
                    if content.as_deref().map(|s| s.trim().len()).unwrap_or(0) < 200 {
                        for (k, v) in &state.artifacts {
                            if k.ends_with("_result") || k.starts_with("repair_") {
                                continue;
                            }
                            if let Some(s) = v.as_str() {
                                let t = s.trim();
                                if t.len() >= 200 {
                                    content = Some(t.to_string());
                                    break;
                                }
                            }
                        }
                    }

                    // Last resort: store the request itself (better than failing the tool call).
                    let content = content.unwrap_or_else(|| state.user_request.clone());
                    obj.insert("content".into(), serde_json::Value::String(content));
                }

                // If this is an auto-save, pass through the configured minimum size threshold
                // so the save_knowledge tool can decide to skip without persisting.
                if is_auto && obj.get("min_chars").and_then(|v| v.as_u64()).is_none() {
                    if let Some(min) = state
                        .capabilities
                        .get("auto_kb_min_chars")
                        .and_then(|v| v.as_u64())
                    {
                        obj.insert(
                            "min_chars".into(),
                            serde_json::Value::Number(serde_json::Number::from(min)),
                        );
                    }
                }

                // If sources weren't provided, collect URLs from search/fetch artifacts.
                let has_sources = obj
                    .get("sources")
                    .map(|v| {
                        v.as_array()
                            .map(|a| !a.is_empty())
                            .or_else(|| v.as_str().map(|s| !s.trim().is_empty()))
                            .unwrap_or(false)
                    })
                    .unwrap_or(false);
                if !has_sources {
                    let mut urls: Vec<String> = Vec::new();
                    for (k, v) in &state.artifacts {
                        if !k.ends_with("_result") {
                            continue;
                        }
                        let out = v.get("output").unwrap_or(v);
                        if let Some(u) = out.get("url").and_then(|x| x.as_str()) {
                            urls.push(u.to_string());
                        }
                        if let Some(results) = out.get("results").and_then(|x| x.as_array()) {
                            for r in results.iter().take(10) {
                                if let Some(u) = r.get("url").and_then(|x| x.as_str()) {
                                    urls.push(u.to_string());
                                }
                            }
                        }
                    }
                    urls.sort();
                    urls.dedup();
                    obj.insert(
                        "sources".into(),
                        serde_json::Value::Array(
                            urls.into_iter().map(serde_json::Value::String).collect(),
                        ),
                    );
                }
            }
        }

        p
    };

    state.log_meta(
        "tool_execution",
        &format!("Running {tool_name}:{}", step.action),
        serde_json::json!({ "step_id": step.step_id }),
    );

    // If tool doesn't exist in registry, treat as a planner error — skip without failure
    if !tools.has(&tool_name) {
        state.log_meta(
            "tool_skip",
            &format!("Unknown tool '{tool_name}' — treating as LLM-only step"),
            serde_json::json!({ "step_id": step.step_id }),
        );

        state.failure_count += 1;
        let tool_output_key = format!("{output_key}_result");
        let result = crate::state::ToolResult::err(
            ErrorType::Env,
            "tool_not_available",
            &format!("Unknown tool '{tool_name}' (planner bound a tool not in registry)"),
        );
        state.apply_tool_result(&result, &tool_output_key);
        if let Some(tx) = &state.sse_tx {
            let _ = tx.send(crate::state::SseEvent::ToolDone {
                step: state.current_step,
                tool: tool_name.clone(),
                success: false,
            });
        }
        return Ok(state);
    }

    if let Some(tx) = &state.sse_tx {
        // Build a human-readable detail for tools that hit external resources
        let detail = match tool_name.as_str() {
            "http_fetch" => tool_params
                .get("url")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            "web_search" => tool_params
                .get("query")
                .and_then(|v| v.as_str())
                .map(|s| format!("🔍 {s}")),
            "parallel_search" => tool_params
                .get("queries")
                .and_then(|v| v.as_array())
                .map(|qs| {
                    let labels: Vec<String> = qs
                        .iter()
                        .filter_map(|q| q.as_str())
                        .take(4)
                        .map(|q| format!("🔍 {q}"))
                        .collect();
                    labels.join("\n")
                }),
            "browser" => tool_params
                .get("url")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            _ => None,
        };
        let _ = tx.send(crate::state::SseEvent::ToolCall {
            step: state.current_step,
            tool: tool_name.clone(),
            action: step.action.clone(),
            detail,
        });
    }

    let is_shell_tool = tool_name == "shell"
        || tool_name == "run_command"
        || tool_name == "bash"
        || tool_name == "execute_command";

    // Emit the command that will be run (before execution so it appears immediately)
    if is_shell_tool {
        if let Some(tx) = &state.sse_tx {
            let cmd = tool_params
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("(unknown command)")
                .to_string();
            // Show the effective cwd: caller-supplied or workspace default
            let cwd = tool_params
                .get("cwd")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .or_else(|| Some(state.workspace_path.clone()));
            let _ = tx.send(crate::state::SseEvent::TerminalCmd {
                step: state.current_step,
                command: cmd,
                cwd,
            });
        }
    }

    let result = tools.execute(&tool_name, tool_params).await;
    let tool_output_key = format!("{output_key}_result");

    // Emit terminal events when the shell tool runs
    if is_shell_tool {
        // Emit the output
        if let Some(tx) = &state.sse_tx {
            let (stdout, stderr, exit_code) = if let Some(output) = &result.output {
                let stdout = output
                    .get("stdout")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let stderr = output
                    .get("stderr")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let exit_code = output
                    .get("exit_code")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0) as i32;
                (stdout, stderr, exit_code)
            } else {
                (String::new(), result.trace.clone().unwrap_or_default(), -1)
            };
            let _ = tx.send(crate::state::SseEvent::TerminalOut {
                step: state.current_step,
                stdout,
                stderr,
                exit_code,
                success: result.success,
            });
        }
    }

    // Emit FileWritten so the UI can auto-refresh the editor if that file is open
    if result.success && (tool_name == "filesystem" || tool_name == "write_file") {
        let wrote_path = result
            .output
            .as_ref()
            .and_then(|o| o.get("path"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        if let (Some(path), Some(tx)) = (wrote_path, &state.sse_tx) {
            // Convert absolute path to relative if it starts with workspace root
            let rel = path
                .strip_prefix(&state.workspace_path)
                .unwrap_or(&path)
                .trim_start_matches(['/', '\\'])
                .replace('\\', "/");
            let _ = tx.send(crate::state::SseEvent::FileWritten { path: rel });
        }
    }

    if result.success {
        state.log_meta(
            "tool_success",
            &format!("{tool_name} completed"),
            serde_json::json!({ "output_key": tool_output_key }),
        );
    } else {
        state.failure_count += 1;
        state.log_meta(
            "tool_failure",
            result.error_code.as_deref().unwrap_or("unknown"),
            serde_json::json!({
                "error_type": result.error_type,
                "trace": result.trace,
            }),
        );
        if let Some(taxonomy) = map_error(&result.error_type) {
            state.failure_taxonomy.push(taxonomy);
        }
    }

    state.apply_tool_result(&result, &tool_output_key);

    if let Some(tx) = &state.sse_tx {
        let _ = tx.send(crate::state::SseEvent::ToolDone {
            step: state.current_step,
            tool: tool_name.clone(),
            success: result.success,
        });
    }

    Ok(state)
}

/// Scan all search result artifacts and return the best URL to fetch.
/// Prefers high-signal sources and avoids obvious search/redirect pages.
fn pick_best_url_from_artifacts(
    artifacts: &std::collections::HashMap<String, serde_json::Value>,
    hint: &str,
    exclude: &std::collections::HashSet<String>,
) -> Option<String> {
    // Collect all URLs from every search result artifact
    let mut candidates: Vec<String> = Vec::new();
    for (key, val) in artifacts {
        // Only look at search result artifacts
        if !key.ends_with("_result") {
            continue;
        }
        let results = val
            .get("output")
            .and_then(|o| o.get("results"))
            .or_else(|| val.get("results"))
            .and_then(|r| r.as_array());
        if let Some(arr) = results {
            for item in arr {
                if let Some(url) = item.get("url").and_then(|u| u.as_str()) {
                    if url.starts_with("http") {
                        candidates.push(url.to_string());
                    }
                }
            }
        }
    }

    if candidates.is_empty() {
        return None;
    }

    let hint_lower = hint.to_lowercase();
    let wants_bug = hint_lower.contains("bug") || hint_lower.contains("issue");
    let wants_build = hint_lower.contains("build")
        || hint_lower.contains("playstyle")
        || hint_lower.contains("guide")
        || hint_lower.contains("skill")
        || hint_lower.contains("item")
        || hint_lower.contains("ability");
    let wants_rust = hint_lower.contains("rust");
    let wants_release_notes = hint_lower.contains("release")
        || hint_lower.contains("changelog")
        || hint_lower.contains("what changed")
        || hint_lower.contains("what's new")
        || hint_lower.contains("recent version")
        || hint_lower.contains("latest version")
        || hint_lower.contains("releases");

    /// Normalize a URL and extract its host domain for de-duplication/exclusion.
    ///
    /// Connection:
    /// - `exclude` is typically a set of previously-fetched URLs; comparing by domain reduces
    ///   repeated fetches from the same site when iterating through candidates.
    fn domain_of(url: &str) -> String {
        let n = crate::memory::sources::normalize_url(url);
        crate::memory::sources::domain_from_url(&n)
    }

    let excluded_domains: std::collections::HashSet<String> = exclude
        .iter()
        .map(|u| domain_of(u))
        .filter(|d| !d.is_empty())
        .collect();

    // Score URLs — higher = better
    let score = |url: &str| -> i32 {
        let u = url.to_lowercase();
        // Immediately disqualify redirect/search pages
        if u.contains("duckduckgo.com") {
            return -100;
        }
        if u.contains("google.com/search") {
            return -100;
        }
        if u.contains("bing.com/search") {
            return -100;
        }

        // Prefer high-signal sources in general
        if wants_rust {
            // For Rust "what changed / release notes" topics, strongly prefer official rust-lang sources
            // and avoid irrelevant "Rust docker image" pages that frequently appear in searches.
            if wants_release_notes {
                if u.contains("hub.docker.com") || u.contains("docker.com/layers") {
                    return -80;
                }
                if u.contains("doc.rust-lang.org/stable/releases.html") {
                    return 40;
                }
                if u.contains("blog.rust-lang.org")
                    && (u.contains("announcing-rust") || u.contains("/releases"))
                {
                    return 36;
                }
                if u.contains("blog.rust-lang.org") {
                    return 30;
                }
                if u.contains("doc.rust-lang.org/") {
                    return 28;
                }
                // Community sources are fine as tertiary references, but should not win over official docs.
                if u.contains("medium.com") {
                    return 4;
                }
                if u.contains("reddit.com") {
                    return 3;
                }
            }
            if u.contains("doc.rust-lang.org/book") || u.contains("doc.rust-lang.org/stable/book") {
                return 30;
            }
            if u.contains("doc.rust-lang.org/reference") {
                return 29;
            }
            if u.contains("doc.rust-lang.org/rust-by-example") {
                return 28;
            }
            if u.contains("doc.rust-lang.org/") {
                return 24;
            }
        }
        if u.contains("github.com") {
            return 12;
        }
        if u.contains("readthedocs.io") {
            return 11;
        }
        if u.contains("docs.") {
            return 10;
        }

        // If the user wants builds/meta, prioritize build sites over bug trackers.
        if wants_build {
            if u.contains("dota2protracker") {
                return 14;
            }
            if u.contains("dotacoach") {
                return 13;
            }
            if u.contains("opendota.com") {
                return 12;
            }
            if u.contains("stratz.com") {
                return 11;
            }
            if u.contains("dotabuff.com") {
                return 11;
            }
            if u.contains("dota2.com/patches") {
                return 10;
            }
            if u.contains("liquipedia.net") {
                return 10;
            }
            // Downweight bug trackers for build-oriented questions
            if u.contains("github.com/valvesoftware/dota2-gameplay/issues") {
                return 3;
            }
        }

        // If the user wants bugs/issues, do the opposite.
        if wants_bug {
            if u.contains("github.com/valvesoftware/dota2-gameplay/issues") {
                return 14;
            }
            if u.contains("old.reddit.com") || u.contains("reddit.com") {
                return 13;
            }
        }

        // Prefer these in order
        if u.contains("old.reddit.com") {
            return 10;
        }
        if u.contains("reddit.com") {
            return 9;
        } // will be rewritten to old.reddit
        if u.contains("steamcommunity.com") {
            return 8;
        }
        if u.contains("dota2.com") {
            return 8;
        }
        if u.contains("dota2protracker") {
            return 7;
        }
        if u.contains("liquipedia.net") {
            return 6;
        }
        if u.contains("fandom.com") {
            return 5;
        }
        if u.contains("dotabuff.com") {
            return 4;
        }
        if u.contains("stratz.com") {
            return 3;
        }
        1 // anything else
    };

    // If hint contains a keyword, give a small bonus to URLs containing it
    let hint_word = hint.split('/').last().unwrap_or("").to_lowercase();

    let score_key = |url: &&String| {
        let mut s = score(url);
        if !hint_word.is_empty() && url.to_lowercase().contains(&hint_word) {
            s += 1;
        }
        // Mild penalty to encourage domain diversity across multiple fetches.
        let d = domain_of(url);
        if !d.is_empty() && excluded_domains.contains(&d) {
            s -= 2;
        }
        s
    };

    // Prefer not-yet-fetched URLs, but never fail outright if everything is excluded.
    let best_new = candidates
        .iter()
        .filter(|u| !exclude.contains(&crate::memory::sources::normalize_url(u)))
        .max_by_key(score_key)
        .cloned();

    if best_new.is_some() {
        return best_new;
    }

    candidates.iter().max_by_key(score_key).cloned()
}

/// Map low-level tool/LLM error types into a coarse taxonomy for analytics and repair strategy.
///
/// Connection:
/// - The orchestrator keeps `failure_taxonomy` so replan/repair decisions can evolve over time.
fn map_error(e: &ErrorType) -> Option<FailureTaxonomy> {
    match e {
        ErrorType::Tool => Some(FailureTaxonomy::ToolFailure),
        ErrorType::Env => Some(FailureTaxonomy::EnvFailure),
        ErrorType::Timeout => Some(FailureTaxonomy::Timeout),
        ErrorType::Permission => Some(FailureTaxonomy::PermissionError),
        ErrorType::Llm => Some(FailureTaxonomy::LogicError),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tool_params_accepts_json_string_with_params() {
        let v = serde_json::Value::String(
            r#"{"tool":"shell","params":{"command":"pwd","timeout_secs":5}}"#.into(),
        );
        let p = parse_tool_params_from_artifact(Some(&v)).expect("params parsed");
        assert_eq!(p.get("command").and_then(|x| x.as_str()), Some("pwd"));
        assert_eq!(p.get("timeout_secs").and_then(|x| x.as_i64()), Some(5));
    }

    #[test]
    fn parse_tool_params_accepts_object_shape() {
        let v = serde_json::json!({ "tool": "shell", "command": "pwd", "timeout_secs": 5 });
        let p = parse_tool_params_from_artifact(Some(&v)).expect("params parsed");
        assert_eq!(p.get("command").and_then(|x| x.as_str()), Some("pwd"));
        assert_eq!(p.get("timeout_secs").and_then(|x| x.as_i64()), Some(5));
        assert!(p.get("tool").is_none());
    }

    #[test]
    fn url_picker_prefers_build_sources_when_hint_is_build() {
        let artifacts: std::collections::HashMap<String, serde_json::Value> = [
            (
                "search_result_result".to_string(),
                serde_json::json!({
                    "output": {
                        "results": [
                            {"url":"https://github.com/ValveSoftware/Dota2-Gameplay/issues/29000"},
                            {"url":"https://www.reddit.com/r/DotA2/comments/1rfo4id/kez_bug_that_has_been_around_since_october_last/"},
                            {"url":"https://dota2protracker.com/hero/Kez"}
                        ]
                    }
                }),
            ),
        ]
        .into_iter()
        .collect();

        let exclude = std::collections::HashSet::<String>::new();
        let picked =
            pick_best_url_from_artifacts(&artifacts, "Kez Dota 2 build guide playstyle", &exclude)
                .expect("picked url");
        assert!(picked.contains("dota2protracker.com"), "picked={picked}");
    }

    #[test]
    fn url_picker_prefers_bug_trackers_when_hint_is_bug() {
        let artifacts: std::collections::HashMap<String, serde_json::Value> = [(
            "search_result_result".to_string(),
            serde_json::json!({
                "output": {
                    "results": [
                        {"url":"https://dota2protracker.com/hero/Kez"},
                        {"url":"https://github.com/ValveSoftware/Dota2-Gameplay/issues/29000"}
                    ]
                }
            }),
        )]
        .into_iter()
        .collect();

        let exclude = std::collections::HashSet::<String>::new();
        let picked =
            pick_best_url_from_artifacts(&artifacts, "Kez aghanim bug issue report", &exclude)
                .expect("picked url");
        assert!(
            picked.contains("github.com/ValveSoftware/Dota2-Gameplay/issues"),
            "picked={picked}"
        );
    }

    #[test]
    fn url_picker_avoids_dockerhub_for_rust_release_notes() {
        let artifacts: std::collections::HashMap<String, serde_json::Value> = [(
            "search_result_result".to_string(),
            serde_json::json!({
                "output": {
                    "results": [
                        {"url":"https://hub.docker.com/layers/library/rust/1.86.0-alpine/images/sha256-deadbeef"},
                        {"url":"https://doc.rust-lang.org/stable/releases.html"},
                        {"url":"https://blog.rust-lang.org/2026/01/01/Announcing-Rust-1.99.0.html"}
                    ]
                }
            }),
        )]
        .into_iter()
        .collect();

        let exclude = std::collections::HashSet::<String>::new();
        let picked = pick_best_url_from_artifacts(
            &artifacts,
            "latest Rust release notes what changed",
            &exclude,
        )
        .expect("picked url");
        assert!(
            picked.contains("doc.rust-lang.org") || picked.contains("blog.rust-lang.org"),
            "picked={picked}"
        );
        assert!(!picked.contains("hub.docker.com"), "picked={picked}");
    }
}
