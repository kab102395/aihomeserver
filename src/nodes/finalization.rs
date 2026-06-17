//! Finalization node.
//!
//! Responsibility:
//! - Record final metadata about the run (scores, failures, produced artifacts).
//! - Ensure the event log contains a clear terminal event for replay/debugging.
//!
//! Note: this node does not “format the final answer” in a complex way; the
//! answer typically lives in an `artifacts["answer"]` string created earlier.

use crate::state::SystemState;
use crate::coder::{latest_browser_output_block, latest_verified_browser_output};
use anyhow::Result;

/// Record terminal metadata/logs for the run before returning the final `SystemState`.
pub async fn run(mut state: SystemState) -> Result<SystemState> {
    let recovered_success = should_mark_recovered_success(&state);
    if !state.termination_met && recovered_success {
        state.termination_met = true;
    }

    if (state.termination_met || recovered_success) && failure_answer_present(&state.artifacts) {
        if let Some(text) = extract_success_output_for_state(&state) {
            state
                .artifacts
                .insert("answer".to_string(), serde_json::Value::String(text));
        }
    }

    if !state.artifacts.contains_key("answer") {
        let answer = if let Some(text) = extract_success_output_for_state(&state) {
            text
        } else if state.termination_met {
            "Task completed successfully.".to_string()
        } else {
            summarize_failure(&state)
        };
        state
            .artifacts
            .insert("answer".to_string(), serde_json::Value::String(answer));
    }

    let avg_score = if state.critic_history.is_empty() {
        None
    } else {
        let sum: f32 = state.critic_history.iter().map(|r| r.score).sum();
        Some(sum / state.critic_history.len() as f32)
    };

    state.log_meta(
        "finalization",
        if state.termination_met {
            "Task completed successfully"
        } else {
            "Task ended (max steps or forced)"
        },
        serde_json::json!({
            "task_id": state.task_id,
            "steps_taken": state.current_step,
            "failure_count": state.failure_count,
            "repair_cycles": state.repair_cycle,
            "critic_reviews": state.critic_history.len(),
            "avg_critic_score": avg_score,
            "artifacts_produced": state.artifacts.keys().collect::<Vec<_>>(),
        }),
    );

    Ok(state)
}

fn failure_answer_present(
    artifacts: &std::collections::HashMap<String, serde_json::Value>,
) -> bool {
    artifacts
        .get("answer")
        .and_then(|v| v.as_str())
        .map(|s| {
            let trimmed = s.trim();
            trimmed.starts_with("Task failed:")
                || trimmed.starts_with("The task did not finish cleanly.")
        })
        .unwrap_or(false)
}

fn latest_tool_result(
    artifacts: &std::collections::HashMap<String, serde_json::Value>,
) -> Option<&serde_json::Value> {
    let mut failures: Vec<&serde_json::Value> = artifacts
        .iter()
        .filter(|(k, v)| k.ends_with("_result") && v.is_object())
        .map(|(_, v)| v)
        .collect();
    failures.sort_by(|a, b| {
        let ta = a
            .get("timestamp")
            .and_then(|x| x.as_str())
            .unwrap_or_default();
        let tb = b
            .get("timestamp")
            .and_then(|x| x.as_str())
            .unwrap_or_default();
        ta.cmp(tb)
    });
    failures.pop()
}

fn result_is_success(result: &serde_json::Value) -> bool {
    result.get("success").and_then(|b| b.as_bool()) != Some(false)
}

