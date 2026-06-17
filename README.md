# aihomeserver

> **A fully self-hosted AI agent server built in Rust.**  
> Orchestrates a deterministic FSM pipeline (Plan → Execute → Critique → Repair) over local Ollama models, with real-time SSE streaming, a structured tool registry, and persistent memory.

Developed and maintained by **Kyle Barrett** · [Ember Tech Solutions LLC](mailto:kab102395@gmail.com)

---

## Table of Contents

1. [What It Is](#what-it-is)
2. [Architecture Overview](#architecture-overview)
3. [Orchestrator FSM](#orchestrator-fsm)
4. [Module Reference](#module-reference)
5. [Tool Registry](#tool-registry)
6. [Memory System](#memory-system)
7. [Coder Pipeline (Phase 1)](#coder-pipeline-phase-1)
8. [Grounding Contract](#grounding-contract)
9. [Streaming & UI](#streaming--ui)
10. [API Reference](#api-reference)
11. [Configuration](#configuration)
12. [Installation — Native](#installation--native)
13. [Installation — Docker + SearXNG](#installation--docker--searxng)
14. [SearXNG Integration](#searxng-integration)
15. [Known Issues & Limitations](#known-issues--limitations)
16. [In Progress](#in-progress)
17. [Security Notice](#security-notice)
18. [Repository Layout](#repository-layout)

---

## What It Is

`aihomeserver` is a local-first AI code agent stack that runs on your own hardware. The current product shape is:

- an Electron desktop launcher
- a Rust coordinator running on the host
- an Ubuntu worker VM for the active execution surface
- a VM-backed workspace for shell, filesystem, and browser tasks
- local Ollama models for planning, execution, critique, and repair

There is no required cloud agent backend. The intended runtime is:

- host machine for UI, orchestration, packaging, and configuration
- VM for task execution and browser automation

**What it is not:** a thin wrapper around a chat API. Every request still runs through a structured orchestration loop that plans, executes tools, verifies results mechanically, critiques with a second pass, and repairs or replans on failure.

**Primary use cases today:**
- VM-first coding tasks with exact file/output verification
- Research-backed Q&A with grounding enforcement
- Browser automation probes and extraction diagnostics
- Workspace file management against the active VM workspace
- Git and repo operations from the coordinator side
- Curated knowledge and replayable execution history

**Current product emphasis:**
- one active task computer instead of split host/remote semantics
- honest blocked-site detection instead of invented extraction
- deterministic browser-task output contracts
- fast desktop reloads without reprovisioning the VM on every iteration

**Reference hardware:**
- NVIDIA RTX 5070 Ti (16 GB VRAM), 64 GB RAM
- Windows host
- Hyper-V Ubuntu worker VM
- Ollama local models

---

## Architecture Overview

```
┌──────────────────────────────────────────────────────────────────────────────┐
│                             Windows Host                                     │
│                                                                              │
│  Electron Desktop Launcher                                                   │
│  ├── boots coordinator container/runtime                                     │
│  ├── provisions or reconnects Hyper-V worker VM                              │
│  ├── probes worker auth/health/capabilities                                  │
│  └── exposes fast "Rebuild App" reload path                                  │
│                                                                              │
│  Rust Coordinator (Axum + Orchestrator)                                      │
│  ├── POST /run                                                               │
│  ├── POST /run/stream                                                        │
│  ├── workspace / git / KB / settings / evals                                 │
│  ├── Planner / Executor / Critic / Repair / Finalization                     │
│  ├── Tool registry with remote-aware shell/filesystem routing                │
│  └── VM preflight sync + capability injection                                │
└──────────────────────────────────────────────────────────────────────────────┘
                 │                                 │
                 ▼                                 ▼
        Ollama (local)                     SQLite + local config
                 │
                 ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│                           Ubuntu Worker VM                                   │
│                                                                              │
│  Python worker API                                                           │
│  ├── GET /health                                                             │
│  ├── GET /capabilities                                                       │
│  ├── POST /workspace/sync                                                    │
│  ├── POST /shell                                                             │
│  ├── POST /browser/fetch                                                     │
│  └── POST /filesystem/*                                                      │
│                                                                              │
│  Active task workspace: /workspace                                           │
│  Browser runtime: Playwright/Chromium when installed                         │
└──────────────────────────────────────────────────────────────────────────────┘
```

**Current design split:**

| Layer | Responsibility |
|-------|----------------|
| **Desktop layer** (`desktop/`) | Electron launcher, Hyper-V bootstrap orchestration, coordinator lifecycle, packaged app behavior |
| **Coordinator layer** (`src/api/`, `src/nodes/`, `src/orchestrator.rs`) | FSM node execution, task lifecycle, SSE, planner/executor/critic/repair/finalization |
| **Worker layer** (`src/bin/worker.rs`, `scripts/hyperv-worker.ps1`) | VM shell/filesystem/browser runtime, capability reporting, workspace sync, health/auth contract |
| **Memory/tool layer** (`src/tools/`, `src/memory/`) | Side effects, persistence, replayable artifacts, long-lived knowledge |

## Desktop + VM-First Model

The most important current architectural change is that the active execution surface is no longer assumed to be the host workspace.

When remote mode is active:

- the coordinator syncs a workspace mirror into the VM
- the worker `/workspace` becomes the active task computer
- shell commands run in the VM
- filesystem actions target the VM
- browser automation runs in the VM
- the Files tab is supposed to reflect VM state, not host repo state

That is what makes the system VM-first instead of a split-brain hybrid.

---

## Orchestrator FSM

Every request passes through an explicit finite-state machine. Nodes communicate exclusively through a shared `SystemState` — no hidden channels, no globals.

```
                    ┌─────────┐
    user request ──►│  Intake  │  loads conversation, KB, semantic context
                    └────┬────┘
                         │
                    ┌────▼──────────────┐
                    │ CodingClassifier  │  keyword heuristic — sets CodingIntent or passes through
                    └────┬──────────────┘
                         │
                    ┌────▼────┐
                    │ Planner │  produces PlannerOutput (steps, tools_required, risk_score 0–10)
                    └────┬────┘
                         │  risk_score ≥ threshold?
                         ├─── YES ──► NeedsApproval SSE ──► await user approve/reject
                         │
                    ┌────▼──────────┐
                    │   Executor    │  one step at a time; generates tool call JSON or LLM answer
                    └────┬──────────┘
                         │
                    ┌────▼──────────────┐
                    │  ToolExecution    │  dispatches to ToolRegistry; stores ToolResult as artifact
                    └────┬──────────────┘
                         │  coding task?
                         ├──► ArtifactVerifier (mechanical; no LLM)
                         │    emits ProjectCard SSE
                         │
                    ┌────▼────┐
              ┌─────│  Critic │  Low: auto-pass │ Standard: FAST_CRITIC │ High: DEEP_CRITIC
              │     └────┬────┘  research step: RESEARCH_CRITIC (hallucination detector)
              │          │       coding: deterministic manifest check first
              │          │
              │   pass?  │ fail (repair_cycle < 2)?
              │          ├────────────────────────────────────────────────────────────┐
              │          │                                                            │
              │          │ fail (repair_cycle ≥ 2)?          replan (replan_count < 3)?
              │          ├────────────────────────────────────────────────────────────┤
              │          │                                                            │
              │          ▼                                                       ┌────▼────┐
              │     ┌────────┐                                                   │  Repair │ targeted for coding tasks
              │     │ Replan │  replan_count < 3; resets repair_cycle            └────┬────┘
              │     └───┬────┘                                                        │
              │         │                                                             │
              │         └─────────────────────────────────────────────────────────────┘
              │                                                    back to Executor
              │
              └──► ┌──────────────────┐
                   │  Finalization    │  saves to episodic/conversation memory, auto-KB if configured
                   └──────────────────┘
```

**Bounds:**
- Repair: max 2 cycles before forcing replan
- Replan: max 3 per task before forced finalization
- Steps: configurable `max_steps` (default 20)

**Risk levels** (from `PlannerOutput.risk_score`):

| Score | Level | Critic path |
|-------|-------|-------------|
| 0–3 | Low | Auto-pass, no LLM critic call |
| 4–7 | Standard | FAST_CRITIC (lightweight single-step check) |
| 8–10 | High | DEEP_CRITIC + human approval gate |
| Research step | Any | RESEARCH_CRITIC (always, regardless of risk) |

---

## Module Reference

```
src/
├── main.rs                  Entry point — starts Axum, initializes SQLite, registers tools
├── config.rs                ServerConfig struct; persisted to config.json; live-reloadable via API
├── state.rs                 SystemState (single source of truth per run), SseEvent variants,
│                            PlannerOutput, StepDefinition, CriticReview, ToolResult
├── orchestrator.rs          Orchestrator struct; FSM loop; ArtifactVerifier routing
├── grounding.rs             Grounding contract enforcement; search query decomposition;
│                            strip_think_tags(); chrono-based dynamic year injection
├── metrics.rs               Shared in-memory metrics (tool call counts, latency histograms)
│
├── api/
│   ├── mod.rs
│   ├── server.rs            Axum router; /run, /run/stream, /task/:id/*, /workspace/*, /git/*,
│   │                        /knowledge/*, /eval/*, /health/*, /metrics, /settings, /sessions/*;
│   │                        injects worker capability snapshot and performs initial VM workspace sync
│   ├── ui.rs                Embedded chat UI HTML/CSS/JS (CHAT_HTML const); SSE rendering;
│   │                        VM workspace browser, settings/admin surface, worker diagnostics
│   └── learn.rs             /learn interview-prep walkthrough UI; embedded doc browser
│
├── nodes/
│   ├── mod.rs
│   ├── intake.rs            Loads conversation history, knowledge context, semantic examples;
│   │                        injects planning_answers; resolves session
│   ├── classifier.rs        CodingClassifier: detect_coding_intent() → sets state.coding_intent;
│   │                        stores adapter_manifest_json artifact
│   ├── planner.rs           Calls fast_model; injects KB/semantic/coding adapter context;
│   │                        emits Plan SSE; injects CODING PLAN CONTRACT for coding tasks
│   ├── executor.rs          Calls fast_model per step; generates tool call JSON or LLM answer;
│   │                        injects GROUNDING_GUARDRAIL for requires_facts steps;
│   │                        injects coding_manifest_prompt() for coding tasks;
│   │                        strip_think_tags() applied to all model output;
│   │                        browser-task verification now prefers exact execution artifacts
│   ├── tool_execution.rs    Dispatches tool calls from executor to ToolRegistry;
│   │                        stores ToolResult as artifact; emits ToolCall/ToolDone SSE
│   ├── critic.rs            Risk-gated critic; RESEARCH_CRITIC for grounded steps;
│   │                        deterministic manifest check for coding tasks;
│   │                        truncate_artifacts() prevents context overflow
│   ├── repair.rs            Generates targeted fix prompt; CodingRepairTarget (WriteFile,
│   │                        PatchSource, RunPackage, General) for coding tasks
│   └── finalization.rs      Persists to episodic/conversation memory; auto-KB save;
│                            emits final answer; browser-task final success now depends on
│                            verified execution output, not generic completion
│
├── tools/
│   ├── mod.rs               Tool trait; ToolRegistry (register/alias/execute/list)
│   ├── filesystem.rs        Local or VM-backed filesystem operations depending on execution mode:
│   │                        read, write, list, find, grep, delete, mkdir, rename, stat, tree, zip_dir
│   ├── shell.rs             Local or worker-routed shell execution depending on execution mode;
│   │                        syntax guard; timeout enforcement
│   ├── git.rs               Git operations within workspace (status, log, diff, add, commit,
│   │                        branch, checkout, clone, pull, push)
│   ├── web_search.rs        SearXNG JSON search with DDG HTML fallback; dynamic year injection
│   ├── http_fetch.rs        HTTP GET/POST with content-type aware extraction (HTML→text,
│   │                        JSON passthrough, size cap)
│   ├── parallel_search.rs   Multi-query parallel search; deduplication; aggregated results
│   └── save_knowledge.rs    Upsert to knowledge store; tag extraction; version tracking
│
├── memory/
│   ├── mod.rs
│   ├── conversation.rs      Session + turn store (SQLite); last-N injection into planning
│   ├── episodic.rs          Full task record per run: plan, artifacts, timings, answer summary
│   ├── semantic.rs          Embedding-free similarity (keyword overlap); few-shot retrieval
│   ├── knowledge.rs         Curated KB: topic/summary/content/tags/version; export to Markdown
│   └── sources.rs           Source URL cache for grounded research (deduplication, age-out)
│
├── llm/
│   ├── mod.rs
│   └── ollama.rs            OllamaClient; chat() and chat_stream() with streaming SSE relay;
│                            strip_think_tags() — strips <think>...</think> from all model output;
│                            Ollama options (num_gpu, num_ctx, num_predict, num_batch, num_thread)
│
├── worker.rs                Host-side worker client; wraps /capabilities, /shell, /workspace/sync,
│                            /browser/fetch, and VM filesystem calls
│
└── coder/
    ├── mod.rs               Re-exports; pub fn verify()
    ├── intent.rs            CodingIntent struct; task-class heuristics for script/file/browser/repo tasks
    ├── adapter.rs           LanguageAdapter trait; AdapterManifest; get_adapter(); adapter_for_intent()
    ├── scaffold.rs          Deterministic coding/browser scaffolds for common task shapes
    ├── browser_contract.rs  Browser-task verification, selector summaries, classification helpers
    ├── manifest.rs          ExecutionManifest; WriteStep; ArtifactVerification
    ├── verifier.rs          verify() — pure filesystem check; zip spot-check via zip crate
    ├── repair_planner.rs    coding_repair_target(); CodingRepairTarget enum
    └── adapters/
        ├── mod.rs
        ├── rust.rs          Full Rust adapter: profiles, recipes, packaging, common failures
        ├── python.rs        Python adapter (markers, toolchain checks, basic recipes)
        ├── javascript.rs    JS/TS adapter (markers, toolchain checks, basic recipes)
        └── go.rs            Go adapter (markers, toolchain checks, basic recipes)
```

---

## Tool Registry

Tools are the only mechanism through which the agent causes side effects. Every tool call goes through `ToolRegistry::execute()`, which records a `ToolResult` as an artifact for audit and replay.

| Tool name | Aliases | What it does |
|-----------|---------|-------------|
| `filesystem` | `file`, `fs` | Read, write, list, find, grep, delete, move, mkdir, copy, diff, stat, tree, zip_dir within workspace |
| `shell` | `run_command`, `bash`, `exec` | Execute shell commands locally or in the VM worker depending on `execution_mode`; timeout-guarded |
| `git` | `git_tool` | git status, log, diff, add, commit, branch, checkout, clone, pull, push |
| `web_search` | `search`, `duckduckgo_search` | Single-query search via SearXNG JSON or DDG HTML fallback |
| `parallel_search` | `multi_search` | Multi-query parallel search with deduplication |
| `http_fetch` | `fetch`, `curl` | HTTP GET/POST; HTML-to-text extraction; JSON passthrough |
| `save_knowledge` | `kb_save`, `knowledge_save` | Persist curated content to the knowledge base |

In remote mode, the important practical rule is:

- `shell` and `filesystem` should operate against the VM workspace, not the host repo tree

## Remote Worker API

The worker running inside the VM exposes the following contract:

| Method | Path | Purpose |
|--------|------|---------|
| `GET` | `/health` | Basic liveness |
| `GET` | `/capabilities` | Worker feature snapshot, including browser automation readiness |
| `POST` | `/workspace/sync` | Replace or refresh the VM task workspace mirror |
| `POST` | `/shell` | Run shell commands inside the VM workspace |
| `POST` | `/browser/fetch` | Simple browser-like fetch path from the VM |
| `POST` | `/filesystem/read` | Read VM file |
| `POST` | `/filesystem/write` | Write VM file |
| `POST` | `/filesystem/list` | List VM tree |
| `POST` | `/filesystem/find` | Find paths in VM workspace |
| `POST` | `/filesystem/grep` | Search file contents in VM workspace |
| `POST` | `/filesystem/delete` | Delete VM path |
| `POST` | `/filesystem/mkdir` | Create VM directory |
| `POST` | `/filesystem/rename` | Rename VM path |

The `/capabilities` response is important because it tells the coordinator whether the worker actually supports:

- shell
- browser fetch
- VM filesystem actions
- browser automation runtime readiness

**`zip_dir` filesystem action** (added in Phase 1):

```json
{
  "action": "zip_dir",
  "source_dir": "my_project",
  "output_path": "my_project.zip",
  "exclude": ["target/**", ".git/**", "*.zip"]
}
```

Returns: `{ "path": "...", "entries": [...], "entry_count": N, "size_bytes": N }`

---

## Memory System

Five independent SQLite stores, each with its own query pattern and trust model:

| Store | File | Purpose |
|-------|------|---------|
| **Conversation** | `conversation.db` | Sessions + chat turns; last-N injected into planner + executor prompts for continuity |
| **Episodic** | `episodic.db` | One full `TaskRecord` per run: plan JSON, artifacts, timings, answer summary; supports replay |
| **Semantic** | `semantic.db` | Embedding-free keyword-overlap similarity; retrieves similar past tasks as few-shot planner examples |
| **Knowledge** | `knowledge.db` | Curated KB entries: topic, summary, full content, tags, version; auto-injected when relevant to request |
| **Sources** | `sources.db` | Grounded source URL cache: deduplication + age-out for research tasks |

**Knowledge base operations:**
- `GET /knowledge` — list entries
- `GET /knowledge/:id` — single entry
- `POST /knowledge` — create
- `PUT /knowledge/:id` — update
- `DELETE /knowledge/:id` — delete
- `GET /knowledge/:id/download` — Markdown download
- `GET /knowledge/export` — full export as one Markdown document

**Auto-save modes** (configurable via `auto_kb_mode`):
- `"off"` — never auto-save (default before sessions)
- `"research"` — save when the run used research tools and answer is ≥ `auto_kb_min_chars` chars
- `"always"` — save for any substantial answer

---

## Coder Pipeline

The coding path is moving away from generic chat behavior and toward explicit task contracts.

### Current behavior

1. **CodingClassifier node** (between Intake and Planner): uses heuristics to detect coding-oriented tasks and browser-task intent so the planner can choose a smaller, more deterministic recipe.

2. **Planner injection**: coding tasks get a stricter contract. Simple file/script tasks should trend toward `write -> run -> read -> answer` instead of broad freeform planning.

3. **ExecutionManifest**: produced as a JSON LLM-only step. Contains: `project_root`, `required_files`, `write_plan`, `verification_plan` (ordered recipe names), `expected_artifacts` (including zip path). Auto-loaded from `coding_execution_manifest` artifact after every ToolExecution.

4. **ArtifactVerifier**: still performs mechanical checks, but browser tasks now also rely on exact execution-artifact verification instead of narrative-only success.

5. **Critic integration**: if `artifact_verification` is present with missing files or build failure, the critic produces a deterministic fail with `confidence=1.0` — no LLM call needed.

6. **Targeted repair**: repair logic is trying to prefer bounded fixes over wandering retries. For browser tasks that means corrected script generation and rerun, not repeated speculative replanning.

7. **ProjectCard SSE event**: still reports coding-task progress to the UI, but the bigger current product need is making the final answer reflect the last verified result instead of the first failure.

### Language adapters

| Language | Profiles | Status |
|----------|----------|--------|
| Rust | cli, terminal-game-crossterm, gui-macroquad, web-axum, library | Full |
| Python | minimal | Markers + toolchain only |
| JavaScript/TypeScript | minimal | Markers + toolchain only |
| Go | minimal | Markers + toolchain only |

**Rust verification recipes:** `check_project` (`cargo check`), `build_release` (`cargo build --release`), `format_check` (`cargo fmt --check`), `lint` (`cargo clippy -- -D warnings`), `test` (`cargo test`)

---

## Grounding Contract

Research tasks (anything involving web search or HTTP fetch) are subject to a grounding contract enforced at multiple layers:

1. **Plan-level**: the planner is instructed to decompose research into: `parallel_search` → `http_fetch` → `facts` extraction step (JSON) → `requires_facts` answer steps. Steps that need specific facts must set `requires_facts: true`.

2. **Executor-level**: for `requires_facts` steps, the GROUNDING_GUARDRAIL is appended to the user message. The model must cite the `facts` artifact for every specific claim. If a fact is not present in research data, it must write `[not in research data]`.

3. **Critic-level**: `RESEARCH_CRITIC_PROMPT` checks whether the final answer contains claims not supported by the `facts` artifact. Hallucinated version numbers, dates, or specifics trigger a fail.

4. **Search query decomposition** (`grounding.rs`): `decompose_search_queries()` recognizes ~40 languages/tools/frameworks and generates targeted queries. Year injection uses `chrono::Local::now()` — never hardcoded.

---

## Streaming & UI

All long-running operations use SSE (`POST /run/stream`). The client receives a stream of typed events:

| SSE event | Payload | Description |
|-----------|---------|-------------|
| `token` | `{ text }` | Streaming model output token |
| `thinking_token` | `{ text }` | Streaming thinking/CoT token (when model separates CoT) |
| `plan` | Full plan JSON | Planner output — rendered in reasoning panel |
| `tool_call` | `{ tool, params }` | Tool about to execute |
| `tool_done` | `{ tool, success, output }` | Tool result |
| `critic_result` | `{ pass, score, issues }` | Critic verdict |
| `repair` | `{ reason }` | Repair triggered |
| `replan` | `{ reason }` | Replan triggered |
| `file_written` | `{ path, size }` | File written to workspace |
| `project_card` | `{ language, profile, status, files_written, build_passed, package_path }` | Coding task status |
| `needs_approval` | `{ task_id, plan_summary, risk_score }` | Human gate (high-risk tasks) |
| `status` | `{ phase }` | Phase label (e.g. `coding_rust`) |
| `done` | `{ answer }` | Task complete |
| `error` | `{ message }` | Fatal error |

**UI panels:**
- **Chat** — streaming conversation with SSE rendering
- **Reasoning** — plan steps, tool calls, critic verdicts, ProjectCard, thinking tokens
- **Files** — workspace browser/editor with upload/download
- **Git** — workspace git status, log, diff, commit
- **Mem** — episodic memory browser (past tasks)
- **KB** — knowledge base CRUD

**Task control:**
- `GET /task/:id/status` — poll for completion
- `POST /task/:id/approve` — approve high-risk task
- `POST /task/:id/reject` — reject high-risk task
- `POST /task/:id/cancel` — cancel running task

**Admin mode toggle** (in UI): when enabled, skips the human approval gate for high-risk tasks. Stored in `SystemState.admin_mode`.

---

## API Reference

### Core

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/run` | Blocking run; returns `{ task_id, answer, artifacts }` |
| `POST` | `/run/stream` | SSE streaming run |
| `GET` | `/task/:id/status` | Poll task status |
| `POST` | `/task/:id/approve` | Approve high-risk task |
| `POST` | `/task/:id/reject` | Reject high-risk task |
| `POST` | `/task/:id/cancel` | Cancel in-progress task |

**Request body (`/run` and `/run/stream`):**

```json
{
  "request": "Your question or task",
  "session_id": "optional-uuid",
  "admin_mode": false,
  "planning_answers": {}
}
```

### Workspace

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/workspace/browse` | List directory |
| `GET` | `/workspace/read?path=...` | Read file content |
| `POST` | `/workspace/write` | Write file |
| `DELETE` | `/workspace/delete?path=...` | Delete file |
| `GET` | `/workspace/download?path=...` | Download file |
| `POST` | `/workspace/upload` | Upload file (multipart) |
| `GET` | `/workspace/search?q=...` | Grep workspace |

### Git

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/git/status` | `git status` |
| `GET` | `/git/log` | `git log` |
| `GET` | `/git/diff?path=...` | `git diff` |
| `POST` | `/git/commit` | `git add` + `git commit` |

### Knowledge Base

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/knowledge` | List all entries |
| `POST` | `/knowledge` | Create entry |
| `GET` | `/knowledge/:id` | Get single entry |
| `PUT` | `/knowledge/:id` | Update entry |
| `DELETE` | `/knowledge/:id` | Delete entry |
| `GET` | `/knowledge/:id/download` | Download as Markdown |
| `GET` | `/knowledge/export` | Export all as Markdown book |

### Sessions & Memory

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/sessions` | List recent sessions |
| `GET` | `/sessions/:id` | Get session turns |
| `DELETE` | `/sessions/:id` | Delete session |

### Observability

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/health` | Liveness check |
| `GET` | `/health/deep` | Deep health (runs quick eval; 200 or 503) |
| `GET` | `/metrics` | Runtime metrics (tool calls, latency, LLM calls) |
| `GET` | `/eval/cases` | List eval cases |
| `POST` | `/eval/run` | Run eval suite: `{"mode":"quick"}` / `{"mode":"full"}` |

### Settings

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/settings` | Get current config |
| `POST` | `/settings` | Update config (live; persisted to `config.json`) |

---

## Configuration

`config.json` is created at first startup in `DATA_DIR`. All fields can be updated live via `POST /settings` without a restart.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `workspace_path` | string | `./workspace` | Root directory for all filesystem and shell tool operations |
| `ollama_url` | string | `http://localhost:11434` | Ollama API base URL |
| `fast_model` | string | `qwen3:14b` | Model for planner, executor, repair |
| `critic_model` | string | `qwen3:14b` | Model for critic passes |
| `max_steps` | int | `20` | Hard cap on orchestrator steps per task (1–50) |
| `risk_gate_threshold` | int | `8` | Risk score (0–10) at which human approval gate fires |
| `search_url` | string | `""` | SearXNG base URL; auto-detects local instance if empty |
| `auto_kb_mode` | string | `"research"` | KB auto-save mode: `"off"` / `"research"` / `"always"` |
| `auto_kb_min_chars` | int | `1800` | Minimum answer length to auto-save to KB |
| `num_gpu` | int | `999` | GPU layers offloaded to VRAM (999 = all; 0 = CPU only) |
| `num_ctx` | int | `8192` | Context window in tokens |
| `num_predict` | int | `2048` | Max tokens generated per LLM call |
| `num_batch` | int | `1024` | Token batch size for prompt evaluation |
| `num_thread` | int | `0` | CPU inference threads (0 = Ollama auto) |

**Environment variable overrides** (read at startup, override config defaults):

| Variable | Description |
|----------|-------------|
| `PORT` | HTTP port (default `3000`) |
| `DATA_DIR` | Directory for `config.json` + SQLite databases |
| `WORKSPACE` | Workspace root (overrides `workspace_path` in config) |
| `OLLAMA_URL` | Ollama API URL |
| `SEARCH_URL` | SearXNG base URL |

---

## Installation — Native

**Prerequisites:**
- Rust stable toolchain (edition 2021)
- Ollama running locally with at least one model pulled (`ollama pull qwen3:14b`)
- Optional: SearXNG for reliable private search

```bash
git clone https://github.com/yourusername/aihomeserver
cd aihomeserver
cargo build --release
```

**Run:**

```bash
# Recommended: set DATA_DIR so databases land in a predictable location
DATA_DIR=./data WORKSPACE=./workspace cargo run --release
```

**PowerShell (Windows):**

```powershell
$env:DATA_DIR = ".\data"
$env:WORKSPACE = ".\workspace"
$env:OLLAMA_URL = "http://localhost:11434"
cargo run --release
```

Open `http://localhost:3000` in your browser.

---

## Installation — Docker + SearXNG

The included `docker-compose.yml` runs `aihomeserver` and a private `searxng` instance on the same Docker network.

```bash
docker compose up --build
```

- `aihomeserver` listens on `http://localhost:3000`
- `searxng` is only accessible to the agent container (not exposed to the host network)
- Ollama is reached via `http://host.docker.internal:11434` (Docker host networking)
- Workspace files are persisted to a Docker volume
- On Windows, the desktop launcher can provision a Hyper-V worker VM and point the coordinator at it automatically.
- If you want the Docker worker instead of Hyper-V, start the worker profile explicitly:

```bash
docker compose --profile worker up --build
```

**NVIDIA GPU support:**

The compose file uses `runtime: nvidia` for the agent container. This requires `nvidia-container-toolkit` to be installed on the host.

**Dev / build environment:**

```bash
# Mounts repo to /workspace so the agent can build/test its own codebase
docker compose -f docker-compose.dev.yml up --build
```

---

## SearXNG Integration

SearXNG provides reliable, private, multi-engine search results as JSON — significantly more dependable than scraping public search pages.

**Auto-detection:** if `SEARCH_URL` is not set, `aihomeserver` tries common local SearXNG URLs at startup. If none respond, it falls back to DuckDuckGo HTML scraping (less reliable, rate-limited).

**Recommended:** always set `SEARCH_URL` to your SearXNG instance.

**Docker (recommended):**
```yaml
SEARCH_URL: http://searxng:8080
```

**Native (if running SearXNG locally):**
```bash
SEARCH_URL=http://localhost:8888
```

SearXNG configuration files are in `searxng/`. The `settings.yml` enables the JSON output format required by `aihomeserver`.

---

## Known Issues & Limitations

### Agent behavior

- **Browser finalization is still imperfect:** a browser task may execute correctly but still surface `unverified_browser_output` if the finalization layer does not accept the produced artifact as verified. This is an agent-layer bug, not necessarily a worker runtime failure.

- **Stale browser summary contamination:** a successful normal-page run can still inherit a stale blocked-site conclusion from earlier browser attempts if finalization chooses the wrong artifact history.

- **Workspace mirror behavior is still too destructive in some flows:** the VM-first sync path can replace more workspace state than intended, which is why some task-created files have appeared to disappear between runs.

- **Repair loop search regression:** when `CodingRepairTarget::RunPackage` fires (zip step failed), the model sometimes regenerates a `parallel_search` step instead of a `zip_dir` call. The typed repair directive is injected but overridden by strong planner prior. A deterministic JSON generation path for zip repair is planned.

- **Hallucination through repair cycles:** even with RESEARCH_CRITIC catching hallucinated specifics, the model may substitute different hallucinated facts in cycle 2+. A planned fix: inject the raw `facts` artifact verbatim into the repair prompt on cycle ≥ 2 so the model cannot generate fresh hallucinations.

- **Knowledge base bypass for coding tasks:** the planner previously answered coding requests from a cached KB entry instead of executing the code generation pipeline. Fixed with "Do NOT answer from knowledge base" injection for coding tasks, but the KB is still checked at intake and could influence planning in subtle ways.

- **Critic step scope:** FAST_CRITIC was designed to evaluate one step at a time, not the full plan. Prompt engineering is imperfect; occasionally the critic evaluates completion criteria that apply to later steps, causing false negatives on early steps.

- **`<think>` tag handling:** qwen3 sometimes embeds chain-of-thought in the content field without wrapper tags. `strip_think_tags()` is applied defensively to all model output, but content-field CoT that lacks tags may still reach the user in edge cases.

### Infrastructure

- **No authentication:** there is currently no API key, session token, or auth layer. The server must only be run on `localhost` or a trusted LAN. Do not expose to the public internet.

- **Shell tool is OS-coupled:** the shell tool runs in the same OS environment as the server. On Windows this means PowerShell; in Docker Linux this means `sh`. Cross-OS plans (e.g. a plan generated against a Windows context but executed in a Linux container) can produce wrong shell syntax. A syntax guard rejects obvious mismatches, but this does not cover all cases.

- **Semantic memory is keyword-based, not embedding-based:** similarity retrieval uses token overlap, not vector embeddings. This means semantically related tasks with different vocabulary may not be retrieved as few-shot examples. True embedding-based retrieval is deferred.

- **Context window pressure:** research tasks with multiple large HTTP fetches can fill the context window. `truncate_artifacts()` caps individual fetch artifacts at 200 characters and total artifact payload at ~4K characters, but this means detailed content may be truncated before reaching the LLM.

- **Single-binary process model:** there is no worker queue, task priority, or request isolation. Concurrent requests share the same Ollama connection and tool registry. Under load, long tasks will queue behind each other.

- **Stop/cancel UX is weaker than it should be:** `/task/:id/cancel` exists and the orchestrator has cancellation tokens, but the UI can still lag or feel sticky during long browser or shell tasks.

- **Browser runtime persistence inside the VM is still operationally fragile:** Playwright and Chromium can work correctly once installed, but the bootstrap path has not yet made that setup as durable and automatic as it should be.

### Coder pipeline (Phase 1 known gaps)

- **Coding task execution reliability is model-sensitive:** the plan contract injection and adapter manifest give the model strong guidance, but output format conformance varies. Models that do not follow JSON step schemas precisely will produce plans that fail at executor parse time.

- **Phase 2 adapters missing:** Java, C#, C++, and Lua have no adapters. Non-Rust projects will use generic (unguided) plan generation.

- **No toolchain detection node:** toolchain presence is checked via shell commands in the plan. A dedicated `ToolchainDetector` node that runs before planning (so the planner knows what's available) is planned for Phase 2.

---

## In Progress

| Item | Status |
|------|--------|
| Phase 2 language adapters (Java, C#, C++, Lua) | Planned |
| ToolchainDetector node | Planned |
| Deterministic zip_dir repair path (bypass LLM for RunPackage repair) | Planned |
| Facts injection into repair cycle ≥ 2 (hallucination suppression) | Planned |
| Embedding-based semantic memory | Planned |
| Eval suite for Rust Snake + multi-language coding | Planned |
| Profile resolver UI (adapter profiles selectable before task start) | Planned |
| Library intelligence layer (per-library docs grounding) | Planned |
| Authentication / API key layer | Planned |
| Phone / mobile client | Long-term |
| Multi-worker / task queue | Long-term |

---

## Security Notice

`aihomeserver` has **no authentication or authorization layer**. It exposes:

- A shell tool that can execute arbitrary commands in the workspace
- A filesystem tool that can read and write any file within the workspace root
- A git tool that can commit and push

**Run this server on `localhost` or a private LAN only. Never expose it to the public internet.**

High-risk tasks (risk score ≥ threshold, default 8) trigger a human approval gate before execution. This is a safety net, not a security boundary.

---

## Repository Layout

```
aihomeserver/
├── desktop/                Electron launcher, packaging scripts, Windows app resources
├── src/                    Rust source (see Module Reference above)
├── docs/                   Design documents, worklogs, specs
├── scripts/                Helper scripts (eval.ps1, metrics.ps1)
├── searxng/                SearXNG configuration (settings.yml, limiter.toml)
├── Cargo.toml              Dependencies
├── Cargo.lock
├── docker-compose.yml      Production compose (aihomeserver + searxng)
├── docker-compose.dev.yml  Dev compose (with repo bind-mount)
├── Dockerfile              Multi-stage Rust build
└── README.md               This file
```

**What to commit:**
- `src/`, `Cargo.toml`, `Cargo.lock`
- `docker-compose*.yml`, `Dockerfile*`
- `searxng/settings.yml`, `searxng/limiter.toml`
- `docs/`, `scripts/`

**What not to commit:**
- `target/` (Rust build artifacts)
- `workspace/` (user files and agent-generated output)
- `*.db`, `*.db-wal`, `*.db-shm` (SQLite databases)
- `config.json` (local settings)
- `.claude/`, `.env` (local tooling)

## Desktop Launcher Notes

The desktop app is now part of the real product surface, not just a thin wrapper.

Important behaviors:

- it can launch or reconnect to the Hyper-V worker VM
- it sets coordinator `WORKER_URL` to a host-routable path for the container
- it exposes desktop controls and a fast runtime rebuild path
- packaging now verifies embedded resources so stale worker/coordinator code does not silently ship

The packaging verification script is:

- `desktop/scripts/verify-packaged-resources.js`

This checks that the packaged app includes the current worker/coordinator surfaces instead of shipping an outdated embedded repo snapshot.

---

*Built with Rust · Ollama · Axum · SQLite · SearXNG*  
*Ember Tech Solutions LLC — Kyle Barrett*

