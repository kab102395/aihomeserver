//! Grounding contract helpers.
//!
//! “Grounding” means: for time-sensitive/research-like tasks, the system should not
//! generate confident answers from training data. Instead it should:
//! - search/fetch sources via tools
//! - extract a structured “facts table” artifact with provenance
//! - require later steps to set `requires_facts: true`
//!
//! The planner uses these heuristics to decide whether grounding is required.

use crate::state::{PlannerOutput, StepDefinition};

/// Heuristic classifier: returns true if a request likely needs grounded evidence.
pub fn request_needs_grounding(user_request: &str) -> bool {
    let s = user_request.to_lowercase();
    if s.contains("patch")
        || s.contains("version")
        || s.contains("latest")
        || s.contains("most recent")
    {
        return true;
    }

    // Dota 2 (and similar live-service game) meta/build questions are time-sensitive
    // even when the user doesn't explicitly mention a patch number.
    let is_dota = s.contains("dota 2") || s.contains("dota2") || s.contains("r/dota2");
    if is_dota
        && (s.contains("build")
            || s.contains("item")
            || s.contains("skill")
            || s.contains("playstyle")
            || s.contains("guide")
            || s.contains("meta")
            || s.contains("how to play")
            || s.contains("ability usage")
            || s.contains("combo"))
    {
        return true;
    }

    // Heuristic: detect digit '.' digit patterns (e.g. 7.41b, 1.2, v2.3)
    let b = s.as_bytes();
    for i in 0..b.len().saturating_sub(2) {
        if !b[i].is_ascii_digit() {
            continue;
        }
        // scan leading digits
        let mut j = i;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j < b.len() && b[j] == b'.' {
            j += 1;
            if j < b.len() && b[j].is_ascii_digit() {
                return true;
            }
        }
    }

    false
}

/// Extract a version-like token such as `7.41b`, `1.2`, `v2.3` from a string.
///
/// Connection:
/// - Used by `decompose_search_queries` to generate better search prompts when the user
///   references a patch/version.
fn extract_version_token(s: &str) -> Option<String> {
    let b = s.as_bytes();
    for i in 0..b.len().saturating_sub(2) {
        if !b[i].is_ascii_digit() {
            continue;
        }
        // leading digits
        let mut j = i;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j >= b.len() || b[j] != b'.' {
            continue;
        }
        j += 1;
        if j >= b.len() || !b[j].is_ascii_digit() {
            continue;
        }
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        // optional trailing letters (e.g. 7.41b)
        while j < b.len() && b[j].is_ascii_alphabetic() {
            j += 1;
        }
        return Some(s[i..j].to_string());
    }
    None
}

/// Extract “subjects” after the word `research` to drive query decomposition.
///
/// Example:
/// - `research Kez and Invoker on Dota 2 patch 7.41b ...` -> `["Kez", "Invoker"]`
///
/// Connection:
/// - Used by `decompose_search_queries` to create multiple targeted searches.
fn extract_subjects(user_request: &str) -> Vec<String> {
    let lower = user_request.to_lowercase();
    let start = lower.find("research").map(|i| i + "research".len());
    let Some(start) = start else {
        return vec![];
    };

    // Find a likely end delimiter after "research ..."
    let mut end = user_request.len();
    for needle in [" on ", " for ", " in ", ". then", ".then", "\n"] {
        if let Some(rel) = lower[start..].find(needle) {
            end = end.min(start + rel);
        }
    }

    let slice = user_request
        .get(start..end)
        .unwrap_or("")
        .trim()
        .trim_matches([':', '-', '—', ' ']);
    if slice.is_empty() {
        return vec![];
    }

    // Split by commas and "and"
    let mut parts: Vec<String> = Vec::new();
    for chunk in slice.split(',') {
        let c = chunk.trim();
        if c.is_empty() {
            continue;
        }
        // further split "A and B" forms
        let mut pushed = false;
        for sub in c.split(" and ") {
            let s = sub.trim().trim_matches(['"', '\'']);
            if !s.is_empty() {
                parts.push(s.to_string());
                pushed = true;
            }
        }
        if !pushed && !c.is_empty() {
            parts.push(c.to_string());
        }
    }

    // De-dupe (case-insensitive) and cap
    let mut seen = std::collections::HashSet::<String>::new();
    let mut out = Vec::new();
    for p in parts {
        let k = p.to_lowercase();
        if seen.insert(k) {
            out.push(p);
        }
        if out.len() >= 8 {
            break;
        }
    }
    out
}

