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
    coder::{
        browser_classification, deterministic_browser_tool_call, latest_verified_browser_output,
        summarize_browser_selectors,
    },
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
fn system_prompt_tool(workspace_path: &str, has_remote_worker: bool) -> String {
    let os = std::env::consts::OS;
    let in_container = in_container();

    let shell_section = if has_remote_worker {
        r#"SHELL TOOL RULES:
- Shell commands run INSIDE the Ubuntu 24.04 Linux VM worker — NOT in the coordinator process or Docker container.
- Always use POSIX/bash syntax: &&, ||, |, $(), head, tail, grep, sed, awk, curl, python3, etc.
- Never use PowerShell syntax (Get-ChildItem, Select-Object, $env:VAR, Write-Host).
- The VM has: bash, python3, curl, wget, git, apt, standard POSIX tools, and full internet access.
- Do NOT use `docker exec` — the VM is not a Docker container.
- Truncate long output with `| head -n 50` or slice in python3 `[:6000]`.

SHELL FETCH PATTERN (when the plan step is shell and action says to fetch a URL):
  Page → text:  curl -sL '<url>' | python3 -c "import sys,re,html; t=sys.stdin.read(); print(re.sub(r'<[^>]+>',' ',html.unescape(t))[:8000])"
  JSON API:     curl -s '<url>' | python3 -m json.tool | head -n 80
  Reddit:       curl -sL -A 'Mozilla/5.0' 'https://old.reddit.com/r/...' | python3 -c "import sys,re; print(re.sub(r'<[^>]+>','',sys.stdin.read())[:6000])"
  Pick the URL directly from the search_result artifact and embed it in the curl command.

HTTP_FETCH TOOL RULES (when the plan step specifies tool_binding="http_fetch"):
  Generate: {"tool": "http_fetch", "params": {"url": "<url>"}}
  If the action says "best URL from search results" and no specific URL is available, use url="".
  The system will auto-resolve url="" to the best available URL from search artifacts.
  Do NOT substitute a shell call when the step binding is http_fetch."#
    } else {
        r#"SHELL TOOL RULES:
- The shell tool runs commands in the coordinator process environment (Linux sh -lc or Windows PowerShell).
- Use syntax appropriate to the runtime OS shown in RUNTIME CONTEXT above.
- On Linux/macOS: avoid PowerShell cmdlets (Select-Object, Get-ChildItem, $env:VAR).
- On Windows: avoid bash-only syntax ($(), backticks, GNU flags).
- Prefer simple commands. Truncate with `| head -n 20` (Linux) or `| Select-Object -First 20` (Windows)."#
    };

    format!(
        r#"You are a tool call generator.
Output ONLY a JSON object describing the tool call. No prose, no markdown.
{{ "tool": "tool_name", "params": {{ ... }} }}

RUNTIME CONTEXT:
- coordinator_os: {os}
- coordinator_in_container: {in_container}
- has_remote_vm_worker: {has_remote_worker}
- workspace_path: {workspace_path}

{shell_section}

FILESYSTEM TOOL RULES:
- In remote mode, the filesystem tool operates on the VM worker workspace and `workspace_path` will normally be `/workspace`.
- In local mode, the filesystem tool is rooted at `workspace_path` on the coordinator side.
- Paths are always relative to the active execution workspace.
- ALWAYS include both "action" and "path" in params. Never emit empty params.
- Write a file:      {{"tool":"filesystem","params":{{"action":"write","path":"hello.py","content":"print('hi')"}}}}
- Read a file:       {{"tool":"filesystem","params":{{"action":"read","path":"hello.py"}}}}
- List directory:    {{"tool":"filesystem","params":{{"action":"list","path":"."}}}}
- Find files:        {{"tool":"filesystem","params":{{"action":"find","path":".","pattern":"planner"}}}}
- Search in files:   {{"tool":"filesystem","params":{{"action":"grep","path":"src","query":"ToolResult"}}}}
- Writes should be precise and scoped (one file per step if possible).

SHELL TOOL SCHEMA (required field: command):
  {{"tool":"shell","params":{{"command":"<bash command string>","cwd":"."}}}}
Examples:
  Run a script:   {{"tool":"shell","params":{{"command":"python3 hello_plan.py","cwd":"."}}}}
  Check output:   {{"tool":"shell","params":{{"command":"cat hello_plan.py","cwd":"."}}}}
  Install pkg:    {{"tool":"shell","params":{{"command":"pip3 install requests","cwd":"."}}}}
  Multi-command:  {{"tool":"shell","params":{{"command":"python3 hello.py && echo done","cwd":"."}}}}
  Append text:    {{"tool":"shell","params":{{"command":"printf '%s\\n' 'edited by shell' >> hello_vm_2.txt","cwd":"."}}}}
- The `command` field is REQUIRED. Omitting it will cause a hard failure — always include it.
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

IMPORTANT — START YOUR RESPONSE WITH THE ANSWER:
Do NOT begin with "Okay", "Sure", "Let me", "First I need to", "Looking at", "I need to",
"Alright", "Let's", "I should", or any other reasoning preamble or thinking-out-loud opener.
Do NOT explain what you are about to do. Just do it.
Start the first word of your response with the actual content, answer, or code."#;

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
        } else if missing.len() == 1
            && missing[0] == "answer"
            && !has_failed_tool_result(&state.artifacts)
        {
            let synthesized = synthesize_success_answer(&state, &plan, &state.artifacts);
            state.artifacts.insert(
                "answer".to_string(),
                serde_json::Value::String(synthesized),
            );
            state.log(
                "executor",
                "All tool steps complete; synthesized final answer artifact",
            );
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

    // Build the evidence bundle the model sees for this step.
    // Tool results come first because they are the most trustworthy record of what happened.
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

    if let Some(intent) = &state.coding_intent {
        if let Some(tool_call) =
            deterministic_browser_tool_call(intent, &state.user_request, &step, &state.artifacts)
        {
            let output_key = step
                .output_key
                .clone()
                .unwrap_or_else(|| format!("step_{}", state.current_step));
            state
                .artifacts
                .insert(output_key, serde_json::Value::String(tool_call.to_string()));
            state.log(
                "executor",
                &format!(
                    "Step {} deterministic browser scaffold/tool call",
                    step.step_id
                ),
            );
            return Ok(state);
        }
    }

    // Fast path: if the planner already filled in complete tool params, use them directly
    // without an LLM call. This is more reliable than asking the model to re-derive params
    // from a vague action description.
    if let Some(tool_name) = &step.tool_binding {
        let params = &step.input_params;
        let can_passthrough = match tool_name.as_str() {
            "shell" => params.get("command").and_then(|v| v.as_str()).map(|s| !s.trim().is_empty()).unwrap_or(false),
            "filesystem" => params.get("action").and_then(|v| v.as_str()).map(|s| !s.trim().is_empty()).unwrap_or(false)
                && (params["action"] != "write" || params.get("content").and_then(|v| v.as_str()).map(|s| !s.trim().is_empty()).unwrap_or(false)),
            _ => false,
        };
        if can_passthrough {
            let tool_call = serde_json::json!({"tool": tool_name, "params": params});
            let output_key = step.output_key.clone().unwrap_or_else(|| format!("step_{}", state.current_step));
            state.artifacts.insert(output_key, serde_json::Value::String(tool_call.to_string()));
            state.log("executor", &format!("Step {} passthrough (planner-provided params)", step.step_id));
            return Ok(state);
        }
    }

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

    let has_remote_worker = state
        .capabilities
        .get("execution_mode")
        .and_then(|v| v.as_str())
        .map(|m| m.eq_ignore_ascii_case("remote") || m.eq_ignore_ascii_case("auto"))
        .unwrap_or(false)
        || state
            .capabilities
            .get("worker_url")
            .and_then(|v| v.as_str())
            .map(|u| !u.trim().is_empty())
            .unwrap_or(false);

    let system_prompt = if has_tool {
        // For coding tool steps, append adapter-specific rules
        let base = system_prompt_tool(&state.workspace_path, has_remote_worker);
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

    let coding_guidance = state
        .artifacts
        .get("coding_executor_guidance")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let user_prompt = if has_tool {
        format!(
            "{}User request: {}\n\nRuntime OS: {}\nIn container: {}\nCapabilities: {}\n\nStep ID: {}\nAction: {}\nInput params: {}\nAvailable artifacts: {}\n\nCoding guidance:\n{}",
            history_block,
            state.user_request,
            std::env::consts::OS,
            in_container(),
            state.capabilities,
            step.step_id,
            step.action,
            step.input_params,
            artifact_context,
            coding_guidance,
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
            "{}User request: {}\n\nCapabilities: {}\nStep ID: {}\nAction: {}\nAvailable artifacts: {}\n\nCoding guidance:\n{}{}",
            history_block,
            state.user_request,
            state.capabilities,
            step.step_id,
            step.action,
            artifact_context,
            coding_guidance,
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
                        let v = if output_key == "facts"
                            && crate::grounding::request_needs_grounding(&state.user_request)
                        {
                            normalize_facts_table(v, &state.user_request)
                        } else {
                            v
                        };
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
                let trimmed = out.trim().to_string();
                if !has_tool
                    && output_key == "answer"
                    && trimmed.is_empty()
                    && !has_failed_tool_result(&state.artifacts)
                {
                    let synthesized = synthesize_success_answer(&state, &plan, &state.artifacts);
                    state.artifacts.insert(
                        output_key,
                        serde_json::Value::String(synthesized),
                    );
                    state.log(
                        "executor",
                        "LLM returned empty final answer; synthesized answer from successful artifacts",
                    );
                } else {
                    state
                        .artifacts
                        .insert(output_key, serde_json::Value::String(out));
                }
            }
        }
        None => {
            let output_key = step
                .output_key
                .clone()
                .unwrap_or_else(|| format!("step_{}", state.current_step));
            if !has_tool
                && output_key == "answer"
                && !has_failed_tool_result(&state.artifacts)
            {
                let synthesized = synthesize_success_answer(&state, &plan, &state.artifacts);
                state.artifacts.insert(
                    output_key,
                    serde_json::Value::String(synthesized),
                );
                state.log(
                    "executor",
                    "LLM final answer step failed; synthesized answer from successful artifacts",
                );
            } else {
                state.log_meta("executor_error", "LLM call failed", serde_json::json!({}));
                state.failure_count += 1;
            }
        }
    }

    // If we just produced a facts table for a grounded request, and it reports missing coverage,
    // automatically expand research (bounded) instead of plowing ahead with shallow evidence.
    maybe_expand_research_loop(&mut state, &step);

    Ok(state)
}

fn has_failed_tool_result(
    artifacts: &std::collections::HashMap<String, serde_json::Value>,
) -> bool {
    artifacts.iter().any(|(k, v)| {
        k.ends_with("_result") && v.get("success").and_then(|b| b.as_bool()) == Some(false)
    })
}

fn tool_result_is_success(result: &serde_json::Value) -> bool {
    result.get("success").and_then(|b| b.as_bool()) != Some(false)
}

fn synthesize_success_answer(
    state: &crate::state::SystemState,
    plan: &crate::state::PlannerOutput,
    artifacts: &std::collections::HashMap<String, serde_json::Value>,
) -> String {
    if let Some(ci) = &state.coding_intent {
        match ci.task_class.as_str() {
            "script_task" | "workspace_file_task" | "browser_automation_task" => {
                let file_views = dedupe_blocks(collect_filesystem_views(plan, artifacts));
                let shell_outputs = dedupe_blocks(collect_shell_outputs(plan, artifacts));
                let warning = recovered_warning_text(artifacts);
                let mut sections: Vec<String> = Vec::new();

                if ci.task_class == "browser_automation_task" {
                    let verified = latest_verified_browser_output(artifacts);
                    if verified.is_none() {
                        return summarize_browser_contract_failure(artifacts);
                    }
                }

                if let Some(text) = warning {
                    sections.push(format!("Warning:\n\n{text}"));
                }
                if !file_views.is_empty() {
                    sections.push(format!("File contents:\n\n{}", file_views.join("\n\n")));
                }
                if !shell_outputs.is_empty() {
                    sections.push(format!("Exact command output:\n\n{}", shell_outputs.join("\n\n")));
                }
                if ci.task_class == "browser_automation_task" {
                    if let Some(verified) = latest_verified_browser_output(artifacts) {
                        if let Some(selectors) = summarize_browser_selectors(&verified) {
                            sections.push(format!("Selectors used:\n\n{selectors}"));
                        }
                    }
                    if let Some(conclusion) = browser_task_conclusion(artifacts) {
                        sections.push(format!("Conclusion:\n\n{conclusion}"));
                    }
                }

                if !sections.is_empty() {
                    return sections.join("\n\n");
                }
            }
            _ => {}
        }
    }

    let mut lines: Vec<String> = Vec::new();

    for step in &plan.steps {
        let Some(tool) = &step.tool_binding else {
            continue;
        };
        let Some(output_key) = &step.output_key else {
            continue;
        };
        let artifact_key = format!("{output_key}_result");
        let Some(v) = artifacts.get(&artifact_key) else {
            continue;
        };
        let out = v.get("output").unwrap_or(v);

        match tool.as_str() {
            "filesystem" => {
                if let Some(path) = out.get("path").and_then(|x| x.as_str()) {
                    if let Some(content) = out.get("content").and_then(|x| x.as_str()) {
                        lines.push(format!("Created or updated `{path}`:"));
                        lines.push(content.trim().to_string());
                    } else if out.get("tree").is_some() {
                        lines.push(format!("Listed `{path}` successfully."));
                    } else {
                        lines.push(format!("Filesystem step completed for `{path}`."));
                    }
                }
            }
            "shell" => {
                if let Some(stdout) = out.get("stdout").and_then(|x| x.as_str()) {
                    let trimmed = stdout.trim();
                    if !trimmed.is_empty() {
                        lines.push(trimmed.to_string());
                    } else {
                        lines.push("Shell command completed successfully.".to_string());
                    }
                }
            }
            _ => {
                if let Some(body) = out.get("body").and_then(|x| x.as_str()) {
                    let trimmed = body.trim();
                    if !trimmed.is_empty() {
                        lines.push(trimmed.to_string());
                    }
                }
            }
        }
    }

    if lines.is_empty() {
        "Task completed successfully.".to_string()
    } else {
        lines.join("\n\n")
    }
}

fn collect_filesystem_views(
    plan: &crate::state::PlannerOutput,
    artifacts: &std::collections::HashMap<String, serde_json::Value>,
) -> Vec<String> {
    let mut blocks = Vec::new();
    for step in &plan.steps {
        if step.tool_binding.as_deref() != Some("filesystem") {
            continue;
        }
        let Some(output_key) = &step.output_key else {
            continue;
        };
        let artifact_key = format!("{output_key}_result");
        let Some(result) = artifacts.get(&artifact_key) else {
            continue;
        };
        if result.get("success").and_then(|b| b.as_bool()) == Some(false) {
            continue;
        }
        let output = result.get("output").unwrap_or(result);
        let Some(path) = output.get("path").and_then(|x| x.as_str()) else {
            continue;
        };
        if output.get("is_text").and_then(|v| v.as_bool()) == Some(false) {
            continue;
        }
        if let Some(content) = output.get("content").and_then(|x| x.as_str()) {
            blocks.push(format!("`{path}`:\n\n{content}"));
        }
    }
    blocks
}

fn collect_shell_outputs(
    plan: &crate::state::PlannerOutput,
    artifacts: &std::collections::HashMap<String, serde_json::Value>,
) -> Vec<String> {
    let mut blocks = Vec::new();
    for step in &plan.steps {
        if step.tool_binding.as_deref() != Some("shell") {
            continue;
        }
        let Some(output_key) = &step.output_key else {
            continue;
        };
        let artifact_key = format!("{output_key}_result");
        let Some(result) = artifacts.get(&artifact_key) else {
            continue;
        };
        if result.get("success").and_then(|b| b.as_bool()) == Some(false) {
            continue;
        }
        let output = result.get("output").unwrap_or(result);
        let command = output
            .get("command")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .trim();
        let stdout = output
            .get("stdout")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .trim();
        let stderr = output
            .get("stderr")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .trim();
        let mut block = String::new();
        if !command.is_empty() {
            block.push_str("$ ");
            block.push_str(command);
            block.push('\n');
        }
        if !stdout.is_empty() {
            block.push_str(stdout);
            if !stdout.ends_with('\n') && !stderr.is_empty() {
                block.push('\n');
            }
        }
        if !stderr.is_empty() {
            block.push_str(stderr);
        }
        let trimmed = block.trim();
        if !trimmed.is_empty() {
            blocks.push(trimmed.to_string());
        }
    }
    blocks
}

fn dedupe_blocks(blocks: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for block in blocks {
        let normalized = block.trim().to_string();
        if normalized.is_empty() {
            continue;
        }
        if seen.insert(normalized.clone()) {
            out.push(normalized);
        }
    }
    out
}

fn browser_task_conclusion(
    artifacts: &std::collections::HashMap<String, serde_json::Value>,
) -> Option<String> {
    let output = latest_verified_browser_output(artifacts)?;
    let joined = output.to_lowercase();
    if joined.trim().is_empty() {
        return None;
    }
    if let Some(line) = output
        .lines()
        .find(|line| line.trim_start().starts_with("Conclusion:"))
    {
        return Some(line.trim().to_string());
    }
    match browser_classification(&output) {
        Some("access denied") => {
            return Some(
                "The page was reached, but the result is an access-denied response. The agent did not claim extraction success beyond that evidence."
                    .to_string(),
            )
        }
        Some("challenge page") => {
            return Some(
                "The target page loaded, but browser automation hit an anti-bot or challenge flow rather than the normal listing content.".to_string(),
            )
        }
        _ => {}
    }
    if joined.contains("timed out after") {
        return Some(
            "The browser task did not complete before timeout, so the page may be slow, blocked, or waiting on a challenge flow.".to_string(),
        );
    }
    if joined.contains("classification: normal page")
        || (joined.contains("page title:") && joined.contains("final url:"))
    {
        if joined.contains("extracted visible content successfully") {
            return Some(
                "The browser task reached a normal page and extracted visible content successfully."
                    .to_string(),
            );
        }
        return Some(
            "The browser probe ran successfully and returned page-level diagnostics from the active VM runtime.".to_string(),
        );
    }
    None
}

fn summarize_browser_contract_failure(
    artifacts: &std::collections::HashMap<String, serde_json::Value>,
) -> String {
    let mut failures: Vec<(&String, &serde_json::Value)> = artifacts
        .iter()
        .filter(|(k, v)| {
            k.ends_with("_result") && v.get("success").and_then(|b| b.as_bool()) == Some(false)
        })
        .collect();
    failures.sort_by_key(|(k, _)| *k);

    if let Some((k, v)) = failures.last() {
        let code = v
            .get("error_code")
            .and_then(|x| x.as_str())
            .unwrap_or("unknown_error");
        let trace = v
            .get("trace")
            .and_then(|x| x.as_str())
            .unwrap_or("The browser task did not return additional details.")
            .trim();
        return format!(
            "The browser task did not finish cleanly. The last failing step was `{k}` ({code}). {trace}"
        );
    }

    "The browser task did not finish cleanly. It never produced a verified browser execution with exact output markers."
        .to_string()
}

fn recovered_warning_text(
    artifacts: &std::collections::HashMap<String, serde_json::Value>,
) -> Option<String> {
    if !has_failed_tool_result(artifacts) {
        return None;
    }
    let latest = artifacts
        .iter()
        .filter(|(k, v)| k.ends_with("_result") && tool_result_is_success(v))
        .max_by_key(|(_, v)| {
            v.get("timestamp")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string()
        })?;
    Some(format!(
        "Completed successfully after recovering from earlier tool failures. Final successful step: `{}`.",
        latest.0
    ))
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

/// Normalize the facts-table JSON shape so downstream automation can rely on required fields.
///
/// Why:
/// - Models occasionally output a paragraph/string for `facts`, omit `sources`, or omit numeric
///   `coverage_score`, which triggers repair loops even though we could recover deterministically.
fn normalize_facts_table(v: serde_json::Value, user_request: &str) -> serde_json::Value {
    use serde_json::json;
    let mut obj = match v.as_object() {
        Some(o) => o.clone(),
        None => {
            return json!({
                "facts": [],
                "sources": [],
                "missing": ["facts output was not a JSON object"],
                "next_queries": crate::grounding::decompose_search_queries(user_request),
                "coverage_score": 0.0
            });
        }
    };

    let mut sources: Vec<String> = obj
        .get("sources")
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|s| s.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let mut missing: Vec<String> = obj
        .get("missing")
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|s| s.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let next_queries: Vec<String> = obj
        .get("next_queries")
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|s| s.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| crate::grounding::decompose_search_queries(user_request));

    let coverage_score = obj
        .get("coverage_score")
        .and_then(|x| x.as_f64())
        .unwrap_or_else(|| {
            // Fall back to a conservative score based on whether any sources exist.
            if !sources.is_empty() { 0.7 } else { 0.2 }
        });

    // Normalize `facts` into an array of {claim,url,evidence} objects.
    let facts_val = obj.get("facts").cloned().unwrap_or_else(|| json!([]));
    let mut facts_out: Vec<serde_json::Value> = Vec::new();
    match facts_val {
        serde_json::Value::Array(arr) => {
            for item in arr {
                if let Some(o) = item.as_object() {
                    let claim = o.get("claim").and_then(|x| x.as_str()).unwrap_or("").trim();
                    let url = o.get("url").and_then(|x| x.as_str()).unwrap_or("").trim();
                    let evidence = o
                        .get("evidence")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .trim();
                    if claim.is_empty() && url.is_empty() && evidence.is_empty() {
                        continue;
                    }
                    if !url.is_empty() {
                        sources.push(url.to_string());
                    }
                    facts_out.push(json!({
                        "claim": claim,
                        "url": url,
                        "evidence": evidence
                    }));
                } else if let Some(s) = item.as_str() {
                    let s = s.trim();
                    if !s.is_empty() {
                        facts_out.push(json!({ "claim": s, "url": "", "evidence": "" }));
                    }
                }
            }
        }
        serde_json::Value::String(s) => {
            let s = s.trim();
            if !s.is_empty() {
                missing.push("facts was a paragraph, not an array; normalized to single claim".into());
                facts_out.push(json!({ "claim": s, "url": "", "evidence": "" }));
            }
        }
        _ => {
            missing.push("facts field had unexpected type; normalized to empty array".into());
        }
    }

    // De-dupe sources.
    sources.sort();
    sources.dedup();

    json!({
        "facts": facts_out,
        "sources": sources,
        "missing": missing,
        "next_queries": next_queries,
        "coverage_score": coverage_score
    })
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

    // Rust release notes have stable, official sources and do not benefit from iterative search loops.
    // The loop can accidentally drag in irrelevant docs pages and create replan churn.
    let req_lower = state.user_request.to_lowercase();
    if req_lower.contains("rust")
        && (req_lower.contains("release") || req_lower.contains("changelog") || req_lower.contains("what changed"))
    {
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
