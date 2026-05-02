# aihomeserver — Worklog (Codex assist) — 2026-05-02

This file summarizes the concrete code + UX changes made while debugging “research + coding + memory/KB + UI” behaviors in `aihomeserver`, plus what’s still missing to reach the “hardcore researcher + software engineer” target.

> Note: `git status` in this repo currently shows many modified files beyond the ones listed below. This worklog focuses on changes directly made to address the issues raised in this thread (stuck runs, weak failure visibility, broken filesystem actions, chat switching/indicators, queuing, workspace visibility, “Task completed.” with no answer, and “latest” queries anchoring on old years).

---

## 1) User-visible outcomes

### Chat UX
- Switch chats while a run is in progress (no longer blocked by `busy`).
- Per-session sidebar badges show run status:
  - `…` running
  - `✓` success
  - `✗` failure
- If you leave a running chat and come back, the UI re-renders the in-progress run state (status / partial stream) instead of visually “wiping”.
- Sending while busy enqueues the message (sequential queue) and shows a `⏳ Queued…` placeholder.

### “Where did it fail?” transparency
- When a run ends with `success=false`, the SSE `done` event now includes compact failure context:
  - which step failed
  - step action
  - tool name (if any)
  - artifact key (e.g. `cargo_toml_written_result`)
  - `error_code`
  - `trace`
- The UI surfaces that failure context in the reasoning trail and uses `✗ failed` labels/badges.

### Workspace/file visibility
- The Files “workspace” tree now auto-refreshes when `file_written` events arrive (debounced), so created/edited files appear while a run is still executing.

### Snake game deliverable
- A working example project was created and packaged:
  - Project: `workspace/snake_rust/`
  - Zip: `workspace/snake_rust.zip` (includes `snake_rust.exe` built from `cargo build --release`)

---

## 2) Root-cause fixes (what was actually wrong)

### A) “Tool unsupported_action: create_dir/create_file”
**Symptom**
- Coding plans repeatedly failed with errors like:
  - `unsupported_action: create_dir`
  - `unsupported_action: create_file`
  - `cargo_toml_written_result (unsupported_action)`

**Cause**
- The model sometimes emits filesystem actions like `create_dir` / `create_file`, but the `filesystem` tool only supported `write/read/list/find/grep/...` and returned `unsupported_action` for the unknown action.

**Fix**
- `src/tools/filesystem.rs` now:
  - maps common synonyms (`create_dir`, `mkdir`, `create_file`, `write_file`, etc.) onto supported actions
  - implements a first-class `mkdir` action (creates directories under the workspace root)

### B) “Task completed.” with no real answer (especially research prompts)
**Symptom**
- A request like “latest Rust release notes” could end with a generic “Task completed.” and no content.

**Cause**
- The executor marked the run as complete simply because it reached the end of the plan, even if the plan never produced the expected `answer` artifact.
- `extract_answer()` then fell back to the placeholder string.

**Fix**
- `src/nodes/executor.rs` now checks that `plan.expected_outputs` exist (especially `answer`) before setting `termination_met = true`.
  - If expected outputs are missing, it logs an `executor_incomplete` event and forces the loop back into Critic/Repair instead of silently completing.

### C) “Latest” queries anchoring on 2024 in searches
**Symptom**
- Search queries for “latest” sometimes included 2024 or otherwise behaved like the model didn’t know the current year.

**Cause**
- Planner prompt/context did not include an explicit “current date/time”, so the model guessed.

**Fix**
- `src/nodes/planner.rs` now injects an explicit current local + UTC timestamp into planner context.

---

## 3) Files changed in this thread (high signal)

### UI
- `src/api/ui.rs`
  - sidebar session badges (`…/✓/✗`)
  - allow chat switching while busy
  - persist/re-render in-progress runs when leaving/returning
  - queued sends while busy (`⏳ Queued…`)
  - file tree auto-refresh on `file_written`
  - show `✗ failed` when `success=false`
  - surface SSE failure context into “reasoning”

### Server streaming (SSE)
- `src/state.rs`
  - added `failure: Option<SseFailureInfo>` to `SseEvent::Done`
  - added `SseFailureInfo` struct (step/tool/artifact/error_code/trace)
- `src/api/server.rs`
  - emits `failure_info` inside `SseEvent::Done` for failed runs
  - helper `build_sse_failure_info()` to extract the failing step/tool/artifact and error details from state

