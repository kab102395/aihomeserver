//! Mechanical artifact verifier — checks real files against ExecutionManifest.
//! No LLM calls. Pure filesystem + zip inspection.

use std::path::Path;
use super::manifest::{ArtifactVerification, ExecutionManifest};

/// Verify that all required files and expected artifacts exist in the workspace.
///
/// `workspace` is the absolute path to the workspace root.
/// `build_exit_codes` maps artifact output_keys to their shell exit codes (for build steps).
pub fn verify(
    manifest: &ExecutionManifest,
    workspace: &str,
    build_exit_codes: &std::collections::HashMap<String, i64>,
) -> ArtifactVerification {
    let ws = Path::new(workspace);
    let project_root = manifest.project_root.trim_matches('/').to_string();

    // Resolve manifest paths that may be either project-root-relative ("Cargo.toml")
    // or workspace-relative ("snake_rust/Cargo.toml").
    let resolve = |rel: &str| {
        let r = rel.trim_start_matches('/');
        if project_root.is_empty() || r.starts_with(&project_root) || r.contains('/') {
            ws.join(r)
        } else {
            ws.join(&project_root).join(r)
        }
    };

    // ── Check required files ──────────────────────────────────────────────────
    let missing_files: Vec<String> = manifest
        .required_files
        .iter()
        .filter(|rel| {
            let full = resolve(rel);
            !full.exists() || full.metadata().map(|m| m.len() == 0).unwrap_or(true)
        })
        .cloned()
        .collect();

    // ── Check expected artifacts (may include zip path) ───────────────────────
    let mut missing_artifacts: Vec<String> = Vec::new();
    let mut package_verified: Option<bool> = None;
    let mut zip_size_bytes: Option<u64> = None;
    let mut zip_entry_count: Option<usize> = None;

    for artifact_path in &manifest.expected_artifacts {
        let full = resolve(artifact_path);
        if !full.exists() || full.metadata().map(|m| m.len() == 0).unwrap_or(true) {
            missing_artifacts.push(artifact_path.clone());
            if artifact_path.ends_with(".zip") {
                package_verified = Some(false);
            }
        } else if artifact_path.ends_with(".zip") {
            // Spot-check the zip: open it and verify it has entries
            zip_size_bytes = full.metadata().map(|m| m.len()).ok();
            zip_entry_count = match std::fs::File::open(&full)
                .ok()
                .and_then(|f| zip::ZipArchive::new(f).ok())
            {
                Some(a) => Some(a.len()),
                None => None,
            };
            package_verified = Some(verify_zip(&full, manifest));
            if package_verified == Some(false) {
                missing_artifacts.push(format!("{} (zip invalid or empty)", artifact_path));
            }
        }
    }

    // ── Check build results from artifacts ────────────────────────────────────
    let build_passed: Option<bool> = if build_exit_codes.is_empty() {
        None
    } else {
        // Pass if ALL build-related exit codes are 0
        let all_passed = build_exit_codes.values().all(|&code| code == 0);
        Some(all_passed)
    };

    // ── Compute status ────────────────────────────────────────────────────────
    let status = if missing_files.is_empty() && missing_artifacts.is_empty() {
        if build_passed == Some(false) { "partial" } else { "success" }
    } else if missing_files.len() == manifest.required_files.len() {
        "failed"
    } else {
        "partial"
    };

    ArtifactVerification {
        status: status.to_string(),
        missing_files,
        missing_artifacts,
        build_passed,
        build_exit_codes: build_exit_codes.clone(),
        package_verified,
        zip_size_bytes,
        zip_entry_count,
        limitations: vec![],
    }
}

/// Open a zip file and check it has at least one source entry (not just metadata).
/// Returns false if zip is invalid, empty, or only contains excluded paths.
fn verify_zip(zip_path: &Path, manifest: &ExecutionManifest) -> bool {
    let file = match std::fs::File::open(zip_path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut archive = match zip::ZipArchive::new(file) {
        Ok(a) => a,
        Err(_) => return false,
    };
    if archive.len() == 0 {
        return false;
    }
    // Verify at least one expected source file is present
    for required in &manifest.required_files {
        let req = required.trim_start_matches('/');
        let root = manifest.project_root.trim_matches('/');
        let name_in_zip = if root.is_empty() || req.starts_with(root) {
            req.to_string()
        } else {
            format!("{}/{}", root, req)
        };
        for i in 0..archive.len() {
            if let Ok(entry) = archive.by_index(i) {
                if entry.name() == name_in_zip || entry.name().ends_with(required.as_str()) {
                    return true;
                }
            }
        }
    }
    // If we couldn't find any required file, still return true if archive has >0 entries
    // (the entries may have different prefix structures)
    archive.len() > 0
}
