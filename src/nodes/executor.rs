use anyhow::Result;
use crate::{
    llm::ollama::{Message, ModelRole, OllamaClient},
    state::SystemState,
};

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

/// Used when the step has a tool_binding — produce a structured tool call JSON
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
  item costs, or tier lists from memory — only use what is in the artifacts below.
- If you have search/fetch artifacts: use them as your ONLY source for patch-specific facts.
  Quote real numbers, real ability names, real item builds from the actual fetched content.
- If you do NOT have search artifacts for a time-sensitive topic: say clearly
  "I don't have current data on this — the search results weren't available." Do NOT fill the
  gap with training-data guesses dressed up as facts.
- Never cite a knowledge cutoff date and then proceed to answer anyway with made-up specifics.
  Either use real artifact data or honestly say what you don't know.

ARTIFACT USAGE RULES:
- web_search results contain search snippets (short). http_fetch results contain full page text (rich).
- A failed http_fetch (403/timeout) does NOT mean the web search failed — use the snippets.
- If a web_search itself returned success=false, say the search failed.
- When you have real data, use it extensively — quote specific details, ability names, numbers.

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

/// Used for LLM-only steps that must emit machine-readable JSON (e.g. evidence-backed fact tables).
const SYSTEM_PROMPT_LLM_JSON: &str = r#"You are a research extraction agent.
Output ONLY valid JSON. No prose, no markdown, no code fences.

Rules:
- Use ONLY the provided artifacts as sources. Do not use memory or training data.
- If a fact is missing from artifacts, leave it null or omit it; do NOT guess.
- Prefer a compact, structured format that can be used for downstream code generation.
- Include source URLs and short evidence snippets for any non-trivial fact."#;

pub async fn run(mut state: SystemState, llm: &OllamaClient) -> Result<SystemState> {
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
        state.log("executor", "All steps complete");
        state.termination_met = true;
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
    let mut ctx_map = serde_json::Map::new();
    for (k, v) in state.artifacts.iter().filter(|(k, _)| k.ends_with("_result")) {
        ctx_map.insert(k.clone(), v.clone());
    }
    for (k, v) in state.artifacts.iter()
        .filter(|(k, _)| !k.ends_with("_result") && !k.starts_with("repair_"))
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
        let has_facts = state.artifacts.contains_key("facts") || state.artifacts.contains_key("facts_json");
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
        system_prompt_tool(&state.workspace_path)
    } else if wants_json {
        SYSTEM_PROMPT_LLM_JSON.to_string()
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
        // For LLM-only steps, lead with history + question so the model sees everything
        format!(
            "{}User request: {}\n\nCapabilities: {}\nStep ID: {}\nAction: {}\nAvailable artifacts: {}",
            history_block,
            state.user_request,
            state.capabilities,
            step.step_id,
            step.action,
            artifact_context,
        )
    };

    let messages = vec![
        Message::system(system_prompt),
        Message::user(user_prompt),
    ];

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
            llm.chat_stream(messages, synthesis_model, &tok_tx, true, Some(&think_tx)).await.ok()
        } else {
            llm.chat(messages, synthesis_model, false, true).await.ok()
        }
    } else {
        // Tool call generation — needs clean JSON, no thinking
        llm.chat(messages, ModelRole::Fast, true, false).await.ok()
    };

    match output {
        Some(out) => {
            let output_key = step.output_key.clone()
                .unwrap_or_else(|| format!("step_{}", state.current_step));
            if wants_json && !has_tool {
                match serde_json::from_str::<serde_json::Value>(&out) {
                    Ok(v) => {
                        state.artifacts.insert(output_key, v);
                    }
                    Err(_) => {
                        // If the model produced invalid JSON, persist the raw output for debugging.
                        state.artifacts.insert(output_key, serde_json::Value::String(out));
                    }
                }
            } else {
                state.artifacts.insert(output_key, serde_json::Value::String(out));
            }
        }
        None => {
            state.log_meta("executor_error", "LLM call failed", serde_json::json!({}));
            state.failure_count += 1;
        }
    }

    Ok(state)
}
