//! CodingClassifier node — detects coding intent before planning.
//!
//! Sits between Intake and Planner. Uses keyword heuristics (no LLM call)
//! to decide if the request is a coding task. If so, it:
//! 1. Sets `state.coding_intent`
//! 2. Looks up the language adapter
//! 3. Stores the adapter manifest as `adapter_manifest_json` artifact
//!    so the planner can inject it into context

use anyhow::Result;
use crate::{
    coder::{adapter_for_intent, detect_coding_intent},
    state::SystemState,
};

pub async fn run(mut state: SystemState) -> Result<SystemState> {
    let intent = detect_coding_intent(&state.user_request);

    match &intent {
        None => {
            state.log("classifier", "Not a coding task — using standard pipeline");
        }
        Some(ci) => {
            state.log_meta(
                "classifier",
                "Coding task detected",
                serde_json::json!({
                    "intent": ci.intent,
                    "language": ci.language,
                    "deliverable": ci.deliverable,
                    "framework": ci.framework,
                    "requires_build": ci.requires_build,
                    "requires_package": ci.requires_package,
                }),
            );

            // Store the adapter manifest as an artifact so planner can reference it
            if let Some(adapter) = adapter_for_intent(ci) {
                let manifest = adapter.manifest();
                let profile = adapter.resolve_profile(
                    ci.framework.as_deref(),
                    Some(&ci.intent),
                );
                state.log_meta(
                    "classifier",
                    "Adapter selected",
                    serde_json::json!({ "language": manifest.language, "profile": profile }),
                );
                if let Ok(manifest_json) = serde_json::to_value(&manifest) {
                    state.artifacts.insert("adapter_manifest_json".to_string(), manifest_json);
                }
                state.artifacts.insert(
                    "adapter_profile".to_string(),
                    serde_json::Value::String(profile),
                );
            }

            // Emit SSE so UI can show "coding task" immediately
            if let Some(tx) = &state.sse_tx {
                let _ = tx.send(crate::state::SseEvent::Status {
                    phase: format!("coding_{}", ci.language.as_deref().unwrap_or("unknown")),
                });
            }
        }
    }

    state.coding_intent = intent;
    Ok(state)
}
