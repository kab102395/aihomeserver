use serde_json::Value;
use std::collections::HashMap;

pub fn browser_output_has_blocking_error(output: &str) -> bool {
    let lower = output.to_lowercase();
    let line_starts_with = |prefix: &str| {
        lower
            .lines()
            .map(str::trim_start)
            .any(|line| line.starts_with(prefix))
    };

    [
        "traceback (most recent call last)",
        "syntaxerror:",
        "modulenotfounderror",
        "module not found",
        "importerror:",
        "typeerror:",
        "nameerror:",
        "attributeerror:",
        "locator.text_content:",
        "strict mode violation",
        "command timed out",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
        || line_starts_with("error:")
}

pub fn browser_output_is_verified(output: &str) -> bool {
    let lower = output.to_lowercase();
    if browser_output_has_blocking_error(output) {
        return false;
    }

    let has_page_markers = lower.contains("page title:") && lower.contains("final url:");
    let has_probe_or_extract_markers = lower.contains("classification:")
        || lower.contains("first 500 visible body characters:")
        || lower.contains("selectors tried:")
        || lower.contains("first 5 extracted texts:");

    has_page_markers && has_probe_or_extract_markers
}

pub fn latest_verified_browser_output(artifacts: &HashMap<String, Value>) -> Option<String> {
    latest_browser_outputs(artifacts)
        .into_iter()
        .rev()
        .find_map(|(_, output)| browser_output_is_verified(&output).then_some(output))
}

pub fn latest_browser_output_block(artifacts: &HashMap<String, Value>) -> Option<String> {
    latest_browser_outputs(artifacts)
        .into_iter()
        .last()
        .map(|(_, output)| output)
}

pub fn browser_selectors_summary(output: &str) -> Option<String> {
    let mut lines = Vec::new();
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Selectors tried:")
            || trimmed.starts_with("Matches for ")
            || trimmed.starts_with("Preview for ")
        {
            lines.push(trimmed.to_string());
        }
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

pub fn browser_classification(output: &str) -> Option<&'static str> {
    let lower = output.to_lowercase();
    if lower.contains("classification: access denied")
        || lower.contains("access to this page has been denied")
        || lower.contains("access denied blocked extraction")
    {
        return Some("access denied");
    }
    if lower.contains("classification: challenge page")
        || lower.contains("captcha")
        || lower.contains("cloudflare")
        || lower.contains("perimeterx")
        || lower.contains("press & hold to confirm you are a human")
        || lower.contains("press and hold to confirm you are a human")
        || lower.contains("human (and not a bot)")
    {
        return Some("challenge page");
    }
    if lower.contains("classification: normal page") {
        return Some("normal page");
    }
    None
}

fn latest_browser_outputs(artifacts: &HashMap<String, Value>) -> Vec<(String, String)> {
    let mut outputs: Vec<(String, String)> = artifacts
        .iter()
        .filter(|(k, v)| k.ends_with("_result") && result_is_success(v))
        .filter_map(|(_, v)| {
            let output = text_from_result_output(v)?;
            let timestamp = v
                .get("timestamp")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string();
            Some((timestamp, output))
        })
        .collect();
    outputs.sort_by(|a, b| a.0.cmp(&b.0));
    outputs
}

fn result_is_success(result: &Value) -> bool {
    result.get("success").and_then(|b| b.as_bool()) != Some(false)
}

fn text_from_result_output(result: &Value) -> Option<String> {
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

    if !command.is_empty() || !stdout.is_empty() || !stderr.is_empty() {
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
            return Some(trimmed.to_string());
        }
    }

    for key in ["body", "text", "content"] {
        if let Some(s) = output.get(key).and_then(|x| x.as_str()) {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }

    None
}
