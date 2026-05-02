//! Finalization node.
//!
//! Responsibility:
//! - Record final metadata about the run (scores, failures, produced artifacts).
//! - Ensure the event log contains a clear terminal event for replay/debugging.
//!
//! Note: this node does not “format the final answer” in a complex way; the
//! answer typically lives in an `artifacts["answer"]` string created earlier.

use crate::state::SystemState;
use anyhow::Result;

/// Record terminal metadata/logs for the run before returning the final `SystemState`.
pub async fn run(mut state: SystemState) -> Result<SystemState> {
    let avg_score = if state.critic_history.is_empty() {
        None
    } else {
        let sum: f32 = state.critic_history.iter().map(|r| r.score).sum();
        Some(sum / state.critic_history.len() as f32)
    };

    state.log_meta(
        "finalization",
        if state.termination_met {
            "Task completed successfully"
        } else {
            "Task ended (max steps or forced)"
        },
        serde_json::json!({
            "task_id": state.task_id,
            "steps_taken": state.current_step,
            "failure_count": state.failure_count,
            "repair_cycles": state.repair_cycle,
            "critic_reviews": state.critic_history.len(),
            "avg_critic_score": avg_score,
            "artifacts_produced": state.artifacts.keys().collect::<Vec<_>>(),
        }),
    );

    Ok(state)
}
