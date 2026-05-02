//! Executor node.
//!
//! Responsibility:
//! - Given a `PlannerOutput`, execute one step at a time.
//! - If a step requires a tool (`tool_binding`), generate a structured tool call JSON
//!   for `tool_execution` to run.
//! - If a step is LLM-only (`tool_binding: null`), produce the step output and store it
//!   under the step’s `output_key` in artifacts.
//!
//! Why split “executor” and “tool execution”:
//! - The executor handles reasoning + schema enforcement.
//! - Tool execution is the only place side effects occur, making auditing/safety easier.

use crate::{
    llm::ollama::{Message, ModelRole, OllamaClient},
    state::{PlannerOutput, StepDefinition, SystemState},
};
use anyhow::Result;
use serde_json::json;

/// Best-effort container detection used to tune prompts (e.g. shell guidance).
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

/// Build the system prompt used when a step has a tool binding.
///
/// Connection:
/// - The executor asks the LLM to emit a *single* JSON object describing the tool call.
/// - The tool_execution node parses and executes that call, so this prompt is the
///   schema contract between reasoning and side effects.
fn system_prompt_tool(workspace_path: &str) -> String {
    let os = std::env::consts::OS;
    let shell_hint = if os == "windows" {
        "PowerShell (powershell -NoProfile -NonInteractive -Command ...)"
    } else {
        "POSIX sh (sh -lc ...)"
    };
    let in_container = in_container();

    format!(
        r#"You are a tool call generator.
Output ONLY a JSON object describing the tool call. No prose, no markdown.
{{ "tool": "tool_name", "params": {{ ... }} }}

RUNTIME CONTEXT:
- runtime_os: {os}
- shell_backend: {shell_hint}
- in_container: {in_container}
- workspace_path: {workspace_path}

SHELL TOOL RULES:
- The shell tool runs commands on the SAME OS/environment as this server process.
  If the server runs inside Docker, your shell commands run inside that container.
- Do NOT use `docker exec ...` unless you are certain the `docker` CLI exists AND you need to target a *different* container.
- Use syntax appropriate to the runtime OS.
  - On Linux/macOS: avoid PowerShell-only cmdlets like `Select-Object`, `Get-ChildItem`, `$env:VAR`.
  - On Windows: avoid bash-only command substitution like `$()`, backticks, and GNU-only flags.
- Prefer simple commands and avoid unnecessary pipes. If you must truncate output:
  - Linux/macOS: `| head -n 20`
  - Windows: `| Select-Object -First 20`

FILESYSTEM TOOL RULES:
- The filesystem tool is rooted at `workspace_path`. Paths are relative to that root.
- Prefer filesystem for repo research instead of fragile shell pipelines:
  - list directories: {{"action":"list","path":"."}}
  - find files:       {{"action":"find","path":".","pattern":"planner"}}
  - search in files:  {{"action":"grep","path":"src","query":"ToolResult"}}
- Writes should be precise and scoped (one file per step if possible).
"#
    )
}

/// Used when there is no tool_binding — produce the actual answer/output directly
const SYSTEM_PROMPT_LLM: &str = r#"You are aihomeserver — a local AI assistant running on Kyle's home server.
You have access to local tools (filesystem, shell, git) and optional web tools (search, fetch).

CRITICAL ANTI-HALLUCINATION RULES:
- Your training data has a cutoff. Game patches, software versions, prices, rankings, and meta
  are almost certainly outdated in your training data. NEVER invent patch notes, hero stats,
  item costs, tier lists, or release changelogs from memory.
- If you have search/fetch artifacts: use them as your ONLY source for any specific fact.
  Quote real numbers, real names, real content directly from the actual fetched content.
- If you do NOT have search artifacts for a time-sensitive topic: say EXACTLY
  "I don't have current data on this — my training data may be outdated and the search
  results weren't sufficient to answer with confidence."
  Do NOT fill the gap with training-data guesses dressed up as facts. Do NOT say "as of my
  knowledge cutoff" and then proceed to give specifics.
- SPECIFIC VERSION NUMBERS, DATES, AND STATISTICS: never state these from memory.
  If a version number, release date, or statistic is not present in the artifacts, say it's
  not available rather than guessing. A wrong version number is worse than no answer.

