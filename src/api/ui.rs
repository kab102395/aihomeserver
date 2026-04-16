pub const CHAT_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>aihomeserver</title>
<style>
*, *::before, *::after { box-sizing: border-box; margin: 0; padding: 0; }

:root {
  --bg: #1a1a1a;
  --surface: #222;
  --surface2: #2a2a2a;
  --border: #333;
  --text: #e8e6e3;
  --text-muted: #7a7a7a;
  --text-dim: #4a4a4a;
  --accent: #c97b4b;
  --accent-soft: rgba(201,123,75,0.12);
  --user-bg: #2d2d2d;
  --code-bg: #161616;
  --radius: 8px;
  --sidebar-w: 240px;
}

body {
  font-family: -apple-system, 'Segoe UI', system-ui, sans-serif;
  background: var(--bg);
  color: var(--text);
  height: 100dvh;
  display: flex;
  overflow: hidden;
  font-size: 15px;
  line-height: 1.6;
}

/* ── Sidebar ── */
#sidebar {
  width: var(--sidebar-w);
  min-width: var(--sidebar-w);
  background: #161616;
  border-right: 1px solid var(--border);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  transition: width 0.2s ease, min-width 0.2s ease;
  flex-shrink: 0;
}
#sidebar.collapsed {
  width: 0;
  min-width: 0;
  border-right: none;
}
#sidebar.collapsed > * { display: none; }

#sidebar-header {
  padding: 16px 14px 10px;
  display: flex;
  align-items: center;
  gap: 8px;
}
#sidebar-header .logo {
  font-size: 14px;
  font-weight: 600;
  color: var(--text);
  flex: 1;
}

/* Toggle button lives in the top bar */
#sidebar-toggle {
  background: none;
  border: none;
  color: var(--text-muted);
  cursor: pointer;
  padding: 4px 6px;
  border-radius: 6px;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  transition: color 0.15s, background 0.15s;
}
#sidebar-toggle:hover { background: var(--surface2); color: var(--text); }
#sidebar-toggle svg { width: 18px; height: 18px; stroke: currentColor; fill: none; stroke-width: 1.8; stroke-linecap: round; }

/* Responsive: start collapsed below 700px */
@media (max-width: 700px) {
  #sidebar { position: absolute; left: 0; top: 0; height: 100%; z-index: 100; }
  #sidebar.collapsed { width: 0; min-width: 0; }
  #sidebar:not(.collapsed) { box-shadow: 4px 0 20px rgba(0,0,0,0.5); }
}
#new-chat {
  background: none;
  border: 1px solid var(--border);
  color: var(--text-muted);
  border-radius: 6px;
  padding: 4px 8px;
  font-size: 12px;
  cursor: pointer;
  transition: all 0.15s;
}
#new-chat:hover { background: var(--surface2); color: var(--text); }
#sidebar-section {
  padding: 8px 14px 4px;
  font-size: 11px;
  font-weight: 500;
  color: var(--text-dim);
  text-transform: uppercase;
  letter-spacing: 0.08em;
}
#history-list {
  flex: 1;
  overflow-y: auto;
  padding: 4px 8px;
}
.history-item {
  padding: 6px 8px;
  border-radius: 6px;
  font-size: 13px;
  color: var(--text-muted);
  cursor: pointer;
  transition: all 0.1s;
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}
.history-item:hover { background: var(--surface2); color: var(--text); }
.history-item.active { background: var(--surface2); color: var(--text); }
.history-item .dot {
  width: 6px; height: 6px;
  border-radius: 50%;
  flex-shrink: 0;
  background: var(--text-dim);
}
.history-item .dot.ok { background: #4a9a5a; }
.history-item .dot.archived { background: #5a5a8a; }
.history-item .item-label {
  flex: 1;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  min-width: 0;
}
/* Action buttons — shown on hover */
.item-actions {
  display: none;
  gap: 2px;
  flex-shrink: 0;
}
.history-item:hover .item-actions { display: flex; }
.item-btn {
  background: none;
  border: none;
  padding: 2px 4px;
  border-radius: 4px;
  cursor: pointer;
  color: var(--text-dim);
  display: flex;
  align-items: center;
  transition: all 0.1s;
}
.item-btn:hover { background: var(--surface); color: var(--text); }
.item-btn.danger:hover { color: #c05050; }
.item-btn svg { width: 13px; height: 13px; stroke: currentColor; fill: none; stroke-width: 1.8; stroke-linecap: round; stroke-linejoin: round; }

/* Archived section */
.sidebar-divider {
  padding: 8px 14px 4px;
  font-size: 11px;
  font-weight: 500;
  color: var(--text-dim);
  text-transform: uppercase;
  letter-spacing: 0.08em;
  display: flex;
  align-items: center;
  gap: 6px;
  cursor: pointer;
  user-select: none;
}
.sidebar-divider:hover { color: var(--text-muted); }
.sidebar-divider .chevron { transition: transform 0.2s; }
.sidebar-divider.open .chevron { transform: rotate(90deg); }
#archived-list { padding: 0 8px 4px; }

#sidebar-footer {
  padding: 12px 14px;
  border-top: 1px solid var(--border);
  font-size: 12px;
  color: var(--text-dim);
}
.model-tag {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 3px 8px;
  border-radius: 4px;
  background: var(--surface2);
  color: var(--text-muted);
  font-size: 11px;
  margin-top: 4px;
}

/* ── Main area ── */
#main {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-width: 0;
  background: var(--bg);
}

#top-bar {
  padding: 12px 20px;
  border-bottom: 1px solid var(--border);
  display: flex;
  align-items: center;
  gap: 10px;
  background: var(--bg);
  flex-shrink: 0;
}
#top-bar .title { font-size: 14px; color: var(--text-muted); flex: 1; }
.status-pill {
  font-size: 11px;
  padding: 3px 9px;
  border-radius: 20px;
  background: #1a2a1a;
  color: #5a9a5a;
  border: 1px solid #2a4a2a;
}

