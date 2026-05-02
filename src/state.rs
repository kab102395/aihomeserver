//! Shared data model for the agent runtime.
//!
//! This module defines the “state” that flows through the orchestrator nodes:
//! planner → executor → tool execution → critic → repair → finalization.
//!
//! Interview talk track:
//! - `SystemState` is the single source of truth for a run.
//! - Nodes only communicate by reading/writing this state (no hidden globals).
//! - Side effects are captured as `ToolResult`s and stored as artifacts for replay.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

// Coder types (re-exported here so nodes can access them from state directly)
pub use crate::coder::{CodingIntent, ExecutionManifest};

// ==================== TOOL RESULT ====================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
/// High-level classification for failures produced by tools/LLM/runtime.
pub enum ErrorType {
    /// No error (success).
    None,
    /// LLM call failed (timeouts, bad responses, etc.).
    Llm,
    /// Tool execution failed.
    Tool,
    /// Environment issue (missing binary, bad PATH, etc.).
    Env,
    /// Timeout (either tool or LLM).
    Timeout,
    /// Permissions / policy block (risk gate, path traversal, etc.).
    Permission,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Normalized output from a tool invocation.
///
/// Tools always return structured data so that:
/// - the agent can reason about success/failure,
/// - the UI can render tool output consistently,
/// - runs can be replayed/debugged from persisted artifacts.
pub struct ToolResult {
    pub success: bool,
    pub error_type: ErrorType,
    pub error_code: Option<String>,
    pub trace: Option<String>,
    pub output: Option<serde_json::Value>,
    pub checkpoint: Option<serde_json::Value>,
    pub observed_state_hash: Option<String>,
    pub timestamp: DateTime<Utc>,
}

impl ToolResult {
    /// Convenience constructor for successful tool calls.
    pub fn ok(output: serde_json::Value, checkpoint: Option<serde_json::Value>) -> Self {
        Self {
            success: true,
            error_type: ErrorType::None,
            error_code: None,
            trace: None,
            output: Some(output),
            checkpoint,
            observed_state_hash: None,
            timestamp: Utc::now(),
        }
    }

