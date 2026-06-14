//! Deterministic repair planner for coding tasks.
//!
//! Goal: when the verifier says a coding run is missing a zip/build artifact, generate the
//! next tool call mechanically (no LLM) so the system doesn't re-search or thrash.

use std::collections::HashMap;

use serde_json::json;

use crate::coder::{get_adapter, ArtifactVerification, ExecutionManifest};
use crate::state::StepDefinition;

/// Best-effort deterministic tool-call generation for coding repairs.
///
/// Returns a JSON tool-call object:
/// `{ "tool": "...", "params": { ... } }`
pub fn plan_deterministic_repair(
    manifest: &ExecutionManifest,
    verif: &ArtifactVerification,
    step: &StepDefinition,
) -> Option<serde_json::Value> {
    let tool = step.tool_binding.as_deref()?;

    // Find the expected zip path (prefer manifest list, else verifier missing list).
    let expected_zip = manifest
        .expected_artifacts
        .iter()
        .find(|p| p.to_lowercase().ends_with(".zip"))
        .cloned()
        .or_else(|| {
            verif.missing_artifacts
                .iter()
                .find(|p| p.to_lowercase().ends_with(".zip"))
                .cloned()
        });

    // Adapter packaging excludes (fallback to safe defaults).
    let excludes: Vec<String> = get_adapter(&manifest.language)
        .map(|a| a.manifest().packaging.source_zip_exclude.clone())
        .unwrap_or_else(|| vec!["target/".into(), ".git/".into()]);

    // Filesystem packaging repair: emit zip_dir deterministically.
    if tool == "filesystem" {
        let package_missing = verif.package_verified == Some(false)
            || expected_zip.is_some()
                && verif
                    .missing_artifacts
                    .iter()
                    .any(|p| p.to_lowercase().ends_with(".zip"));

        if package_missing {
            let zip_path = expected_zip.unwrap_or_else(|| format!("{}.zip", manifest.project_root));
            return Some(json!({
                "tool": "filesystem",
                "params": {
                    "action": "zip_dir",
                    "source_dir": manifest.project_root,
                    "output_path": zip_path,
                    "exclude": excludes,
                }
            }));
        }
    }

    // Shell build repair: rerun the adapter's canonical build/check command with correct cwd.
    if tool == "shell" {
        let build_failed = verif.build_passed == Some(false);
        if build_failed {
            let adapter = get_adapter(&manifest.language);
            let recipes: HashMap<String, String> = adapter
                .map(|a| a.manifest().verification_recipes)
                .unwrap_or_default();

            // Prefer build_release, else the first recipe listed in the manifest verification_plan.
            let cmd = recipes
                .get("build_release")
                .cloned()
                .or_else(|| {
                    manifest
                        .verification_plan
                        .iter()
                        .find_map(|k| recipes.get(k).cloned())
                })
                .or_else(|| recipes.get("check_project").cloned())
                .unwrap_or_else(|| "echo \"no build recipe available\"".to_string());

            return Some(json!({
                "tool": "shell",
                "params": {
                    "command": cmd,
                    "cwd": manifest.project_root,
                    "timeout_secs": 120
                }
            }));
        }
    }

    None
}

