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

`aihomeserver` is a local-first AI agent server that runs entirely on your own hardware. There is no cloud dependency beyond the model provider (Ollama, which itself runs locally). It is written in Rust for correctness, determinism, and performance on constrained home hardware.

**What it is not:** a thin wrapper around a chat API. Every request runs through a structured orchestration loop that plans, executes tools, verifies results mechanically, critiques with a second LLM pass, and repairs or replans on failure — all with full auditability through a persistent artifact log.

**Primary use cases today:**
- Research-backed Q&A with grounding enforcement (no hallucinated citations)
- Coding project generation with artifact verification (file existence, build checks, zip packaging)
- Workspace file management (read, write, search, diff, zip)
- Git operations within the workspace
- Curated knowledge base that grows over time

**Hardware it runs on (reference):**
- NVIDIA RTX 5070 Ti (16 GB VRAM), 64 GB RAM
- Models: `qwen3:14b` (fast), configurable critic model
- OS: Windows with WSL / Docker for Linux containers

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────────┐
│                        aihomeserver process                             │
│                                                                         │
│   Axum HTTP Server                                                      │
│   ├── POST /run (blocking)                                              │
│   ├── POST /run/stream (SSE)              ┌──────────────────────────┐  │
│   ├── REST workspace / git / KB / evals  │      Orchestrator        │  │
│   └── Embedded Web UI                    │  (deterministic FSM)     │  │
│                                          │                          │  │
│   Tool Registry ─────────────────────────│──► ToolExecution node    │  │
│   Memory (SQLite) ───────────────────────│──► Intake / Finalization │  │
│   OllamaClient ──────────────────────────│──► Planner/Executor/     │  │
│   Coder Pipeline ────────────────────────│    Critic/Repair nodes   │  │
│                                          └──────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────────┘
         │                              │
         ▼                              ▼
   Ollama (local)                 SQLite DBs
   (qwen3:14b or                  (episodic, conversation,
    any Ollama model)              semantic, knowledge, sources)
```

**Three-layer design:**

| Layer | Responsibility |
|-------|----------------|
| **API layer** (`src/api/`) | HTTP routing, SSE fan-out, embedded UI HTML/CSS/JS, approval gate endpoints |
| **Orchestration layer** (`src/nodes/`, `src/orchestrator.rs`) | FSM node execution, risk routing, repair/replan bounds, coder pipeline |
| **Tool & memory layer** (`src/tools/`, `src/memory/`) | All side effects (file I/O, shell, git, HTTP, search) and persistence |

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
│   │                        /knowledge/*, /eval/*, /health/*, /metrics, /settings, /sessions/*
│   ├── ui.rs                Embedded chat UI HTML/CSS/JS (CHAT_HTML const); SSE event rendering
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
│   │                        strip_think_tags() applied to all model output
│   ├── tool_execution.rs    Dispatches tool calls from executor to ToolRegistry;
│   │                        stores ToolResult as artifact; emits ToolCall/ToolDone SSE
│   ├── critic.rs            Risk-gated critic; RESEARCH_CRITIC for grounded steps;
│   │                        deterministic manifest check for coding tasks;
│   │                        truncate_artifacts() prevents context overflow
│   ├── repair.rs            Generates targeted fix prompt; CodingRepairTarget (WriteFile,
│   │                        PatchSource, RunPackage, General) for coding tasks
│   └── finalization.rs      Persists to episodic/conversation memory; auto-KB save;
│                            emits final answer; source URL collection
│
├── tools/
│   ├── mod.rs               Tool trait; ToolRegistry (register/alias/execute/list)
│   ├── filesystem.rs        File I/O: read, write, list, find, grep, delete, move,
│   │                        mkdir, copy, diff, stat, tree, zip_dir
│   ├── shell.rs             Safe shell execution; OS-aware (sh on Linux, PowerShell on Windows);
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
└── coder/
    ├── mod.rs               Re-exports; pub fn verify()
    ├── intent.rs            CodingIntent struct; detect_coding_intent() keyword heuristic
    ├── adapter.rs           LanguageAdapter trait; AdapterManifest; get_adapter(); adapter_for_intent()
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
| `shell` | `run_command`, `bash`, `exec` | Execute shell commands in the workspace; OS-aware; timeout-guarded |
| `git` | `git_tool` | git status, log, diff, add, commit, branch, checkout, clone, pull, push |
| `web_search` | `search`, `duckduckgo_search` | Single-query search via SearXNG JSON or DDG HTML fallback |
| `parallel_search` | `multi_search` | Multi-query parallel search with deduplication |
| `http_fetch` | `fetch`, `curl` | HTTP GET/POST; HTML-to-text extraction; JSON passthrough |
| `save_knowledge` | `kb_save`, `knowledge_save` | Persist curated content to the knowledge base |

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

## Coder Pipeline (Phase 1)

Phase 1 adds typed contracts for coding tasks so the agent can be mechanically verified rather than relying solely on LLM self-assessment.

### How it works

1. **CodingClassifier node** (between Intake and Planner): calls `detect_coding_intent()` — a pure keyword heuristic (no LLM). Detects intents: `new_project`, `modify_existing`, `debug_existing`, `add_feature`, `package_existing`. Sets `state.coding_intent`; stores adapter manifest as artifact.

2. **Planner injection**: when `coding_intent` is set, the planner receives a CODING PLAN CONTRACT enforcing: workspace inspection first → manifest step (output_key=`coding_execution_manifest`) → one file write step per file → verification steps using adapter recipes → zip_dir if packaging required → answer step.

3. **ExecutionManifest**: produced as a JSON LLM-only step. Contains: `project_root`, `required_files`, `write_plan`, `verification_plan` (ordered recipe names), `expected_artifacts` (including zip path). Auto-loaded from `coding_execution_manifest` artifact after every ToolExecution.

4. **ArtifactVerifier** (after ToolExecution, when manifest is set): mechanical filesystem check — no LLM. Checks required files exist with size > 0; checks zip entries include expected source files and exclude `target/`; reads `*_result` artifacts for shell exit codes. Returns `ArtifactVerification { status, missing_files, missing_artifacts, build_passed, package_verified }`.

5. **Critic integration**: if `artifact_verification` is present with missing files or build failure, the critic produces a deterministic fail with `confidence=1.0` — no LLM call needed.

6. **Targeted repair**: `coding_repair_target()` reads `ArtifactVerification` and returns `WriteFile(path)`, `PatchSource(root)`, `RunPackage(zip_path)`, or `General` — injected into the repair prompt so the model generates a targeted fix rather than a generic retry.

7. **ProjectCard SSE event**: emitted after each verification pass, rendered as a status card in the UI showing language, profile, files written, build status, and a download link for the zip.

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

---

*Built with Rust · Ollama · Axum · SQLite · SearXNG*  
*Ember Tech Solutions LLC — Kyle Barrett*

