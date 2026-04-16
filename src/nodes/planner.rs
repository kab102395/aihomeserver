use anyhow::Result;
use crate::{
    llm::ollama::{Message, ModelRole, OllamaClient},
    state::{PlannerOutput, SystemState},
};

const SYSTEM_PROMPT: &str = r#"You are a task planner. Decompose the user request into structured steps.

Output ONLY valid JSON — no prose, no markdown fences, no explanations:
{
  "steps": [
    {
      "step_id": "1",
      "action": "concise description",
      "tool_binding": "filesystem",
      "input_params": {"action": "write", "path": "hello.txt", "content": "Hello!"},
      "output_key": "write_result",
      "expected_output": null
    }
  ],
  "tools_required": ["filesystem"],
  "risk_score": 3,
  "expected_outputs": ["write_result"],
  "completion_criteria": ["hello.txt written successfully"],
  "dependencies": {}
}

STRICT RULES:
- tool_binding must be EXACTLY ONE of: "filesystem", "shell", "git", or null
- Never combine tools with | or ,
- null means an LLM-only step with no tool call
- input_params must include ALL required params for the tool:
    filesystem write:  {"action":"write","path":"filename.txt","content":"..."}
    filesystem read:   {"action":"read","path":"filename.txt"}
    filesystem list:   {"action":"list","path":"."}
    shell:             {"command":"the command","timeout_secs":30}
    git status:        {"action":"status"}
    git log:           {"action":"log","n":10}
    git commit:        {"action":"commit","message":"msg"}
- risk_score: 0-3 low (no critic), 4-7 standard (fast critic), 8-10 high (deep critic + human gate)
- Pure JSON only. No explanations."#;

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
    let mut ctx = format!("User request: {}\n", state.user_request);

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
