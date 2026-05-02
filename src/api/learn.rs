//! Embedded HTML for the `/learn` UI.
//!
//! Keeping the UI markup in a real `.html` file makes it dramatically easier to:
//! - read and edit without Rust raw-string noise
//! - add comments/explanations (this repo’s `/learn` is intended to be educational)
//! - avoid accidental escaping/encoding issues
//!
//! The server serves this directly from memory (no template engine).

/// The `/learn` single-page app (SPA) HTML.
///
/// Implementation lives in `src/api/learn.html` and is embedded at compile time.
pub const LEARN_HTML: &str = include_str!("learn.html");
