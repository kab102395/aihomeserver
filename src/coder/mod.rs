//! Coder Executor pipeline — typed structures and logic for code-generation tasks.
//!
//! This module is intentionally free of LLM calls. It provides:
//! - `CodingIntent` detection from the user request (keyword heuristics)
//! - `LanguageAdapter` trait + per-language manifests (Rust, Python, JS, Go)
//! - `ExecutionManifest` + `ArtifactVerification` structs
//! - Mechanical `verify()` — checks real files, no LLM needed
//!
//! The orchestrator, planner, executor, critic, and repair nodes consume these
//! types to make coding tasks deterministically verifiable.

pub mod adapter;
pub mod adapters;
pub mod intent;
pub mod manifest;
pub mod repair_planner;
pub mod verifier;

pub use adapter::{adapter_for_intent, get_adapter, AdapterManifest, LanguageAdapter};
pub use intent::{detect_coding_intent, CodingIntent};
pub use manifest::{ArtifactVerification, ExecutionManifest, WriteStep};
pub use repair_planner::plan_deterministic_repair;
pub use verifier::verify;