/* ── Messages ── */
#messages {
  flex: 1;
  overflow-y: auto;
  padding: 24px 0;
  scroll-behavior: smooth;
}
#messages::-webkit-scrollbar { width: 6px; }
#messages::-webkit-scrollbar-track { background: transparent; }
#messages::-webkit-scrollbar-thumb { background: var(--border); border-radius: 3px; }

.turn {
  max-width: 720px;
  margin: 0 auto;
  padding: 4px 24px;
  width: 100%;
}
.turn + .turn { margin-top: 4px; }

/* User message */
.turn.user {
  display: flex;
  justify-content: flex-end;
  padding-top: 12px;
}
.user-bubble {
  background: var(--user-bg);
  border: 1px solid var(--border);
  border-radius: 16px 16px 4px 16px;
  padding: 10px 16px;
  max-width: 85%;
  font-size: 15px;
  white-space: pre-wrap;
  word-break: break-word;
  color: var(--text);
}

/* AI message */
.turn.ai { padding-top: 16px; }
.ai-label {
  font-size: 12px;
  font-weight: 600;
  color: var(--accent);
  letter-spacing: 0.03em;
  margin-bottom: 8px;
  display: flex;
  align-items: center;
  gap: 6px;
}
.ai-content {
  font-size: 15px;
  color: var(--text);
  line-height: 1.75;
}