    /// Convenience constructor for failed tool calls.
    pub fn err(error_type: ErrorType, code: &str, trace: &str) -> Self {
        Self {
            success: false,
            error_type,
            error_code: Some(code.to_string()),
            trace: Some(trace.to_string()),
            output: None,
            checkpoint: None,
            observed_state_hash: None,
            timestamp: Utc::now(),
        }
    }
}

// ==================== PLAN TYPES ====================

#[derive(Debug, Clone, Serialize, Deserialize)]
/// One step in a planner-produced plan (what to do next and how).
pub struct StepDefinition {
    pub step_id: String,
    pub action: String,
    /// LLMs sometimes emit the tool as a plain string ("web_search") and sometimes
    /// as an object ({"tool_name":"web_search","params":{...}}). We normalise to
    /// Option<String> via a custom deserializer so both forms work.
    #[serde(default, deserialize_with = "deserialize_tool_binding")]
    pub tool_binding: Option<String>,
    /// Optional output format hint for LLM-only steps.
    /// When "json", the executor will require the model to emit valid JSON and will
    /// parse it into artifacts as structured data.
    #[serde(default)]
    pub output_format: Option<String>,
    /// When true, the executor refuses to generate an ungrounded answer unless a
    /// prior facts artifact exists (e.g. for patch-specific game data).
    #[serde(default)]
    pub requires_facts: bool,
    pub input_params: serde_json::Value,
    pub output_key: Option<String>,
    pub expected_output: Option<serde_json::Value>,
}

/// Accepts either a string tool name or an object containing a tool name, and normalizes
/// the result into `Option<String>`.
///
/// This handles inconsistent planner outputs across different models.
fn deserialize_tool_binding<'de, D>(de: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v: serde_json::Value = serde::Deserialize::deserialize(de)?;
    match v {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::String(s) => Ok(if s.is_empty() { None } else { Some(s) }),
        serde_json::Value::Object(map) => {
            // {"tool_name":"web_search",...} or {"tool":"web_search",...}
            let name = map
                .get("tool_name")
                .or_else(|| map.get("tool"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            Ok(name)
        }
        _ => Ok(None),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Planner output stored in `SystemState.current_plan`.
pub struct PlannerOutput {
    /// Ordered list of steps to execute.
    pub steps: Vec<StepDefinition>,
    /// Allow-list of tools the plan expects to use (useful for UI/tool gating).
    pub tools_required: Vec<String>,
    /// 0–10: 0-3 low (no critic), 4-7 standard (fast critic), 8-10 high (deep critic + human gate)
    pub risk_score: u8,
    /// A list of artifact keys or conceptual outputs the plan expects to produce.
    pub expected_outputs: Vec<String>,
    /// Text criteria that indicate the task is done.
    pub completion_criteria: Vec<String>,
    /// Dependency map — shape varies by model output, never used at runtime.
    #[serde(default)]
    pub dependencies: serde_json::Value,
}

// ==================== CRITIC REVIEW ====================

fn default_timestamp() -> DateTime<Utc> {
    Utc::now()
}

/// Deserialize a “list of strings” from multiple possible shapes.
///
/// Why this exists:
/// - Different models sometimes emit `issues`/`recommendations` as a string, array of strings,
///   or array of objects with a `description` field.
/// - The critic/repair pipeline wants a stable `Vec<String>` regardless.
fn deserialize_string_vec<'de, D>(de: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v: serde_json::Value = serde::Deserialize::deserialize(de)?;
    let mut out = Vec::new();

    match v {
        serde_json::Value::Null => {}
        serde_json::Value::String(s) => out.push(s),
        serde_json::Value::Array(arr) => {
            for item in arr {
                match item {
                    serde_json::Value::String(s) => out.push(s),
                    serde_json::Value::Object(map) => {
                        if let Some(desc) = map.get("description").and_then(|v| v.as_str()) {
                            out.push(desc.to_string());
                        } else {
                            out.push(serde_json::Value::Object(map).to_string());
                        }
                    }
                    other => out.push(other.to_string()),
                }
            }
        }
        other => out.push(other.to_string()),
    }

    Ok(out)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Result of a critic pass after tool execution and/or LLM reasoning.
pub struct CriticReview {
    pub overall_pass: bool,
    pub score: f32,
    pub confidence: f32,
    #[serde(default, deserialize_with = "deserialize_string_vec")]
    pub issues: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_string_vec")]
    pub recommendations: Vec<String>,
    /// Added by us after deserialization — LLM output won't include this
    #[serde(default = "default_timestamp")]
    pub timestamp: DateTime<Utc>,
}

// ==================== FAILURE TAXONOMY ====================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Lightweight categorization of failure types for analytics and repair strategies.
pub enum FailureTaxonomy {
    SchemaMismatch,
    LogicError,
    ToolFailure,
    EnvFailure,
    Timeout,
    PermissionError,
}

// ==================== CONVERSATION ====================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationTurn {
    pub role: String, // "user" or "assistant"
    pub content: String,
    pub timestamp: chrono::DateTime<Utc>,
}

// ==================== EVENT LOG ====================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEvent {
    pub timestamp: DateTime<Utc>,
    pub step: usize,
    pub event_type: String,
    pub message: String,
    pub metadata: serde_json::Value,
}

// ==================== SEMANTIC CONTEXT ====================

/// A retrieved past task injected as a few-shot example into the planner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticExample {
    pub user_request: String,
    pub answer_summary: String,
    pub similarity: f32,
}

// ==================== KNOWLEDGE CONTEXT ====================

/// A relevant knowledge base entry injected into the planner before it generates a plan.
/// Tells the AI what it already knows so it can skip re-searching known topics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeContext {
    pub id: String,
    pub topic: String,
    pub summary: String,
    /// Full content — only included when the topic is a primary focus of the request
    pub content: Option<String>,
    pub tags: String,
    pub age_days: i64,
    pub version: i64,
}

// ==================== SYSTEM STATE ====================

