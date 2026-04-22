# aihomeserver — Honest Assessment + Highest-Impact Next Steps (2026-04-22)

This is a candid review of where the project is strong today, where it’s fragile, and what to build next (prioritized).

---

## What’s already strong

- **Clear architecture**: the Planner → Executor → ToolExecution → Critic → Repair/Replan loop is a solid “agent core” with a clean tool contract.
- **Tool contract + artifacts**: consistent `ToolResult` + artifacts as a shared “working memory” is the right pattern.
- **UI direction**: having Files/Git/Mem/KB tabs + SSE eventing is already ahead of many hobby agents.
- **Evals + health**: `/eval/run`, `/health/deep`, and `/metrics` give you measurable reliability and faster debugging.
- **Grounding direction**: the “facts step + requires_facts” gating is exactly the kind of constraint that prevents “confident garbage”.
- **Pragmatic docker**: adding a dev/test mode is the right response to “my runtime doesn’t have cargo/rg”.

---

## Biggest issues right now (highest impact)

### 1) Reliability: regression baselines + CI gate
UPDATE (2026-04-22): you now have an eval runner + deep health + metrics, and preflight is integrated into every task run. The remaining P0 work is turning that into a regression gate (baseline + CI) and adding a few auto-policy hooks so the agent adapts its plan when capabilities are missing (e.g., search not configured).
Right now, it’s hard to *prove* the system got better. You need a repeatable suite of tasks (web research, code edits, file ops, shell ops) with pass/fail metrics so improvements don’t accidentally break things.

**Impact:** without evals, you’ll chase failures forever and won’t know which prompt/tool change caused what.

### 2) Safety/security model is still loose
You have shell + filesystem write/delete + network tools. That’s powerful, but you need guardrails:

- explicit permissioning / capabilities per tool and per step
- workspace sandbox enforcement (already partially present) + “no deletes outside workspace”
- prompt-injection defenses for web content (“ignore page instructions”, quote-only facts with evidence)
- audit logs of every write/delete + shell command

**Impact:** this is where “FAANG-level” is most visible: safety-by-design, not “please be careful” prompts.

### 3) Tool-call correctness and recovery is improving, but still brittle
You’re now failing fast on invalid tool-call JSON and repairing tool calls, which is great. The remaining fragility is:

- tool schemas aren’t explicit/validated (models still guess keys/types)
- artifacts contain mixed shapes (strings vs structured JSON) depending on step
- repair needs more deterministic “what to fix” signals (per-tool error codes help)

**Impact:** tool-call bugs are the #1 reason agent systems feel “random”.

### 4) Research grounding needs “provenance first-class”
You’ve started this with `facts` + evidence snippets, but the system should make provenance unavoidable:

- every “fact” is `{ value, url, snippet, confidence }`
- the code-generation step references fact IDs, not raw prose
- block generation if facts are missing/low-confidence

**Impact:** this turns “research” into something inspectable and trustworthy.

### 5) UX: editing is good, but not yet “workspace IDE”
You already have a file explorer + editor + upload/download. The gap to “VS Code feel” is:

- multi-tab editor
- syntax highlighting + formatting
- quick-open (Ctrl+P) + search-in-files results linked to editor
- diffs/patch preview before applying AI edits

**Impact:** makes the system feel like a real dev environment, not a chat toy.

---

## Highest-priority roadmap (what I’d build next)

### P0 (must-have): Regression baselines + CI gates
UPDATE (2026-04-22): implemented as `/eval/cases`, `/eval/run`, `/health/deep`, `/metrics`, plus a UI button and helper scripts. Next: baseline snapshots + CI gating.
1. **Eval runner** (CLI or internal endpoint) with 20–50 canned tasks:
   - tool-call JSON validity
   - repo exploration (“find where X is defined”)
   - safe file edits (apply patch; verify output)
   - web research → facts table → code generation (grounding)
2. **Metrics**:
   - tool success rates
   - average repair cycles
   - time to first token / total latency
   - hallucination indicators (facts missing but code produced)
3. **CI gates**:
   - `cargo fmt`, `clippy`, `cargo test`
   - run eval suite; fail PR/build on regressions

### P0 (must-have): Permissions + audit logging
1. Add a **capability model**:
   - tools declare operations (read/write/delete/shell/net)
   - planner sets required capabilities per step
   - executor/tool_execution enforces a policy (configurable)
2. **Audit log** every:
   - file write/delete/rename path + hash before/after
   - shell command + cwd + exit
3. “Safe mode” defaults:
   - no delete without explicit user request (or risk gate)
   - require confirmation for recursive deletes / renames

### P1: Strong schemas for tool calls + artifacts
1. Define explicit JSON schemas per tool (serde structs), e.g.:
   - `ShellParams { command, cwd, timeout_secs }`
   - `FsFindParams { path, pattern, max_depth, ... }`
2. Validate tool call JSON before execution:
   - parse into typed struct
   - return a structured error (`invalid_params`) if parsing fails
3. Normalize artifacts:
   - tool call artifacts stored as JSON objects
   - tool results stored under `*_result` with consistent keys

### P1: Better “research mode”
1. Add a dedicated **research extractor** step type:
   - emits structured facts
   - enforces URL + snippet per fact
2. Add web prompt-injection hardening:
   - strip/ignore “instructions” in fetched pages
   - prefer official sources; downrank Reddit unless asked

### P2: IDE-grade workspace UX
1. Replace `<textarea>` editor with **Monaco** or **CodeMirror 6**:
   - syntax highlighting, bracket matching, search
2. Multi-file tabs + dirty indicators
3. Diff viewer before “Apply AI change”
4. (Optional) LSP integration:
   - rust-analyzer, tsserver, pyright

### P2: Code generation quality improvements
1. “Edit strategy” improvements:
   - always read target files first
   - propose a patch; apply via filesystem tool
2. Build/test step selection:
   - detect toolchain availability
   - choose correct commands (`cargo test`, `npm test`, etc.)

---

## Quick wins (low effort, high value)

- Add `.gitignore` entries for `target/` and database files if you plan to commit.
- Add a “copy artifact” + “download artifact JSON” button in UI (debugging is faster).
- Make upload choose folder without `prompt()` (small modal).
- Add “Open in editor” from search results (jump to file + line).

---

## My honest take

You’ve built something with the right *shape*: a real agent loop, a UI, and tool plumbing. The core risk is that without:

1) **evals**, and  
2) a **permissions/provenance model**,

it will keep feeling inconsistent and occasionally untrustworthy (especially around research + code generation). If you nail evals + permissions + provenance, the rest (IDE polish, LSP, nicer UI) becomes straightforward and you’ll have a system that feels legitimately “senior-engineered”.
