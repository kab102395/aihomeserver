//! Coding task-class scaffolds and prompt guidance.
//!
//! This module does not generate files directly. It provides deterministic
//! execution contracts and examples so planner/executor prompts stop treating
//! every coding request like a generic multi-file project.

use super::intent::CodingIntent;
use crate::state::StepDefinition;
use serde_json::{json, Value};
use std::collections::HashMap;

pub fn should_use_manifest(intent: &CodingIntent) -> bool {
    matches!(intent.task_class.as_str(), "repo_patch_task")
}

pub fn planner_contract(intent: &CodingIntent) -> String {
    match intent.task_class.as_str() {
        "script_task" => script_task_contract(intent),
        "workspace_file_task" => workspace_file_task_contract(intent),
        "browser_automation_task" => browser_automation_task_contract(intent),
        "environment_bootstrap_task" => environment_task_contract(intent),
        _ => repo_patch_task_contract(intent),
    }
}

pub fn executor_guidance(intent: &CodingIntent) -> String {
    let mut lines = vec![
        format!("Task class: {}", intent.task_class),
        "Prefer the smallest step sequence that can complete the request.".to_string(),
        "Do not invent extra files, package manifests, or installs unless they are required for the requested output.".to_string(),
    ];

    if intent.disallow_installs {
        lines.push("The user explicitly prohibited installing packages or tools. You must probe availability and work with the existing runtime only.".to_string());
    }
    if intent.wants_exact_outputs {
        lines.push("The user wants exact artifacts. Final answers must include the exact file contents and exact command output when requested.".to_string());
    }
    if intent.wants_selector_validation {
        lines.push("For browser extraction tasks, validate selectors with counts and visible matches before claiming success.".to_string());
    }

    match intent.task_class.as_str() {
        "script_task" => {
            lines.push("For one-file scripts: write exactly one script, run it, read it back, then answer.".to_string());
            lines.push("Do not create requirements.txt unless a non-stdlib import is actually required.".to_string());
        }
        "workspace_file_task" => {
            lines.push("Use filesystem operations first. Shell is for append-style edits, command-driven transforms, or recovery after filesystem failure.".to_string());
        }
        "browser_automation_task" => {
            lines.push("Browser tasks must separate runtime readiness, probe script creation, script execution, and extraction validation.".to_string());
            lines.push("Do not treat page load alone as success when the user asked for extracted structured data.".to_string());
            lines.push("Use a deterministic probe shape: title, final URL, selectors tried, match counts, extracted texts, and a short blocker explanation if extraction fails.".to_string());
            lines.push("Before running a generated Python browser script, syntax-check it with `python3 -m py_compile <file>`.".to_string());
            lines.push("Do not put escaped join expressions directly inside f-strings. Compute joined_text = '\\n'.join(items) in a separate variable, then print it.".to_string());
            lines.push("If `python3 -m playwright --version` fails and installs are forbidden, stop and report the runtime as unavailable instead of retrying execution.".to_string());
        }
        "environment_bootstrap_task" => {
            lines.push("Environment tasks start with version/capability probes and report what is installed before attempting any repair.".to_string());
        }
        _ => {
            lines.push("Repo tasks should inspect existing files before writing and should keep edits scoped to the requested surface.".to_string());
        }
    }

    lines.join("\n")
}

pub fn deterministic_browser_tool_call(
    intent: &CodingIntent,
    user_request: &str,
    step: &StepDefinition,
    artifacts: &HashMap<String, Value>,
) -> Option<Value> {
    if intent.task_class != "browser_automation_task" {
        return None;
    }

    let tool = step.tool_binding.as_deref()?;
    match tool {
        "filesystem" => deterministic_browser_filesystem_call(user_request, step, artifacts),
        "shell" => deterministic_browser_shell_call(user_request, step, artifacts),
        _ => None,
    }
}

