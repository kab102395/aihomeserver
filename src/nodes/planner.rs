//! Planner node.
//!
//! Responsibility:
//! - Turn `SystemState.user_request` (+ injected context like conversation history, knowledge, and
//!   semantic examples) into a structured `PlannerOutput` JSON plan.
//! - Choose whether tools are required and set an initial `risk_score`.
//!
//! Why a dedicated planner node:
//! - It isolates prompt design and plan schema enforcement from execution logic.
//! - It makes the agent loop explainable: “planning” is a distinct, inspectable phase.

use crate::{
    llm::ollama::{Message, ModelRole, OllamaClient},
    state::{PlannerOutput, SystemState},
};
use anyhow::Result;
use chrono::{Local, Utc};

/// Best-effort container detection used to tune model choices/defaults.
///
/// This is intentionally heuristic; failure here should not break planning.
/// Best-effort container detection used to tune planner behavior/prompts.
///
/// Connection:
/// - Helps the planner avoid generating commands that assume host tools exist when running in Docker.
fn in_container() -> bool {
    if std::path::Path::new("/.dockerenv").exists() {
        return true;
    }
    if let Ok(cgroup) = std::fs::read_to_string("/proc/1/cgroup") {
        let c = cgroup.to_lowercase();
        return c.contains("docker") || c.contains("containerd") || c.contains("kubepods");
    }
    false
}

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

Each entry in "steps" must be an object:
{
  "step_id": "1",
  "action": "what to do",
  "tool_binding": "tool_name" | null,
  "input_params": {},
  "output_key": "artifact_key",
  "expected_output": null,
  "output_format": null | "json",
  "requires_facts": false
}

🚨 RULE #1 — SEARCH BEFORE CODE OR GUIDES (ABSOLUTE, NO EXCEPTIONS):
  If the request asks you to write code, a guide, a script, or an analysis about ANY of:
    • a game, hero, champion, character, ability, item, mechanic
    • a specific patch/version number (7.41b, 1.5, v2.3, etc.)
    • how to play something / strategy / tier list / meta
    • current prices, rankings, statistics, news, events
  Then you MUST plan a parallel_search step FIRST. No exceptions. Not even if you think
  you know the answer. Not even if the knowledge base has an entry. Your training data
  is stale — hero stats, ability values, item costs, and meta ALL change with every patch.
  Writing code or giving advice from training data = guaranteed wrong numbers and hallucination.

  CORRECT plan for "write Lua bot scripts for Dota 2 heroes patch 7.41b":
    Step 1: parallel_search — research all heroes and patch notes
    Step 2: http_fetch — fetch most reliable URL
    Step 3: http_fetch — fetch second URL
    Step 4: LLM — write the code using ONLY the fetched data
    Step 5: save_knowledge — store what was learned


  GROUNDING CONTRACT (MANDATORY FOR PATCH/GAME RESEARCH):
  - After search/fetch, you MUST create a dedicated fact-table step:
      tool_binding: null
      output_key: "facts"
      output_format: "json"
    The output must be valid JSON and include source URLs + short evidence snippets.
    If a fact is missing from artifacts, set it to null/unknown — NEVER guess.
  - Any later step that generates patch-specific guidance or code MUST set:
      requires_facts: true
    The executor will refuse to proceed if the "facts" artifact is missing.
  - If the user asked for runnable code/scripts, prefer writing files via the filesystem tool
    (the executor will fill the tool params at execution time based on facts/artifacts).

  WRONG plan (never do this):
    Step 1: LLM — "write scripts using stored knowledge"   ← THIS IS ALWAYS WRONG FOR GAME DATA

🚨 RULE #2 — EXPLICIT SEARCH REQUESTS:
  If the user says "please search", "search for", "look it up", "find the latest" — they are
  explicitly asking you to use a search tool. Always plan parallel_search in this case.
  Never respond with a 1-2 step plan that skips searching when the user asked to search.

DECISION RULES — choose the right tool_binding:

USE null (LLM-only, no tool) for:
  - Timeless questions: math, logic, language, historical facts, stable concepts
  - Writing essays, emails, stories, creative content
  - Code generation for standard algorithms (not library versions or configs)
  - Explaining how something works at a conceptual level
  - ONLY when the answer cannot possibly have changed since 2023
  Example: {"step_id":"1","action":"Answer the question thoroughly","tool_binding":null,"input_params":{},"output_key":"answer","expected_output":null}