fn text_from_result_output(result: &serde_json::Value) -> Option<String> {
    let output = result.get("output").unwrap_or(result);
    for key in ["stdout", "body", "text", "content"] {
        if let Some(s) = output.get(key).and_then(|x| x.as_str()) {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn extract_latest_success_output(
    artifacts: &std::collections::HashMap<String, serde_json::Value>,
) -> Option<String> {
    let mut candidates: Vec<(&str, String)> = artifacts
        .iter()
        .filter(|(k, v)| k.ends_with("_result") && result_is_success(v))
        .filter_map(|(k, v)| text_from_result_output(v).map(|text| (k.as_str(), text)))
        .collect();

    candidates.sort_by(|(ka, ta), (kb, tb)| {
        let rank = |k: &str, text: &str| -> (u8, usize) {
            let priority = if k.contains("shell") {
                3
            } else if k.contains("read") || k.contains("file") {
                2
            } else {
                1
            };
            (priority, text.len())
        };
        rank(ka, ta).cmp(&rank(kb, tb))
    });

    candidates.pop().map(|(_, text)| text)
}

fn should_mark_recovered_success(state: &SystemState) -> bool {
    let is_browser_task = state
        .coding_intent
        .as_ref()
        .map(|ci| ci.task_class == "browser_automation_task")
        .unwrap_or(false);

    if is_browser_task {
        let Some(plan) = state.current_plan.as_ref() else {
            return false;
        };
        if state.current_step < plan.steps.len() {
            return false;
        }
        return extract_success_output_for_state(state).is_some();
    }

    let has_non_failure_answer = state
        .artifacts
        .get("answer")
        .and_then(|v| v.as_str())
        .map(|s| {
            let trimmed = s.trim();
            !trimmed.is_empty()
                && !trimmed.starts_with("Task failed:")
                && !trimmed.starts_with("The task did not finish cleanly.")
        })
        .unwrap_or(false);
    if has_non_failure_answer {
        return true;
    }

    let Some(plan) = state.current_plan.as_ref() else {
        return false;
    };
    if state.current_step < plan.steps.len() {
        return false;
    }
    extract_success_output_for_state(state).is_some()
}

fn summarize_failure(state: &SystemState) -> String {
    let artifacts = &state.artifacts;
    let mut failures: Vec<(&String, &serde_json::Value)> = artifacts
        .iter()
        .filter(|(k, v)| {
            k.ends_with("_result") && v.get("success").and_then(|b| b.as_bool()) == Some(false)
        })
        .collect();
    failures.sort_by_key(|(k, _)| *k);

    if let Some((k, v)) = failures.last() {
        let code = v
            .get("error_code")
            .and_then(|x| x.as_str())
            .unwrap_or("unknown_error");
        let trace = v
            .get("trace")
            .and_then(|x| x.as_str())
            .unwrap_or("The tool did not return additional details.")
            .trim();
        if state
            .coding_intent
            .as_ref()
            .map(|ci| ci.task_class == "browser_automation_task")
            .unwrap_or(false)
        {
            return build_browser_failure_answer(state, k, code, trace);
        }
        return format!(
            "The task did not finish cleanly. The last failing step was `{k}` ({code}). {trace}"
        );
    }

    if state
        .coding_intent
        .as_ref()
        .map(|ci| ci.task_class == "browser_automation_task")
        .unwrap_or(false)
    {
        return build_browser_failure_answer(
            state,
            "browser_execution",
            "unverified_browser_output",
            "The task never produced a verified browser execution with exact output markers.",
        );
    }

    "The task did not finish cleanly. It reached the end of execution without producing the final requested answer.".to_string()
}

fn extract_success_output_for_state(state: &SystemState) -> Option<String> {
    if state
        .coding_intent
        .as_ref()
        .map(|ci| ci.task_class == "browser_automation_task")
        .unwrap_or(false)
    {
        return extract_verified_browser_output(&state.artifacts);
    }
    extract_latest_success_output(&state.artifacts)
}

fn extract_verified_browser_output(
    artifacts: &std::collections::HashMap<String, serde_json::Value>,
) -> Option<String> {
    latest_verified_browser_output(artifacts)
}

fn build_browser_failure_answer(
    state: &SystemState,
    step_key: &str,
    code: &str,
    trace: &str,
) -> String {
    let mut sections = vec![format!(
        "The browser task did not finish cleanly. The last failing step was `{step_key}` ({code}). {trace}"
    )];

    let file_views = collect_browser_file_views(state);
    if !file_views.is_empty() {
        sections.push(format!("File contents:\n\n{}", file_views.join("\n\n")));
    }

    if let Some(shell_block) = latest_browser_output_block(&state.artifacts) {
        sections.push(format!("Exact command output:\n\n{shell_block}"));
    }

    sections.join("\n\n")
}

fn collect_browser_file_views(state: &SystemState) -> Vec<String> {
    let target_path = latest_browser_script_path(state);
    let mut blocks = Vec::new();
    for value in state.artifacts.values() {
        let output = value.get("output").unwrap_or(value);
        let Some(path) = output.get("path").and_then(|x| x.as_str()) else {
            continue;
        };
        if !path.ends_with(".py") {
            continue;
        }
        if let Some(target_path) = target_path.as_deref() {
            if path != target_path {
                continue;
            }
        }
        if let Some(content) = output.get("content").and_then(|x| x.as_str()) {
            let block = format!("`{path}`:\n\n{content}");
            if !blocks.contains(&block) {
                blocks.push(block);
            }
        }
    }
    blocks
}

fn latest_browser_script_path(state: &SystemState) -> Option<String> {
    let mut candidates: Vec<(String, String)> = Vec::new();

    if let Some(plan) = state.current_plan.as_ref() {
        for step in &plan.steps {
            let Some(output_key) = &step.output_key else {
                continue;
            };
            let artifact_key = format!("{output_key}_result");
            let Some(value) = state.artifacts.get(&artifact_key) else {
                continue;
            };
            let output = value.get("output").unwrap_or(value);
            let Some(path) = output.get("path").and_then(|x| x.as_str()) else {
                continue;
            };
            if !path.ends_with(".py") {
                continue;
            }
            let timestamp = value
                .get("timestamp")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string();
            candidates.push((timestamp, path.to_string()));
        }
    }

    candidates.sort_by(|a, b| a.0.cmp(&b.0));
    candidates.pop().map(|(_, path)| path)
}
