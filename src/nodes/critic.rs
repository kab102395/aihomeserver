//! Critic node.
//!
//! Responsibility:
//! - Decide whether the most recent execution step “passes” given completion criteria.
//! - Emit actionable issues/recommendations when something is wrong.
//! - Enforce deterministic failures for certain safety contracts (e.g. facts/grounding gate).
//!
//! Why a critic node:
//! - Separates “doing” (executor/tool execution) from “checking”.
//! - Enables repair/replan loops instead of silently producing bad output.

use anyhow::Result;
use chrono::Utc;

use crate::{
    llm::ollama::{Message, ModelRole, OllamaClient},
    state::{CriticReview, RiskLevel, SystemState},
};

const FAST_CRITIC_PROMPT: &str = r#"You are a fast critic. Check whether the tool execution results satisfy the completion criteria.

Rules:
- If a tool result shows success=true, the action succeeded — mark overall_pass: true
- Only fail if there is a clear, specific, actionable problem
- Do NOT fail because of missing information or uncertainty
- File writes, shell commands, and git ops that returned success ARE complete

Output ONLY valid JSON:
{
  "overall_pass": true,
  "score": 8.5,
  "confidence": 0.85,
  "issues": [],
  "recommendations": []
}"#;

const DEEP_CRITIC_PROMPT: &str = r#"You are a deep critic for HIGH-RISK operations. Perform thorough validation before any irreversible action.

Check ALL of the following:
- Logical correctness and edge cases
- Security implications (injection, path traversal, privilege escalation)
- Side effects and reversibility
- Schema / contract compliance
- Data integrity risks

Output ONLY valid JSON:
{
  "overall_pass": false,
  "score": 6.0,
  "confidence": 0.95,
  "issues": ["specific issue description"],
  "recommendations": ["specific actionable fix"]
}

Be thorough. A false pass here may trigger irreversible production actions."#;

/// Run the critic pass for the current step and append a `CriticReview`.
pub async fn run(mut state: SystemState, llm: &OllamaClient) -> Result<SystemState> {
    // The critic is the system’s “quality gate”.
    // It can:
    // - deterministically fail when a contract is violated (e.g. missing grounded facts),
    // - deterministically fail when the most recent tool call failed,
    // - or ask the LLM to evaluate output quality (fast/deep depending on risk).
    // Deterministic fail: facts gate tripped (grounded research attempted without evidence).
    if state
        .artifacts
        .get("facts_gate_result")
        .and_then(|v| v.get("success"))
        .and_then(|b| b.as_bool())
        == Some(false)
    {
        state.critic_history.push(CriticReview {
            overall_pass: false,
            score: 0.0,
            confidence: 1.0,
            issues: vec![
                "Missing grounded facts (facts_gate_result). Cannot generate patch-specific output without evidence.".into(),
            ],
            recommendations: vec![
                "Replan: run parallel_search/http_fetch, extract facts JSON, then generate code using facts.".into(),
            ],
            timestamp: Utc::now(),
        });
        return Ok(state);
    }

    // If the most recent step was a tool step and that tool failed, fail the critic
    // deterministically (even on the low-risk path) so the orchestrator can repair/replan.
    if let Some(plan) = state.current_plan.as_ref() {
        if state.current_step >= 1 && state.current_step <= plan.steps.len() {
            let step = &plan.steps[state.current_step - 1];
            if let Some(tool_name) = step.tool_binding.as_ref() {
                let output_key = step
                    .output_key
                    .clone()
                    .unwrap_or_else(|| format!("step_{}", state.current_step));
                let tool_output_key = format!("{output_key}_result");
                if let Some(val) = state.artifacts.get(&tool_output_key) {
                    let failed = val
                        .get("success")
                        .and_then(|v| v.as_bool())
                        .map(|b| !b)
                        .unwrap_or(false);
                    if failed {
                        let code = val
                            .get("error_code")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        let trace = val
                            .get("trace")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        state.critic_history.push(CriticReview {
                            overall_pass: false,
                            score: 0.0,
                            confidence: 1.0,
                            issues: vec![format!("Tool '{tool_name}' failed ({code}): {trace}")],
                            recommendations: vec![
                                "Retry with different parameters, check networking, or replan."
                                    .into(),
                            ],
                            timestamp: Utc::now(),
                        });
                        return Ok(state);
                    }
                }
            }
        }
    }

    let risk = RiskLevel::from_score(state.risk_score());

    // Low risk path: skip critic entirely, auto-pass
    if risk == RiskLevel::Low {
        state.log("critic", "Skipping (low risk path)");
        state.critic_history.push(CriticReview {
            overall_pass: true,
            score: 9.0,
            confidence: 1.0,
            issues: vec![],
            recommendations: vec![],
            timestamp: Utc::now(),
        });
        return Ok(state);
    }

    let (prompt, model_role) = match risk {
        RiskLevel::Standard => (FAST_CRITIC_PROMPT, ModelRole::Fast),
        RiskLevel::High => (DEEP_CRITIC_PROMPT, ModelRole::Critic),
        RiskLevel::Low => unreachable!(),
    };

    state.log_meta(
        "critic",
        "Running critique",
        serde_json::json!({ "risk": format!("{risk:?}") }),
    );

    // Show tool results (_result keys) prominently so critic can verify execution
    let tool_results: serde_json::Map<String, serde_json::Value> = state
        .artifacts
        .iter()
        .filter(|(k, _)| k.ends_with("_result"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    let context = format!(
        "Completion criteria:\n{}\n\nTool execution results:\n{}\n\nAll artifacts:\n{}",
        state
            .current_plan
            .as_ref()
            .map(|p| p.completion_criteria.join("\n"))
            .unwrap_or_default(),
        serde_json::to_string_pretty(&tool_results).unwrap_or_default(),
        serde_json::to_string_pretty(&state.artifacts).unwrap_or_default(),
    );

    let messages = vec![Message::system(prompt), Message::user(context)];

    let review: CriticReview = match llm.complete_json(messages, model_role, false).await {
        Ok(r) => r,
        Err(e) => {
            state.log_meta(
                "critic_error",
                "Critic LLM call failed",
                serde_json::json!({ "error": e.to_string() }),
            );
            // Treat a failed critic as a failed review so repair/replan can kick in
            CriticReview {
                overall_pass: false,
                score: 0.0,
                confidence: 0.0,
                issues: vec![format!("Critic invocation failed: {e}")],
                recommendations: vec!["Retry or escalate".to_string()],
                timestamp: Utc::now(),
            }
        }
    };

    state.log_meta(
        "critic_result",
        if review.overall_pass { "PASS" } else { "FAIL" },
        serde_json::json!({
            "score": review.score,
            "confidence": review.confidence,
            "issues": review.issues,
        }),
    );

    // NOTE: do NOT set termination_met here — the orchestrator routes back to
    // Executor which will set it once all plan steps are exhausted.

    // Emit critic result so the UI can show pass/fail inline
    if let Some(tx) = &state.sse_tx {
        let _ = tx.send(crate::state::SseEvent::CriticResult {
            passed: review.overall_pass,
            score: review.score,
            issues: review.issues.clone(),
        });
    }

    state.critic_history.push(review);
    Ok(state)
}