USE "filesystem" when you need to inspect or modify files in the workspace.
  write: {"action":"write","path":"file.txt","content":"..."}
  read:  {"action":"read","path":"file.txt"}
  list:  {"action":"list","path":"."}
  find:  {"action":"find","path":".","pattern":"planner"}
  grep:  {"action":"grep","path":"src","query":"risk_gate_threshold"}
  NOTE: If the user asks you to "write scripts/code" and the output is longer than ~80 lines,
  prefer writing files into the workspace (one file per script) via the filesystem tool,
  then return an "answer" step that points to the created file paths.

USE "shell" only when explicitly asked to run a command or script.
  {"command":"the command","timeout_secs":30,"cwd":"optional"}
  IMPORTANT: The shell tool runs PowerShell on Windows and sh -lc on Linux/macOS.
  Use syntax appropriate to the runtime OS shown in the planning context ("Runtime OS: ...").
  Prefer chaining with semicolons (portable): cmd1 ; cmd2
  If you must truncate output:
    - Linux/macOS: ... | head -n 20
    - Windows:     ... | Select-Object -First 20

USE "git" only when explicitly asked about git history, status, or commits.
  {"action":"status"} / {"action":"log","n":10} / {"action":"commit","message":"msg"}

CODING / PROJECT WORKFLOW (strongly preferred for non-trivial code tasks):
  - First: inspect the repo using filesystem (list/find/grep/read) before writing anything.
  - Then: write/modify files with filesystem.write (one logical file per step).
  - Finally: validate with shell (build/tests) IF the runtime has the toolchain installed.
    If the toolchain isn't available (e.g. `cargo` missing in a slim runtime container),
    plan a dev/test docker mode or skip validation and state what would be run.

USE "http_fetch" when asked to fetch, visit, retrieve, or analyze a URL or website.
  params: {"url":"https://example.com"}
  Plan: step 1 fetches (tool_binding="http_fetch"), step 2 analyzes with LLM (tool_binding=null).
  The second step action should reference the fetch result: "Analyze the fetched page content and answer the user's question"
  The second step output_key must be "answer".
  risk_score for fetch-only tasks: 2

USE "web_search" for a single targeted query when one search is clearly sufficient.
  params: {"query": "search terms here"}
  For SIMPLE lookups (one clear fact): 2 steps — web_search then LLM answer.

USE "parallel_search" for RESEARCH questions — it runs multiple queries simultaneously
  so 5 searches take the same time as 1. This is the preferred tool for any research task.
  params: {"queries": ["query1", "query2", "query3", ...]}   (up to 6 queries)

  RESEARCH QUESTION DECOMPOSITION — break the question into its component parts:
  For "how to play Kez in Dota 2 turbo patch 7.41b" decompose into:
    - "[hero] Dota 2 hero abilities kit overview"          → core mechanics
    - "[hero] Dota 2 best item builds [patch]"            → itemization
    - "[hero] Dota 2 playstyle role laning"               → how to play
    - "[hero] Dota 2 [patch] changes patch notes"         → what changed
    - "[hero] Dota 2 turbo mode guide reddit"             → community tips
    - "[hero] Dota 2 [patch] reddit guide tips"           → recent community knowledge
  Decompose every distinct aspect of the question into its own query.
  Always include at least one Reddit-targeted query ("reddit" in the search terms).

  For RESEARCH tasks, plan these steps:
    1. parallel_search — all decomposed queries in one shot   output_key: "search_result"
    2. http_fetch — most relevant URL from search results     output_key: "fetch1_result"
       RELIABLE sources (prefer in this order):
         old.reddit.com threads  ← best, almost never blocks
         liquipedia.net
         dota2.fandom.com/wiki
         steamcommunity.com/app/570/discussions
       AVOID: dotabuff.com, stratz.com, anything behind login
    3. http_fetch — second reliable URL, different type       output_key: "fetch2_result"
       Mix source types: wiki + Reddit, or guide site + patch notes
    4. LLM synthesis step — "Using all search results and fetched pages, write a comprehensive
       detailed answer covering every aspect the user asked about"   output_key: "answer"

  For http_fetch steps set url to "" — the system auto-picks the best URL from search artifacts.
  Do NOT put a real URL in http_fetch params — the URL resolver handles it automatically.
  risk_score for research tasks: 1

