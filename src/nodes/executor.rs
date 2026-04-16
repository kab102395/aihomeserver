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

    // Build a conversation history block so the model has full context
    let history_block = if !state.conversation_history.is_empty() {
        let lines: Vec<String> = state
            .conversation_history
            .iter()
            .map(|t| format!("{}: {}", t.role.to_uppercase(), t.content))
            .collect();
        format!("Prior conversation:\n{}\n\n", lines.join("\n"))
    } else {
        String::new()
    };

    let user_prompt = if has_tool {
        format!(
            "{}User request: {}\n\nStep ID: {}\nAction: {}\nInput params: {}\nAvailable artifacts: {}",
            history_block,
            state.user_request,
            step.step_id,
            step.action,
            step.input_params,
            artifact_context,
        )
    } else {
        // For LLM-only steps, lead with history + question so the model sees everything
        format!(
            "{}User request: {}\n\nStep ID: {}\nAction: {}\nAvailable artifacts: {}",
            history_block,
            state.user_request,
            step.step_id,
            step.action,
            artifact_context,
        )
    };

    let messages = vec![
        Message::system(system_prompt),
        Message::user(user_prompt),
    ];

    let output = if !has_tool {
        if let Some(sse_tx) = &state.sse_tx {
            // Stream tokens live
            let (tok_tx, mut tok_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
            let sse_tx2 = sse_tx.clone();
            tokio::spawn(async move {
                while let Some(tok) = tok_rx.recv().await {
                    let _ = sse_tx2.send(crate::state::SseEvent::Token { text: tok });
                }
            });
            // emit step status
            let _ = sse_tx.send(crate::state::SseEvent::Status {
                phase: format!("executing_step_{}", state.current_step),
            });
            llm.chat_stream(messages, ModelRole::Fast, &tok_tx).await.ok()
        } else {
            llm.chat(messages, ModelRole::Fast, false).await.ok()
        }
    } else {
        llm.chat(messages, ModelRole::Fast, true).await.ok()
    };

    match output {
        Some(out) => {
            let output_key = step.output_key.clone()
                .unwrap_or_else(|| format!("step_{}", state.current_step));
            state.artifacts.insert(output_key, serde_json::Value::String(out));
        }
        None => {
            state.log_meta("executor_error", "LLM call failed", serde_json::json!({}));
            state.failure_count += 1;
        }
    }

    Ok(state)
}
