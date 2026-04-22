# aihomeserver — Features + UI Review and Roadmap (2026-04-22)

This doc is a product-facing view of what you have today, what’s missing, and what to build next to make it feel “FAANG-level” while still being a practical home AI.

---

## Current product surface (what exists now)

### Chat + agent loop UX
- Streaming assistant output (plus separate “thinking” stream when supported).
- Planner → Executor → ToolExecution → Critic → Repair/Replan loop with UI events (plan, tool calls, terminal output, repairs, approvals).
- High-risk approval modal (approve/reject) for dangerous actions.
- Planning questionnaire overlay (pre-run constraints).

### Files / Workspace
- Sidebar “Files” tab with:
  - workspace tree browser
  - file open/edit/save
  - mkdir/rename/delete
  - upload files (`POST /workspace/upload`)
  - download files (`GET /workspace/download`)
- Workspace search endpoint (`GET /workspace/search`) and overlay UI.
- Diagnostics button (runs evals).

### Git panel
- Basic repo visibility: status/log/diff/stage/commit endpoints and UI panel.

### Memory + Knowledge
- Episodic memory browser (“Mem” tab).
- Knowledge base CRUD (“KB” tab) with add/edit modal.

### Settings / ops
- Settings endpoints to update runtime configuration (models, workspace, etc.).
- Deep health + eval runner:
  - `POST /eval/run`, `GET /eval/cases`, `GET /health/deep`
- Runtime metrics:
  - `GET /metrics` (task/tool/eval counters + latency aggregates)
- Automatic **preflight** per task run (capability snapshot injected into planner/executor).

---

## UX gaps (what will make it feel “real”)

### P0 — must-have (quality + trust)
- Replace “eval results in alert()” with a **Diagnostics panel**:
  - show quick vs full suites, per-case details, timestamps, and “copy JSON”
  - show “capabilities” snapshot for the current run
- Add an **Artifacts / Debug panel**:
  - browse artifacts keys/values, download as JSON, and mark key artifacts as “pinned”
- Make the run timeline explicit:
  - “Step 3/12 (tool: http_fetch)” with durations
  - “retry #2” and “replan #1” visible without scrolling raw logs
- “Trust UX” for grounded research:
  - show the extracted `facts` table in a dedicated viewer (value/url/snippet)
  - block/label any output that is not backed by facts (when the contract applies)

### P0 — must-have (workspace/dev UX)
- Multi-file editing:
  - tabs, dirty indicators, and “save all”
- Diff-first edits:
  - show a patch/diff preview before applying AI changes
  - one-click revert (based on audit checkpoints)
- Better search experience:
  - search results click-to-open at file + line
  - “search in files” results grouped by file

---

## Roadmap (features to add)

### P1 — IDE-grade workspace
- Replace `<textarea>` editor with Monaco or CodeMirror 6:
  - syntax highlighting, bracket matching, inline search
  - optional formatting hooks (Rustfmt / Prettier)
- LSP integration (later, optional):
  - rust-analyzer first; then tsserver/pyright

### P1 — multi-user + “home AI” readiness
- Authentication + accounts:
  - per-user sessions, memory isolation, admin controls
- Profiles/policies:
  - “kid mode” content restrictions and tool limits
  - per-user web access toggles (“web off by default” for kids)
- Mobile-first UI pass:
  - responsive layout, larger hit targets, less hover-dependence

### P1 — safety + auditability (UI-visible)
- Audit log viewer:
  - file writes/renames/deletes with before/after hash and “open diff”
  - shell commands + cwd + exit code
- Safer destructive flows:
  - confirm deletes/renames
  - protect “outside workspace” operations (hard deny + clear UI message)

### P2 — “features out the wazoo” (optional but high value)
- Voice:
  - STT/TTS, “hands free” mode, push-to-talk on phone
- Attachments everywhere:
  - drag/drop into chat (files, images) with automatic placement into workspace
- Knowledge UX:
  - cite sources, staleness indicators, versioning, “refresh this entry” button

---

## What “FAANG-level” looks like in the UI

Not visual polish; it’s *predictability*:
- Every run has: preflight → plan → step-by-step execution → visible artifacts → reproducible eval results.
- When something fails, the UI shows:
  - **what** failed (case/step/tool),
  - **why** (error_code + trace),
  - **what it did next** (repair vs replan),
  - and **how to fix** (actionable recommendation).

---

## Recommended next implementation (if we keep momentum)

1) Diagnostics panel (replace alert) + “download eval JSON”
2) Artifacts viewer + “download artifacts JSON”
3) Search results → open file at line
4) Diff-first apply flow for AI edits

