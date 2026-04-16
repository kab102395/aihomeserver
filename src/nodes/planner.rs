use anyhow::Result;
use crate::{
    llm::ollama::{Message, ModelRole, OllamaClient},
    state::{PlannerOutput, SystemState},
};

const SYSTEM_PROMPT: &str = r#"You are a task planner. Your job is to decide whether a request needs tool use or can be answered directly by an LLM.

Output ONLY valid JSON:
{
  "steps": [...],
  "tools_required": [],
  "risk_score": 0,
  "expected_outputs": ["answer"],
  "completion_criteria": ["question answered"],
  "dependencies": {}
}

DECISION RULES — choose the right tool_binding:

USE null (LLM-only, no tool) for:
  - Questions, explanations, analysis, summaries, comparisons
  - Writing essays, emails, stories, plans
  - Code generation (output is just text)
  - Anything that is purely informational
  Example: {"step_id":"1","action":"Answer the question thoroughly","tool_binding":null,"input_params":{},"output_key":"answer","expected_output":null}

USE "filesystem" only when explicitly asked to READ or WRITE a file on disk.
  write: {"action":"write","path":"file.txt","content":"..."}
  read:  {"action":"read","path":"file.txt"}
  list:  {"action":"list","path":"."}

USE "shell" only when explicitly asked to run a command or script.
  {"command":"the command","timeout_secs":30}

USE "git" only when explicitly asked about git history, status, or commits.
  {"action":"status"} / {"action":"log","n":10} / {"action":"commit","message":"msg"}

NEVER invent tools. NEVER use "clarity", "summarization", "search", or any other tool name.
NEVER ask for clarification — always plan to answer to the best of your ability.

risk_score: 0-3 low (Q&A, reads), 4-7 standard (file writes, shell), 8-10 high (destructive ops, git push)

Pure JSON only. No explanations."#;

pub async fn run(mut state: SystemState, llm: &OllamaClient) -> Result<SystemState> {
    state.log("planner", "Generating plan");

    let context = build_context(&state);
    let messages = vec![
        Message::system(SYSTEM_PROMPT),
        Message::user(context),
    ];

    match llm.complete_json::<PlannerOutput>(messages, ModelRole::Fast).await {
        Ok(plan) => {
            state.log_meta(
                "planner",
                "Plan ready",
                serde_json::json!({
                    "steps": plan.steps.len(),
                    "risk_score": plan.risk_score,
                    "tools": plan.tools_required,
                }),
            );
            state.current_plan = Some(plan);
        }
        Err(e) => {
            state.log_meta(
                "planner_error",
                "Plan generation failed",
                serde_json::json!({ "error": e.to_string() }),
            );
            state.failure_count += 1;
        }
    }

    Ok(state)
}

fn build_context(state: &SystemState) -> String {
    let mut ctx = String::new();

    // Inject conversation history so the planner understands multi-turn context
    if !state.conversation_history.is_empty() {
        ctx.push_str("Conversation history (oldest → newest):\n");
        for turn in &state.conversation_history {
            ctx.push_str(&format!("{}: {}\n", turn.role.to_uppercase(), turn.content));
        }
        ctx.push('\n');
    }

    ctx.push_str(&format!("Current user request: {}\n", state.user_request));

    // Inject similar past tasks as few-shot examples
    if !state.semantic_context.is_empty() {
        ctx.push_str("\nSimilar past tasks you handled successfully (use as reference):\n");
        for ex in &state.semantic_context {
            ctx.push_str(&format!(
                "---\nRequest: {}\nHow it was handled: {}\n",
                ex.user_request,
                ex.answer_summary.chars().take(400).collect::<String>(),
            ));
        }
        ctx.push_str("---\n");
    }

    if !state.failure_taxonomy.is_empty() {
        ctx.push_str(&format!(
            "\nPrevious failures (use to avoid repeating mistakes): {:?}\nRepair cycles exhausted: {}\n",
            state.failure_taxonomy, state.repair_cycle
        ));
    }

    if let Some(last_cp) = state.checkpoints.last() {
        ctx.push_str(&format!("\nLast successful checkpoint:\n{last_cp}\n"));
    }

    if !state.artifacts.is_empty() {
        let keys: Vec<&String> = state.artifacts.keys().collect();
        ctx.push_str(&format!("\nExisting artifacts: {keys:?}\n"));
    }

    ctx
}
