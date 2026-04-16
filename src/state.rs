use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ==================== TOOL RESULT ====================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorType {
    None,
    Llm,
    Tool,
    Env,
    Timeout,
    Permission,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
pub struct StepDefinition {
    pub step_id: String,
    pub action: String,
    pub tool_binding: Option<String>,
    pub input_params: serde_json::Value,
    pub output_key: Option<String>,
    pub expected_output: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannerOutput {
    pub steps: Vec<StepDefinition>,
    pub tools_required: Vec<String>,
    /// 0–10: 0-3 low (no critic), 4-7 standard (fast critic), 8-10 high (deep critic + human gate)
    pub risk_score: u8,
    pub expected_outputs: Vec<String>,
    pub completion_criteria: Vec<String>,
    pub dependencies: HashMap<String, Vec<String>>,
}

// ==================== CRITIC REVIEW ====================

fn default_timestamp() -> DateTime<Utc> {
    Utc::now()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CriticReview {
    pub overall_pass: bool,
    pub score: f32,
    pub confidence: f32,
    pub issues: Vec<String>,
    pub recommendations: Vec<String>,
    /// Added by us after deserialization — LLM output won't include this
    #[serde(default = "default_timestamp")]
    pub timestamp: DateTime<Utc>,
}

// ==================== FAILURE TAXONOMY ====================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
    pub role: String,    // "user" or "assistant"
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

// ==================== SYSTEM STATE ====================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemState {
    pub task_id: Uuid,
    pub session_id: Option<Uuid>,
    pub user_request: String,
    /// Last N turns from this session — injected into planner + executor prompts.
    pub conversation_history: Vec<ConversationTurn>,
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
}

impl SystemState {
    pub fn new(user_request: impl Into<String>) -> Self {
        Self {
            task_id: Uuid::new_v4(),
            session_id: None,
            user_request: user_request.into(),
            conversation_history: Vec::new(),
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
        }
    }

    pub fn log(&mut self, event_type: &str, message: &str) {
        self.log_meta(event_type, message, serde_json::Value::Null);
    }

    pub fn log_meta(&mut self, event_type: &str, message: &str, metadata: serde_json::Value) {
        self.event_log.push(LogEvent {
            timestamp: Utc::now(),
            step: self.current_step,
            event_type: event_type.to_string(),
            message: message.to_string(),
            metadata,
        });
    }

    pub fn risk_score(&self) -> u8 {
        self.current_plan.as_ref().map(|p| p.risk_score).unwrap_or(5)
    }

    pub fn apply_tool_result(&mut self, result: &ToolResult, output_key: &str) {
        if let Some(cp) = &result.checkpoint {
            self.checkpoints.push(cp.clone());
        }
        if result.success {
            if let Some(output) = &result.output {
                self.artifacts.insert(output_key.to_string(), output.clone());
            }
        }
    }
}

// ==================== ROUTING ====================

#[derive(Debug, Clone, PartialEq)]
pub enum OrchestratorNode {
    Intake,
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
pub enum RiskLevel {
    /// 0–3: executor only, no critic
    Low,
    /// 4–7: fast critic
    Standard,
    /// 8–10: deep critic + human gate before destructive actions
    High,
}

impl RiskLevel {
    pub fn from_score(score: u8) -> Self {
        match score {
            0..=3 => RiskLevel::Low,
            4..=7 => RiskLevel::Standard,
            _ => RiskLevel::High,
        }
    }
}
