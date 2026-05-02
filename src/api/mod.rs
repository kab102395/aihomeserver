//! HTTP API and embedded UIs.
//!
//! This module defines:
//! - the Axum router and handlers (`server.rs`)
//! - the chat UI (`ui.rs`) embedded as HTML
//! - the learn/interview-prep UI (`learn.rs` + `learn.html`)
//! - eval endpoints (`evals.rs`) used for trust and deep health checks

pub mod evals;
pub mod learn;
pub mod learn_docs;
pub mod server;
pub mod ui;