### Executor / planning correctness
- `src/nodes/executor.rs`
  - do not “complete” if `expected_outputs` (e.g. `answer`) are missing; instead mark incomplete and let Critic/Repair run
- `src/nodes/planner.rs`
  - inject `Current date/time` into planner context for “latest” correctness

### Tooling
- `src/tools/filesystem.rs`
  - accept filesystem action synonyms and add `mkdir`

---

## 4) “Learning center” / Learn page (what it is)

The repo includes a learn/walkthrough UI page referenced in README:
- `src/api/learn.html`
- `src/api/learn.rs`
- `src/api/learn_docs.rs`

Purpose (as implemented in repo docs/UI):
- explains the architecture and where to look for things (planner/executor/tools/memory/KB)
- provides a guided “interview prep / system tour” view

If you want this to become a true “learning center” for *you* (not just a system tour), what’s missing is:
- a “KB book” viewer/editor/export UX (chapters, ToC, per-domain shelves)
- clear provenance: which KB entries are sourced from web fetch vs model synthesis
- stable “curriculum” job type that writes a multi-file artifact to workspace + exports to KB

---

## 5) Where we’re still going wrong / missing pieces (to ground what you want)

You want one assistant that can be:
- hardcore researcher
- dumb/life Q&A
- biblically grounded
- adaptable software engineer
- gaming advisor/modder

The main missing pieces are *not* more prompts; they’re **routing + contracts + verification**:

### A) No dedicated “Coder Executor” contract (yet)
Even with filesystem fixes, coding can still drift because the system doesn’t enforce:
- project scaffolding contract (what files must exist)
- build/test/package verification contract (tool proof required)
- deterministic artifact verifier (zip exists, binary exists, tests pass)

Your spec (`CODEX_UNIFIED_CODER_EXECUTOR_FULL_SPEC_REMADE.md`) describes exactly what’s missing:
- intent classifier → language adapter → scaffold/edit → build/test → artifact verify → targeted repair

### B) KB/memory still lacks “textbook-grade” structure + provenance
To become “scrape + learn”:
- KB needs a schema for multi-chapter books (ToC, chapters, tags, sources)
- entries need provenance fields:
  - source URLs
  - fetch timestamps
  - “grounded vs synthesized”
- auto-save needs a policy that avoids writing tiny/partial outputs and prefers verified artifacts.

### C) True parallel runs aren’t implemented (UI queue is sequential)
We implemented a UI queue, but “multiple chats running simultaneously” requires server-side concurrency:
- per-session task runner (separate task IDs + cancellation)
- resource limits (LLM concurrency, tool concurrency)
- UI needs per-session SSE channels or long-poll status hydration

### D) “Biblically sound” requires an explicit policy
This cannot be reliably achieved without a configured profile, e.g.:
- which translation(s) are acceptable
- denominational lens (or “ecumenical, cite multiple”)
- when to cite scripture vs commentary
- whether web fetch is required for claims about doctrine/history

### E) Research mode needs better source diversity + rate-limit strategy
You already observed SearXNG engines returning 403 / suspended time. To make research robust:
- engine rotation / backoff (already partially present)
- cache previous fetches
- prefer stable sources for “official docs” tasks
- persist a “facts table” artifact with per-fact citations (you have this concept; needs consistent enforcement + better UI surfacing)

---

## 6) Recommended next steps (high leverage)

1) Implement the “Coder Executor” path from your spec:
   - intent classifier + coding profile
   - artifact verifier (zip exists, binary exists, commands succeeded)
   - targeted repair keyed to verifier failures

2) Upgrade KB into “books”:
   - chaptered entries + export-to-workspace + download
   - provenance (sources + timestamps) and “grounded vs synthesized” flags

3) Add server-side task queue per session (and later true concurrency):
   - queue API, cancel button, and per-session running indicators

---

## 7) Quick pointers (paths)

- UI + SSE rendering: `src/api/ui.rs`
- Streaming API: `src/api/server.rs` (`POST /run/stream`)
- SSE event types: `src/state.rs`
- Planner prompt/context: `src/nodes/planner.rs`
- Executor completion semantics: `src/nodes/executor.rs`
- Workspace file ops tool: `src/tools/filesystem.rs`

