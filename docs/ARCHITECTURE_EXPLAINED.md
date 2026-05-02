# aihomeserver architecture (explained)

This doc is a “talk track” for explaining the codebase in an interview:
what each layer does, how requests flow, and *why* specific design choices were made.

If you want to learn by clicking through code, open `/learn` in the UI and use the “Open …” buttons.

## 1) What problem this server solves

This project runs an **agent loop** behind an HTTP API:

- A user sends a natural-language request.
- The system plans steps, executes tools, critiques results, repairs/replans when needed, then produces an answer.
- Everything is logged and persisted so you can **replay and debug** runs.

Key constraints the architecture optimizes for:

- **Auditability**: tool calls and intermediate artifacts are stored.
- **Safety**: side effects are gated through explicit tools and optional human approvals.
- **Reproducibility**: built-in evals and “deep health” checks validate capabilities.
- **Practical UX**: a lightweight embedded UI + `/learn` makes the system demo-able.

## 2) The high-level layers

### HTTP layer (Axum)

Files:
- `src/api/server.rs`
- `src/api/ui.rs` and `src/api/learn.rs` (embedded UIs)

Why Axum:
- Rust-native async server that composes cleanly with Tokio.
- “Extractors” (e.g. `State`, `Json`, `Path`) keep handlers small and typed.

### Orchestrator (agent runtime)

Files:
- `src/orchestrator.rs`
- `src/nodes/*`
- `src/state.rs`

This is the core: a finite-state machine that routes through nodes like:
intake → planner → executor → tool_execution → critic → repair → finalization.

Why a state machine instead of ad-hoc loops:
- Predictability: you can reason about transitions and termination.
- Debuggability: each node can log events + write artifacts.
- Extensibility: add a node without rewriting the whole flow.

### Tools (side effects)

Files:
- `src/tools/*`

Tools are the only way the agent can do “real world” actions (filesystem, shell, HTTP fetch, git, web search…).

Why explicit tools:
- Enforces a capability boundary: the LLM can’t directly do side effects.
- Every tool call returns structured output which becomes an artifact.
- Makes it possible to add policy (path traversal checks, allow-lists, approvals).

### Memory (persistence + retrieval)

Files:
- `src/memory/*`

There are multiple “memory” concepts:

- **Conversation memory**: sessions and chat turns for continuity.
- **Episodic memory**: a record per run (request, plan JSON, artifacts JSON, success, timings).
- **Semantic memory**: embeddings for few-shot retrieval of similar past runs.
- **Knowledge base**: curated notes you can upsert and retrieve by relevance.
- **Source cache**: grounded sources that back “facts”.

Why split them:
- Different query patterns (by session, by recency, by similarity, by tags).
- Different trust models (curated KB vs. auto-generated artifacts).

## 3) The request lifecycle (end-to-end)

### A) Start a run

Endpoint: `POST /run`

The handler:
- builds a `SystemState`
- injects config + conversation history + semantic examples + relevant KB entries
- spawns the background task
- returns quickly with `{ task_id, session_id }`

Why background tasks + polling:
- Avoids holding an HTTP request open for long runs.
- Lets the UI display progress and handle cancellation/approval in the future.

### B) Poll for completion

Endpoint: `GET /task/:id/status`

Returns a `TaskStatusPayload`:
- `running`
- `done { response: RunResponse }`
- `failed { error }`

Why an in-memory task store:
- Fast UI polling without hitting SQLite for every tick.
- SQLite persistence still happens at completion for replay/history.

### C) Orchestrate nodes

The orchestrator loops until a termination condition is met (or max steps).

Common reasons to transition:
- Planner produced steps → executor runs them.
- Executor needs a tool → tool_execution runs it.
- Critic finds problems → repair proposes a fix or replan.
- Finalization assembles the final answer artifact.

### D) Persist a record

On completion:
- conversation turns are saved
- `TaskRecord` is written to episodic memory
- semantic embedding may be stored (often only for successful runs)

Why store `artifacts_json` as JSON text:
- Artifact schemas evolve; JSON avoids constant DB migrations.
- Still provides a stable “replay” record for debugging.

## 4) “Why did you use X?” (common interview answers)

### Arc / Mutex / RwLock

Used for shared state across concurrent tasks:
- `Arc<T>`: shared ownership across threads/tasks.
- `Mutex<T>`: exclusive access for mutation.
- `RwLock<T>`: many readers or one writer; good for config.

Why:
- Axum handlers run concurrently.
- Background runs need access to the same stores/config.
- Rust’s type system forces you to be explicit about concurrency and mutation.

### Tokio async/await

Why:
- Non-blocking I/O (HTTP requests, DB calls, tool operations).
- Concurrency without a thread-per-request model.

### Evals + deep health

Files:
- `src/api/evals.rs`

Why:
- Makes the system demo-able and trustworthy.
- Detects broken dependencies early (models, tools, permissions).

### Grounding contract

Files:
- `src/grounding.rs`

Why:
- For “research / latest / verify” tasks, require evidence before answering.
- Prevents confident nonsense; sources/facts become auditable artifacts.

## 5) How to study the codebase quickly

1) Start at `src/main.rs` (wiring and router setup)
2) Read `src/api/server.rs` (endpoints and state injection)
3) Read `src/orchestrator.rs` and `src/nodes/*` (the runtime)
4) Skim `src/tools/*` (capabilities + safety)
5) Skim `src/memory/*` (what gets persisted and why)