/* Markdown styles */
.ai-content p { margin-bottom: 12px; }
.ai-content p:last-child { margin-bottom: 0; }
.ai-content h1, .ai-content h2, .ai-content h3 {
  font-weight: 600;
  margin: 20px 0 8px;
  color: var(--text);
  line-height: 1.3;
}
.ai-content h1 { font-size: 20px; }
.ai-content h2 { font-size: 17px; }
.ai-content h3 { font-size: 15px; }
.ai-content strong { font-weight: 600; color: #f0ede8; }
.ai-content em { font-style: italic; color: #c8c4be; }
.ai-content ul, .ai-content ol {
  padding-left: 20px;
  margin-bottom: 12px;
}
.ai-content li { margin-bottom: 4px; }
.ai-content li::marker { color: var(--text-muted); }
.ai-content blockquote {
  border-left: 3px solid var(--border);
  padding-left: 12px;
  margin: 12px 0;
  color: var(--text-muted);
}
.ai-content a { color: var(--accent); text-decoration: underline; }
.ai-content hr { border: none; border-top: 1px solid var(--border); margin: 16px 0; }
.ai-content table { border-collapse: collapse; width: 100%; margin-bottom: 12px; font-size: 14px; }
.ai-content th, .ai-content td {
  border: 1px solid var(--border);
  padding: 6px 10px;
  text-align: left;
}
.ai-content th { background: var(--surface2); font-weight: 600; }

/* Inline code */
.ai-content code {
  font-family: 'Cascadia Code', 'Fira Code', 'Consolas', monospace;
  font-size: 13px;
  background: var(--code-bg);
  border: 1px solid var(--border);
  border-radius: 4px;
  padding: 1px 5px;
  color: #d4a574;
}

/* Code blocks */
.code-block {
  margin: 12px 0;
  border-radius: var(--radius);
  overflow: hidden;
  border: 1px solid var(--border);
}
.code-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 6px 12px;
  background: #111;
  font-size: 12px;
  color: var(--text-dim);
  border-bottom: 1px solid var(--border);
}
.code-header .lang { color: var(--text-muted); font-family: monospace; }
.copy-btn {
  background: none;
  border: none;
  color: var(--text-dim);
  font-size: 11px;
  cursor: pointer;
  padding: 2px 6px;
  border-radius: 4px;
  transition: all 0.15s;
}
.copy-btn:hover { background: var(--surface2); color: var(--text); }
.copy-btn.copied { color: #5a9a5a; }
.code-block pre {
  margin: 0;
  padding: 14px 16px;
  background: var(--code-bg);
  overflow-x: auto;
  font-family: 'Cascadia Code', 'Fira Code', 'Consolas', monospace;
  font-size: 13px;
  line-height: 1.6;
  color: #c9c5be;
}
.code-block pre::-webkit-scrollbar { height: 4px; }
.code-block pre::-webkit-scrollbar-thumb { background: var(--border); }

/* Meta row */
.ai-meta {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-top: 10px;
  flex-wrap: wrap;
}
.meta-chip {
  font-size: 11px;
  padding: 2px 8px;
  border-radius: 4px;
  background: var(--surface2);
  color: var(--text-dim);
  border: 1px solid var(--border);
}
.meta-chip.ok { color: #5a9a5a; border-color: #2a4a2a; background: #1a2a1a; }
.meta-chip.fail { color: #9a5a5a; border-color: #4a2a2a; background: #2a1a1a; }

/* Details toggles */
.details-toggle {
  font-size: 12px;
  color: var(--text-dim);
  cursor: pointer;
  user-select: none;
  padding: 4px 0;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  transition: color 0.15s;
}
.details-toggle:hover { color: var(--text-muted); }
.details-body {
  display: none;
  margin-top: 6px;
  padding: 10px 12px;
  background: #111;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  font-family: monospace;
  font-size: 12px;
  color: var(--text-muted);
  line-height: 1.6;
  white-space: pre-wrap;
  word-break: break-all;
  max-height: 280px;
  overflow-y: auto;
}
.details-body.log-view { white-space: pre; }

/* Thinking indicator */
.thinking {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 14px;
  color: var(--text-muted);
}
.dots span {
  display: inline-block;
  width: 5px; height: 5px;
  border-radius: 50%;
  background: var(--text-dim);
  animation: pulse 1.4s ease-in-out infinite;
}
.dots span:nth-child(2) { animation-delay: 0.2s; }
.dots span:nth-child(3) { animation-delay: 0.4s; }
@keyframes pulse {
  0%, 80%, 100% { opacity: 0.3; transform: scale(0.9); }
  40% { opacity: 1; transform: scale(1); }
}

/* Welcome */
.welcome {
  max-width: 720px;
  margin: 60px auto 0;
  padding: 0 24px;
  text-align: center;
}
.welcome h2 { font-size: 22px; font-weight: 500; color: var(--text); margin-bottom: 10px; }
.welcome p { color: var(--text-muted); font-size: 14px; line-height: 1.7; }
.suggestion-row {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
  justify-content: center;
  margin-top: 24px;
}
.suggestion {
  padding: 8px 14px;
  border-radius: 20px;
  border: 1px solid var(--border);
  background: var(--surface);
  font-size: 13px;
  color: var(--text-muted);
  cursor: pointer;
  transition: all 0.15s;
}
.suggestion:hover { border-color: var(--accent); color: var(--text); }

/* Input area */
#input-wrap {
  padding: 16px 24px 20px;
  background: var(--bg);
  flex-shrink: 0;
}
#input-box {
  max-width: 720px;
  margin: 0 auto;
  position: relative;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 12px;
  display: flex;
  align-items: flex-end;
  gap: 0;
  transition: border-color 0.15s;
}
#input-box:focus-within { border-color: #555; }
#input {
  flex: 1;
  background: none;
  border: none;
  outline: none;
  color: var(--text);
  font-size: 15px;
  font-family: inherit;
  resize: none;
  padding: 12px 14px;
  line-height: 1.5;
  max-height: 160px;
  overflow-y: auto;
}
#input::placeholder { color: var(--text-dim); }
#send-btn {
  flex-shrink: 0;
  margin: 6px;
  width: 34px; height: 34px;
  border-radius: 8px;
  background: var(--accent);
  border: none;
  color: #fff;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: opacity 0.15s;
}
#send-btn:hover { opacity: 0.85; }
#send-btn:disabled { opacity: 0.3; cursor: default; }
#send-btn svg { width: 16px; height: 16px; fill: none; stroke: #fff; stroke-width: 2; stroke-linecap: round; stroke-linejoin: round; }
#input-hint { text-align: center; font-size: 11px; color: var(--text-dim); margin-top: 8px; }

