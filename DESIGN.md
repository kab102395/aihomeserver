# aihomeserver — Design Document

> Local LLM orchestration engine. All inference runs on your hardware via Ollama.
> No cloud, no telemetry, no Python.

---

## Vision

A home server that acts as a capable AI agent — not a chatbot wrapper. It plans multi-step tasks, executes tools autonomously, critiques its own work, repairs mistakes, and remembers everything across sessions. The system grows smarter over time by retrieving relevant past experience when handling new requests.

Hardware target: RTX 5070 Ti (16 GB VRAM), 64 GB RAM.

---

## Architecture — Three Layers

```
┌─────────────────────────────────────────────────────┐
│  INFERENCE LAYER                                    │
│  Fast model: qwen2.5:14b  (~8 GB VRAM)             │
│  Deep critic: qwen2.5:32b (~19 GB, spills to RAM)  │
│  Embeddings: nomic-embed-text (~500 MB)             │
└──────────────────────┬──────────────────────────────┘
                       │
┌──────────────────────▼──────────────────────────────┐
│  ORCHESTRATION LAYER (state machine)                │
│                                                     │
│  Intake → Planner → Executor → ToolExecution        │
│             ↑                        ↓              │
│           Replan ← Repair ←──── Critic              │
│                                     ↓               │
│                              Finalization           │
└──────────────────────┬──────────────────────────────┘
                       │
┌──────────────────────▼──────────────────────────────┐
│  TOOL LAYER                                         │
│  filesystem · shell · git                           │
│  http_fetch · web_search (planned)                  │
│  sql · apex (planned)                               │
└─────────────────────────────────────────────────────┘
```

---

## Orchestration State Machine

```rust
enum OrchestratorNode {
    Intake,         // classify request, set risk score
    Planner,        // generate JSON plan (fast model)
    Executor,       // execute each step (fast model)
    ToolExecution,  // dispatch tool calls from plan
    Critic,         // validate output (tiered by risk)
    Repair,         // fix issues identified by critic
    Replan,         // full replan if repair cycles exhausted
    Finalization,   // summarize, persist to memory
    Done,
}
```

**Risk routing:**

| Score | Path | Critic |
|-------|------|--------|
| 0–3   | Low  | Skipped — auto-pass |
| 4–7   | Standard | Fast critic (14b) |
| 8–10  | High | Deep critic (32b) + human gate before destructive action |

**Repair limits:**
- Max 2 repair cycles → trigger Replan
- Max 3 replans → force Finalization

---

## Memory — Three Layers

### 1. Working Memory (in-flight)
`SystemState` — lives for the duration of one orchestrator run. Holds the plan, artifacts, event log, critic history, failure taxonomy, and conversation history for the current session turn.

### 2. Episodic Memory (SQLite — `episodic.db`)
Completed task records. Every finished run is persisted with:
- `task_id`, `user_request`, `plan_json`, `artifacts_json`
- `critic_scores`, `failure_count`, `repair_cycles`, `duration_ms`
- `success`, `created_at`

Also stores **conversation sessions** and **turns**:
- `sessions` table: `session_id`, `created_at`, `last_active`
- `conversation_turns` table: `session_id`, `role`, `content`, `timestamp`

The last N turns of the active session are injected into every planner and executor prompt so the model always has conversational context.

### 3. Semantic Memory (planned — Phase 3)
Vector embeddings of completed tasks using `nomic-embed-text`. Stored in a local HNSW index (hnswlib via FFI, or a pure-Rust crate). On each new request, the top-3 most similar past successes are retrieved and injected as few-shot examples into the planner prompt.

This is what makes the system genuinely learn — a new SQL task can reference how a similar query was structured last week; a Blender script can reuse a proven checkpoint pattern.

---

## Tool Contract

Every tool returns a `ToolResult`:

```rust
pub struct ToolResult {
    pub success: bool,
    pub error_type: ErrorType,   // None | Llm | Tool | Env | Timeout | Permission
    pub error_code: Option<String>,
    pub trace: Option<String>,
    pub output: Option<serde_json::Value>,
    pub checkpoint: Option<serde_json::Value>,  // saved to state.checkpoints
    pub observed_state_hash: Option<String>,
    pub timestamp: DateTime<Utc>,
}
```

Tools never panic. Unknown tool names are skipped gracefully (logged, not counted as failures).

---

## Docker Modes (Runtime vs Dev/Test)

There are two supported Docker ways to run the server:

- **Runtime (default)**: built binary on a slim Debian image. Good for always-on use, but it **does not include** developer tools like `cargo`.
- **Dev/Test (optional)**: runs from source in a Rust image with common CLI tools (`cargo`, `git`, `rg`, `curl`). This is the mode you want if you expect the AI to run build/test commands or search your repo tree.

**Runtime**

```bash
docker compose up -d --build
```

**Dev/Test (bind-mounts the repo into the container at `/workspace`)**

```bash
docker compose -f docker-compose.yml -f docker-compose.dev.yml up -d --build
```

In Dev/Test mode, set `WORKSPACE=/workspace`, so the `filesystem` and `shell` tools can read/search your project tree.

---

## Conversation Sessions

Each request carries an optional `session_id`. If omitted, a new session is created and its ID is returned in the response. Subsequent requests with the same `session_id` receive the last 10 turns as context.

**API flow:**
```
POST /run  { "request": "...", "session_id": "uuid-or-null" }
→ { "session_id": "uuid", "success": true, "artifacts": {...}, ... }

GET /sessions          — list recent sessions for sidebar
GET /session/:id       — load all turns for a session
GET /task/:id          — load a single completed task record
GET /history           — last 50 completed tasks
GET /health            — liveness check
```

---

## Current Implementation Status

| Component | Status | Notes |
|-----------|--------|-------|
| Orchestrator state machine | ✅ Done | Intake→Plan→Execute→Tool→Critic→Repair→Replan→Final |
| Dual model routing (14b/32b) | ✅ Done | Fast + deep critic |
| Risk-based critic tiering | ✅ Done | Low skip / standard fast / high deep |
| Repair + replan loops | ✅ Done | 2 repair cycles, 3 replan max |
| Tool: filesystem | ✅ Done | read / write / list in ./workspace |
| Tool: shell | ✅ Done | subprocess execution with timeout |
| Tool: git | ✅ Done | status / log / commit |
| Episodic memory (SQLite) | ✅ Done | save / recent / get_by_id |
| HTTP API (axum) | ✅ Done | /run, /history, /task/:id, /health |
| Chat UI | ✅ Done | markdown, code blocks, event log, artifact viewer |
| Executor receives user_request | ✅ Fixed | Was missing — LLM could not see the question |
| Conversation sessions | 🔨 In progress | session_id threading, turn storage, history injection |
| Semantic memory (RAG) | ⬜ Phase 3 | nomic-embed-text + HNSW vector index |
| Streaming responses (SSE) | ⬜ Phase 4 | Token-by-token in UI |
| Tool: http_fetch | ⬜ Phase 4 | reqwest-based web fetching |
| Tool: web_search | ⬜ Phase 4 | Local SearXNG or similar |
| Domain agents (SQL, Blender, APEX) | ⬜ Phase 5 | Specialized system prompts + routing |
| Human gate (risk 8–10) | ⬜ Phase 5 | /approve endpoint, UI confirmation dialog |
| Critic quality for LLM answers | ⬜ Phase 5 | Currently only checks tool result flags |

---

## Roadmap

### Phase 1 — Core Scaffold ✅
Orchestrator, planner, executor, tool layer, critic, repair/replan, episodic memory, HTTP API, chat UI.

### Phase 2 — Conversation Context 🔨
- Session table + turn table in SQLite
- `session_id` in request/response
- Last 10 turns injected into planner + executor prompts
- UI sidebar shows sessions instead of flat task list
- New Chat button starts a fresh session

