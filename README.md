# aihomeserver

Local-first Rust “agent server” with a web UI, tool access (filesystem/shell/git/web), memory, evals, and a self-repair loop (Planner → Executor → ToolExecution → Critic → Repair/Replan).

This is designed to be:
- **FAANG-level infrastructure** (evals, metrics, safe defaults, reproducible behavior)
- **a practical home AI** (files/code workflows, local search, knowledge base, future phone clients)

## Security note

This server currently has powerful tools and **no authentication**. Run it on `localhost` / your LAN only. Do not expose it to the public internet.

---

## What it can do (today)

- **Agent loop**: plans tasks, executes steps, uses tools, critiques results, repairs/replans on failure.
- **Grounded research mode**: patch/version/meta requests must search/fetch and produce a `facts` table; code generation is blocked without evidence.
- **Web UI**: Chat + Files + Git + Mem + KB, streaming tokens, tool logs, approvals.
- **Workspace IDE-lite**: browse/edit files, upload/download, search in workspace.
- **Evals & health**:
  - `POST /eval/run` (quick/full suites)
  - `GET /health/deep` (quick eval, returns 200/503)
  - `GET /metrics` (runtime counters + latency aggregates)
- **Preflight per run**: quick eval runs at the start of every task; a compact capability snapshot is injected into planning/execution.

---

## Quickstart (native)

Prereqs:
- Rust toolchain (stable, edition 2021)
- Ollama running locally (or reachable)
- Optional: SearXNG (recommended) for reliable search

Run:
```bash
cargo run --release
```

Open:
- `http://localhost:3000`
- `http://localhost:3000/learn` (interview-prep walkthrough + read-only repo browser)

Recommended (keeps your repo clean):
- Set `DATA_DIR=./data` so `config.json` + DBs don’t land in random build output folders.
  - See `.env.example` for a good starting point.

PowerShell example:
```powershell
$env:DATA_DIR = ".\\data"
$env:WORKSPACE = ".\\workspace"
$env:OLLAMA_URL = "http://localhost:11434"
cargo run --release
```

---

## Quickstart (Docker + SearXNG)

This repo includes a `docker-compose.yml` that runs:
- `aihomeserver` (the agent server)
- `searxng` (private search, only on the docker network)

```bash
docker compose up --build
```

Then open:
- `http://localhost:3000`

Notes:
- Compose defaults `OLLAMA_URL` to `http://host.docker.internal:11434` so the container can reach Ollama on the host.
- `WORKSPACE` inside the container is `/data/workspace` (a Docker volume).

---

## Dev/Test Docker (cargo + rg available)

If you want the AI to be able to build/test the repo from inside its own runtime, use:
```bash
docker compose -f docker-compose.dev.yml up --build
```

This bind-mounts your repo to `/workspace` in the container and runs `cargo run --release`.

---

## Configuration

Configuration is persisted to `config.json` in `DATA_DIR` (default: next to the binary; in Docker: `/data`).

Environment variables override config at startup:
- `PORT` (default `3000`)
- `DATA_DIR` (default: binary directory)
- `WORKSPACE` (filesystem/shell tool root)
- `OLLAMA_URL` (default `http://localhost:11434`)
- `SEARCH_URL` (optional; recommended: SearXNG base URL like `http://searxng:8080` in Docker or `http://localhost:8888` on host)

If `SEARCH_URL` is not set, aihomeserver will try to auto-detect a local SearXNG instance on common URLs.

You can also update settings live via `POST /settings` in the UI.

---

## Knowledge Base (KB)

- The AI can persist researched material into the KB via the `save_knowledge` tool.
- KB entries can be downloaded:
  - `GET /knowledge/:id/download` (single entry as Markdown)
  - `GET /knowledge/export` (all entries as one Markdown “book”)

For “textbook/curriculum” requests (e.g. “learn Rust in depth”), the planner will generate a multi-chapter book and save chapters into the KB in one pass.

---

## Evals, health, and metrics

- List eval cases:
  - `GET /eval/cases`
- Run eval suite:
  - `POST /eval/run` with JSON:
    - `{"mode":"quick"}`
    - `{"mode":"full"}`
    - `{"cases":["workspace.exists","tool.shell.echo"]}`
- Deep health:
  - `GET /health/deep`
- Metrics snapshot:
  - `GET /metrics`

Helper scripts:
- `scripts/eval.ps1 quick`
- `scripts/eval.ps1 full`
- `scripts/metrics.ps1`

---

## Tooling model (important)

The `shell` tool runs **in the same OS/environment as the server process**:
- If the server runs in Docker Linux, shell commands run in that container (`sh -lc`)
- If the server runs on Windows, shell commands run in PowerShell

This is why cross-OS command syntax mismatches can happen. The system includes a syntax-guard that rejects obvious PowerShell commands on `sh` (and vice versa) to force replanning.

---

## Repo layout (high level)

- `src/`
  - `api/` HTTP routes + UI HTML
  - `nodes/` planner/executor/critic/repair orchestration nodes
  - `tools/` filesystem/shell/git/http_fetch/search tools
  - `memory/` episodic + semantic + knowledge storage
  - `metrics.rs` in-memory counters/latency snapshots
  - `grounding.rs` grounding contract enforcement (facts gating)
- `docker-compose.yml` runtime compose + SearXNG
- `docker-compose.dev.yml` dev/test compose
- `searxng/` SearXNG configuration

---

## .gitignore guidance (what to commit vs not)

This repo should generally **commit**:
- `src/`, `Cargo.toml`, `Cargo.lock`
- `docker-compose*.yml`, `Dockerfile*`
- `searxng/settings.yml`, `searxng/limiter.toml`
- docs like `DESIGN.md`, `PROGRESS_*.md`, `NEXT_STEPS_*.md`

This repo should generally **not commit**:
- `target/` (Rust builds)
- `workspace/` (your private files / generated output)
- local databases (`episodic.db`, `knowledge.db`, `*.db-wal`, etc.)
- `config.json` (local settings)
- tool-specific dirs like `.claude/`, `.claire/`, `.sixth/`

See `.gitignore` in the repo root.

If you previously committed local tool folders (like `.claude/` / `.claire/`) or databases, remove them from git history/index and rely on `.gitignore` going forward (example: `git rm -r --cached .claude .claire workspace`).