/* Scrollbar for history */
#history-list::-webkit-scrollbar { width: 4px; }
#history-list::-webkit-scrollbar-thumb { background: var(--border); border-radius: 2px; }
</style>
</head>
<body>

<!-- Sidebar -->
<div id="sidebar">
  <div id="sidebar-header">
    <span class="logo">aihomeserver</span>
    <button id="new-chat" onclick="newChat()">+ New</button>
  </div>
  <div id="sidebar-section">Recent</div>
  <div id="history-list"></div>
  <div class="sidebar-divider" id="archived-toggle" onclick="toggleArchived()">
    <svg class="chevron" viewBox="0 0 10 10" width="10" height="10" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M3 2l4 3-4 3"/></svg>
    Archived
  </div>
  <div id="archived-list" style="display:none"></div>
  <div id="sidebar-footer">
    <div style="color:var(--text-muted);font-size:12px">Models</div>
    <div class="model-tag">qwen2.5:14b · fast</div>
    <div class="model-tag" style="margin-top:4px">qwen2.5:32b · critic</div>
  </div>
</div>

<!-- Main -->
<div id="main">
  <div id="top-bar">
    <button id="sidebar-toggle" onclick="toggleSidebar()" title="Toggle sidebar">
      <svg viewBox="0 0 18 18"><line x1="2" y1="4" x2="16" y2="4"/><line x1="2" y1="9" x2="16" y2="9"/><line x1="2" y1="14" x2="16" y2="14"/></svg>
    </button>
    <span class="title" id="top-title">New conversation</span>
    <span class="status-pill" id="status-pill">● online</span>
  </div>

  <div id="messages">
    <div class="welcome" id="welcome">
      <h2>What can I help you with?</h2>
      <p>Ask anything, generate code, write to files, run shell commands,<br>or query your git history. All local, all yours.</p>
      <div class="suggestion-row">
        <div class="suggestion" onclick="useSuggestion(this)">Explain a concept</div>
        <div class="suggestion" onclick="useSuggestion(this)">Write a Python script</div>
        <div class="suggestion" onclick="useSuggestion(this)">List files in workspace</div>
        <div class="suggestion" onclick="useSuggestion(this)">Show git log</div>
      </div>
    </div>
  </div>

  <div id="input-wrap">
    <div id="input-box">
      <textarea id="input" rows="1" placeholder="Message aihomeserver…" onkeydown="handleKey(event)"></textarea>
      <button id="send-btn" onclick="send()" disabled>
        <svg viewBox="0 0 24 24"><line x1="12" y1="19" x2="12" y2="5"/><polyline points="5 12 12 5 19 12"/></svg>
      </button>
    </div>
    <div id="input-hint">Enter to send &nbsp;·&nbsp; Shift+Enter for new line</div>
  </div>
</div>

<script>
const msgsEl = document.getElementById('messages');
const inputEl = document.getElementById('input');
const sendBtn = document.getElementById('send-btn');
let busy = false;
let currentSessionId = null;  // tracks the active session across turns

function toggleSidebar() {
  document.getElementById('sidebar').classList.toggle('collapsed');
}