### Phase 3 — Semantic Memory (RAG)
- Embed completed tasks with `nomic-embed-text` (runs in Ollama)
- Store embeddings in a local HNSW index
- On each new request: retrieve top-3 similar past successes
- Inject as few-shot examples into planner prompt
- Dramatic improvement on repeated/similar task types

### Phase 4 — Streaming + Tool Expansion
- Switch `/run` to Server-Sent Events for token streaming
- Add `http_fetch` tool (reqwest, respects robots.txt)
- Add `web_search` tool (local SearXNG or Brave API)
- Improved critic for LLM-only (Q&A) tasks: score answer quality, not just tool flags

### Phase 5 — Domain Agents + Human Gate
- Domain routing: detect SQL / Blender / code gen / APEX intent at intake
- Inject domain-specific system prompts for each agent type
- Human gate: risk 8–10 tasks pause and return a `pending_approval` state
- `/approve/:task_id` endpoint to resume
- UI shows approval dialog for high-risk actions before execution

### Phase 6 — Multi-Agent Coordination
- Spawn sub-agents for parallel sub-tasks
- Aggregate sub-agent results in a coordinator agent
- Useful for large codebases, multi-file refactors, batch operations

---

## Model Configuration

```
Fast model (planner, executor, repair): qwen2.5:14b
  VRAM: ~8 GB (Q4_K_M quantization)
  Use: all non-critical paths

Deep critic (high-risk validation): qwen2.5:32b  
  VRAM: ~19 GB (spills ~3 GB to 64 GB system RAM)
  Use: risk score 8–10 only

Embeddings (Phase 3): nomic-embed-text
  VRAM: ~500 MB
  Use: semantic memory indexing + retrieval
```

Models run one at a time via Ollama. The 32b critic only loads when triggered by a high-risk task.

---

## File Structure

```
src/
├── main.rs                  — startup: LLM, tools, memory, HTTP server
├── state.rs                 — SystemState, PlannerOutput, ToolResult, ConversationTurn
├── orchestrator.rs          — state machine router
├── nodes/
│   ├── intake.rs            — request classification
│   ├── planner.rs           — JSON plan generation (fast model)
│   ├── executor.rs          — step execution (fast model)
│   ├── tool_execution.rs    — tool dispatch
│   ├── critic.rs            — tiered output validation
│   ├── repair.rs            — apply critic feedback
│   └── finalization.rs      — summarize + persist
├── tools/
│   ├── mod.rs               — ToolRegistry, BaseTool trait
│   ├── filesystem.rs        — read / write / list
│   ├── shell.rs             — subprocess execution
│   └── git.rs               — git operations
├── memory/
│   ├── episodic.rs          — SQLite task records
│   └── conversation.rs      — SQLite session + turn store
├── llm/
│   └── ollama.rs            — reqwest HTTP client → Ollama /api/chat
└── api/
    ├── server.rs            — axum routes + handlers
    └── ui.rs                — embedded chat UI (HTML/CSS/JS)
```

---

## Key Design Decisions

**Why Rust?** Zero-cost abstractions, ownership model prevents entire classes of bugs, no GIL, async with tokio is excellent for I/O-bound LLM calls, single static binary for deployment.

**Why no LangGraph?** LangGraph is Python-only and adds a heavy dependency. Rust enums + match are cleaner, fully type-safe, and compile-time exhaustive. The state machine is 150 lines.

**Why Ollama?** Wraps llama.cpp with CUDA support, handles model loading/unloading, provides an OpenAI-compatible API. The 32b model can spill to RAM gracefully with Ollama's memory management.

**Why SQLite?** Zero infrastructure — one file, fully ACID, sufficient for millions of records. LanceDB or Chroma for the vector index in Phase 3 (both have Rust bindings).

**Why embedded HTML?** Single binary deployment. No separate frontend server, no npm build step, no static file serving configuration. The UI compiles into the binary.
