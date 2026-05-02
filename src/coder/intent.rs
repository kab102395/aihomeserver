//! Detect whether a user request is a coding task and extract intent.
//!
//! No LLM calls — pure keyword heuristics so detection is instant and deterministic.

use serde::{Deserialize, Serialize};

/// Describes what kind of coding task the user wants.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CodingIntent {
    /// new_project | modify_existing | debug_existing | add_feature | package_existing | refactor
    pub intent: String,
    /// source | binary | zip | docker | web_app | library | script
    pub deliverable: String,
    /// Detected language (lowercase): rust | python | javascript | typescript | go | java | csharp | cpp | lua | bash
    pub language: Option<String>,
    /// Detected framework/library hint
    pub framework: Option<String>,
    pub requires_build: bool,
    pub requires_package: bool,
    pub requires_tests: bool,
}

/// Detect coding intent from the user request text.
/// Returns `None` if the request is clearly not a coding task.
pub fn detect_coding_intent(request: &str) -> Option<CodingIntent> {
    let lower = request.to_lowercase();

    // ── Language detection ────────────────────────────────────────────────────
    let language = detect_language(&lower);

    // ── Action keywords ───────────────────────────────────────────────────────
    let action_keywords = [
        "create", "build", "generate", "scaffold", "write", "implement",
        "make", "develop", "code", "program", "package", "compile", "run",
        "debug", "fix", "refactor", "add feature", "add a feature",
        "create a project", "create an app", "create a game", "create a script",
        "create a library", "create an api", "create a website", "create a bot",
        "write a", "write me", "build me", "make me",
    ];
    let has_action = action_keywords.iter().any(|kw| lower.contains(kw));

    // ── Deliverable keywords ──────────────────────────────────────────────────
    let is_project = lower.contains("project") || lower.contains("app") || lower.contains("game")
        || lower.contains("script") || lower.contains("library") || lower.contains("api")
        || lower.contains("website") || lower.contains("bot") || lower.contains("tool")
        || lower.contains("program") || lower.contains("binary") || lower.contains("executable");

    // Only classify as coding if we have action + (language or project noun)
    if !has_action || (language.is_none() && !is_project) {
        return None;
    }

    // Also skip if it looks like a research/explanation-only request
    let research_only = (lower.contains("explain") || lower.contains("what is")
        || lower.contains("how does") || lower.contains("tell me about"))
        && !has_action;
    if research_only {
        return None;
    }

    // ── Intent classification ────────────────────────────────────────────────
    let intent = if lower.contains("fix") || lower.contains("debug") || lower.contains("error")
        || lower.contains("bug") || lower.contains("compile error") || lower.contains("doesn't work")
    {
        "debug_existing"
    } else if lower.contains("refactor") || lower.contains("clean up") || lower.contains("improve") {
        "refactor"
    } else if lower.contains("add feature") || lower.contains("add a feature")
        || lower.contains("extend") || lower.contains("modify existing")
    {
        "add_feature"
    } else if lower.contains("package") && !lower.contains("create") && !lower.contains("build") {
        "package_existing"
    } else {
        "new_project"
    };

    // ── Deliverable ───────────────────────────────────────────────────────────
    let deliverable = if lower.contains("zip") || lower.contains("archive") || lower.contains("package it") {
        "zip"
    } else if lower.contains("docker") || lower.contains("container") {
        "docker"
    } else if lower.contains("website") || lower.contains("web app") || lower.contains("react")
        || lower.contains("frontend") || lower.contains("html")
    {
        "web_app"
    } else if lower.contains("library") || lower.contains("crate") || lower.contains("package") {
        "library"
    } else if lower.contains("script") || lower.contains("automation") {
        "script"
    } else {
        "source"
    };

    // ── Framework hints ───────────────────────────────────────────────────────
    let framework = detect_framework(&lower);

    // ── Build / package requirements ──────────────────────────────────────────
    let requires_build = intent != "package_existing" && deliverable != "script";
    let requires_package = deliverable == "zip" || deliverable == "docker"
        || lower.contains("zip") || lower.contains("package");
    let requires_tests = lower.contains("test") || lower.contains("tdd") || lower.contains("unit test");

    Some(CodingIntent {
        intent: intent.to_string(),
        deliverable: deliverable.to_string(),
        language,
        framework,
        requires_build,
        requires_package,
        requires_tests,
    })
}

fn detect_language(lower: &str) -> Option<String> {
    // Order matters — check more specific patterns first
    if lower.contains(" rust ") || lower.contains(" rust\n") || lower.ends_with(" rust")
        || lower.contains("cargo") || lower.contains(".rs") || lower.contains("crate")
        || lower.contains("in rust") || lower.contains("rust game") || lower.contains("rust app")
        || lower.contains("rust project") || lower.contains("rust script")
    {
        return Some("rust".to_string());
    }
    if lower.contains("typescript") || lower.contains(".ts") || lower.contains("tsx") {
        return Some("typescript".to_string());
    }
    if lower.contains("javascript") || lower.contains("node.js") || lower.contains("nodejs")
        || lower.contains(".js") || lower.contains("npm") || lower.contains("react")
        || lower.contains("vue") || lower.contains("express")
    {
        return Some("javascript".to_string());
    }
    if lower.contains("python") || lower.contains(".py") || lower.contains("pip")
        || lower.contains("django") || lower.contains("fastapi") || lower.contains("flask")
    {
        return Some("python".to_string());
    }
    if lower.contains(" go ") || lower.contains("golang") || lower.contains("go project")
        || lower.contains(".go") || lower.contains("go module")
    {
        return Some("go".to_string());
    }
    if lower.contains("java") && !lower.contains("javascript") {
        return Some("java".to_string());
    }
    if lower.contains("c#") || lower.contains("csharp") || lower.contains(".net")
        || lower.contains("dotnet") || lower.contains(".csproj")
    {
        return Some("csharp".to_string());
    }
    if lower.contains("c++") || lower.contains("cpp") || lower.contains("cmake") {
        return Some("cpp".to_string());
    }
    if lower.contains(" lua ") || lower.contains("lua script") || lower.contains(".lua") {
        return Some("lua".to_string());
    }
    if lower.contains("bash") || lower.contains("shell script") || lower.contains("#!/bin") {
        return Some("bash".to_string());
    }
    None
}

fn detect_framework(lower: &str) -> Option<String> {
    if lower.contains("macroquad") { return Some("macroquad".to_string()); }
    if lower.contains("bevy") { return Some("bevy".to_string()); }
    if lower.contains("crossterm") { return Some("crossterm".to_string()); }
    if lower.contains("axum") { return Some("axum".to_string()); }
    if lower.contains("actix") { return Some("actix-web".to_string()); }
    if lower.contains("tokio") { return Some("tokio".to_string()); }
    if lower.contains("react") { return Some("react".to_string()); }
    if lower.contains("next.js") || lower.contains("nextjs") { return Some("next".to_string()); }
    if lower.contains("vite") { return Some("vite".to_string()); }
    if lower.contains("express") { return Some("express".to_string()); }
    if lower.contains("fastapi") { return Some("fastapi".to_string()); }
    if lower.contains("django") { return Some("django".to_string()); }
    if lower.contains("flask") { return Some("flask".to_string()); }
    if lower.contains("spring") { return Some("spring-boot".to_string()); }
    None
}