USE "save_knowledge" after researching a topic to store it permanently for future chats.
  Always use this as the final step after any research task so the knowledge persists.
  params: {
    "topic":   "clear topic name (e.g. 'Kez Dota 2 hero guide')",
    "summary": "1-3 sentence overview for quick injection into future chats",
    "content": "full detailed research content",
    "tags":    ["tag1", "tag2", ...],
    "sources": ["url1", "url2", ...]
  }
  output_key: "knowledge_saved"

TEXTBOOK / CURRICULUM MODE:
  If the user asks to "learn" something in depth (e.g. "learn Rust", "deep dive all sources",
  "make a textbook"), plan a curriculum-style research run that results in MULTIPLE KB chapters.
  Pattern:
    1. parallel_search — official docs + authoritative tutorials (output_key: "search_result")
    2. http_fetch — 2–4 high-quality sources, diverse domains (output_key: "fetch1_result", "fetch2_result", ...)
    3. LLM-only synthesis — produce a structured "book" as JSON (output_key: "kb_textbook", output_format: "json"):
       {
         "book_title": "...",
         "chapters": [
           { "topic": "Book Title — Ch 01: ...", "summary": "...", "content": "Markdown...", "tags": "comma,tags", "sources": ["url", ...] }
         ]
       }
    4. save_knowledge — save all chapters at once using params: { "chapters": [...] } (output_key: "knowledge_saved")
  Keep chapter count reasonable (6–14) so it fits in context windows. Prefer official docs for languages/frameworks.

AUTO-KB MODE (configurable; see runtime capabilities):
  The runtime may set:
    - capabilities.auto_kb_mode: "off" | "research" | "always"
    - capabilities.auto_kb_min_chars: number

  If auto_kb_mode is "always":
    - For most non-trivial user requests, plan to save the final synthesized answer into the KB.
    - Add a final `save_knowledge` step with output_key "knowledge_saved".
    - Only do this when the answer is expected to be substantial (>= auto_kb_min_chars).
    - NEVER save secrets or sensitive personal data; if the request includes passwords/keys/tokens, keep auto_kb off.

  If auto_kb_mode is "research":
    - Only save when the plan includes web research tools (parallel_search/http_fetch) or when explicitly asked to "save" or "learn".

  WHEN TO RESEARCH + SAVE vs USE STORED KNOWLEDGE:
  - If the KNOWLEDGE BASE has a relevant entry AND the request is NOT about patches/versions/meta
    AND the entry is fresh (<14 days) → USE IT, answer directly (single LLM step).
  - If the request mentions "latest", a patch number, or asks about current meta/guides →
    ALWAYS re-research regardless of what's in the knowledge base. Game patches change weekly.
  - If the entry is stale (>14 days) → re-research and save an updated version.
  - If no relevant knowledge exists → research it and always save with save_knowledge at the end.
  - NEVER answer game meta / patch / "how to play" questions from training data alone —
    even a 2-month-old knowledge entry may be outdated for a live game.

  RESEARCH + SAVE plan pattern (6 steps):
    1. parallel_search — decomposed queries          output_key: "search_result"
    2. http_fetch — reliable URL #1                  output_key: "fetch1_result"
    3. http_fetch — reliable URL #2 (Reddit preferred) output_key: "fetch2_result"
    4. LLM: "Synthesize into comprehensive answer"   output_key: "answer"
    5. save_knowledge — persist the research         output_key: "knowledge_saved"

IMPORTANT: tool_binding must ALWAYS be a plain string (the tool name) or null. NEVER put an object in tool_binding.
  CORRECT:   "tool_binding": "web_search"
  INCORRECT: "tool_binding": {"tool_name": "web_search", "params": {...}}

NEVER invent tools. NEVER use "clarity", "summarization", or any other tool name not listed above.
NEVER ask for clarification — always plan to answer to the best of your ability.