/// Produce multiple search queries for time-sensitive requests.
///
/// Why this exists:
/// - Grounded runs often need more than one source (patch notes + guides + trackers).
/// - Parallel searches reduce the chance a single blocked/low-quality page breaks the run.
///
/// Connection:
/// - Used by `enforce_grounding_contract` to add a `parallel_search` step.
fn decompose_search_queries(user_request: &str) -> Vec<String> {
    let lower = user_request.to_lowercase();
    let version = extract_version_token(&lower);
    let subjects = extract_subjects(user_request);

    let is_dota = lower.contains("dota");
    let game = if is_dota { "Dota 2" } else { "" };
    let mut queries: Vec<String> = Vec::new();

    if let Some(v) = &version {
        if !game.is_empty() {
            queries.push(format!("{game} {v} patch notes changes"));
        } else {
            queries.push(format!("{v} patch notes changes"));
        }
    }

    if !subjects.is_empty() {
        for s in subjects.iter().take(4) {
            if let Some(v) = &version {
                queries.push(format!("{s} {game} {v} skill build item build"));
                queries.push(format!("{s} {game} {v} ability changes"));
            } else {
                queries.push(format!("{s} {game} build guide playstyle"));
            }
        }
        if let Some(v) = &version {
            if is_dota {
                queries.push(format!("{game} {v} meta builds protracker"));
            }
        }
    } else {
        // Fallback: keep one literal query but add a couple of "source finding" variants.
        queries.push(user_request.to_string());
        if let Some(v) = &version {
            queries.push(format!("{v} changes summary"));
        }
        queries.push(format!("{user_request} sources"));
    }

    // De-dupe and cap at 6 (parallel_search internal cap).
    let mut seen = std::collections::HashSet::<String>::new();
    let mut out = Vec::new();
    for q in queries {
        let q2 = q.trim().to_string();
        if q2.is_empty() {
            continue;
        }
        let key = q2.to_lowercase();
        if seen.insert(key) {
            out.push(q2);
        }
        if out.len() >= 6 {
            break;
        }
    }
    out
}

/// Renumber `step_id` fields to keep them sequential after inserting/removing steps.
///
/// Connection:
/// - `enforce_grounding_contract` inserts mandatory research/facts steps and must keep
///   the plan schema tidy for the executor/UI.
fn renumber_steps(steps: &mut [StepDefinition]) {
    for (idx, s) in steps.iter_mut().enumerate() {
        s.step_id = (idx + 1).to_string();
    }
}

/// Check whether an artifact output key is already used by the plan.
///
/// Connection:
/// - The UI and runtime treat `output_key` as the artifact map key; collisions would
///   overwrite earlier outputs and break replay/debugging.
fn output_key_available(plan: &PlannerOutput, key: &str) -> bool {
    !plan
        .steps
        .iter()
        .any(|s| s.output_key.as_deref() == Some(key))
}

/// Pick a non-colliding `output_key` based on a preferred base name.
///
/// Connection:
/// - Used when inserting mandatory steps (search/facts) so artifacts remain stable.
fn pick_unique_output_key(plan: &PlannerOutput, base: &str) -> Option<String> {
    if output_key_available(plan, base) {
        return Some(base.to_string());
    }
    for i in 2..=9 {
        let k = format!("{base}_{i}");
        if output_key_available(plan, &k) {
            return Some(k);
        }
    }
    None
}

