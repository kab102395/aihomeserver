//! Language adapter trait and manifest schema.
//!
//! An adapter is a language-specific knowledge module: it describes the project
//! layout, toolchain commands, verification recipes, common failures, and repair
//! strategies for one ecosystem. The orchestrator uses it to inject concrete
//! context into planner/executor prompts and to drive the artifact verifier.

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

use super::intent::CodingIntent;

// ── Manifest schema ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolchainCheck {
    pub name: String,
    pub command: String,
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilesystemContract {
    pub required_actions: Vec<String>,
    pub auto_create_parent_dirs: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationLevels {
    pub minimal: Vec<String>,
    pub standard: Vec<String>,
    pub strict: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackagingSpec {
    /// Glob-style path prefixes to exclude from source ZIP
    pub source_zip_exclude: Vec<String>,
    /// Template for binary artifact name, e.g. "{project_name}" or "{project_name}.exe"
    pub binary_name_template: Option<String>,
}

/// Full language adapter manifest — everything the system needs to know about
/// one language/ecosystem without asking the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterManifest {
    pub language: String,
    /// Alternative names / aliases recognised in user requests
    pub aliases: Vec<String>,
    /// Files that indicate an existing project of this language
    pub project_markers: Vec<String>,
    /// Available build profiles (e.g. "cli", "gui-macroquad", "web-axum")
    pub profiles: Vec<String>,
    /// Toolchain binaries to check before planning
    pub toolchain_checks: Vec<ToolchainCheck>,
    /// Which filesystem actions this adapter needs
    pub filesystem_contract: FilesystemContract,
    /// Mapping from recipe name -> shell command
    pub verification_recipes: HashMap<String, String>,
    /// Which recipes to run at each verification level
    pub verification_levels: VerificationLevels,
    /// Things the model often gets wrong for this language
    pub common_failures: Vec<String>,
    /// How to fix those failures
    pub repair_strategies: Vec<String>,
    pub packaging: PackagingSpec,
    /// Extra system-prompt text injected when this adapter is active
    pub system_prompt_addition: String,
}

// ── Adapter trait ─────────────────────────────────────────────────────────────

pub trait LanguageAdapter: Send + Sync {
    /// Build and return the full adapter manifest.
    /// Called infrequently (once per task), so returning owned is fine.
    fn manifest(&self) -> AdapterManifest;

    /// Select the best profile given optional framework/project-type hints.
    fn resolve_profile(&self, framework: Option<&str>, project_type: Option<&str>) -> String;
}

// ── Registry ─────────────────────────────────────────────────────────────────

/// Return the adapter for the given language name (case-insensitive).
pub fn get_adapter(language: &str) -> Option<Box<dyn LanguageAdapter>> {
    match language.to_lowercase().as_str() {
        "rust" | "rs" | "cargo" => Some(Box::new(super::adapters::rust::RustAdapter)),
        "python" | "py" => Some(Box::new(super::adapters::python::PythonAdapter)),
        "javascript" | "js" | "typescript" | "ts" | "node" => {
            Some(Box::new(super::adapters::javascript::JavaScriptAdapter))
        }
        "go" | "golang" => Some(Box::new(super::adapters::go::GoAdapter)),
        _ => None,
    }
}

/// Return the best adapter for a detected CodingIntent.
pub fn adapter_for_intent(intent: &CodingIntent) -> Option<Box<dyn LanguageAdapter>> {
    intent.language.as_deref().and_then(get_adapter)
}
