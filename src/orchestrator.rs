use anyhow::Result;
use std::sync::Arc;
use tracing::{info, warn};

use crate::{
    llm::ollama::OllamaClient,
    nodes::{critic, executor, finalization, intake, planner, repair, tool_execution},
    state::{OrchestratorNode, RiskLevel, SystemState},
    tools::ToolRegistry,
};

pub struct Orchestrator {
    pub llm: OllamaClient,
    pub tools: Arc<ToolRegistry>,
}

impl Orchestrator {
    pub fn new(llm: OllamaClient, tools: ToolRegistry) -> Self {
        Self {
            llm,
            tools: Arc::new(tools),
        }
    }

    pub async fn run(&self, initial_state: SystemState) -> Result<SystemState> {
        let mut state = initial_state;
        let mut node = OrchestratorNode::Intake;
        let mut replan_count: u32 = 0;
        const MAX_REPLANS: u32 = 3;

        loop {
            info!(
                task_id = %state.task_id,
                step = state.current_step,
                node = ?node,
                "Orchestrator tick"
            );

            node = match node {
                OrchestratorNode::Intake => {
                    state = intake::run(state).await?;
                    OrchestratorNode::Planner
                }

                OrchestratorNode::Planner => {
                    state = planner::run(state, &self.llm).await?;
                    if state.current_plan.is_none() {
                        // Planning failed — go straight to finalization
                        warn!("Planning produced no plan, finalizing");
                        OrchestratorNode::Finalization
                    } else {
                        OrchestratorNode::Executor
                    }
                }

                OrchestratorNode::Executor => {
                    // ── High-risk human gate ──────────────────────────────────
                    if RiskLevel::from_score(state.risk_score()) == RiskLevel::High {
                        if let (Some(gate_store), Some(sse_tx)) =
                            (state.gate_store.as_ref(), state.sse_tx.as_ref())
                        {
                            let (action, tool) = state
                                .current_plan
                                .as_ref()
                                .and_then(|p| {
                                    // current_step is pre-increment; fall back to first step
                                    p.steps.get(state.current_step)
                                        .or_else(|| p.steps.first())
                                })
                                .map(|s| (s.action.clone(), s.tool_binding.clone()))
                                .unwrap_or_else(|| ("Execute planned action".into(), None));

                            let (tx, rx) = tokio::sync::oneshot::channel::<bool>();
                            {
                                let mut store = gate_store.write().await;
                                store.insert(state.task_id, tx);
                            }
                            let _ = sse_tx.send(crate::state::SseEvent::NeedsApproval {
                                task_id: state.task_id.to_string(),
                                step: state.current_step + 1,
                                action,
                                tool,
                                risk: state.risk_score(),
                            });

                            match rx.await {
                                Ok(true) => {
                                    // Approved — continue to executor
                                    state.log("gate", "High-risk action approved by user");
                                }
                                _ => {
                                    // Rejected or channel dropped
                                    state.log("gate", "High-risk action rejected by user");
                                    state.termination_met = false;
                                    state = finalization::run(state).await?;
                                    break;
                                }
                            }
                        }
                    }
                    // ─────────────────────────────────────────────────────────
                    state = executor::run(state, &self.llm).await?;
                    if state.termination_met {
                        OrchestratorNode::Finalization
                    } else {
                        OrchestratorNode::ToolExecution
                    }
                }

                OrchestratorNode::ToolExecution => {
                    state = tool_execution::run(state, &self.tools).await?;
                    OrchestratorNode::Critic
                }

                OrchestratorNode::Critic => {
                    state = critic::run(state, &self.llm).await?;
                    self.route_from_critic(&state)
                }

                OrchestratorNode::Repair => {
                    state = repair::run(state, &self.llm).await?;
                    OrchestratorNode::Critic
                }

                OrchestratorNode::Replan => {
                    replan_count += 1;
                    warn!(
                        task_id = %state.task_id,
                        failures = state.failure_count,
                        replan = replan_count,
                        "Triggering replan"
                    );
                    if replan_count >= MAX_REPLANS {
                        warn!(task_id = %state.task_id, "Max replans reached, forcing finalization");
                        OrchestratorNode::Finalization
                    } else {
                        // Reset step counter and plan, keep failure taxonomy + checkpoints
                        state.current_step = 0;
                        state.current_plan = None;
                        state.repair_cycle = 0;
                        state.termination_met = false;
                        state = planner::run(state, &self.llm).await?;
                        OrchestratorNode::Executor
                    }
                }

                OrchestratorNode::Finalization => {
                    state = finalization::run(state).await?;
                    OrchestratorNode::Done
                }

                OrchestratorNode::Done => break,
            };

            if state.current_step >= state.max_steps {
                warn!(
                    task_id = %state.task_id,
                    "Max steps ({}) reached, forcing finalization",
                    state.max_steps
                );
                state = finalization::run(state).await?;
                break;
            }
        }

        Ok(state)
    }

    fn route_from_critic(&self, state: &SystemState) -> OrchestratorNode {
        let passed = state
            .critic_history
            .last()
            .map(|r| r.overall_pass)
            .unwrap_or(false);

        if passed {
            // If executor already flagged all steps done, finalize.
            // Otherwise loop back to execute the next step.
            if state.termination_met {
                return OrchestratorNode::Finalization;
            }
            return OrchestratorNode::Executor;
        }

        // Critic failed — escalate or repair.
        if RiskLevel::from_score(state.risk_score()) == RiskLevel::High {
            warn!(
                task_id = %state.task_id,
                "High-risk task failed critic — human gate required before retry"
            );
        }

        // Repair cycle limit: max 2 cycles, then replan from scratch
        if state.repair_cycle >= 2 {
            warn!(
                task_id = %state.task_id,
                repair_cycle = state.repair_cycle,
                "Repair limit reached — triggering replan"
            );
            return OrchestratorNode::Replan;
        }

        OrchestratorNode::Repair
    }
}