// ── Input handling ────────────────────────────────────────────
inputEl.addEventListener('input', () => {
  inputEl.style.height = 'auto';
  inputEl.style.height = Math.min(inputEl.scrollHeight, 160) + 'px';
  sendBtn.disabled = !inputEl.value.trim() || busy;
});
function handleKey(e) {
  if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); send(); }
}
function useSuggestion(el) {
  inputEl.value = el.textContent;
  inputEl.dispatchEvent(new Event('input'));
  inputEl.focus();
}
function newChat() {
  currentSessionId = null;
  msgsEl.innerHTML = '';
  const w = document.createElement('div');
  w.className = 'welcome'; w.id = 'welcome';
  w.innerHTML = `<h2>What can I help you with?</h2><p>Ask anything, generate code, write to files, run shell commands,<br>or query your git history. All local, all yours.</p><div class="suggestion-row"><div class="suggestion" onclick="useSuggestion(this)">Explain a concept</div><div class="suggestion" onclick="useSuggestion(this)">Write a Rust function</div><div class="suggestion" onclick="useSuggestion(this)">List files in workspace</div><div class="suggestion" onclick="useSuggestion(this)">Show git log</div></div>`;
  msgsEl.appendChild(w);
  document.getElementById('top-title').textContent = 'New conversation';
  document.querySelectorAll('.history-item').forEach(el => el.classList.remove('active'));
  inputEl.focus();
}

// ── Send ──────────────────────────────────────────────────────
async function send() {
  const text = inputEl.value.trim();
  if (!text || busy) return;
  busy = true;
  sendBtn.disabled = true;
  inputEl.value = '';
  inputEl.style.height = 'auto';

  const welcome = document.getElementById('welcome');
  if (welcome) welcome.remove();
  document.getElementById('top-title').textContent = text.slice(0, 50) + (text.length > 50 ? '…' : '');

  addUserTurn(text);
  const thinkEl = addThinkingTurn();

  try {
    const body = { request: text };
    if (currentSessionId) body.session_id = currentSessionId;

    const res = await fetch('/run', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });
    if (!res.ok) throw new Error(`Server error ${res.status}`);
    const data = await res.json();

    currentSessionId = data.session_id;

    // Poll for completion — request runs in the background
    await pollTask(data.task_id, thinkEl);
    refreshSessions();
  } catch (err) {
    thinkEl.remove();
    addErrorTurn(String(err));
  }
  busy = false;
  sendBtn.disabled = !inputEl.value.trim();
  inputEl.focus();
}

async function pollTask(taskId, thinkEl) {
  while (true) {
    await new Promise(r => setTimeout(r, 1000));
    let status;
    try {
      const res = await fetch(`/task/${taskId}/status`);
      status = await res.json();
    } catch (_) {
      thinkEl.remove();
      addErrorTurn('Lost connection while waiting for response.');
      return;
    }
    if (status.status === 'done') {
      thinkEl.remove();
      addAiTurn(status.response);
      return;
    }
    if (status.status === 'failed') {
      thinkEl.remove();
      addErrorTurn(status.error || 'Task failed.');
      return;
    }
    // still running — keep polling
  }
}

// ── Turn builders ─────────────────────────────────────────────
function addUserTurn(text) {
  const div = document.createElement('div');
  div.className = 'turn user';
  div.innerHTML = `<div class="user-bubble">${esc(text)}</div>`;
  msgsEl.appendChild(div);
  scrollBottom();
}

function addThinkingTurn() {
  const div = document.createElement('div');
  div.className = 'turn ai';
  div.innerHTML = `<div class="ai-label">aihomeserver</div><div class="ai-content"><div class="thinking"><div class="dots"><span></span><span></span><span></span></div>Working…</div></div>`;
  msgsEl.appendChild(div);
  scrollBottom();
  return div;
}

function addErrorTurn(msg) {
  const div = document.createElement('div');
  div.className = 'turn ai';
  div.innerHTML = `<div class="ai-label">aihomeserver</div><div class="ai-content" style="color:#9a5a5a">${esc(msg)}</div>`;
  msgsEl.appendChild(div);
  scrollBottom();
}