/// Enforce the "facts table + requires_facts" contract for patch/version/meta research so
/// the system cannot generate patch-specific code/guides without grounded evidence.
pub fn enforce_grounding_contract(
    user_request: &str,
    capabilities: &serde_json::Value,
    plan: &mut PlannerOutput,
) {
    if !request_needs_grounding(user_request) {
        // Sanity: if grounding isn't required, we must not allow a stray `requires_facts`
        // flag from the planner to deadlock execution.
        let has_facts_step = plan.steps.iter().any(|s| {
            s.output_key.as_deref() == Some("facts")
                && s.tool_binding.is_none()
                && s.output_format.as_deref().map(|f| f.eq_ignore_ascii_case("json")) == Some(true)
        });
        if !has_facts_step {
            for step in plan.steps.iter_mut() {
                step.requires_facts = false;
            }
        }
        return;
    }

    // Grounded research flows are inherently more failure-prone (network, parsing, etc.).
    // Force Standard risk so the Critic runs and the system can repair/replan rather than
    // blindly continuing after missing/failed evidence.
    plan.risk_score = plan.risk_score.max(4);

    let search_configured = capabilities
        .get("search_url_configured")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    // If web research is mandatory but search is not configured, do not force a broken tool
    // step into the plan. Instead, return a single step that tells the user what to fix.
    if !search_configured {
        plan.steps = vec![StepDefinition {
            step_id: "1".into(),
            action: "Explain that this request requires web research, but search is not configured; instruct user to set SEARCH_URL / configure SearXNG and retry".into(),
            tool_binding: None,
            input_params: serde_json::json!({}),
            output_key: Some("answer".into()),
            expected_output: None,
            output_format: None,
            requires_facts: false,
        }];
        plan.expected_outputs = vec!["answer".into()];
        plan.completion_criteria = vec![
            "User informed that web research is required and how to enable it (SEARCH_URL/SearXNG).".into(),
        ];
        return;
    }

    // Ensure we actually do web research before extracting facts.
    let has_research_tool = plan.steps.iter().any(|s| {
        matches!(
            s.tool_binding.as_deref(),
            Some("parallel_search") | Some("web_search") | Some("http_fetch")
        )
    });
    if !has_research_tool {
        let output_key = if plan
            .steps
            .iter()
            .any(|s| s.output_key.as_deref() == Some("search_result"))
        {
            None
        } else {
            Some("search_result".into())
        };

        let queries = decompose_search_queries(user_request);

        plan.steps.insert(
            0,
            StepDefinition {
                step_id: String::new(),
                action: "Search the web for up-to-date sources relevant to the user's request"
                    .into(),
                tool_binding: Some("parallel_search".into()),
                input_params: serde_json::json!({ "queries": queries }),
                output_key,
                expected_output: None,
                output_format: None,
                requires_facts: false,
            },
        );
        renumber_steps(&mut plan.steps);
    }

    // If the plan already had a parallel_search but only searched the raw user request,
    // expand it into multiple targeted queries for deeper research.
    if let Some(step) = plan
        .steps
        .iter_mut()
        .find(|s| s.tool_binding.as_deref() == Some("parallel_search"))
    {
        let current_queries = step
            .input_params
            .get("queries")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        let is_trivial = current_queries <= 1;
        if is_trivial {
            let queries = decompose_search_queries(user_request);
            if queries.len() >= 2 {
                step.input_params = serde_json::json!({ "queries": queries });
            }
        }
    }

    let facts_idx = plan.steps.iter().position(|s| {
        s.output_key.as_deref() == Some("facts")
            && s.tool_binding.is_none()
            && s.output_format
                .as_deref()
                .map(|f| f.eq_ignore_ascii_case("json"))
                .unwrap_or(false)
    });

    let mut facts_idx = match facts_idx {
        Some(i) => i,
        None => {
            // Insert facts extraction step after the last research tool step (search/fetch).
            let insert_at = plan
                .steps
                .iter()
                .rposition(|s| {
                    matches!(
                        s.tool_binding.as_deref(),
                        Some("parallel_search") | Some("web_search") | Some("http_fetch")
                    )
                })
                .map(|i| i + 1)
                .unwrap_or(0);

            plan.steps.insert(
                insert_at,
                StepDefinition {
                    step_id: String::new(),
                    action: "Extract a grounded fact table from research artifacts (include URLs + evidence snippets). Output JSON with: facts[] (each has claim+url+evidence), sources[] (urls), missing[] (what is still unknown), next_queries[] (<=6 targeted searches to fill missing), coverage_score (0..1).".into(),
                    tool_binding: None,
                    input_params: serde_json::json!({}),
                    output_key: Some("facts".into()),
                    expected_output: None,
                    output_format: Some("json".into()),
                    requires_facts: false,
                },
            );
            renumber_steps(&mut plan.steps);
            insert_at
        }
    };

    // If a facts step already existed, strengthen its instructions so downstream automation
    // can expand research when coverage is missing.
    if let Some(facts_step) = plan
        .steps
        .iter_mut()
        .find(|s| s.output_key.as_deref() == Some("facts"))
    {
        if !facts_step.action.to_lowercase().contains("next_queries") {
            facts_step.action = "Extract a grounded fact table from research artifacts (include URLs + evidence snippets). Output JSON with: facts[] (each has claim+url+evidence), sources[] (urls), missing[] (what is still unknown), next_queries[] (<=6 targeted searches to fill missing), coverage_score (0..1).".into();
            facts_step.output_format = Some("json".into());
        }
    }

    // Ensure we fetch at least one (ideally two) pages before extracting facts; search snippets alone
    // are too shallow for patch/version questions.
    let has_fetch_before_facts = plan
        .steps
        .iter()
        .take(facts_idx)
        .any(|s| s.tool_binding.as_deref() == Some("http_fetch"));
    if !has_fetch_before_facts {
        // Insert just before the facts step so the extractor sees the fetched content.
        let fetch1_key = pick_unique_output_key(plan, "fetch1_result");
        let fetch2_key = pick_unique_output_key(plan, "fetch2_result");

        // First fetch: best URL overall from search artifacts
        plan.steps.insert(
            facts_idx,
            StepDefinition {
                step_id: String::new(),
                action: "Fetch the most relevant source URL from the search results".into(),
                tool_binding: Some("http_fetch".into()),
                input_params: serde_json::json!({ "url": "" }),
                output_key: fetch1_key,
                expected_output: None,
                output_format: None,
                requires_facts: false,
            },
        );
        // Second fetch: different URL (tool execution excludes already fetched URLs)
        plan.steps.insert(
            facts_idx + 1,
            StepDefinition {
                step_id: String::new(),
                action: "Fetch a second independent source URL (different domain if possible)"
                    .into(),
                tool_binding: Some("http_fetch".into()),
                input_params: serde_json::json!({ "url": "" }),
                output_key: fetch2_key,
                expected_output: None,
                output_format: None,
                requires_facts: false,
            },
        );
        renumber_steps(&mut plan.steps);
        // Facts step shifted down by 2.
        facts_idx = plan
            .steps
            .iter()
            .position(|s| s.output_key.as_deref() == Some("facts"))
            .unwrap_or(facts_idx + 2);
    }

    // Any later step (tool or LLM) that could generate patch-specific output must require facts.
    for (idx, step) in plan.steps.iter_mut().enumerate() {
        if idx <= facts_idx {
            continue;
        }
        if step.output_key.as_deref() == Some("facts") {
            continue;
        }
        step.requires_facts = true;
    }
}

