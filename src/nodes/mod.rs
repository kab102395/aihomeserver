//! Orchestrator nodes.
//!
//! Each node is a small, testable unit that:
//! - takes ownership of `SystemState`
//! - reads artifacts + plan + config in the state
//! - writes new artifacts/logs back into the state
//! - returns the updated state
//!
//! Keeping nodes separated is what makes the agent loop explainable: you can point
//! to exactly where planning happens, where tools are called, where critique/repair
//! occurs, and where the final answer is assembled.

pub mod critic;
pub mod executor;
pub mod finalization;
pub mod intake;
pub mod planner;
pub mod repair;
pub mod tool_execution;