fn deterministic_browser_filesystem_call(
    user_request: &str,
    step: &StepDefinition,
    artifacts: &HashMap<String, Value>,
) -> Option<Value> {
    let action = step.input_params.get("action").and_then(|v| v.as_str());
    match action {
        Some("write") => {
            let path = detect_python_path(user_request, step, artifacts)?;
            let content = build_browser_script(user_request, &path);
            Some(json!({
                "tool": "filesystem",
                "params": {
                    "action": "write",
                    "path": path,
                    "content": content,
                }
            }))
        }
        Some("read") => {
            let path = detect_python_path(user_request, step, artifacts)?;
            Some(json!({
                "tool": "filesystem",
                "params": {
                    "action": "read",
                    "path": path,
                }
            }))
        }
        _ => None,
    }
}

fn deterministic_browser_shell_call(
    user_request: &str,
    step: &StepDefinition,
    artifacts: &HashMap<String, Value>,
) -> Option<Value> {
    let action = step.action.to_lowercase();
    let output_key = step.output_key.as_deref().unwrap_or_default().to_lowercase();

    if action.contains("check playwright")
        || action.contains("runtime check")
        || action.contains("verify playwright")
        || action.contains("playwright availability")
        || output_key.contains("playwright_version")
        || output_key.contains("runtime_check")
    {
        return Some(json!({
            "tool": "shell",
            "params": {
                "command": "python3 -m playwright --version",
                "cwd": ".",
                "timeout_secs": 20
            }
        }));
    }

    let path = detect_python_path(user_request, step, artifacts)?;
    Some(json!({
        "tool": "shell",
        "params": {
            "command": format!("python3 -m py_compile {path} && python3 {path}"),
            "cwd": ".",
            "timeout_secs": 60
        }
    }))
}

fn detect_python_path(
    user_request: &str,
    step: &StepDefinition,
    artifacts: &HashMap<String, Value>,
) -> Option<String> {
    if let Some(path) = step.input_params.get("path").and_then(|v| v.as_str()) {
        if path.trim().ends_with(".py") {
            return Some(path.trim().to_string());
        }
    }
    if let Some(path) = first_token_with_suffix(&step.action, ".py") {
        return Some(path);
    }
    if let Some(path) = first_token_with_suffix(user_request, ".py") {
        return Some(path);
    }
    latest_filesystem_python_path(artifacts)
}

fn latest_filesystem_python_path(artifacts: &HashMap<String, Value>) -> Option<String> {
    let mut best: Option<(String, String)> = None;
    for (key, value) in artifacts {
        if !key.ends_with("_result") {
            continue;
        }
        let Some(path) = value
            .get("output")
            .and_then(|v| v.get("path").or_else(|| v.get("name")))
            .and_then(|v| v.as_str())
        else {
            continue;
        };
        if !path.ends_with(".py") {
            continue;
        }
        let ts = value
            .get("timestamp")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        match &best {
            Some((best_ts, _)) if best_ts >= &ts => {}
            _ => best = Some((ts, path.to_string())),
        }
    }
    best.map(|(_, path)| path)
}

fn first_token_with_suffix(text: &str, suffix: &str) -> Option<String> {
    text.split_whitespace().find_map(|token| {
        let trimmed = token
            .trim_matches(|c: char| matches!(c, '"' | '\'' | '`' | ',' | '.' | ':' | ';' | ')' | '('));
        if trimmed.ends_with(suffix) {
            Some(trimmed.to_string())
        } else {
            None
        }
    })
}

fn first_url(text: &str) -> Option<String> {
    text.split_whitespace().find_map(|token| {
        let trimmed = token
            .trim_matches(|c: char| matches!(c, '"' | '\'' | '`' | ',' | ')' | '('));
        if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
            Some(trimmed.to_string())
        } else {
            None
        }
    })
}

fn screenshot_path(text: &str) -> Option<String> {
    first_token_with_suffix(text, ".png")
}

