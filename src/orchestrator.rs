use anyhow::Result;
use std::sync::Arc;
use tracing::{info, warn};

use std::collections::HashMap;

use crate::{
    coder::{manifest::ArtifactVerification, verify},
    llm::ollama::OllamaClient,
    nodes::{classifier, critic, executor, finalization, intake, planner, repair, tool_execution},
    state::{OrchestratorNode, RiskLevel, SystemState},
    tools::ToolRegistry,
};

/// The agent runtime: a deterministic loop that routes a `SystemState` through
/// well-defined “nodes” (planner/executor/tool execution/critic/repair/finalization).
///
/// Why an explicit orchestrator:
/// - Debuggability: each node can log events + write artifacts into `SystemState`.
/// - Extensibility: adding a node is a localized change.
/// - Safety: all side effects happen through tools invoked in `tool_execution`.
pub struct Orchestrator {
    /// LLM client used by planner/executor/critic/repair nodes.
    pub llm: OllamaClient,
    /// Shared tool registry (filesystem, shell, http, etc.).
    pub tools: Arc<ToolRegistry>,
}

impl Orchestrator {
    /// Construct a new orchestrator with an LLM client and a tool registry.
    ///
    /// The tool registry is wrapped in an `Arc` because multiple concurrent runs
    /// may execute tools at the same time.
    pub fn new(llm: OllamaClient, tools: ToolRegistry) -> Self {
        Self {
            llm,
            tools: Arc::new(tools),
        }
    }

    /// Run the orchestration loop until:
    /// - the system reaches `Finalization`/`Done`, or
    /// - `max_steps` is exceeded (forced finalization).
    ///
    /// This function returns the final `SystemState` so callers can persist artifacts,
    /// event logs, and summary answers for replay.
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
                    OrchestratorNode::CodingClassifier
                }

                OrchestratorNode::CodingClassifier => {
                    state = classifier::run(state).await?;
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
                    if state.risk_score() >= state.risk_gate_threshold && !state.admin_mode {
                        if let (Some(gate_store), Some(sse_tx)) =
                            (state.gate_store.as_ref(), state.sse_tx.as_ref())
                        {
                            let (action, tool) = state
                                .current_plan
                                .as_ref()
                                .and_then(|p| {
                                    // current_step is pre-increment; fall back to first step
                                    p.steps.get(state.current_step).or_else(|| p.steps.first())
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
                    // For coding tasks: run mechanical artifact verification so critic has
                    // a deterministic ground truth rather than just LLM judgement.
                    if state.coding_intent.is_some() {
                        if let Some(manifest) = &state.execution_manifest.clone() {
                            let build_codes = collect_build_exit_codes(&state);
                            let verif = verify(manifest, &state.workspace_path, &build_codes);
                            let files_written = count_files_written(&state);
                            // Emit ProjectCard SSE so UI shows live project status
                            let pkg_path = manifest.expected_artifacts.iter()
                                .find(|p| p.ends_with(".zip"))
                                .and_then(|p| {
                                    let full = std::path::Path::new(&state.workspace_path).join(p);
                                    if full.exists() { Some(p.clone()) } else { None }
                                });
                            if let Some(tx) = &state.sse_tx {
                                let _ = tx.send(crate::state::SseEvent::ProjectCard {
                                    project_name: manifest.project_name.clone(),
                                    language: manifest.language.clone(),
                                    profile: manifest.profile.clone(),
                                    status: verif.status.clone(),
                                    files_written,
                                    build_passed: verif.build_passed,
                                    package_path: pkg_path,
                                });
                            }
                            // Store as artifact for critic to read
                            if let Ok(v) = serde_json::to_value(&verif) {
                                state.artifacts.insert("artifact_verification".to_string(), v);
                            }
                        } else {
                            // Try to extract execution manifest from artifacts
                            // (planner writes it as coding_execution_manifest)
                            self.try_load_execution_manifest(&mut state);
                        }
                    }
                    OrchestratorNode::Critic
                }

                OrchestratorNode::Critic => {
                    state = critic::run(state, &self.llm).await?;
                    self.route_from_critic(&state)
                }

                OrchestratorNode::Repair => {
                    // Tell the UI we're retrying
                    let issues = state
                        .critic_history
                        .last()
                        .map(|r| r.issues.clone())
                        .unwrap_or_default();
                    if let Some(tx) = &state.sse_tx {
                        let _ = tx.send(crate::state::SseEvent::Repair {
                            cycle: state.repair_cycle + 1,
                            issues: issues.iter().take(3).cloned().collect(),
                        });
                    }
                    state = repair::run(state, &self.llm).await?;

                    // If we're repairing a tool step, retry the tool execution with the repaired tool call.
                    let retry_tool = state
                        .current_plan
                        .as_ref()
                        .and_then(|p| p.steps.get(state.current_step.saturating_sub(1)))
                        .and_then(|s| s.tool_binding.as_ref())
                        .is_some();
                    if retry_tool {
                        OrchestratorNode::ToolExecution
                    } else {
                        OrchestratorNode::Critic
                    }
                }

                OrchestratorNode::Replan => {
                    replan_count += 1;
                    warn!(
                        task_id = %state.task_id,
                        failures = state.failure_count,
                        replan = replan_count,
                        "Triggering replan"
                    );
                    if let Some(tx) = &state.sse_tx {
                        let _ = tx.send(crate::state::SseEvent::Replan {
                            attempt: replan_count,
                        });
                    }
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

    /// Try to deserialize the execution manifest from `coding_execution_manifest` artifact.
    fn try_load_execution_manifest(&self, state: &mut SystemState) {
        if state.execution_manifest.is_some() {
            return;
        }
        if let Some(val) = state.artifacts.get("coding_execution_manifest") {
            if let Ok(manifest) = serde_json::from_value::<crate::coder::ExecutionManifest>(val.clone()) {
                state.log_meta(
                    "orchestrator",
                    "Loaded execution manifest from artifact",
                    serde_json::json!({
                        "project": manifest.project_name,
                        "files": manifest.required_files.len(),
                    }),
                );
                state.execution_manifest = Some(manifest);
            }
        }
    }

    /// Decide which node to run next after a critic pass.
    ///
    /// The critic can:
    /// - pass: go back to `Executor` (or finalize if already terminated)
    /// - fail: attempt `Repair` up to a limit, then `Replan`
    /// - trigger special-case replans (e.g. grounding/facts-gate failures)
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

        // Special-case: facts-gate failures should trigger a replan immediately.
        if state
            .artifacts
            .get("facts_gate_result")
            .and_then(|v| v.get("success"))
            .and_then(|b| b.as_bool())
            == Some(false)
        {
            warn!(
                task_id = %state.task_id,
                "Missing facts gate tripped — replanning"
            );
            return OrchestratorNode::Replan;
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

// ── Helper free functions ─────────────────────────────────────────────────────

/// Scan artifacts for shell result exit codes from build/test steps.
/// Keys that end in "_result" and have a numeric exit_code are included.
fn collect_build_exit_codes(state: &SystemState) -> HashMap<String, i64> {
    state
        .artifacts
        .iter()
        .filter(|(k, _)| k.ends_with("_result"))
        .filter_map(|(k, v)| {
            v.get("exit_code")
                .and_then(|ec| ec.as_i64())
                .map(|ec| (k.clone(), ec))
        })
        .collect()
}

/// Count how many FileWritten SSE events have been emitted (proxy for files written).
/// We use the event log since SSE events themselves aren't persisted.
fn count_files_written(state: &SystemState) -> usize {
    state
        .event_log
        .iter()
        .filter(|e| e.event_type == "file_written")
        .count()
}
