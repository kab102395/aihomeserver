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

/// Describes what kind of repair is most appropriate for a coding task failure.
enum CodingRepairTarget {
    /// A required source file is missing — must be written
    WriteFile(String),
    /// Build failed — patch the source at project_root
    PatchSource(String),
    /// Expected artifact (e.g. zip) is missing — run packaging step
    RunPackage(String),
    /// Fallback: let LLM decide
    General,
}

/// Determine the most targeted repair action for a coding task based on verifier output.
fn coding_repair_target(state: &SystemState) -> CodingRepairTarget {
    let verif = match state.artifacts.get("artifact_verification") {
        Some(v) => v,
        None => return CodingRepairTarget::General,
    };

    // Priority 1: missing source files — write the first one
    if let Some(missing_files) = verif.get("missing_files").and_then(|v| v.as_array()) {
        if let Some(first) = missing_files.first().and_then(|v| v.as_str()) {
            return CodingRepairTarget::WriteFile(first.to_string());
        }
    }

    // Priority 2: build failure — patch source
    if verif.get("build_passed").and_then(|v| v.as_bool()) == Some(false) {
        let root = state
            .execution_manifest
            .as_ref()
            .map(|m| m.project_root.clone())
            .unwrap_or_else(|| "project".to_string());
        return CodingRepairTarget::PatchSource(root);
    }

    // Priority 3: zip/artifact missing — run packaging
    if let Some(missing_artifacts) = verif
        .get("missing_artifacts")
        .and_then(|v| v.as_array())
    {
        if let Some(first) = missing_artifacts.first().and_then(|v| v.as_str()) {
            return CodingRepairTarget::RunPackage(first.to_string());
        }
    }

    CodingRepairTarget::General
}

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
        // For browser automation tasks, keep retries on the deterministic scaffold.
        // This prevents repair cycles from drifting into ad-hoc Playwright scripts.
        if let Some(intent) = state.coding_intent.as_ref() {
            if intent.task_class == "browser_automation_task" {
                if let Some(tool_call) = crate::coder::deterministic_browser_tool_call(
                    intent,
                    &state.user_request,
                    step,
                    &state.artifacts,
                ) {
                    state.log_meta(
                        "repair",
                        "Applied deterministic browser repair tool call",
                        serde_json::json!({
                            "output_key": output_key,
                            "tool": tool_name,
                            "step": state.current_step,
                        }),
                    );
                    state.artifacts.insert(output_key, tool_call);
                    return Ok(state);
                }
            }
        }

        // For other coding tasks, try a deterministic repair first (no LLM):
        // - packaging/build tool calls are generated mechanically from the manifest + verifier.
        if state.coding_intent.is_some() {
            if let (Some(manifest), Some(verif_val)) =
                (state.execution_manifest.as_ref(), state.artifacts.get("artifact_verification"))
            {
                if let Ok(verif) =
                    serde_json::from_value::<crate::coder::ArtifactVerification>(verif_val.clone())
                {
                    if let Some(tool_call) =
                        crate::coder::plan_deterministic_repair(manifest, &verif, step)
                    {
                        state.log_meta(
                            "repair",
                            "Applied deterministic coding repair tool call",
                            serde_json::json!({
                                "output_key": output_key,
                                "tool": tool_name,
                                "step": state.current_step,
                            }),
                        );
                        state.artifacts.insert(output_key, tool_call);
                        return Ok(state);
                    }
                }
            }
        }

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

        // For coding tasks, add a targeted repair directive based on verifier output
        let coding_directive = if state.coding_intent.is_some() {
            match coding_repair_target(&state) {
                CodingRepairTarget::WriteFile(path) => format!(
                    "\n\nCODING REPAIR TARGET: Write missing file '{}' using the filesystem tool.\nAction: write\nParams: {{\"path\": \"{}\", \"content\": \"<complete implementation>\"}}\nDo NOT output a stub or placeholder — write the full implementation.",
                    path, path
                ),
                CodingRepairTarget::PatchSource(root) => format!(
                    "\n\nCODING REPAIR TARGET: Build failed. Read the build error from artifacts and patch the source file in '{}'.\nUse filesystem read to inspect current content, then write the corrected version.",
                    root
                ),
                CodingRepairTarget::RunPackage(artifact) => format!(
                    "\n\nCODING REPAIR TARGET: Missing artifact '{}'. Run the packaging step.\nIf this is a zip: use filesystem zip_dir action.\nParams example: {{\"action\": \"zip_dir\", \"source_dir\": \"<project_root>\", \"output_path\": \"{}\", \"exclude\": [\"target/\", \".git/\"]}}",
                    artifact, artifact
                ),
                CodingRepairTarget::General => String::new(),
            }
        } else {
            String::new()
        };

        let messages = vec![
            Message::system(repair_tool_prompt()),
            Message::user(format!(
                "Tool: {tool_name}\nStep: {}\nOutput key: {output_key}\n\nIssues:\n{}\n\nRecommendations:\n{}{}\n\nPrior tool call (may be JSON string):\n{}\n\nLast tool result:\n{}\n\nAll artifacts:\n{}",
                step.action,
                last_review.issues.join("\n"),
                last_review.recommendations.join("\n"),
                coding_directive,
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
