//! Persistence and retrieval (“memory”).
//!
//! This project uses multiple memory concepts:
//! - `conversation`: sessions + chat turns for continuity
//! - `episodic`: one record per run (plan/artifacts/timings) for replay
//! - `semantic`: embeddings for few-shot retrieval of similar past runs
//! - `knowledge`: curated notes that can be injected into planning
//! - `sources`: grounded source cache used by the facts/grounding contract
//!
//! Keeping these stores separate matches their query patterns and trust models.

pub mod conversation;
pub mod episodic;
pub mod knowledge;
pub mod semantic;
pub mod sources;