fn build_browser_script(user_request: &str, script_path: &str) -> String {
    let url = first_url(user_request).unwrap_or_else(|| "https://example.com/".to_string());
    let screenshot = screenshot_path(user_request);
    let wants_extraction = wants_extraction(user_request);
    let wait_snippet = wait_snippet(user_request);
    let selectors = selectors_for_url_and_request(&url, user_request);
    let selectors_literal = python_list_literal(&selectors);
    let screenshot_literal = screenshot
        .as_ref()
        .map(|path| format!("{path:?}"))
        .unwrap_or_else(|| "None".to_string());
    let extraction_snippet = if wants_extraction {
        format!(
            "    selectors = {selectors_literal}\n    print(f\"Selectors tried: {{selectors}}\")\n    extracted = []\n    for selector in selectors:\n        locator = page.locator(selector)\n        count = locator.count()\n        print(f\"Matches for {{selector}}: {{count}}\")\n        selector_texts = []\n        for idx in range(min(count, 8)):\n            item = locator.nth(idx)\n            try:\n                if not item.is_visible():\n                    continue\n            except Exception:\n                pass\n            text = normalize_text(item.text_content() or \"\")\n            if not text:\n                continue\n            selector_texts.append(text)\n            if text not in extracted:\n                extracted.append(text)\n        if selector_texts:\n            joined_preview = \"\\n\".join(selector_texts[:5])\n            print(f\"Preview for {{selector}}:\")\n            print(joined_preview)\n\n    first_five = extracted[:5]\n    print(f\"Useful text count: {{len(extracted)}}\")\n    print(\"First 5 extracted texts:\")\n    if first_five:\n        joined_text = \"\\n\".join(first_five)\n        print(joined_text)\n    else:\n        print(\"[none]\")\n\n    if len(first_five) >= 5:\n        print(\"Conclusion: extracted visible content successfully\")\n    elif classification == \"access denied\":\n        print(\"Conclusion: access denied blocked extraction\")\n    elif classification == \"challenge page\":\n        print(\"Conclusion: challenge behavior likely blocked extraction\")\n    else:\n        print(\"Conclusion: page structure likely changed or selectors did not match enough visible elements\")\n"
        )
    } else {
        String::new()
    };

    format!(
        "from playwright.sync_api import sync_playwright\n\nURL = {url:?}\nSCRIPT_PATH = {script_path:?}\nSCREENSHOT_PATH = {screenshot_literal}\nDENY_TERMS = [\n    \"access to this page has been denied\",\n    \"access denied\",\n    \"request denied\",\n    \"unable to access this page\",\n    \"you don't have permission to access\",\n    \"403 forbidden\",\n]\nCHALLENGE_TERMS = [\n    \"captcha\",\n    \"verify\",\n    \"challenge\",\n    \"cloudflare\",\n    \"perimeterx\",\n    \"press & hold to confirm you are a human\",\n    \"press and hold to confirm you are a human\",\n    \"human (and not a bot)\",\n    \"are you a robot\",\n]\n\n\ndef normalize_text(value: str) -> str:\n    return \" \".join((value or \"\").split())\n\n\ndef classify_page(title: str, body_text: str, status, matched_terms):\n    title_lower = title.lower()\n    body_lower = body_text.lower()\n    if status in {{401, 403, 429}}:\n        return \"access denied\"\n    if any(term in title_lower or term in body_lower for term in DENY_TERMS):\n        return \"access denied\"\n    if matched_terms:\n        return \"challenge page\"\n    return \"normal page\"\n\n\nwith sync_playwright() as p:\n    browser = p.chromium.launch(headless=True)\n    page = browser.new_page()\n    response = page.goto(URL, wait_until=\"domcontentloaded\", timeout=30000)\n{wait_snippet}    title = page.title()\n    final_url = page.url\n    try:\n        body_text = normalize_text(page.locator(\"body\").inner_text(timeout=3000))\n    except Exception:\n        body_text = normalize_text(page.text_content(\"body\") or \"\")\n    body_preview = body_text[:500]\n    content_lower = page.content().lower()\n    matched_terms = [\n        term for term in CHALLENGE_TERMS\n        if term in title.lower() or term in body_text.lower() or term in content_lower\n    ]\n    classification = classify_page(title, body_text, response.status if response else None, matched_terms)\n\n    print(f\"Page title: {{title}}\")\n    print(f\"Final URL: {{final_url}}\")\n    if response is not None:\n        print(f\"Main navigation status: {{response.status}}\")\n    else:\n        print(\"Main navigation status: unavailable\")\n    print(f\"Matched challenge or deny terms: {{matched_terms}}\")\n    print(\"First 500 visible body characters:\")\n    print(body_preview or \"[empty]\")\n{extraction_snippet}    print(f\"Classification: {{classification}}\")\n    if SCREENSHOT_PATH:\n        page.screenshot(path=SCREENSHOT_PATH, full_page=True)\n        print(f\"Screenshot saved: {{SCREENSHOT_PATH}}\")\n    browser.close()\n"
    )
}

