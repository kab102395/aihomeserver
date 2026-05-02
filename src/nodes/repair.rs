//! Repair node.
//!
//! Responsibility:
//! - Take critic feedback and attempt to fix the *last* step’s output/tool call.
//! - Increment `repair_cycle` and write the repaired result back into artifacts.
//!
//! Why repair exists instead of replanning immediately:
//! - Many failures are “local” (bad JSON, wrong shell syntax, missing param).
//! - Repair is cheaper/faster than throwing away the plan and starting over.
//! - A hard limit prevents infinite loops; after N repairs we replan.

use anyhow::Result;

use crate::{
    llm::ollama::{Message, ModelRole, OllamaClient},
    state::SystemState,
};

/// Best-effort container detection used to tune shell syntax guidance for repairs.
fn in_container() -> bool {
    if std::path::Path::new("/.dockerenv").exists() {
        return true;
    }
    if let Ok(cgroup) = std::fs::read_to_string("/proc/1/cgroup") {
        let c = cgroup.to_lowercase();
        return c.contains("docker") || c.contains("containerd") || c.contains("kubepods");
    }
    false
}

/// Prompt used when the previous step was an LLM-only output.
const REPAIR_TEXT_PROMPT: &str = r#"You are a repair agent. Apply the critic's feedback to fix the last step output.
Output only the corrected content (no explanations, no preamble)."#;

/// Prompt used when the previous step was a tool call that needs to be regenerated.
fn repair_tool_prompt() -> String {
    let os = std::env::consts::OS;
    let shell_hint = if os == "windows" {
        "PowerShell syntax"
    } else {
        "POSIX sh syntax"
    };

    format!(
        r#"You are a repair agent for a tool call.
Output ONLY valid JSON. No prose, no markdown.

Schema:
{{ "tool": "tool_name", "params": {{ ... }} }}

Runtime context:
- runtime_os: {os}
- shell_syntax: {shell_hint}
- in_container: {}

Rules:
- Preserve the tool name exactly as requested.
- Fix parameters so the tool call succeeds.
- If the tool is `shell`, use the runtime OS syntax (no PowerShell cmdlets on Linux; no bashisms on Windows).
- Prefer simple commands and avoid unnecessary pipes.
"#,
        in_container()
    )
}

/// Attempt to repair the last failed step using the latest critic feedback.
pub async fn run(mut state: SystemState, llm: &OllamaClient) -> Result<SystemState> {
    state.repair_cycle += 1;
    state.log_meta(
        "repair",
        &format!("Repair cycle {}", state.repair_cycle),
        serde_json::json!({ "cycle": state.repair_cycle }),
    );

    let last_review = match state.critic_history.last().cloned() {
        Some(r) => r,
        None => {
            state.log("repair_error", "No critic review to repair from");
            return Ok(state);
        }
    };

    let Some(plan) = state.current_plan.as_ref() else {
        state.log("repair_error", "No plan available during repair");
        return Ok(state);
    };
    if state.current_step == 0 || state.current_step > plan.steps.len() {
        state.log("repair_error", "Invalid current_step during repair");
        return Ok(state);
    }

    let step = &plan.steps[state.current_step - 1];
    let output_key = step
        .output_key
        .clone()
        .unwrap_or_else(|| format!("step_{}", state.current_step));

    // Tool step: repair by generating a corrected tool call JSON and overwrite output_key.
    if let Some(tool_name) = step.tool_binding.as_ref() {
        let tool_output_key = format!("{output_key}_result");
        let last_tool_result = state
            .artifacts
            .get(&tool_output_key)
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let prior_tool_call = state
            .artifacts
            .get(&output_key)
            .cloned()
            .unwrap_or(serde_json::Value::Null);

        let messages = vec![
            Message::system(repair_tool_prompt()),
            Message::user(format!(
                "Tool: {tool_name}\nStep: {}\nOutput key: {output_key}\n\nIssues:\n{}\n\nRecommendations:\n{}\n\nPrior tool call (may be JSON string):\n{}\n\nLast tool result:\n{}\n\nAll artifacts:\n{}",
                step.action,
                last_review.issues.join("\n"),
                last_review.recommendations.join("\n"),
                serde_json::to_string_pretty(&prior_tool_call).unwrap_or_default(),
                serde_json::to_string_pretty(&last_tool_result).unwrap_or_default(),
                serde_json::to_string_pretty(&state.artifacts).unwrap_or_default(),
            )),
        ];

        match llm
            .complete_json::<serde_json::Value>(messages, ModelRole::Fast, true)
            .await
        {
            Ok(v) => {
                state.log_meta(
                    "repair",
                    "Repaired tool call",
                    serde_json::json!({ "output_key": output_key, "tool": tool_name }),
                );
                state.artifacts.insert(output_key, v);
            }
            Err(e) => {
                state.log_meta(
                    "repair_error",
                    "Repair tool-call generation failed",
                    serde_json::json!({ "error": e.to_string() }),
                );
                state.failure_count += 1;
            }
        }

        return Ok(state);
    }

    // LLM-only step: repair by regenerating corrected text and overwrite output_key.
    let messages = vec![
        Message::system(REPAIR_TEXT_PROMPT),
        Message::user(format!(
            "Step: {}\nOutput key: {output_key}\n\nIssues:\n{}\n\nRecommendations:\n{}\n\nCurrent artifacts:\n{}",
            step.action,
            last_review.issues.join("\n"),
            last_review.recommendations.join("\n"),
            serde_json::to_string_pretty(&state.artifacts).unwrap_or_default(),
        )),
    ];

    match llm.chat(messages, ModelRole::Fast, false, false).await {
        Ok(repaired) => {
            state.log_meta(
                "repair",
                "Repaired text artifact",
                serde_json::json!({ "output_key": output_key }),
            );
            state
                .artifacts
                .insert(output_key, serde_json::Value::String(repaired));
        }
        Err(e) => {
            state.log_meta(
                "repair_error",
                "Repair LLM call failed",
                serde_json::json!({ "error": e.to_string() }),
            );
            state.failure_count += 1;
        }
    }

    Ok(state)
}