/// Enforce "auto KB saving" policy by appending a final `save_knowledge` step when enabled.
///
/// This is deterministic (no reliance on the model remembering a long instruction block).
#[allow(dead_code)]
pub fn enforce_auto_kb_policy(
    user_request: &str,
    capabilities: &serde_json::Value,
    plan: &mut PlannerOutput,
) {
    let mode = capabilities
        .get("auto_kb_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("off")
        .to_lowercase();
    if mode == "off" {
        return;
    }

    // Never auto-save potentially sensitive requests.
    let s = user_request.to_lowercase();
    let sensitive = [
        "password",
        "api key",
        "apikey",
        "token",
        "secret",
        "private key",
        "ssh key",
        "seed phrase",
    ]
    .iter()
    .any(|w| s.contains(w));
    if sensitive || s.contains("don't save") || s.contains("do not save") {
        return;
    }

    let already_saves = plan
        .steps
        .iter()
        .any(|st| st.tool_binding.as_deref() == Some("save_knowledge"));
    if already_saves {
        return;
    }

    let uses_research = plan.steps.iter().any(|st| {
        matches!(
            st.tool_binding.as_deref(),
            Some("parallel_search") | Some("web_search") | Some("http_fetch")
        )
    });

    if mode == "research" && !uses_research {
        return;
    }

    // Append a final save step. Tool execution will auto-fill missing topic/content and can
    // also skip saving if content is too short (see tool_execution save_knowledge handler).
    plan.steps.push(StepDefinition {
        step_id: (plan.steps.len() + 1).to_string(),
        action: "Auto-save the final answer into the knowledge base for future reuse".into(),
        tool_binding: Some("save_knowledge".into()),
        input_params: serde_json::json!({ "auto": true }),
        output_key: Some("knowledge_saved".into()),
        expected_output: None,
        output_format: None,
        requires_facts: false,
    });
}

/// Heuristic classifier: returns true if a request is explicitly asking for a deep,
/// curriculum/textbook-like artifact rather than a short answer.
pub fn request_needs_curriculum(user_request: &str) -> bool {
    let s = user_request.to_lowercase();
    [
        "deep dive",
        "deep research",
        "research mode",
        "textbook",
        "curriculum",
        "learn all",
        "masterclass",
        "course",
        "comprehensive guide",
    ]
    .iter()
    .any(|w| s.contains(w))
}

/// Enforce a "textbook/curriculum" plan shape for explicit deep-research requests.
///
/// This makes depth deterministic (not dependent on the model choosing to be verbose).
pub fn enforce_curriculum_contract(
    user_request: &str,
    capabilities: &serde_json::Value,
    plan: &mut PlannerOutput,
) {
    if !request_needs_curriculum(user_request) {
        return;
    }

    let search_configured = capabilities
        .get("search_url_configured")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    if !search_configured {
        return;
    }

    // Ensure parallel_search is present as the first step.
    let has_parallel = plan
        .steps
        .iter()
        .any(|s| s.tool_binding.as_deref() == Some("parallel_search"));
    if !has_parallel {
        let queries = decompose_search_queries(user_request);
        plan.steps.insert(
            0,
            StepDefinition {
                step_id: String::new(),
                action:
                    "Curriculum research: search authoritative sources for the requested topic"
                        .into(),
                tool_binding: Some("parallel_search".into()),
                input_params: serde_json::json!({ "queries": queries }),
                output_key: Some("search_result".into()),
                expected_output: None,
                output_format: None,
                requires_facts: false,
            },
        );
        renumber_steps(&mut plan.steps);
    }

    // Insert multiple fetches before synthesis (snippets aren't enough for a 'textbook').
    let fetch_count = plan
        .steps
        .iter()
        .filter(|s| s.tool_binding.as_deref() == Some("http_fetch"))
        .count();
    let target_fetches = 4usize;
    if fetch_count < target_fetches {
        let insert_at = plan
            .steps
            .iter()
            .position(|s| s.tool_binding.as_deref() == Some("parallel_search"))
            .map(|i| i + 1)
            .unwrap_or(1);
        for i in fetch_count..target_fetches {
            plan.steps.insert(
                insert_at + i,
                StepDefinition {
                    step_id: String::new(),
                    action: format!(
                        "Curriculum research: fetch high-quality source URL #{} (diverse domains)",
                        i + 1
                    ),
                    tool_binding: Some("http_fetch".into()),
                    input_params: serde_json::json!({ "url": "" }),
                    output_key: Some(format!("fetch{}_result", i + 1)),
                    expected_output: None,
                    output_format: None,
                    requires_facts: false,
                },
            );
        }
        renumber_steps(&mut plan.steps);
    }

    // Ensure a dedicated textbook synthesis step exists.
    let has_textbook_step = plan.steps.iter().any(|s| {
        s.output_key.as_deref() == Some("kb_textbook")
            && s.tool_binding.is_none()
            && s.output_format.as_deref().map(|f| f.eq_ignore_ascii_case("json")) == Some(true)
    });
    if !has_textbook_step {
        let insert_at = plan
            .steps
            .iter()
            .position(|s| s.tool_binding.as_deref() == Some("save_knowledge"))
            .unwrap_or(plan.steps.len());
        plan.steps.insert(insert_at, StepDefinition {
            step_id: String::new(),
            action: "Using all search results and fetched pages, synthesize a structured textbook in Markdown chapters. Output JSON: {book_title, chapters:[{topic,summary,content,tags,sources[]}]} with 6–14 chapters and a Table of Contents in chapter 1.".into(),
            tool_binding: None,
            input_params: serde_json::json!({}),
            output_key: Some("kb_textbook".into()),
            expected_output: None,
            output_format: Some("json".into()),
            requires_facts: false,
        });
        renumber_steps(&mut plan.steps);
    }

    // Ensure a final save_knowledge step persists all chapters in one call, *after* kb_textbook.
    let kb_idx = plan
        .steps
        .iter()
        .position(|s| s.output_key.as_deref() == Some("kb_textbook"))
        .unwrap_or(usize::MAX);

    if kb_idx != usize::MAX {
        // Drop any save_knowledge steps that appear before kb_textbook (wrong ordering).
        plan.steps = plan
            .steps
            .clone()
            .into_iter()
            .enumerate()
            .filter_map(|(i, s)| {
                if i < kb_idx && s.tool_binding.as_deref() == Some("save_knowledge") {
                    None
                } else {
                    Some(s)
                }
            })
            .collect();
        renumber_steps(&mut plan.steps);
    }

    let kb_idx2 = plan
        .steps
        .iter()
        .position(|s| s.output_key.as_deref() == Some("kb_textbook"))
        .unwrap_or(usize::MAX);
    let mut save_idx_after: Option<usize> = None;
    for (i, s) in plan.steps.iter().enumerate() {
        if i > kb_idx2 && s.tool_binding.as_deref() == Some("save_knowledge") {
            save_idx_after = Some(i);
            break;
        }
    }

    match save_idx_after {
        Some(i) => {
            if let Some(st) = plan.steps.get_mut(i) {
                st.input_params =
                    serde_json::json!({ "chapters_from": "kb_textbook", "auto": true });
                st.output_key = Some("knowledge_saved".into());
            }
        }
        None => {
            plan.steps.push(StepDefinition {
                step_id: String::new(),
                action: "Save the synthesized textbook chapters into the knowledge base".into(),
                tool_binding: Some("save_knowledge".into()),
                input_params: serde_json::json!({ "chapters_from": "kb_textbook", "auto": true }),
                output_key: Some("knowledge_saved".into()),
                expected_output: None,
                output_format: None,
                requires_facts: false,
            });
            renumber_steps(&mut plan.steps);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::PlannerOutput;

    fn base_plan() -> PlannerOutput {
        PlannerOutput {
            steps: vec![],
            tools_required: vec![],
            risk_score: 1,
            expected_outputs: vec!["answer".into()],
            completion_criteria: vec!["done".into()],
            dependencies: serde_json::Value::Null,
        }
    }

    #[test]
    fn grounding_contract_inserts_search_and_facts_and_requires_facts() {
        let mut plan = base_plan();
        plan.steps.push(StepDefinition {
            step_id: "1".into(),
            action: "Generate code for heroes in patch 7.41b".into(),
            tool_binding: None,
            input_params: serde_json::json!({}),
            output_key: Some("answer".into()),
            expected_output: None,
            output_format: None,
            requires_facts: false,
        });

        enforce_grounding_contract(
            "Research Dota 2 patch 7.41b and write scripts",
            &serde_json::json!({ "search_url_configured": true }),
            &mut plan,
        );

        assert!(
            plan.steps
                .iter()
                .any(|s| s.tool_binding.as_deref() == Some("parallel_search")),
            "expected a parallel_search step to be inserted",
        );
        assert!(
            plan.steps.iter().any(|s| {
                s.output_key.as_deref() == Some("facts")
                    && s.tool_binding.is_none()
                    && s.output_format.as_deref() == Some("json")
            }),
            "expected a facts JSON extraction step to be inserted",
        );

        // Any steps after facts should require facts (including answer step).
        let facts_idx = plan
            .steps
            .iter()
            .position(|s| s.output_key.as_deref() == Some("facts"))
            .expect("facts step present");
        for (i, s) in plan.steps.iter().enumerate() {
            if i > facts_idx && s.output_key.as_deref() != Some("facts") {
                assert!(s.requires_facts, "step {} should require facts", s.step_id);
            }
        }
    }

    #[test]
    fn curriculum_contract_inserts_textbook_and_save() {
        let mut plan = base_plan();
        plan.steps.push(StepDefinition {
            step_id: "1".into(),
            action: "Answer about Rust memory management".into(),
            tool_binding: None,
            input_params: serde_json::json!({}),
            output_key: Some("answer".into()),
            expected_output: None,
            output_format: None,
            requires_facts: false,
        });

        enforce_curriculum_contract(
            "Enter research mode and deep dive Rust memory management; make a textbook.",
            &serde_json::json!({ "search_url_configured": true }),
            &mut plan,
        );

        assert!(
            plan.steps
                .iter()
                .any(|s| s.tool_binding.as_deref() == Some("parallel_search")),
            "expected parallel_search"
        );
        assert!(
            plan.steps.iter().any(|s| s.output_key.as_deref() == Some("kb_textbook")),
            "expected kb_textbook synthesis step"
        );
        assert!(
            plan.steps
                .iter()
                .any(|s| s.tool_binding.as_deref() == Some("save_knowledge")),
            "expected save_knowledge step"
        );
    }
}