function addAiTurn(data) {
  const ok = data.success;
  const artifacts = data.artifacts || {};
  const uid = 'u' + Date.now();

  // Extract main readable content
  const mainText = extractMainText(artifacts);

  // Build chips
  const stepChip = `<span class="meta-chip">${data.steps_taken} step${data.steps_taken !== 1 ? 's' : ''}</span>`;
  const statusChip = `<span class="meta-chip ${ok ? 'ok' : 'fail'}">${ok ? '✓ done' : '✗ failed'}</span>`;
  const failChip = data.failure_count > 0 ? `<span class="meta-chip">${data.failure_count} failure${data.failure_count !== 1 ? 's' : ''}</span>` : '';
  const repairChip = data.repair_cycles > 0 ? `<span class="meta-chip">${data.repair_cycles} repair${data.repair_cycles !== 1 ? 's' : ''}</span>` : '';

  // Tool results summary
  const toolResults = Object.entries(artifacts)
    .filter(([k]) => k.endsWith('_result'))
    .map(([k, v]) => {
      if (v && typeof v === 'object' && v.path) return `<span class="meta-chip ok">→ ${esc(v.path)}</span>`;
      if (v && typeof v === 'object' && v.stdout) return `<span class="meta-chip">stdout captured</span>`;
      return null;
    }).filter(Boolean).join('');

  // Event log
  const logHtml = (data.event_log || []).map(e => {
    const t = e.timestamp.split('T')[1].split('.')[0];
    const cls = e.event_type;
    const color = { planner:'#4a8abf', tool_success:'#4a9a5a', tool_failure:'#9a4a4a', critic_result:'#b07a3a', repair:'#8a6abf' }[cls] || '#5a5a5a';
    return `<span style="color:${color}">[${t}] ${esc(e.event_type)}</span>: ${esc(e.message)}`;
  }).join('\n');

  const rawArtifacts = JSON.stringify(
    Object.fromEntries(Object.entries(artifacts).filter(([k]) => !k.startsWith('repair_'))),
    null, 2
  );

  const div = document.createElement('div');
  div.className = 'turn ai';
  div.innerHTML = `
    <div class="ai-label">aihomeserver</div>
    <div class="ai-content">${mainText ? renderMarkdown(mainText) : '<span style="color:var(--text-muted)">Task complete.</span>'}</div>
    <div class="ai-meta">${statusChip}${stepChip}${failChip}${repairChip}${toolResults}</div>
    ${logHtml ? `
    <div class="details-toggle" onclick="toggleDetails('${uid}-log')">
      <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M2 3l3 3 3-3" id="${uid}-arrow"/></svg>
      Event log
    </div>
    <div class="details-body log-view" id="${uid}-log">${logHtml}</div>` : ''}
    ${rawArtifacts !== '{}' ? `
    <div class="details-toggle" onclick="toggleDetails('${uid}-art')" style="margin-left:12px">
      <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M2 3l3 3 3-3"/></svg>
      Artifacts
    </div>
    <div class="details-body" id="${uid}-art">${esc(rawArtifacts)}</div>` : ''}
  `;
  msgsEl.appendChild(div);
  scrollBottom();
}

// ── Text extraction ───────────────────────────────────────────
function extractMainText(artifacts) {
  const keys = Object.keys(artifacts);
  // 1. Non-result, non-repair keys (direct LLM output)
  for (const k of keys.filter(k => !k.endsWith('_result') && !k.startsWith('repair_'))) {
    const t = deepExtractText(artifacts[k]);
    if (t && t.length > 20) return t;
  }
  // 2. Tool result content
  for (const k of keys.filter(k => k.endsWith('_result'))) {
    const v = artifacts[k];
    if (v && typeof v === 'object') {
      if (v.content) return v.content;
      if (v.stdout && v.stdout.trim()) return v.stdout.trim();
    }
  }
  return null;
}

function deepExtractText(val) {
  if (typeof val === 'string') {
    try {
      const inner = JSON.parse(val);
      if (typeof inner === 'string') return inner;
      if (inner && typeof inner === 'object') {
        for (const f of ['content','text','output','result','answer','summary','response','stdout']) {
          if (typeof inner[f] === 'string' && inner[f].length > 10) return inner[f];
        }
      }
    } catch (_) {}
    return val.trim();
  }
  if (val && typeof val === 'object') {
    for (const f of ['content','text','output','result','answer','summary','response']) {
      if (typeof val[f] === 'string' && val[f].length > 10) return val[f];
    }
  }
  return null;
}

