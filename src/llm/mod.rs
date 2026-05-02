//! LLM integration layer.
//!
//! This repo currently targets Ollama (`ollama.rs`) for local model execution.
//! The rest of the system depends on an abstract-ish client API (messages, roles,
//! structured JSON completion) to keep prompts and parsing centralized.

pub mod ollama;
