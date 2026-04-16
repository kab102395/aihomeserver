use anyhow::Result;
use crate::{
    llm::ollama::{Message, ModelRole, OllamaClient},
    state::SystemState,
};

const REPAIR_PROMPT: &str = r#"You are a repair agent. Apply the critic's feedback to fix the execution output.
Output only the corrected artifact — no explanations, no preamble."#;

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

    let messages = vec![
        Message::system(REPAIR_PROMPT),
        Message::user(format!(
            "Issues identified:\n{}\n\nRecommendations:\n{}\n\nCurrent artifacts:\n{}",
            last_review.issues.join("\n"),
            last_review.recommendations.join("\n"),
            serde_json::to_string_pretty(&state.artifacts).unwrap_or_default(),
        )),
    ];

    match llm.chat(messages, ModelRole::Fast, false).await {
        Ok(repaired) => {
            let key = format!("repair_cycle_{}", state.repair_cycle);
            state.log_meta("repair", "Repair applied", serde_json::json!({ "key": key }));
            state.artifacts.insert(key, serde_json::Value::String(repaired));
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
