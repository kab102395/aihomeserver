use anyhow::Result;
use std::sync::Arc;

use crate::{
    state::{ErrorType, FailureTaxonomy, SystemState},
    tools::ToolRegistry,
};

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

pub async fn run(mut state: SystemState, tools: &Arc<ToolRegistry>) -> Result<SystemState> {
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
                    let raw = executor_tool_call.cloned().unwrap_or(serde_json::Value::Null);
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

        // Shell: inject workspace as cwd if missing
        let is_shell = tool_name == "shell" || tool_name == "run_command"
            || tool_name == "bash" || tool_name == "execute_command";
        if is_shell && p.get("cwd").and_then(|v| v.as_str()).is_none() {
            if let Some(obj) = p.as_object_mut() {
                obj.insert("cwd".into(), serde_json::Value::String(state.workspace_path.clone()));
            }
        }

        // http_fetch: resolve URL from search artifacts if the planner left a placeholder
        // or if the URL is missing/invalid. This avoids the LLM guessing wrong URLs.
        if tool_name == "http_fetch" {
            let url = p.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let needs_resolution = url.is_empty()
                || url == "FROM_SEARCH_RESULTS"
                || !url.starts_with("http");

            if needs_resolution {
                if let Some(best_url) = pick_best_url_from_artifacts(&state.artifacts, &url) {
                    if let Some(obj) = p.as_object_mut() {
                        obj.insert("url".into(), serde_json::Value::String(best_url));
                    }
                }
            }

            // Defaults for research: allow fallback and return more text.
            if let Some(obj) = p.as_object_mut() {
                if obj.get("allow_reddit_fallback").and_then(|v| v.as_bool()).is_none() {
                    obj.insert("allow_reddit_fallback".into(), serde_json::Value::Bool(true));
                }
                if obj.get("max_chars").and_then(|v| v.as_u64()).is_none() {
                    obj.insert(
                        "max_chars".into(),
                        serde_json::Value::Number(serde_json::Number::from(12000u64)),
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
            "http_fetch" => tool_params.get("url")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            "web_search" => tool_params.get("query")
                .and_then(|v| v.as_str())
                .map(|s| format!("🔍 {s}")),
            "parallel_search" => tool_params.get("queries")
                .and_then(|v| v.as_array())
                .map(|qs| {
                    let labels: Vec<String> = qs.iter()
                        .filter_map(|q| q.as_str())
                        .take(4)
                        .map(|q| format!("🔍 {q}"))
                        .collect();
                    labels.join("\n")
                }),
            _ => None,
        };
        let _ = tx.send(crate::state::SseEvent::ToolCall {
            step: state.current_step,
            tool: tool_name.clone(),
            action: step.action.clone(),
            detail,
        });
    }

    let is_shell_tool = tool_name == "shell" || tool_name == "run_command"
        || tool_name == "bash" || tool_name == "execute_command";

    // Emit the command that will be run (before execution so it appears immediately)
    if is_shell_tool {
        if let Some(tx) = &state.sse_tx {
            let cmd = tool_params.get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("(unknown command)")
                .to_string();
            // Show the effective cwd: caller-supplied or workspace default
            let cwd = tool_params.get("cwd")
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
                let stdout = output.get("stdout").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let stderr = output.get("stderr").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let exit_code = output.get("exit_code").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
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
        let wrote_path = result.output.as_ref()
            .and_then(|o| o.get("path"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        if let (Some(path), Some(tx)) = (wrote_path, &state.sse_tx) {
            // Convert absolute path to relative if it starts with workspace root
            let rel = path.strip_prefix(&state.workspace_path)
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
/// Prefers old.reddit.com, then liquipedia, then fandom wikis, then anything else.
/// Skips known-blocked domains (dotabuff, stratz) and DDG redirect URLs.
fn pick_best_url_from_artifacts(
    artifacts: &std::collections::HashMap<String, serde_json::Value>,
    hint: &str,
) -> Option<String> {
    // Collect all URLs from every search result artifact
    let mut candidates: Vec<String> = Vec::new();
    for (key, val) in artifacts {
        // Only look at search result artifacts
        if !key.ends_with("_result") { continue; }
        let results = val
            .get("output").and_then(|o| o.get("results"))
            .or_else(|| val.get("results"))
            .and_then(|r| r.as_array());
        if let Some(arr) = results {
            for item in arr {
                if let Some(url) = item.get("url").and_then(|u| u.as_str()) {
                    if url.starts_with("http") { candidates.push(url.to_string()); }
                }
            }
        }
    }

    if candidates.is_empty() { return None; }

    // Score URLs — higher = better
    let score = |url: &str| -> i32 {
        let u = url.to_lowercase();
        // Immediately disqualify known blockers and DDG redirects
        if u.contains("dotabuff.com")  { return -100; }
        if u.contains("stratz.com")    { return -100; }
        if u.contains("duckduckgo.com"){ return -100; }
        if u.contains("google.com/search") { return -100; }
        if u.contains("bing.com/search")   { return -100; }
        // Prefer these in order
        if u.contains("old.reddit.com")    { return 10; }
        if u.contains("reddit.com")        { return  9; }  // will be rewritten to old.reddit
        if u.contains("liquipedia.net")    { return  8; }
        if u.contains("dota2.fandom.com")  { return  7; }
        if u.contains("steamcommunity.com"){ return  6; }
        if u.contains("dota2.com")         { return  5; }
        if u.contains("dota2protracker")   { return  4; }
        1 // anything else
    };

    // If hint contains a keyword, give a small bonus to URLs containing it
    let hint_word = hint.split('/').last().unwrap_or("").to_lowercase();

    let best = candidates.iter().max_by_key(|url| {
        let mut s = score(url);
        if !hint_word.is_empty() && url.to_lowercase().contains(&hint_word) { s += 1; }
        s
    });

    best.cloned()
}

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
        let v = serde_json::Value::String(r#"{"tool":"shell","params":{"command":"pwd","timeout_secs":5}}"#.into());
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
}