#[derive(Debug, Clone, Serialize, Deserialize)]
/// The single source of truth for a run.
///
/// Every orchestrator node takes ownership of this state, mutates it, and returns it.
/// This makes the runtime easier to reason about (no hidden side channels).
pub struct SystemState {
    pub task_id: Uuid,
    pub session_id: Option<Uuid>,
    pub user_request: String,
    /// Last N turns from this session — injected into planner + executor prompts.
    pub conversation_history: Vec<ConversationTurn>,
    /// Relevant knowledge base entries injected before planning.
    pub knowledge_context: Vec<KnowledgeContext>,
    /// Top-k similar past tasks retrieved from semantic memory.
    pub semantic_context: Vec<SemanticExample>,
    pub current_plan: Option<PlannerOutput>,
    pub artifacts: HashMap<String, serde_json::Value>,
    pub checkpoints: Vec<serde_json::Value>,
    pub event_log: Vec<LogEvent>,
    pub critic_history: Vec<CriticReview>,
    pub failure_count: u32,
    /// Resets to 0 on each replan. Max 2 before triggering replan.
    pub repair_cycle: u32,
    pub current_step: usize,
    pub max_steps: usize,
    pub termination_met: bool,
    pub failure_taxonomy: Vec<FailureTaxonomy>,
    /// When true, skip the human gate for high-risk tasks.
    /// Set via the admin toggle in the UI.
    pub admin_mode: bool,
    /// Detected coding intent — set by CodingClassifier if the request is a code task.
    /// None for non-coding requests.
    #[serde(default)]
    pub coding_intent: Option<CodingIntent>,
    /// Execution manifest produced by the planner for coding tasks.
    /// Drives the artifact verifier and targeted repair.
    #[serde(default)]
    pub execution_manifest: Option<ExecutionManifest>,
    /// Answers from the planning questionnaire, keyed by question id.
    /// Injected into the planner prompt as hard constraints.
    pub planning_answers: HashMap<String, String>,
    /// Default working directory for shell commands — set from ServerConfig.
    pub workspace_path: String,
    /// Runtime capability snapshot (preflight evals + config-derived flags).
    /// Used to help the planner/executor avoid calling broken tools and to support self-repair.
    #[serde(default)]
    pub capabilities: serde_json::Value,
    /// Risk score threshold at which the human gate fires (from ServerConfig).
    pub risk_gate_threshold: u8,
    /// SSE sender — present only during a streaming request. Skipped in serialization.
    #[serde(skip)]
    pub sse_tx: Option<tokio::sync::mpsc::UnboundedSender<SseEvent>>,
    /// Approval gate store — shared with the HTTP server so approve/reject endpoints can
    /// resolve a pending oneshot. Only set on streaming requests; None for non-streaming.
    #[serde(skip)]
    pub gate_store:
        Option<Arc<tokio::sync::RwLock<HashMap<Uuid, tokio::sync::oneshot::Sender<bool>>>>>,
}

impl SystemState {
    /// Create a new `SystemState` with safe defaults.
    ///
    /// Callers typically fill in:
    /// - `session_id`, `conversation_history`
    /// - `knowledge_context`, `semantic_context`
    /// - `workspace_path`, `risk_gate_threshold`, `capabilities`
    pub fn new(user_request: impl Into<String>) -> Self {
        Self {
            task_id: Uuid::new_v4(),
            session_id: None,
            user_request: user_request.into(),
            conversation_history: Vec::new(),
            knowledge_context: Vec::new(),
            semantic_context: Vec::new(),
            current_plan: None,
            artifacts: HashMap::new(),
            checkpoints: Vec::new(),
            event_log: Vec::new(),
            critic_history: Vec::new(),
            failure_count: 0,
            repair_cycle: 0,
            current_step: 0,
            max_steps: 20,
            termination_met: false,
            failure_taxonomy: Vec::new(),
            admin_mode: false,
            coding_intent: None,
            execution_manifest: None,
            planning_answers: HashMap::new(),
            workspace_path: "./workspace".into(),
            capabilities: serde_json::json!({}),
            risk_gate_threshold: 8,
            sse_tx: None,
            gate_store: None,
        }
    }

    /// Append an event to the run log.
    pub fn log(&mut self, event_type: &str, message: &str) {
        self.log_meta(event_type, message, serde_json::Value::Null);
    }

    /// Append an event with structured metadata to the run log.
    pub fn log_meta(&mut self, event_type: &str, message: &str, metadata: serde_json::Value) {
        self.event_log.push(LogEvent {
            timestamp: Utc::now(),
            step: self.current_step,
            event_type: event_type.to_string(),
            message: message.to_string(),
            metadata,
        });
    }

    /// Risk score for the current run (defaults to “standard” if no plan is present).
    pub fn risk_score(&self) -> u8 {
        self.current_plan
            .as_ref()
            .map(|p| p.risk_score)
            .unwrap_or(5)
    }

    /// Merge a tool result into artifacts and checkpoints.
    ///
    /// Behavior:
    /// - on success: store the output under `output_key`
    /// - on failure: store a normalized error object under `output_key`
    ///
    /// This ensures the executor/critic can “see” failures and attempt repairs.
    pub fn apply_tool_result(&mut self, result: &ToolResult, output_key: &str) {
        if let Some(cp) = &result.checkpoint {
            self.checkpoints.push(cp.clone());
        }
        if result.success {
            if let Some(output) = &result.output {
                self.artifacts
                    .insert(output_key.to_string(), output.clone());
            }
            return;
        }

        // Persist failures too so the LLM can diagnose what went wrong (network, DNS,
        // bot protection, missing binaries, etc.). Previously failures were dropped,
        // which made the executor think "no artifacts were returned".
        let mut value = result
            .output
            .clone()
            .unwrap_or_else(|| serde_json::json!({}));
        if !value.is_object() {
            value = serde_json::json!({ "output": value });
        }
        if let Some(obj) = value.as_object_mut() {
            obj.insert("success".into(), serde_json::json!(false));
            obj.insert("error_type".into(), serde_json::json!(result.error_type));
            if let Some(code) = &result.error_code {
                obj.insert("error_code".into(), serde_json::json!(code));
            }
            if let Some(trace) = &result.trace {
                obj.insert("trace".into(), serde_json::json!(trace));
            }
            obj.insert(
                "timestamp".into(),
                serde_json::json!(result.timestamp.to_rfc3339()),
            );
        }
        self.artifacts.insert(output_key.to_string(), value);
    }
}

