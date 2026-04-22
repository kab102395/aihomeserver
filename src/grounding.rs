use crate::state::{PlannerOutput, StepDefinition};

fn request_needs_grounding(user_request: &str) -> bool {
    let s = user_request.to_lowercase();
    if s.contains("patch")
        || s.contains("version")
        || s.contains("latest")
        || s.contains("most recent")
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

fn renumber_steps(steps: &mut [StepDefinition]) {
    for (idx, s) in steps.iter_mut().enumerate() {
        s.step_id = (idx + 1).to_string();
    }
}

/// Enforce the "facts table + requires_facts" contract for patch/version/meta research so
/// the system cannot generate patch-specific code/guides without grounded evidence.
pub fn enforce_grounding_contract(
    user_request: &str,
    capabilities: &serde_json::Value,
    plan: &mut PlannerOutput,
) {
    if !request_needs_grounding(user_request) {
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

        plan.steps.insert(
            0,
            StepDefinition {
                step_id: String::new(),
                action: "Search the web for up-to-date sources relevant to the user's request"
                    .into(),
                tool_binding: Some("parallel_search".into()),
                input_params: serde_json::json!({ "queries": [user_request] }),
                output_key,
                expected_output: None,
                output_format: None,
                requires_facts: false,
            },
        );
        renumber_steps(&mut plan.steps);
    }

    let facts_idx = plan.steps.iter().position(|s| {
        s.output_key.as_deref() == Some("facts")
            && s.tool_binding.is_none()
            && s.output_format
                .as_deref()
                .map(|f| f.eq_ignore_ascii_case("json"))
                .unwrap_or(false)
    });

    let facts_idx = match facts_idx {
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
                    action: "Extract a grounded fact table from research artifacts (include URLs + evidence snippets)".into(),
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
}
