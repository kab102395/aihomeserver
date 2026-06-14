//! ExecutionManifest — the typed contract produced before file-writing begins.
//!
//! The planner creates a coding_execution_manifest step (LLM-only, output_format:"json")
//! whose output is deserialized into this struct and stored in SystemState.
//! The critic and repair nodes use it as the ground truth for what must exist.

use serde::{Deserialize, Serialize};

/// A single file-write step described in the manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteStep {
    /// Relative path within workspace (e.g. "snake_rust/src/main.rs")
    pub path: String,
    /// Human-readable description of what this file is for
    pub purpose: String,
}

/// The full execution contract for a coding task.
/// Produced by the planner before any files are written.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionManifest {
    pub project_name: String,
    /// Path relative to workspace root (e.g. "snake_rust")
    pub project_root: String,
    pub language: String,
    pub profile: String,
    pub intent: String,
    /// source | binary | zip | docker | web_app | library | script
    pub deliverable: String,
    /// Files that MUST exist when the task is complete (relative to workspace)
    pub required_files: Vec<String>,
    /// Ordered list of files to write (used for repair targeting)
    #[serde(default)]
    pub write_plan: Vec<WriteStep>,
    /// Ordered recipe names to run for verification (e.g. ["check_project", "build_release"])
    #[serde(default)]
    pub verification_plan: Vec<String>,
    /// All paths (files + packages) that must exist at completion
    pub expected_artifacts: Vec<String>,
}

/// Result of mechanically verifying an ExecutionManifest against the filesystem.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactVerification {
    /// "success" | "partial" | "failed"
    pub status: String,
    /// required_files that are missing or empty
    pub missing_files: Vec<String>,
    /// expected_artifacts that are missing or empty
    pub missing_artifacts: Vec<String>,
    /// Whether the build commands returned exit_code=0 (None if not run yet)
    pub build_passed: Option<bool>,
    /// Raw exit codes captured from shell tool results (keys are *_result artifact keys)
    #[serde(default)]
    pub build_exit_codes: std::collections::HashMap<String, i64>,
    /// Whether the zip/package was created and non-empty (None if not applicable)
    pub package_verified: Option<bool>,
    /// Zip size in bytes when a zip artifact exists (None if not applicable)
    pub zip_size_bytes: Option<u64>,
    /// Zip entry count when a zip artifact exists (None if not applicable)
    pub zip_entry_count: Option<usize>,
    /// Human-readable notes (e.g. "cargo not available, build skipped")
    #[serde(default)]
    pub limitations: Vec<String>,
}

impl ArtifactVerification {
    /// True if there are no missing files or artifacts.
    pub fn is_complete(&self) -> bool {
        self.missing_files.is_empty() && self.missing_artifacts.is_empty()
    }
}
