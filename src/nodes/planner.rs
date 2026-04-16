use anyhow::Result;
use crate::{
    llm::ollama::{Message, ModelRole, OllamaClient},
    state::{PlannerOutput, SystemState},
};

const SYSTEM_PROMPT: &str = r#"You are the planning brain of aihomeserver — a local AI assistant running on Kyle's home server.
You have full access to the local machine: filesystem, shell, git, and the web.
You remember past conversations and learn from previous tasks via semantic memory.
When asked "what can you do", "what tools do you have", or similar — plan a single LLM-only step that answers from the identity above.

Your job is to decide whether a request needs tool use or can be answered directly by an LLM.

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

USE "http_fetch" when asked to fetch, visit, retrieve, or analyze a URL or website.
  params: {"url":"https://example.com"}
  ALWAYS plan TWO steps: step 1 fetches (tool_binding="http_fetch"), step 2 analyzes with LLM (tool_binding=null).
  The second step action should reference the fetch result: "Analyze the fetched page content and answer the user's question"
  The second step output_key must be "answer".
  risk_score for fetch-only tasks: 2

USE "web_search" when asked to search the web, find information about a topic, research something, or when you need to discover relevant URLs before fetching.
  params: {"query": "search terms here"}
  ALWAYS plan TWO steps: step 1 searches (tool_binding="web_search"), step 2 synthesizes with LLM (tool_binding=null).
  The second step output_key must be "answer".
  risk_score for search tasks: 1

IMPORTANT: tool_binding must ALWAYS be a plain string (the tool name) or null. NEVER put an object in tool_binding.
  CORRECT:   "tool_binding": "web_search"
  INCORRECT: "tool_binding": {"tool_name": "web_search", "params": {...}}

NEVER invent tools. NEVER use "clarity", "summarization", or any other tool name not listed above.
NEVER ask for clarification — always plan to answer to the best of your ability.

risk_score: 0-3 low (Q&A, reads), 4-7 standard (file writes, shell), 8-10 high (destructive ops, git push)

Pure JSON only. No explanations."#;

pub async fn run(mut state: SystemState, llm: &OllamaClient) -> Result<SystemState> {
    state.log("planner", "Generating plan");
    if let Some(tx) = &state.sse_tx {
        let _ = tx.send(crate::state::SseEvent::Status { phase: "planning".into() });
    }

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
            if let Some(plan) = &state.current_plan {
                if let Some(tx) = &state.sse_tx {
                    let steps: Vec<String> = plan.steps.iter().map(|s| {
                        if let Some(tool) = &s.tool_binding {
                            format!("[{}] {}", tool, s.action)
                        } else {
                            s.action.clone()
                        }
                    }).collect();
                    let _ = tx.send(crate::state::SseEvent::Plan {
                        steps,
                        risk: plan.risk_score,
                    });
                }
            }
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