fn wants_extraction(user_request: &str) -> bool {
    let lower = user_request.to_lowercase();
    lower.contains("selectors tried")
        || lower.contains("extract the first 5")
        || lower.contains("first 5 visible")
        || lower.contains("first 5 extracted")
        || lower.contains("listing-like elements")
        || lower.contains("story titles")
        || lower.contains("listing titles")
}

fn wait_snippet(user_request: &str) -> String {
    let lower = user_request.to_lowercase();
    if lower.contains("networkidle") {
        return "    try:\n        page.wait_for_load_state(\"networkidle\", timeout=8000)\n    except Exception:\n        pass\n".to_string();
    }
    if lower.contains("wait for the page to settle") || lower.contains("wait for page to settle") {
        return "    page.wait_for_timeout(3000)\n".to_string();
    }
    if lower.contains("wait for domcontentloaded") {
        return String::new();
    }
    "    page.wait_for_timeout(1500)\n".to_string()
}

fn selectors_for_url_and_request(url: &str, user_request: &str) -> Vec<String> {
    let lower = user_request.to_lowercase();
    if url.contains("news.ycombinator.com") || lower.contains("hacker news") {
        return vec![
            "span.titleline > a".to_string(),
            "tr.athing span.titleline > a".to_string(),
            "a.storylink".to_string(),
            "td.title span.titleline a".to_string(),
            "tr.athing td.title a".to_string(),
        ];
    }
    if url.contains("zillow.com") {
        return vec![
            "article[data-test='property-card']".to_string(),
            "article[data-testid='property-card']".to_string(),
            "a[data-test='property-card-link']".to_string(),
            "ul li article".to_string(),
            "main a".to_string(),
        ];
    }
    vec![
        "[data-testid*='card']".to_string(),
        "article".to_string(),
        "main a".to_string(),
        "main h1, main h2, main h3".to_string(),
        ".card, .listing, .item, .result".to_string(),
    ]
}

