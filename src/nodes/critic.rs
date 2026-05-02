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
- Only fail if there is a clear, specific, actionable problem (wrong output, tool failure, missing required artifact)
- File writes, shell commands, and git ops that returned success ARE complete
- Do NOT fail because of cosmetic or style issues

Output ONLY valid JSON:
{
  "overall_pass": true,
  "score": 8.5,
  "confidence": 0.85,
  "issues": [],
  "recommendations": []
}"#;

/// Prompt used when an LLM-only answer step followed a grounded research chain.
/// This checks whether the answer is grounded in the facts artifact or invented.
const RESEARCH_CRITIC_PROMPT: &str = r#"You are a research quality critic. Your job is to detect hallucination in grounded answer steps.

A "grounded answer step" is one where the planner ran web search + fetch + facts extraction,
and then asked the LLM to synthesize an answer. You must check whether the answer actually
uses the research data or invents specifics from training-data memory.

Check the following:
1. HALLUCINATED SPECIFICS — Does the answer contain specific version numbers, dates, stats,
   patch values, prices, or API names that are NOT present in the 'facts' artifact or search
   results? If so: FAIL. These are training-data inventions, not grounded claims.
2. TRAINING DATA LEAK — Does the answer confidently state time-sensitive facts (e.g. "the
   latest version is X.Y.Z") when the artifacts either don't mention that version or show a
   different one? If so: FAIL.
3. PROPER "I DON'T KNOW" — If the artifacts have thin/failed results, did the answer honestly
   say so, or did it pretend to have data? Honest "search failed / data not available" = PASS.
4. SOURCE USAGE — For answers that DO have good artifacts: did the answer quote and cite real
   content from the fetched pages? If it ignored rich artifact content and answered vaguely,
   that's a soft fail (score 4-5, pass=false with recommendation to quote specific evidence).

IMPORTANT: Do NOT fail if:
- The answer is honest about uncertainty or missing data
- Tool results show success=true for search/fetch steps (those steps succeeded)
- The answer is about a timeless topic (math, concepts, code that doesn't depend on versions)

Output ONLY valid JSON:
{
  "overall_pass": false,
  "score": 3.0,
  "confidence": 0.9,
  "issues": ["Answer states 'Rust 1.85 introduces X' but 1.85 is not mentioned in the facts artifact"],
  "recommendations": ["Remove version-specific claims not present in artifacts; state '[not in research data]' instead"]
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

    // Deterministic fail: artifact verification found missing files or artifacts.
    // This check runs only when a coding task is active AND we have a verification result.
    // It is intentionally placed BEFORE the LLM critic so we never spend tokens on a
    // coding task that mechanically failed (e.g. source file never written, zip missing).
    if state.coding_intent.is_some() {
        if let Some(verif) = state.artifacts.get("artifact_verification") {
            let missing_files: Vec<String> = verif
                .get("missing_files")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .map(|s| s.to_string())
                        .collect()
                })
                .unwrap_or_default();
            let missing_artifacts: Vec<String> = verif
                .get("missing_artifacts")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .map(|s| s.to_string())
                        .collect()
                })
                .unwrap_or_default();
            let build_failed = verif
                .get("build_passed")
                .and_then(|v| v.as_bool())
                == Some(false);

            if !missing_files.is_empty() || !missing_artifacts.is_empty() || build_failed {
                let mut issues = Vec::new();
                if !missing_files.is_empty() {
                    issues.push(format!("Missing required files: {:?}", missing_files));
                }
                if !missing_artifacts.is_empty() {
                    issues.push(format!("Missing expected artifacts: {:?}", missing_artifacts));
                }
                if build_failed {
                    issues.push("Build step failed (non-zero exit code)".to_string());
                }

                let mut recommendations = Vec::new();
                if let Some(first_missing) = missing_files.first() {
                    recommendations.push(format!("Write missing file: {first_missing}"));
                }
                if let Some(first_artifact) = missing_artifacts.first() {
                    if first_artifact.ends_with(".zip") {
                        recommendations.push(format!(
                            "Run zip_dir to package project into: {first_artifact}"
                        ));
                    } else {
                        recommendations.push(format!(
                            "Produce missing artifact: {first_artifact}"
                        ));
                    }
                }
                if build_failed {
                    recommendations.push(
                        "Fix compilation errors and re-run build verification".to_string(),
                    );
                }

                state.log_meta(
                    "critic_artifact_fail",
                    "Deterministic artifact verification failure",
                    serde_json::json!({
                        "missing_files": missing_files,
                        "missing_artifacts": missing_artifacts,
                        "build_failed": build_failed,
                    }),
                );

                if let Some(tx) = &state.sse_tx {
                    let _ = tx.send(crate::state::SseEvent::CriticResult {
                        passed: false,
                        score: 0.0,
                        issues: issues.clone(),
                    });
                }

                state.critic_history.push(CriticReview {
                    overall_pass: false,
                    score: 0.0,
                    confidence: 1.0,
                    issues,
                    recommendations,
                    timestamp: Utc::now(),
                });
                return Ok(state);
            }
        }
    }

    let risk = RiskLevel::from_score(state.risk_score());

    // Detect whether the current step is a grounded research answer step.
    // If it is, use the research critic regardless of risk level — the standard
    // low-risk auto-pass would let hallucinated answers through silently.
    let current_step_requires_facts = state
        .current_plan
        .as_ref()
        .and_then(|p| {
            if state.current_step >= 1 && state.current_step <= p.steps.len() {
                Some(p.steps[state.current_step - 1].requires_facts)
            } else {
                None
            }
        })
        .unwrap_or(false);
    let has_facts_artifact = state.artifacts.contains_key("facts");
    let is_grounded_answer_step = current_step_requires_facts && has_facts_artifact;

    // Low risk path: skip critic entirely, auto-pass.
    // Exception: grounded research answer steps always need the research critic
    // because the auto-pass would silently accept hallucinated version numbers.
    if risk == RiskLevel::Low && !is_grounded_answer_step {
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

    // Choose critic prompt — research steps get a dedicated hallucination detector
    let (prompt, model_role) = if is_grounded_answer_step {
        state.log("critic", "Running research grounding critic");
        (RESEARCH_CRITIC_PROMPT, ModelRole::Fast)
    } else {
        match risk {
            RiskLevel::Standard | RiskLevel::Low => (FAST_CRITIC_PROMPT, ModelRole::Fast),
            RiskLevel::High => (DEEP_CRITIC_PROMPT, ModelRole::Critic),
        }
    };

    state.log_meta(
        "critic",
        "Running critique",
        serde_json::json!({
            "risk": format!("{risk:?}"),
            "grounded_answer": is_grounded_answer_step,
        }),
    );

    // Show tool results (_result keys) prominently so critic can verify execution.
    // For grounded steps, also surface the facts artifact and the answer being reviewed.
    let tool_results: serde_json::Map<String, serde_json::Value> = state
        .artifacts
        .iter()
        .filter(|(k, _)| k.ends_with("_result"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();

    let grounding_context = if is_grounded_answer_step {
        let facts = state
            .artifacts
            .get("facts")
            .map(|v| serde_json::to_string_pretty(v).unwrap_or_default())
            .unwrap_or_default();
        let answer = state
            .current_plan
            .as_ref()
            .and_then(|p| {
                if state.current_step >= 1 && state.current_step <= p.steps.len() {
                    p.steps[state.current_step - 1].output_key.as_deref()
                } else {
                    None
                }
            })
            .and_then(|key| state.artifacts.get(key))
            .map(|v| serde_json::to_string_pretty(v).unwrap_or_default())
            .unwrap_or_default();
        format!("\n\nFacts artifact (grounded research data):\n{facts}\n\nAnswer artifact (what the LLM produced — check for hallucination):\n{answer}")
    } else {
        String::new()
    };

    let context = format!(
        "Completion criteria:\n{}\n\nTool execution results:\n{}\n\nAll artifacts:\n{}{}",
        state
            .current_plan
            .as_ref()
            .map(|p| p.completion_criteria.join("\n"))
            .unwrap_or_default(),
        serde_json::to_string_pretty(&tool_results).unwrap_or_default(),
        serde_json::to_string_pretty(&state.artifacts).unwrap_or_default(),
        grounding_context,
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