// ── Markdown renderer ─────────────────────────────────────────
function renderMarkdown(md) {
  let uid = 0;
  // Code blocks first (protect from other transforms)
  const codeBlocks = [];
  md = md.replace(/```(\w*)\n?([\s\S]*?)```/g, (_, lang, code) => {
    const id = 'cb' + Date.now() + (uid++);
    const label = lang || 'code';
    codeBlocks.push({ id, html:
      `<div class="code-block">` +
      `<div class="code-header"><span class="lang">${esc(label)}</span>` +
      `<button class="copy-btn" onclick="copyCode('${id}')">copy</button></div>` +
      `<pre id="${id}">${esc(code.replace(/\n$/, ''))}</pre></div>`
    });
    return `\x00cb${codeBlocks.length - 1}\x00`;
  });

  // Inline elements
  md = md
    .replace(/\*\*\*(.+?)\*\*\*/g, '<strong><em>$1</em></strong>')
    .replace(/\*\*(.+?)\*\*/g, '<strong>$1</strong>')
    .replace(/\*(.+?)\*/g, '<em>$1</em>')
    .replace(/`([^`]+)`/g, (_, c) => `<code>${esc(c)}</code>`)
    .replace(/\[([^\]]+)\]\(([^)]+)\)/g, '<a href="$2" target="_blank" rel="noopener">$1</a>');

  // Block elements (line by line)
  const lines = md.split('\n');
  const out = [];
  let inList = false, listTag = '';

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    if (/^\x00cb\d+\x00$/.test(line.trim())) {
      if (inList) { out.push(`</${listTag}>`); inList = false; }
      const idx = parseInt(line.trim().replace(/\x00cb(\d+)\x00/, '$1'));
      out.push(codeBlocks[idx].html);
      continue;
    }
    if (/^#{1,3}\s/.test(line)) {
      if (inList) { out.push(`</${listTag}>`); inList = false; }
      const m = line.match(/^(#{1,3})\s+(.*)/);
      out.push(`<h${m[1].length}>${m[2]}</h${m[1].length}>`);
    } else if (/^[-*]\s/.test(line)) {
      if (!inList || listTag !== 'ul') {
        if (inList) out.push(`</${listTag}>`);
        out.push('<ul>'); inList = true; listTag = 'ul';
      }
      out.push(`<li>${line.replace(/^[-*]\s/, '')}</li>`);
    } else if (/^\d+\.\s/.test(line)) {
      if (!inList || listTag !== 'ol') {
        if (inList) out.push(`</${listTag}>`);
        out.push('<ol>'); inList = true; listTag = 'ol';
      }
      out.push(`<li>${line.replace(/^\d+\.\s/, '')}</li>`);
    } else if (/^>\s/.test(line)) {
      if (inList) { out.push(`</${listTag}>`); inList = false; }
      out.push(`<blockquote>${line.replace(/^>\s/, '')}</blockquote>`);
    } else if (/^---+$/.test(line.trim())) {
      if (inList) { out.push(`</${listTag}>`); inList = false; }
      out.push('<hr>');
    } else if (line.trim() === '') {
      if (inList) { out.push(`</${listTag}>`); inList = false; }
      out.push('');
    } else {
      if (inList) { out.push(`</${listTag}>`); inList = false; }
      out.push(`<p>${line}</p>`);
    }
  }
  if (inList) out.push(`</${listTag}>`);
  return out.join('\n');
}

function copyCode(id) {
  const el = document.getElementById(id);
  if (!el) return;
  navigator.clipboard.writeText(el.textContent).then(() => {
    const btn = el.closest('.code-block').querySelector('.copy-btn');
    if (btn) { btn.textContent = 'copied'; btn.classList.add('copied'); setTimeout(() => { btn.textContent = 'copy'; btn.classList.remove('copied'); }, 2000); }
  });
}

// ── Helpers ───────────────────────────────────────────────────
function toggleDetails(id) {
  const el = document.getElementById(id);
  if (!el) return;
  const open = el.style.display === 'block';
  el.style.display = open ? 'none' : 'block';
}

function esc(s) {
  return String(s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;');
}

function scrollBottom() {
  msgsEl.scrollTop = msgsEl.scrollHeight;
}

// ── Session sidebar ───────────────────────────────────────────
const ICON_ARCHIVE = `<svg viewBox="0 0 16 16"><rect x="1" y="3" width="14" height="3" rx="1"/><path d="M2 6v7a1 1 0 001 1h10a1 1 0 001-1V6"/><path d="M6 10h4"/></svg>`;
const ICON_RESTORE = `<svg viewBox="0 0 16 16"><polyline points="1 4 1 1 4 1"/><path d="M1 1l4 4"/><path d="M15 8A7 7 0 113 4.3"/></svg>`;
const ICON_TRASH   = `<svg viewBox="0 0 16 16"><polyline points="2 4 14 4"/><path d="M5 4V2h6v2"/><path d="M3 4l1 10a1 1 0 001 1h6a1 1 0 001-1l1-10"/><line x1="6" y1="7" x2="6" y2="11"/><line x1="10" y1="7" x2="10" y2="11"/></svg>`;

function makeSessionItem(s, archived) {
  const item = document.createElement('div');
  item.className = 'history-item' + (s.session_id === currentSessionId ? ' active' : '');
  item.dataset.sid = s.session_id;
  const preview = s.first_message
    ? s.first_message.slice(0, 32) + (s.first_message.length > 32 ? '…' : '')
    : 'Empty session';
  item.title = s.first_message || '';

  const archiveBtn = `<button class="item-btn" title="${archived ? 'Restore' : 'Archive'}" onclick="event.stopPropagation();${archived ? 'doUnarchive' : 'doArchive'}('${s.session_id}')">${archived ? ICON_RESTORE : ICON_ARCHIVE}</button>`;
  const deleteBtn  = `<button class="item-btn danger" title="Delete permanently" onclick="event.stopPropagation();doDelete('${s.session_id}')">${ICON_TRASH}</button>`;

  item.innerHTML = `
    <span class="dot ${archived ? 'archived' : 'ok'}"></span>
    <span class="item-label">${esc(preview)}</span>
    <span class="item-actions">${archiveBtn}${deleteBtn}</span>`;
  item.onclick = () => loadSession(s.session_id, s.first_message);
  return item;
}

async function refreshSessions() {
  try {
    const res = await fetch('/sessions');
    const sessions = await res.json();
    const list = document.getElementById('history-list');
    list.innerHTML = '';
    for (const s of sessions.slice(0, 40)) {
      list.appendChild(makeSessionItem(s, false));
    }
  } catch (_) {}
  // also refresh archived if panel is open
  if (document.getElementById('archived-toggle').classList.contains('open')) {
    refreshArchived();
  }
}

async function refreshArchived() {
  try {
    const res = await fetch('/sessions/archived');
    const sessions = await res.json();
    const list = document.getElementById('archived-list');
    list.innerHTML = '';
    for (const s of sessions.slice(0, 40)) {
      list.appendChild(makeSessionItem(s, true));
    }
    if (sessions.length === 0) {
      list.innerHTML = '<div style="padding:6px 8px;font-size:12px;color:var(--text-dim)">Nothing archived</div>';
    }
  } catch (_) {}
}

function toggleArchived() {
  const toggle = document.getElementById('archived-toggle');
  const list = document.getElementById('archived-list');
  const open = toggle.classList.toggle('open');
  list.style.display = open ? 'block' : 'none';
  if (open) refreshArchived();
}

async function doArchive(sessionId) {
  await fetch(`/session/${sessionId}/archive`, { method: 'POST' });
  if (currentSessionId === sessionId) newChat();
  refreshSessions();
}

async function doUnarchive(sessionId) {
  await fetch(`/session/${sessionId}/unarchive`, { method: 'POST' });
  refreshSessions();
}

async function doDelete(sessionId) {
  if (!confirm('Permanently delete this conversation? This cannot be undone.')) return;
  await fetch(`/session/${sessionId}`, { method: 'DELETE' });
  if (currentSessionId === sessionId) newChat();
  refreshSessions();
}

async function loadSession(sessionId, firstMessage) {
  if (busy) return;
  try {
    const res = await fetch(`/session/${sessionId}`);
    if (!res.ok) return;
    const turns = await res.json();

    currentSessionId = sessionId;
    msgsEl.innerHTML = '';

    const label = (firstMessage || 'Session').slice(0, 50);
    document.getElementById('top-title').textContent =
      label + (label.length >= 50 ? '…' : '');

    for (const turn of turns) {
      if (turn.role === 'user') {
        addUserTurn(turn.content);
      } else {
        const div = document.createElement('div');
        div.className = 'turn ai';
        div.innerHTML = `<div class="ai-label">aihomeserver</div><div class="ai-content">${renderMarkdown(turn.content)}</div>`;
        msgsEl.appendChild(div);
      }
    }

    scrollBottom();
    document.querySelectorAll('.history-item').forEach(el => {
      el.classList.toggle('active', el.dataset.sid === sessionId);
    });
  } catch (_) {}
}

// ── Init ──────────────────────────────────────────────────────
fetch('/health').then(r => r.json()).then(d => {
  document.getElementById('status-pill').textContent = '● online v' + d.version;
}).catch(() => {
  const p = document.getElementById('status-pill');
  p.textContent = '● offline';
  p.style.cssText = 'background:#2a1a1a;color:#9a5a5a;border-color:#4a2a2a';
});
// Collapse sidebar by default on narrow screens
if (window.innerWidth <= 700) {
  document.getElementById('sidebar').classList.add('collapsed');
}
refreshSessions();
inputEl.focus();
</script>
</body>
</html>
"#;
