use anyhow::Result;
use crate::{
    llm::ollama::{Message, ModelRole, OllamaClient},
    state::SystemState,
};

/// Used when the step has a tool_binding — produce a structured tool call JSON
const SYSTEM_PROMPT_TOOL: &str = r#"You are a tool call generator.
Output ONLY a JSON object describing the tool call. No prose, no markdown.
{ "tool": "tool_name", "params": { ... } }"#;

/// Used when there is no tool_binding — produce the actual answer/output directly
const SYSTEM_PROMPT_LLM: &str = r#"You are a helpful AI assistant. Complete the requested task directly and thoroughly.
Output plain text only — no JSON wrappers, no tool calls, no metadata.
Just the answer, code, or content that was asked for."#;

pub async fn run(mut state: SystemState, llm: &OllamaClient) -> Result<SystemState> {
    state.current_step += 1;

    let plan = match state.current_plan.clone() {
        Some(p) => p,
        None => {
            state.log("executor_error", "No plan available");
            state.failure_count += 1;
            return Ok(state);
        }
    };

    if state.current_step > plan.steps.len() {
        state.log("executor", "All steps complete");
        state.termination_met = true;
        return Ok(state);
    }

    let step = plan.steps[state.current_step - 1].clone();
    state.log_meta(
        "executor",
        &format!("Step {}: {}", step.step_id, step.action),
        serde_json::json!({
            "tool": step.tool_binding,
            "output_key": step.output_key,
        }),
    );

    // Pull relevant artifacts as context (first 3 to keep prompt tight)
    // HashMap::iter() doesn't implement DoubleEndedIterator so no .rev()
    let artifact_context: serde_json::Value = state
        .artifacts
        .iter()
        .take(3)
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect::<serde_json::Map<String, serde_json::Value>>()
        .into();

    let has_tool = step.tool_binding.is_some();
    let system_prompt = if has_tool { SYSTEM_PROMPT_TOOL } else { SYSTEM_PROMPT_LLM };

    let messages = vec![
        Message::system(system_prompt),
        Message::user(format!(
            "Step ID: {}\nAction: {}\nInput params: {}\nAvailable artifacts: {}",
            step.step_id,
            step.action,
            step.input_params,
            artifact_context,
        )),
    ];

    match llm.chat(messages, ModelRole::Fast, has_tool).await {
        Ok(output) => {
            let output_key = step
                .output_key
                .clone()
                .unwrap_or_else(|| format!("step_{}", state.current_step));
            state
                .artifacts
                .insert(output_key, serde_json::Value::String(output));
        }
        Err(e) => {
            state.log_meta(
                "executor_error",
                "LLM call failed",
                serde_json::json!({ "error": e.to_string() }),
            );
            state.failure_count += 1;
        }
    }

    Ok(state)
}