fn python_list_literal(items: &[String]) -> String {
    let inner = items
        .iter()
        .map(|s| format!("{s:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{inner}]")
}

fn script_task_contract(intent: &CodingIntent) -> String {
    let mut s = String::from(
        r#"LIGHTWEIGHT SCRIPT TASK CONTRACT (MANDATORY):
Use at most 4 steps unless the user explicitly asked for extra validation.
1. FILE WRITE STEP: filesystem.write for the requested script/file.
2. EXECUTION STEP: shell run the script/command if the user asked to run it.
3. FILE READ STEP: filesystem.read the written file when the user asked to see contents.
4. ANSWER STEP: summarize the result and include exact file contents/output when requested.

Rules:
- Do NOT create requirements.txt, package.json, or extra scaffolding unless the requested code truly needs third-party dependencies.
- Prefer one file, one command, one answer.
- If the script needs no external deps, write it directly and run it.
"#,
    );
    if intent.disallow_installs {
        s.push_str("- Never add install steps for this task.\n");
    }
    if intent.wants_exact_outputs {
        s.push_str("- The final answer must contain the exact file contents and the exact command output.\n");
    }
    s
}

fn workspace_file_task_contract(intent: &CodingIntent) -> String {
    let mut s = String::from(
        r#"VM WORKSPACE FILE TASK CONTRACT (MANDATORY):
Use filesystem operations as the primary path.
Preferred step patterns:
- write -> read -> answer
- mkdir -> write -> read -> answer
- read -> rename/delete -> read/list -> answer

Rules:
- Use filesystem first for create/read/list/mkdir/rename/delete.
- Use shell only for append-style edits, text transforms, or fallback after a filesystem failure.
- If a filesystem step fails and a shell fallback succeeds, the final result is success with a warning, not overall failure.
"#,
    );
    if intent.wants_exact_outputs {
        s.push_str("- Include the exact final file contents in the answer when the user asked to read the file back.\n");
    }
    s
}

fn browser_automation_task_contract(intent: &CodingIntent) -> String {
    let mut s = String::from(
        r#"BROWSER AUTOMATION CODING CONTRACT (MANDATORY):
Use exactly this step order unless the user explicitly asked for something else:
1. RUNTIME CHECK STEP: verify browser automation readiness before any install attempt.
   Preferred probes:
   - python3 -m playwright --version
   - node --version
   - ls /var/lib/aihomeserver/ms-playwright
2. FILE WRITE STEP: write the browser probe/extraction script.
3. SYNTAX CHECK STEP: run `python3 -m py_compile <file>` before the real execution step.
4. EXECUTION STEP: run the script.
5. FILE READ STEP: read the written script when the user asked to see it.
6. ANSWER STEP: include exact script contents, exact output, and selector diagnostics if extraction was requested.

Rules:
- Do not plan installation unless the user explicitly asked for installation.
- If the runtime check shows Playwright is unavailable and installs are forbidden, stop after the probe and answer with the exact probe output. Do not keep retrying the script run.
- For extraction tasks, you must report selectors tried, match counts, and why extraction failed if fewer than requested items were found.
- Page load or title-only output is not enough when the user asked for structured extraction.
- Prefer domcontentloaded or explicit short waits over blind long networkidle waits on bot-protected pages.
- If the page appears challenged or denied, say that directly and stop claiming extraction success.
- For Python output formatting, do not write f-strings like `{\"\n\".join(items)}` or `{\n.join(items)}`. Build the joined string in a normal variable first, then print that variable.
- If a syntax check fails, the next repair must rewrite the script before rerunning it.
"#,
    );
    if intent.disallow_installs {
        s.push_str("- Installation is forbidden for this task. Probe only.\n");
    }
    s
}

fn environment_task_contract(intent: &CodingIntent) -> String {
    let mut s = String::from(
        r#"ENVIRONMENT / RUNTIME TASK CONTRACT (MANDATORY):
1. Probe versions/capabilities first.
2. Report exactly what exists and what is missing.
3. Only propose or perform installs if the user explicitly asked for them.

Rules:
- Prefer shell version checks and filesystem inspection.
- Final answer must separate 'available', 'missing', and 'blocked'.
"#,
    );
    if intent.disallow_installs {
        s.push_str("- Do not install anything during this task.\n");
    }
    s
}

fn repo_patch_task_contract(_intent: &CodingIntent) -> String {
    r#"REPO PATCH / MULTI-FILE CODING CONTRACT (MANDATORY):
1. FIRST STEP: inspect the workspace using filesystem list/find/read.
2. MANIFEST STEP: output_key="coding_execution_manifest", output_format="json".
3. FILE WRITE STEPS: one filesystem write per logical file.
4. VERIFICATION STEPS: shell build/test using adapter recipes.
5. PACKAGE STEP only if the request needs packaging.
6. ANSWER STEP with changed files, validation result, and remaining limitations.
"#
    .to_string()
}
