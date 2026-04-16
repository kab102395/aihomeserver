use anyhow::Result;
use crate::state::SystemState;

pub async fn run(mut state: SystemState) -> Result<SystemState> {
    state.log("intake", "Task received");
    state.log_meta(
        "intake",
        "Request preview",
        serde_json::json!({
            "preview": &state.user_request[..state.user_request.len().min(200)]
        }),
    );
    // Reset for a fresh run
    state.current_step = 0;
    state.termination_met = false;
    state.failure_count = 0;
    state.repair_cycle = 0;
    Ok(state)
}
