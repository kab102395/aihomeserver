//! Embedded repo docs used by `/learn`.
//!
//! The `/learn/file` endpoint normally reads from the repo root, but these docs are also
//! embedded at compile time so the learn UI still works even when the server’s repo root
//! isn’t present on disk (or is configured differently).

/// Return embedded markdown content for a known doc path, if available.
///
/// Connection:
/// - `GET /learn/file` uses this to serve README/DESIGN/roadmap docs even when file IO fails.
pub fn embedded_doc(path: &str) -> Option<&'static str> {
    match path.replace('\\', "/").as_str() {
        "README.md" => Some(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/README.md"
        ))),
        "DESIGN.md" => Some(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/DESIGN.md"
        ))),
        "NEXT_STEPS_2026-04-22.md" => Some(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/NEXT_STEPS_2026-04-22.md"
        ))),
        "PROGRESS_2026-04-22.md" => Some(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/PROGRESS_2026-04-22.md"
        ))),
        "UI_FEATURES_ROADMAP_2026-04-22.md" => Some(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/UI_FEATURES_ROADMAP_2026-04-22.md"
        ))),
        _ => None,
    }
}