ARTIFACT USAGE RULES:
- web_search results contain search snippets (short). http_fetch results contain full page text (rich).
- A failed http_fetch (403/timeout) does NOT mean the web search failed — use the snippets.
- If a web_search itself returned success=false, say the search failed.
- When you have real data, use it extensively — quote specific details, exact version numbers,
  exact names from the actual fetched content. Cite the source URL.

CODING QUALITY RULES:
- Prefer concrete, runnable implementations. Avoid placeholders unless you also implement them.
- If the user wants project/file creation, output file paths and commands to build/test.
- If you do not have enough repo context, say what file(s) you need and why.

MATH / LATEX RULES:
- When asked for math, prefer LaTeX for formulas.
- Use `$...$` for inline math and `$$...$$` for display math.
- If producing LaTeX output, keep it syntactically valid and paste-ready.

Complete the requested task directly and thoroughly. Be specific and detailed.
Output plain text only — no JSON wrappers, no tool calls, no metadata.
Just the answer, code, or content that was asked for."#;

/// Grounding guardrail injected into the user-message for `requires_facts` steps.
///
/// This is appended to the user prompt (not system prompt) so it appears close to
/// the end of context where modern LLMs give it the most weight.
const GROUNDING_GUARDRAIL: &str = "

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
GROUNDING ENFORCEMENT — THIS STEP REQUIRES FACTS ARTIFACTS
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
The planner flagged this step as requiring grounded evidence.
You MUST follow these rules without exception:

1. Every specific claim (version number, date, stat, price, hero value, API name) MUST come
   from the 'facts' or search/fetch artifacts shown above. Cite the source.

2. If a specific fact is NOT present in the artifacts: write exactly
   \"[not in research data]\" in place of the missing fact.
   Do NOT substitute your training-data memory for missing evidence.

3. If the artifacts are empty or failed: respond with a short paragraph explaining
   what was searched and that the results were insufficient — do not attempt the answer.

4. Do NOT begin your response with reasoning, meta-commentary, or caveats about your
   training cutoff. Jump directly into the answer using the artifact data.
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━";

/// Used for LLM-only steps that must emit machine-readable JSON (e.g. evidence-backed fact tables).
const SYSTEM_PROMPT_LLM_JSON: &str = r#"You are a research extraction agent.
Output ONLY valid JSON. No prose, no markdown, no code fences.

Rules:
- Use ONLY the provided artifacts as sources. Do not use memory or training data.
- If a fact is missing from artifacts, leave it null or omit it; do NOT guess.
- Prefer a compact, structured format that can be used for downstream code generation.
- Include source URLs and short evidence snippets for any non-trivial fact."#;

/// System prompt for the execution manifest step in coding tasks.
///
/// Connection:
/// - Used only when `step.output_key == "coding_execution_manifest"` and a coding intent is set.
/// - The manifest drives the verifier, critic, and repair nodes — it must be precise.
fn coding_manifest_prompt() -> String {
    r#"You are a coding project manifest generator.
Output ONLY a single valid JSON object matching the ExecutionManifest schema below.
No prose, no markdown, no code fences, no trailing commas.

SCHEMA:
{
  "project_name": "string — short identifier, no spaces (e.g. snake_rust)",
  "project_root": "string — path relative to workspace root (e.g. snake_rust)",
  "language": "string — rust | python | javascript | typescript | go",
  "profile": "string — profile from adapter (e.g. terminal-game-crossterm, cli, library)",
  "intent": "string — new_project | modify_existing | add_feature | debug_existing | package_existing",
  "deliverable": "string — source | binary | zip | library | web_app | script",
  "required_files": ["array of relative paths that MUST exist for the project to be complete"],
  "write_plan": [
    {"path": "relative/path/to/file.ext", "purpose": "one-line description"}
  ],
  "verification_plan": ["ordered list of adapter recipe names, e.g. check_project, build_release, test"],
  "expected_artifacts": ["paths that must exist when the task is done, including any .zip files"]
}

RULES:
- project_root is relative to workspace. Do NOT include workspace_path in the value.
- required_files and write_plan must cover every source file needed for the project to compile/run.
- expected_artifacts MUST include the zip path if deliverable is zip (e.g. "snake_rust/snake_rust.zip").
- expected_artifacts MUST include the binary path if deliverable is binary (e.g. "snake_rust/target/release/snake_rust").
- verification_plan must be ordered: compile check first, then test, then package.
- Do not include "target/" paths in required_files or write_plan.
- Be specific: include all source files (main.rs, lib.rs, modules, etc.) in required_files."#
        .to_string()
}