// ==================== SSE EVENTS ====================

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
/// Events streamed to the UI during `POST /run/stream`.
///
/// This is a UX layer: it does not affect correctness, but it makes the system
/// transparent by showing plan/tool/critic transitions live.
pub enum SseEvent {
    Status {
        phase: String,
    },
    Token {
        text: String,
    },
    /// Emitted during qwen3 thinking mode — chain-of-thought tokens before the answer
    ThinkingToken {
        text: String,
    },
    Done {
        task_id: String,
        session_id: String,
        success: bool,
        answer: String,
        duration_ms: i64,
        #[serde(skip_serializing_if = "Option::is_none")]
        failure: Option<SseFailureInfo>,
    },
    Error {
        message: String,
    },
    /// Emitted after planner finishes — tells UI what steps are planned
    Plan {
        steps: Vec<String>, // human-readable step descriptions
        risk: u8,
    },
    /// Emitted before a tool executes
    ToolCall {
        step: usize,
        tool: String,
        action: String,
        /// URL being fetched, search query, etc. — shown in the reasoning panel
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    /// Emitted after a tool returns
    ToolDone {
        step: usize,
        tool: String,
        success: bool,
    },
    /// Emitted when a high-risk step is about to execute — UI shows approve/reject modal
    NeedsApproval {
        task_id: String,
        step: usize,
        action: String,
        tool: Option<String>,
        risk: u8,
    },
    /// Emitted before a shell command runs — shows the command in the terminal panel
    TerminalCmd {
        step: usize,
        command: String,
        /// Working directory the command ran in (resolved by the shell tool)
        cwd: Option<String>,
    },
    /// Emitted after a shell command returns — shows its output
    TerminalOut {
        step: usize,
        stdout: String,
        stderr: String,
        exit_code: i32,
        success: bool,
    },
    /// Emitted after the critic finishes — always, pass or fail
    CriticResult {
        passed: bool,
        score: f32,
        issues: Vec<String>,
    },
    /// Emitted when entering the Repair node
    Repair {
        cycle: u32,
        issues: Vec<String>,
    },
    /// Emitted when triggering a full replan
    Replan {
        attempt: u32,
    },
    /// Emitted when the filesystem tool successfully writes a file
    FileWritten {
        /// Relative path within the workspace
        path: String,
    },
    /// Emitted for coding tasks — shows project status in the UI reasoning panel
    ProjectCard {
        project_name: String,
        language: String,
        profile: String,
        /// "building" | "verified" | "partial" | "failed"
        status: String,
        files_written: usize,
        build_passed: Option<bool>,
        package_path: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize)]
/// Compact failure context for UI debugging (which step/tool failed and why).
pub struct SseFailureInfo {
    pub step: usize,
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace: Option<String>,
}

// ==================== ROUTING ====================

#[derive(Debug, Clone, PartialEq)]
/// Discrete states in the orchestrator’s finite state machine.
pub enum OrchestratorNode {
    Intake,
    /// Keyword-based coding intent detection — sits between Intake and Planner.
    CodingClassifier,
    Planner,
    Executor,
    ToolExecution,
    Critic,
    Repair,
    Replan,
    Finalization,
    Done,
}

#[derive(Debug, Clone, PartialEq)]
/// Human-readable bucketed risk level derived from a numeric risk score.
pub enum RiskLevel {
    /// 0–3: executor only, no critic
    Low,
    /// 4–7: fast critic
    Standard,
    /// 8–10: deep critic + human gate before destructive actions
    High,
}

impl RiskLevel {
    /// Convert a numeric risk score (0–10) into a coarse bucket used by the orchestrator.
    ///
    /// Connection:
    /// - The orchestrator uses this to decide critic depth and whether to require human approval.
    pub fn from_score(score: u8) -> Self {
        match score {
            0..=3 => RiskLevel::Low,
            4..=7 => RiskLevel::Standard,
            _ => RiskLevel::High,
        }
    }
}
