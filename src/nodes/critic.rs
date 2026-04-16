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

pub async fn run(mut state: SystemState, llm: &OllamaClient) -> Result<SystemState> {
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
        state.termination_met = true;
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

    let messages = vec![
        Message::system(prompt),
        Message::user(context),
    ];

    let review: CriticReview = match llm.complete_json(messages, model_role).await {
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

    if review.overall_pass {
        state.termination_met = true;
    }

    state.critic_history.push(review);
    Ok(state)
}