/// Execute the next plan step (LLM-only output or tool-call generation).
pub async fn run(mut state: SystemState, llm: &OllamaClient) -> Result<SystemState> {
    // The executor advances `state.current_step` and either:
    // - generates a tool call artifact (for tool-bound steps), or
    // - generates the step output directly (for LLM-only steps).
    state.current_step += 1;

    let plan = match state.current_plan.clone() {
        Some(p) => p,
        None => {
            state.log("executor_error", "No plan available");
            state.failure_count += 1;
            return Ok(state);
        }
    };

    if state.current_step > plan.steps.len() {
        // All steps executed, but only mark the run "done" if we actually produced the
        // expected outputs (most importantly: `answer`). Otherwise the UI will show a
        // misleading "Task completed." placeholder.
        let expected = plan.expected_outputs.clone();
        let mut missing: Vec<String> = Vec::new();
        for k in expected.iter() {
            match state.artifacts.get(k) {
                Some(serde_json::Value::String(s)) if !s.trim().is_empty() => {}
                Some(serde_json::Value::Null) | None => missing.push(k.clone()),
                Some(_) => {} // non-string structured output is acceptable
            }
        }

        if missing.is_empty() {
            state.log("executor", "All steps complete");
            state.termination_met = true;
        } else {
            state.log_meta(
                "executor_incomplete",
                "All steps executed but expected outputs missing",
                serde_json::json!({
                    "missing_outputs": missing,
                    "expected_outputs": expected,
                    "artifacts_present": state.artifacts.keys().collect::<Vec<_>>(),
                }),
            );
            state.failure_count += 1;
            state.termination_met = false;
        }
        return Ok(state);
    }

    let step = plan.steps[state.current_step - 1].clone();
    state.log_meta(
        "executor",
        &format!("Step {}: {}", step.step_id, step.action),
        serde_json::json!({
            "tool": step.tool_binding,
            "output_key": step.output_key,
        }),
    );

    // Build artifact context for LLM steps:
    // Prioritise *_result keys (tool outputs, including failures) then fill
    // with up to 3 non-result keys so the model sees what actually happened.
    // Always include `facts` if present so grounded steps don't hallucinate.
    let mut ctx_map = serde_json::Map::new();
    for (k, v) in state
        .artifacts
        .iter()
        .filter(|(k, _)| k.ends_with("_result"))
    {
        ctx_map.insert(k.clone(), v.clone());
    }
    if let Some(v) = state.artifacts.get("facts") {
        ctx_map.insert("facts".into(), v.clone());
    }
    if let Some(v) = state.artifacts.get("facts_gate_result") {
        ctx_map.insert("facts_gate_result".into(), v.clone());
    }
    if let Some(v) = state.artifacts.get("research_expansions") {
        ctx_map.insert("research_expansions".into(), v.clone());
    }
    for (k, v) in state
        .artifacts
        .iter()
        .filter(|(k, _)| {
            !k.ends_with("_result")
                && !k.starts_with("repair_")
                && k.as_str() != "facts"
                && k.as_str() != "facts_gate_result"
                && k.as_str() != "research_expansions"
        })
        .take(3)
    {
        ctx_map.insert(k.clone(), v.clone());
    }
    let artifact_context: serde_json::Value = ctx_map.into();

    let has_tool = step.tool_binding.is_some();
    let wants_json = !has_tool
        && step
            .output_format
            .as_deref()
            .map(|f| f.eq_ignore_ascii_case("json"))
            .unwrap_or(false);

    // Guardrail: if a step requires grounded facts, refuse to proceed unless a facts artifact exists.
    if step.requires_facts {
        let has_facts =
            state.artifacts.contains_key("facts") || state.artifacts.contains_key("facts_json");
        if !has_facts {
            // Record a deterministic failure artifact so the Critic can fail and trigger a replan,
            // instead of the system continuing and hallucinating patch-specific outputs.
            state.artifacts.insert(
                "facts_gate_result".into(),
                serde_json::json!({
                    "success": false,
                    "error_type": "tool",
                    "error_code": "missing_facts",
                    "trace": "requires_facts step blocked: no facts artifact present",
                    "timestamp": chrono::Utc::now().to_rfc3339(),
                }),
            );

            let output_key = step
                .output_key
                .clone()
                .unwrap_or_else(|| format!("step_{}", state.current_step));
            state.log_meta(
                "executor_error",
                "Missing facts artifact for requires_facts step",
                serde_json::json!({ "step_id": step.step_id, "output_key": output_key }),
            );
            state.failure_count += 1;
            state.artifacts.insert(
                output_key,
                serde_json::Value::String(
                    "Cannot proceed: missing grounded facts from research. Re-run search/fetch and extract a fact table before generating code."
                        .into(),
                ),
            );
            return Ok(state);
        }
    }

    let system_prompt = if has_tool {
        // For coding tool steps, append adapter-specific rules
        let base = system_prompt_tool(&state.workspace_path);
        if state.coding_intent.is_some() {
            let adapter_addition = state
                .artifacts
                .get("adapter_manifest_json")
                .and_then(|v| v.get("system_prompt_addition"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if adapter_addition.is_empty() {
                base
            } else {
                format!("{}\n\n{}", base, adapter_addition)
            }
        } else {
            base
        }
    } else if wants_json {
        // Special prompt for execution manifest step
        if state.coding_intent.is_some()
            && step.output_key.as_deref() == Some("coding_execution_manifest")
        {
            coding_manifest_prompt()
        } else {
            SYSTEM_PROMPT_LLM_JSON.to_string()
        }
    } else {
        SYSTEM_PROMPT_LLM.to_string()
    };

    // Build a conversation history block so the model has full context
    let history_block = if !state.conversation_history.is_empty() {
        let lines: Vec<String> = state
            .conversation_history
            .iter()
            .map(|t| format!("{}: {}", t.role.to_uppercase(), t.content))
            .collect();
        format!("Prior conversation:\n{}\n\n", lines.join("\n"))
    } else {
        String::new()
    };

    let user_prompt = if has_tool {
        format!(
            "{}User request: {}\n\nRuntime OS: {}\nIn container: {}\nCapabilities: {}\n\nStep ID: {}\nAction: {}\nInput params: {}\nAvailable artifacts: {}",
            history_block,
            state.user_request,
            std::env::consts::OS,
            in_container(),
            state.capabilities,
            step.step_id,
            step.action,
            step.input_params,
            artifact_context,
        )
    } else {
        // For LLM-only steps, lead with history + question so the model sees everything.
        // Append the grounding guardrail at the END of the user message for requires_facts
        // steps — placing it last maximises the weight modern LLMs give to the instruction.
        let grounding_suffix = if step.requires_facts {
            GROUNDING_GUARDRAIL
        } else {
            ""
        };
        format!(
            "{}User request: {}\n\nCapabilities: {}\nStep ID: {}\nAction: {}\nAvailable artifacts: {}{}",
            history_block,
            state.user_request,
            state.capabilities,
            step.step_id,
            step.action,
            artifact_context,
            grounding_suffix,
        )
    };

    let messages = vec![Message::system(system_prompt), Message::user(user_prompt)];

    // Always use the Fast model for streaming — the Critic (32b) is 3-4x slower per token
    // and makes streaming feel terrible. The Critic node still runs separately after each
    // step to evaluate quality; it just doesn't generate the user-facing response.
    let synthesis_model = ModelRole::Fast;

    let output = if !has_tool {
        if wants_json {
            llm.complete_json::<serde_json::Value>(messages, synthesis_model, true)
                .await
                .ok()
                .and_then(|v| serde_json::to_string_pretty(&v).ok())
        } else if let Some(sse_tx) = &state.sse_tx {
            // Regular answer tokens → SSE Token events
            let (tok_tx, mut tok_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
            let sse_tx2 = sse_tx.clone();
            tokio::spawn(async move {
                while let Some(tok) = tok_rx.recv().await {
                    let _ = sse_tx2.send(crate::state::SseEvent::Token { text: tok });
                }
            });
            // Thinking tokens → SSE ThinkingToken events (qwen3 CoT)
            let (think_tx, mut think_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
            let sse_tx3 = sse_tx.clone();
            tokio::spawn(async move {
                while let Some(tok) = think_rx.recv().await {
                    let _ = sse_tx3.send(crate::state::SseEvent::ThinkingToken { text: tok });
                }
            });
            // emit step status
            let _ = sse_tx.send(crate::state::SseEvent::Status {
                phase: format!("executing_step_{}", state.current_step),
            });
            llm.chat_stream(messages, synthesis_model, &tok_tx, true, Some(&think_tx))
                .await
                .ok()
        } else {
            llm.chat(messages, synthesis_model, false, true).await.ok()
        }
    } else {
        // Tool call generation — needs clean JSON, no thinking
        llm.chat(messages, ModelRole::Fast, true, false).await.ok()
    };

    match output {
        Some(out) => {
            let output_key = step
                .output_key
                .clone()
                .unwrap_or_else(|| format!("step_{}", state.current_step));
            if wants_json && !has_tool {
                match serde_json::from_str::<serde_json::Value>(&out) {
                    Ok(v) => {
                        state.artifacts.insert(output_key, v);
                    }
                    Err(_) => {
                        // If the model produced invalid JSON, persist the raw output for debugging.
                        state
                            .artifacts
                            .insert(output_key, serde_json::Value::String(out));
                    }
                }
            } else {
                state
                    .artifacts
                    .insert(output_key, serde_json::Value::String(out));
            }
        }
        None => {
            state.log_meta("executor_error", "LLM call failed", serde_json::json!({}));
            state.failure_count += 1;
        }
    }

    // If we just produced a facts table for a grounded request, and it reports missing coverage,
    // automatically expand research (bounded) instead of plowing ahead with shallow evidence.
    maybe_expand_research_loop(&mut state, &step);

    Ok(state)
}

/// Parse an integer from JSON, accepting both `u64` and `i64` representations.
///
/// Connection:
/// - Some models emit numeric fields as signed ints; this keeps executor logic tolerant.
fn get_u64(v: Option<&serde_json::Value>) -> Option<u64> {
    v.and_then(|x| x.as_u64()).or_else(|| {
        v.and_then(|x| x.as_i64())
            .and_then(|n| u64::try_from(n).ok())
    })
}

/// Normalize a JSON value into a string array.
///
/// Connection:
/// - Used when interpreting “missing coverage” lists in facts tables.
fn get_string_array(v: Option<&serde_json::Value>) -> Vec<String> {
    v.and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|q| q.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

/// Renumber plan steps sequentially after programmatic insertion.
///
/// Connection:
/// - `maybe_expand_research_loop` can insert additional tool steps; the UI expects stable numbering.
fn renumber_steps(plan: &mut PlannerOutput) {
    for (idx, s) in plan.steps.iter_mut().enumerate() {
        s.step_id = (idx + 1).to_string();
    }
}

/// If the facts table indicates missing coverage, insert additional research steps (bounded).
///
/// Why this exists:
/// - Grounded runs can easily “underfetch” evidence (blocked sites, missing patch notes, etc.).
/// - This bounded loop increases the chance of producing a complete facts table before code/guidance is generated.
///
/// Connection:
/// - Called by the executor after each step; modifies `state.current_plan` when appropriate.
fn maybe_expand_research_loop(state: &mut SystemState, executed_step: &StepDefinition) {
    // Only trigger after the facts step (LLM-only JSON).
    let is_facts_step = executed_step.tool_binding.is_none()
        && executed_step.output_key.as_deref() == Some("facts")
        && executed_step
            .output_format
            .as_deref()
            .map(|f| f.eq_ignore_ascii_case("json"))
            .unwrap_or(false);
    if !is_facts_step {
        return;
    }
    if !crate::grounding::request_needs_grounding(&state.user_request) {
        return;
    }

    let expansions = get_u64(state.artifacts.get("research_expansions")).unwrap_or(0);
    const MAX_EXPANSIONS: u64 = 2;
    if expansions >= MAX_EXPANSIONS {
        return;
    }

    let facts = match state.artifacts.get("facts") {
        Some(v) => v,
        None => return,
    };

    // Prefer explicit missing/coverage from the fact extractor.
    let missing_len = facts
        .get("missing")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let coverage_score = facts
        .get("coverage_score")
        .and_then(|v| v.as_f64())
        .unwrap_or(1.0);
    let sources_len = facts
        .get("sources")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let facts_len = facts
        .get("facts")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);

    // Heuristic: treat as incomplete if it explicitly says missing, or if it has too few sources/facts.
    let incomplete = missing_len > 0 || coverage_score < 0.85 || sources_len < 2 || facts_len < 6;
    if !incomplete {
        return;
    }

    let mut next_queries = get_string_array(facts.get("next_queries"));
    if next_queries.is_empty() {
        // Fallback: ask for sources + patch changes explicitly.
        next_queries.push(format!("{} sources", state.user_request));
        next_queries.push(format!("{} patch notes changes", state.user_request));
    }
    next_queries.truncate(6);

    let Some(plan) = state.current_plan.as_mut() else {
        return;
    };

    // Insert loop steps immediately after the just-executed facts step.
    let insert_at = state.current_step; // current_step is 1-based; insert after (idx = current_step-1)
    let loop_n = expansions + 1;

    let search_key = format!("search_result_loop{loop_n}");
    let fetch1_key = format!("fetch_loop{loop_n}_1");
    let fetch2_key = format!("fetch_loop{loop_n}_2");

    let steps_to_insert = vec![
        StepDefinition {
            step_id: String::new(),
            action: "Expand research: run additional targeted searches to fill missing facts".into(),
            tool_binding: Some("parallel_search".into()),
            input_params: json!({ "queries": next_queries }),
            output_key: Some(search_key.clone()),
            expected_output: None,
            output_format: None,
            requires_facts: false,
        },
        StepDefinition {
            step_id: String::new(),
            action: "Expand research: fetch the best new source URL from the latest search results".into(),
            tool_binding: Some("http_fetch".into()),
            input_params: json!({ "url": "" }),
            output_key: Some(fetch1_key),
            expected_output: None,
            output_format: None,
            requires_facts: false,
        },
        StepDefinition {
            step_id: String::new(),
            action: "Expand research: fetch a second independent new source URL".into(),
            tool_binding: Some("http_fetch".into()),
            input_params: json!({ "url": "" }),
            output_key: Some(fetch2_key),
            expected_output: None,
            output_format: None,
            requires_facts: false,
        },
        StepDefinition {
            step_id: String::new(),
            action: "Re-extract a grounded fact table from all research artifacts (include URLs + evidence snippets). Output JSON with: facts[] (each has claim+url+evidence), sources[] (urls), missing[] (what is still unknown), next_queries[] (<=6 targeted searches to fill missing), coverage_score (0..1).".into(),
            tool_binding: None,
            input_params: json!({}),
            output_key: Some("facts".into()),
            expected_output: None,
            output_format: Some("json".into()),
            requires_facts: false,
        },
    ];

    // Defensive: don't insert the same loop twice.
    if plan
        .steps
        .iter()
        .any(|s| s.output_key.as_deref() == Some(&search_key))
    {
        return;
    }

    plan.steps.splice(insert_at..insert_at, steps_to_insert);
    renumber_steps(plan);

    state
        .artifacts
        .insert("research_expansions".into(), json!(loop_n));
    state.log_meta(
        "research_expand",
        "Facts coverage incomplete â€” expanding research loop",
        json!({
            "loop": loop_n,
            "missing_len": missing_len,
            "coverage_score": coverage_score,
            "sources_len": sources_len,
            "facts_len": facts_len
        }),
    );
}
