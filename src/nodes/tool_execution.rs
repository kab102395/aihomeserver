use anyhow::Result;
use std::sync::Arc;

use crate::{
    state::{ErrorType, FailureTaxonomy, SystemState},
    tools::ToolRegistry,
};

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

    // Always use the planner's input_params for tool execution.
    // These are structured and reliable; the executor's free-form output is
    // only for code-generation steps (no tool_binding), not tool dispatch.
    let tool_params = step.input_params.clone();

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
        return Ok(state);
    }

    if let Some(tx) = &state.sse_tx {
        let _ = tx.send(crate::state::SseEvent::ToolCall {
            step: state.current_step,
            tool: tool_name.clone(),
            action: step.action.clone(),
        });
    }

    let result = tools.execute(&tool_name, tool_params).await;
    let tool_output_key = format!("{output_key}_result");

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