risk_score: 0-3 low (Q&A, reads), 4-7 standard (file writes, shell), 8-10 high (destructive ops, git push)

Pure JSON only. No explanations."#;

/// Generate a `PlannerOutput` plan for the current request and store it in `SystemState`.
pub async fn run(mut state: SystemState, llm: &OllamaClient) -> Result<SystemState> {
    // Planner prompt is a strict JSON contract. The rest of the runtime relies on:
    // - `steps[*].tool_binding` to decide when to call tools
    // - `risk_score` to choose critic depth and whether to require human approval
    // - `requires_facts` to enforce grounded outputs when necessary
    state.log("planner", "Generating plan");
    if let Some(tx) = &state.sse_tx {
        let _ = tx.send(crate::state::SseEvent::Status {
            phase: "planning".into(),
        });
    }

    let context = build_context(&state);
    let messages = vec![Message::system(SYSTEM_PROMPT), Message::user(context)];

    // think=true: planner reasons through whether to search vs answer directly
    // before committing to a plan — prevents it from skipping research
    match llm
        .complete_json::<PlannerOutput>(messages, ModelRole::Fast, true)
        .await
    {
        Ok(mut plan) => {
            crate::grounding::enforce_grounding_contract(
                &state.user_request,
                &state.capabilities,
                &mut plan,
            );
            crate::grounding::enforce_curriculum_contract(
                &state.user_request,
                &state.capabilities,
                &mut plan,
            );
            // Auto-KB saving is handled as a post-task hook in the HTTP layer to keep responses fast.
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
                    let steps: Vec<String> = plan
                        .steps
                        .iter()
                        .map(|s| {
                            if let Some(tool) = &s.tool_binding {
                                format!("[{}] {}", tool, s.action)
                            } else {
                                s.action.clone()
                            }
                        })
                        .collect();
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

/// Build the planner input: request + injected context (history, KB, semantic examples, config).
///
/// Connection:
/// - `run()` sends this as the user message alongside `SYSTEM_PROMPT`, and expects strict JSON back.
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
    ctx.push_str(&format!("Runtime OS: {}\n", std::env::consts::OS));
    // Make "today" explicit so "latest" requests don't accidentally anchor on old years.
    ctx.push_str(&format!(
        "Current date/time: {} (local) | {} (UTC)\n",
        Local::now().to_rfc3339(),
        Utc::now().to_rfc3339()
    ));
    ctx.push_str(&format!("In container: {}\n", in_container()));
    ctx.push_str("Shell tool backend: PowerShell on Windows; sh -lc on Linux/macOS.\n");
    if !state.capabilities.is_null() {
        ctx.push_str(&format!(
            "Runtime capabilities (preflight): {}\n",
            serde_json::to_string(&state.capabilities).unwrap_or_else(|_| "{}".into())
        ));
    }

    // ── Coding task injection ─────────────────────────────────────────────────
    // If CodingClassifier detected a coding task, inject the adapter manifest and
    // mandatory coding plan contract so the planner generates a verifiable plan.
    if let Some(ci) = &state.coding_intent {
        let profile = state
            .artifacts
            .get("adapter_profile")
            .and_then(|v| v.as_str())
            .unwrap_or("cli");
        let adapter_json = state
            .artifacts
            .get("adapter_manifest_json")
            .map(|v| serde_json::to_string_pretty(v).unwrap_or_default())
            .unwrap_or_default();

        ctx.push_str("\n=== CODING TASK DETECTED ===\n");
        ctx.push_str(&format!("Language:    {}\n", ci.language.as_deref().unwrap_or("unknown")));
        ctx.push_str(&format!("Profile:     {}\n", profile));
        ctx.push_str(&format!("Intent:      {}\n", ci.intent));
        ctx.push_str(&format!("Deliverable: {}\n", ci.deliverable));
        ctx.push_str(&format!("Requires build:   {}\n", ci.requires_build));
        ctx.push_str(&format!("Requires package: {}\n", ci.requires_package));
        if !adapter_json.is_empty() {
            ctx.push_str(&format!("\nLanguage adapter manifest:\n{}\n", adapter_json));
        }
        if let Some(adapter) = crate::coder::adapter_for_intent(ci) {
            ctx.push_str(&format!("\n{}\n", adapter.manifest().system_prompt_addition));
        }

        ctx.push_str(r#"
CODING PLAN CONTRACT (MANDATORY — follow exactly):
1. FIRST STEP: filesystem list/find to inspect workspace (for modify_existing) OR skip for new projects.
2. MANIFEST STEP: tool_binding=null, output_key="coding_execution_manifest", output_format="json"
   The LLM must output a JSON object with these fields:
   {
     "project_name": "...",
     "project_root": "...",        // relative to workspace, e.g. "snake_rust"
     "language": "...",
     "profile": "...",
     "intent": "...",
     "deliverable": "...",
     "required_files": ["Cargo.toml", "src/main.rs", ...],   // relative to project_root
     "write_plan": [{"path": "...", "purpose": "..."}],      // full relative paths from workspace root
     "verification_plan": ["check_project", "build_release"],// recipe names from adapter
     "expected_artifacts": ["snake_rust/Cargo.toml", "snake_rust/src/main.rs", "snake_rust_source.zip"]
   }
3. FILE WRITE STEPS: one filesystem step per file. Use action="write" (creates parent dirs).
4. VERIFICATION STEPS: shell steps using the exact commands from verification_recipes.
   Shell cwd MUST be set to the project_root inside the workspace.
5. PACKAGE STEP (if requires_package): filesystem action="zip_dir" with:
   {"action":"zip_dir","source_dir":"<project_root>","output_path":"<name>.zip","exclude":["target/",".git/"]}
6. ANSWER STEP: final LLM step summarizing paths, build status, and download link.

FILESYSTEM ACTIONS SUPPORTED: write, read, mkdir, list, find, grep, delete, zip_dir
DO NOT USE: create_file, create_dir, save, touch (these return unsupported_action)

risk_score for new coding projects: 4-6 (needs verification but not destructive)
"#);
        ctx.push_str("=== END CODING CONTRACT ===\n\n");
        ctx.push_str("IMPORTANT: Do NOT answer this request from the knowledge base or training data.\n");
        ctx.push_str("The user wants actual files created in the workspace. Use the CODING PLAN CONTRACT above.\n\n");

        // Also inject execution manifest if already loaded (from replan)
        if let Some(manifest) = &state.execution_manifest {
            if let Ok(mj) = serde_json::to_string_pretty(manifest) {
                ctx.push_str(&format!("\nExisting execution manifest (from prior attempt):\n{}\n", mj));
            }
        }
    }

    // Inject knowledge base entries — what the AI already knows about relevant topics
    if !state.knowledge_context.is_empty() {
        ctx.push_str("\n=== KNOWLEDGE BASE (pre-researched topics) ===\n");
        ctx.push_str(
            "You already have stored knowledge on these topics. USE IT instead of re-searching.\n",
        );
        ctx.push_str("Only re-search if the user explicitly wants fresh/updated info, or if the entry is stale (>14 days).\n\n");
        for kb in &state.knowledge_context {
            let stale_note = if kb.age_days >= 14 {
                format!(" ⚠ STALE ({} days old — consider refreshing)", kb.age_days)
            } else {
                format!(" ({}d old, v{})", kb.age_days, kb.version)
            };
            ctx.push_str(&format!("TOPIC: {}{}\n", kb.topic, stale_note));
            ctx.push_str(&format!("TAGS: {}\n", kb.tags));
            ctx.push_str(&format!("SUMMARY: {}\n", kb.summary));
            if let Some(content) = &kb.content {
                ctx.push_str(&format!(
                    "FULL CONTENT:\n{}\n",
                    &content.chars().take(2000).collect::<String>()
                ));
            }
            ctx.push('\n');
        }
        ctx.push_str("=== END KNOWLEDGE BASE ===\n\n");
    }

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

    // Inject planning questionnaire answers as hard constraints
    if !state.planning_answers.is_empty() {
        ctx.push_str("\nUser's pre-run planning choices (treat as hard constraints):\n");
        for (qid, answer) in &state.planning_answers {
            ctx.push_str(&format!("  - {qid}: {answer}\n"));
        }
        ctx.push('\n');
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

// Grounding contract enforcement lives in `crate::grounding` so it can be
// unit-tested and reused by runtime diagnostics.
