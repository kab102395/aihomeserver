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
  --sidebar-min: 180px;
  --sidebar-max: 560px;
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
  border-right: none;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  transition: width 0.2s ease, min-width 0.2s ease;
  flex-shrink: 0;
  position: relative;
}
/* Drag handle — sits on the right edge of the sidebar */
#sidebar-resize {
  position: absolute;
  top: 0; right: 0;
  width: 5px;
  height: 100%;
  cursor: col-resize;
  background: transparent;
  z-index: 50;
  transition: background 0.15s;
}
#sidebar-resize:hover,
#sidebar-resize.dragging { background: var(--accent); opacity: 0.6; }
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
.history-item .sess-badge {
  font-size: 10px;
  line-height: 1.2;
  padding: 1px 6px;
  border-radius: 999px;
  border: 1px solid var(--border);
  color: var(--text-dim);
  background: rgba(255,255,255,0.03);
  flex-shrink: 0;
}
.history-item .sess-badge.running {
  color: #d7c88a;
  border-color: rgba(215,200,138,0.35);
  background: rgba(215,200,138,0.08);
}
.history-item .sess-badge.ok {
  color: #7fd18e;
  border-color: rgba(127,209,142,0.35);
  background: rgba(127,209,142,0.08);
}
.history-item .sess-badge.fail {
  color: #e08a8a;
  border-color: rgba(224,138,138,0.35);
  background: rgba(224,138,138,0.08);
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

/* ── Sidebar tabs ── */
#sidebar-tabs {
  display: flex;
  border-bottom: 1px solid var(--border);
  flex-shrink: 0;
}
.tab-btn {
  flex: 1;
  background: none;
  border: none;
  color: var(--text-dim);
  font-size: 12px;
  font-weight: 500;
  padding: 8px 0;
  cursor: pointer;
  transition: all 0.15s;
  border-bottom: 2px solid transparent;
  margin-bottom: -1px;
}
.tab-btn:hover { color: var(--text-muted); }
.tab-btn.active { color: var(--text); border-bottom-color: var(--accent); }
#tab-chat, #tab-files {
  flex: 1;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  min-height: 0;
}
#tab-files { display: none; }

/* ── File tree ── */
#file-tree-toolbar {
  padding: 6px 10px;
  display: flex;
  align-items: center;
  gap: 6px;
  flex-shrink: 0;
}
#file-tree-toolbar .tree-root-label {
  font-size: 11px;
  color: var(--text-dim);
  flex: 1;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.tree-refresh-btn {
  background: none;
  border: none;
  color: var(--text-dim);
  cursor: pointer;
  padding: 2px 4px;
  border-radius: 4px;
  font-size: 13px;
  line-height: 1;
  flex-shrink: 0;
  transition: all 0.12s;
}
.tree-refresh-btn:hover { color: var(--text); background: var(--surface2); }
#file-tree {
  flex: 1;
  overflow-y: auto;
  padding: 2px 6px 8px;
}
#file-tree.drop-target {
  outline: 2px dashed var(--accent);
  outline-offset: -3px;
  background: rgba(90, 132, 255, 0.08);
}
#file-tree::-webkit-scrollbar { width: 4px; }
#file-tree::-webkit-scrollbar-thumb { background: var(--border); border-radius: 2px; }
.tree-empty {
  padding: 16px 8px;
  font-size: 12px;
  color: var(--text-dim);
  text-align: center;
  line-height: 1.6;
}
/* Directory node */
.tree-dir > .tree-row {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 3px 4px;
  border-radius: 5px;
  cursor: pointer;
  user-select: none;
  transition: background 0.1s;
}
.tree-dir > .tree-row:hover { background: var(--surface2); }
.tree-dir-icon { font-size: 11px; color: var(--text-dim); flex-shrink: 0; transition: transform 0.15s; }
.tree-dir.open > .tree-row .tree-dir-icon { transform: rotate(90deg); }
.tree-dir-name { font-size: 12px; color: var(--text-muted); flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.tree-children { display: none; padding-left: 14px; }
.tree-dir.open > .tree-children { display: block; }
/* File node */
.tree-file {
  display: flex;
  align-items: center;
  gap: 5px;
  padding: 3px 4px;
  border-radius: 5px;
  cursor: pointer;
  transition: background 0.1s;
}
.tree-file:hover { background: var(--surface2); }
.tree-file.attached { background: rgba(90,120,200,0.15); }
.tree-file-icon { font-size: 11px; color: var(--text-dim); flex-shrink: 0; }
.tree-file-name { font-size: 12px; color: var(--text-muted); flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.tree-file:hover .tree-file-name, .tree-file.attached .tree-file-name { color: var(--text); }
.tree-file-size { font-size: 10px; color: var(--text-dim); flex-shrink: 0; }
.tree-file.attached .tree-file-size { color: #8aaaee; }
.tree-file-edit {
  background: none;
  border: none;
  color: var(--text-dim);
  cursor: pointer;
  padding: 1px 3px;
  border-radius: 3px;
  font-size: 11px;
  opacity: 0;
  flex-shrink: 0;
  line-height: 1;
  transition: all 0.1s;
}
.tree-file:hover .tree-file-edit { opacity: 1; }
.tree-file-edit:hover { color: var(--accent); background: var(--surface); }

.tree-file-dl {
  background: none;
  border: none;
  color: var(--text-dim);
  cursor: pointer;
  padding: 1px 3px;
  border-radius: 3px;
  font-size: 11px;
  opacity: 0;
  flex-shrink: 0;
  line-height: 1;
  transition: all 0.1s;
}
.tree-file:hover .tree-file-dl { opacity: 1; }
.tree-file-dl:hover { color: var(--accent); background: var(--surface); }

/* ── File tree context menu ── */
.ctx-menu {
  position: fixed;
  background: var(--surface);
  border: 1px solid #444;
  border-radius: 7px;
  padding: 4px;
  z-index: 500;
  min-width: 140px;
  box-shadow: 0 8px 24px rgba(0,0,0,0.5);
}
.ctx-menu-item {
  padding: 7px 12px;
  font-size: 12px;
  color: var(--text-muted);
  cursor: pointer;
  border-radius: 4px;
  white-space: nowrap;
  transition: all 0.1s;
}
.ctx-menu-item:hover { background: var(--surface2); color: var(--text); }
.ctx-menu-item.danger:hover { color: #c05050; background: rgba(200,60,60,0.1); }
.ctx-menu-sep { border: none; border-top: 1px solid var(--border); margin: 3px 4px; }
/* Inline rename input in tree */
.tree-rename-input {
  background: var(--surface2);
  border: 1px solid #5a7abe;
  border-radius: 4px;
  color: var(--text);
  font-size: 12px;
  padding: 2px 6px;
  outline: none;
  width: 100%;
  font-family: inherit;
}

/* ── Git tab ── */
#tab-git { display: none; flex: 1; flex-direction: column; overflow: hidden; min-height: 0; }

#git-branch-bar {
  padding: 7px 12px;
  display: flex;
  align-items: center;
  gap: 7px;
  border-bottom: 1px solid var(--border);
  flex-shrink: 0;
}
#git-branch-name {
  font-size: 12px;
  font-family: 'Cascadia Code', monospace;
  color: var(--text);
  flex: 1;
}
.git-sync-badge {
  font-size: 10px;
  padding: 1px 6px;
  border-radius: 4px;
  background: var(--surface2);
  color: var(--text-dim);
  font-family: monospace;
}
.git-refresh-btn {
  background: none; border: none; color: var(--text-dim); cursor: pointer;
  font-size: 13px; padding: 2px 4px; border-radius: 3px; transition: all 0.12s;
}
.git-refresh-btn:hover { color: var(--text); background: var(--surface2); }

#git-body { flex: 1; overflow-y: auto; }
#git-body::-webkit-scrollbar { width: 4px; }
#git-body::-webkit-scrollbar-thumb { background: var(--border); border-radius: 2px; }

.git-section-header {
  padding: 6px 12px 3px;
  font-size: 10px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.08em;
  color: var(--text-dim);
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.git-stage-all-btn {
  background: none; border: 1px solid var(--border); color: var(--text-dim);
  font-size: 10px; padding: 1px 7px; border-radius: 4px; cursor: pointer; transition: all 0.12s;
}
.git-stage-all-btn:hover { background: var(--surface2); color: var(--text); }

.git-file-row {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 4px 12px;
  font-size: 12px;
  cursor: pointer;
  transition: background 0.1s;
}
.git-file-row:hover { background: var(--surface2); }
.git-status-badge {
  font-size: 10px;
  font-family: monospace;
  font-weight: 700;
  width: 16px;
  text-align: center;
  flex-shrink: 0;
}
.s-M { color: #c09030; } /* modified */
.s-A { color: #4a9a5a; } /* added    */
.s-D { color: #9a4a4a; } /* deleted  */
.s-R { color: #4a7abf; } /* renamed  */
.s-u { color: var(--text-dim); } /* untracked */
.git-file-path { flex: 1; color: var(--text-muted); font-family: monospace; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.git-file-row:hover .git-file-path { color: var(--text); }
.git-stage-btn {
  background: none; border: 1px solid var(--border); color: var(--text-dim);
  font-size: 10px; padding: 1px 6px; border-radius: 4px; cursor: pointer;
  flex-shrink: 0; transition: all 0.1s;
}
.git-stage-btn:hover { background: var(--surface2); color: var(--text); }

/* Diff inline panel */
.git-diff-wrap {
  display: none;
  background: #0a0a0a;
  border-top: 1px solid var(--border);
  margin: 0 0 4px;
  max-height: 240px;
  overflow-y: auto;
  font-family: 'Cascadia Code', 'Consolas', monospace;
  font-size: 11px;
  line-height: 1.5;
}
.git-diff-wrap.open { display: block; }
.diff-line { padding: 0 12px; white-space: pre; }
.diff-add  { background: rgba(40,100,40,0.25); color: #7acc7a; }
.diff-del  { background: rgba(100,40,40,0.25); color: #cc7a7a; }
.diff-hunk { color: #5a7abf; background: rgba(30,40,80,0.3); }
.diff-meta { color: var(--text-dim); }

/* Commit form */
#git-commit-form {
  padding: 10px 12px;
  border-top: 1px solid var(--border);
  flex-shrink: 0;
}
#git-commit-msg {
  width: 100%;
  background: var(--surface2);
  border: 1px solid var(--border);
  border-radius: 6px;
  color: var(--text);
  font-size: 12px;
  padding: 7px 10px;
  outline: none;
  resize: none;
  font-family: inherit;
  transition: border-color 0.15s;
}
#git-commit-msg:focus { border-color: #555; }
#git-commit-msg::placeholder { color: var(--text-dim); }
.git-commit-row { display: flex; gap: 8px; margin-top: 7px; align-items: center; }
.git-commit-btn {
  background: #1a3a1a; border: 1px solid #2a5a2a; color: #5a9a5a;
  border-radius: 6px; padding: 6px 14px; font-size: 12px; cursor: pointer;
  transition: background 0.15s; white-space: nowrap;
}
.git-commit-btn:hover { background: #224422; }
.git-commit-btn:disabled { opacity: 0.4; cursor: default; }
#git-commit-status { font-size: 11px; color: var(--text-dim); flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }

/* Recent commits */
.git-commit-item {
  padding: 5px 12px;
  font-size: 11px;
  border-bottom: 1px solid #1a1a1a;
  cursor: default;
}
.git-commit-hash { font-family: monospace; color: var(--accent); margin-right: 6px; font-size: 11px; }
.git-commit-msg-text { color: var(--text-muted); }
.git-commit-meta { color: var(--text-dim); font-size: 10px; margin-top: 1px; }

#git-empty { padding: 16px 12px; font-size: 12px; color: var(--text-dim); text-align: center; line-height: 1.7; }

/* ── Memory tab ── */
#tab-mem { display: none; flex: 1; flex-direction: column; overflow: hidden; min-height: 0; }
#tab-kb  { display: none; flex: 1; flex-direction: column; overflow: hidden; min-height: 0; }

/* Knowledge tab */
#kb-toolbar { display:flex; gap:6px; padding:8px; border-bottom:1px solid var(--border); flex-shrink:0; }
#kb-search { flex:1; background:var(--surface); border:1px solid var(--border); border-radius:4px;
  color:var(--text); padding:4px 8px; font-size:12px; }
.kb-add-btn { background:var(--accent); color:#fff; border:none; border-radius:4px;
  padding:4px 8px; font-size:11px; cursor:pointer; white-space:nowrap; }
.kb-add-btn:hover { opacity:0.85; }
#kb-list { flex:1; overflow-y:auto; padding:4px 0; }
.kb-entry { border-bottom:1px solid #1f1f1f; padding:8px 10px; }
.kb-entry-header { display:flex; align-items:flex-start; gap:6px; cursor:pointer; }
.kb-entry-topic { flex:1; font-size:12px; font-weight:600; color:var(--text); line-height:1.4; }
.kb-entry-meta { font-size:10px; color:var(--text-muted); white-space:nowrap; }
.kb-entry-summary { font-size:11px; color:var(--text-dim); margin-top:4px; line-height:1.4; }
.kb-entry-tags { margin-top:4px; display:flex; flex-wrap:wrap; gap:3px; }
.kb-tag { font-size:10px; background:#2a2a2a; color:var(--text-muted); border-radius:3px; padding:1px 5px; }
.kb-entry-full { display:none; margin-top:8px; font-size:11px; color:var(--text-dim);
  background:#111; border-radius:4px; padding:8px; max-height:300px; overflow-y:auto;
  white-space:pre-wrap; line-height:1.5; border:1px solid var(--border); }
.kb-entry-full a { color: var(--accent); text-decoration: none; }
.kb-entry-full a:hover { text-decoration: underline; }
.kb-entry.open .kb-entry-full { display:block; }
.kb-entry-actions { display:flex; gap:6px; margin-top:6px; }
.kb-btn { font-size:10px; padding:2px 8px; border-radius:3px; border:1px solid var(--border);
  background:transparent; color:var(--text-dim); cursor:pointer; }
.kb-btn:hover { border-color:var(--text-muted); color:var(--text); }
.kb-btn.danger:hover { border-color:#e05; color:#e05; }
.kb-btn.refresh-btn { border-color:var(--accent); color:var(--accent); }
#kb-footer { padding:8px 10px; border-top:1px solid var(--border); font-size:11px;
  color:var(--text-muted); display:flex; justify-content:space-between; align-items:center;
  flex-shrink:0; }
/* Add/edit knowledge modal */
#kb-modal { display:none; position:fixed; inset:0; background:rgba(0,0,0,0.7);
  z-index:1000; align-items:center; justify-content:center; }
#kb-modal.open { display:flex; }
#kb-modal-card { background:var(--surface); border:1px solid var(--border); border-radius:8px;
  padding:20px; width:min(560px,90vw); display:flex; flex-direction:column; gap:10px; }
#kb-modal-card h3 { margin:0; font-size:14px; color:var(--text); }
#kb-modal-card input, #kb-modal-card textarea {
  background:#111; border:1px solid var(--border); border-radius:4px;
  color:var(--text); padding:8px; font-size:12px; font-family:inherit; }
#kb-modal-card input:focus, #kb-modal-card textarea:focus { border-color:#555; outline:none; }
#kb-modal-content { resize:vertical; min-height:120px; }
#kb-modal-footer { display:flex; justify-content:flex-end; gap:8px; }
.kb-modal-save { background:var(--accent); color:#fff; border:none; border-radius:4px;
  padding:6px 16px; cursor:pointer; font-size:13px; }
.kb-modal-cancel { background:transparent; border:1px solid var(--border); color:var(--text-dim);
  border-radius:4px; padding:6px 16px; cursor:pointer; font-size:13px; }
#mem-toolbar {
  padding: 6px 10px;
  display: flex;
  align-items: center;
  gap: 6px;
  border-bottom: 1px solid var(--border);
  flex-shrink: 0;
}
#mem-search {
  flex: 1;
  background: var(--surface2);
  border: 1px solid var(--border);
  border-radius: 5px;
  color: var(--text);
  font-size: 12px;
  padding: 4px 8px;
  outline: none;
  font-family: inherit;
  transition: border-color 0.15s;
}
#mem-search:focus { border-color: #555; }
#mem-search::placeholder { color: var(--text-dim); }
#mem-list { flex: 1; overflow-y: auto; }
#mem-list::-webkit-scrollbar { width: 4px; }
#mem-list::-webkit-scrollbar-thumb { background: var(--border); border-radius: 2px; }
.mem-empty { padding: 16px 12px; font-size: 12px; color: var(--text-dim); text-align: center; line-height: 1.7; }
.mem-task {
  padding: 6px 12px 5px;
  border-bottom: 1px solid #1a1a1a;
  cursor: pointer;
  transition: background 0.1s;
}
.mem-task:hover { background: var(--surface2); }
.mem-task-header {
  display: flex;
  align-items: center;
  gap: 5px;
}
.mem-task-badge { font-size: 11px; flex-shrink: 0; }
.mem-task-req {
  flex: 1;
  font-size: 12px;
  color: var(--text-muted);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  min-width: 0;
}
.mem-task-time { font-size: 10px; color: var(--text-dim); flex-shrink: 0; }
.mem-delete-btn {
  background: none;
  border: none;
  color: var(--text-dim);
  cursor: pointer;
  font-size: 14px;
  padding: 0 2px;
  line-height: 1;
  flex-shrink: 0;
  transition: color 0.1s;
}
.mem-delete-btn:hover { color: #c05050; }
.mem-task-meta { font-size: 10px; color: var(--text-dim); margin-top: 1px; padding-left: 18px; }
.mem-task-detail {
  display: none;
  background: #0a0a0a;
  border: 1px solid var(--border);
  border-radius: 4px;
  margin: 5px 0 3px 18px;
  padding: 7px 10px;
  font-size: 11px;
  color: var(--text-dim);
  font-family: 'Cascadia Code', 'Consolas', monospace;
  max-height: 120px;
  overflow-y: auto;
  white-space: pre-wrap;
  word-break: break-all;
}
.mem-task-detail.open { display: block; }
#mem-footer {
  padding: 8px 12px;
  border-top: 1px solid var(--border);
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 11px;
  flex-shrink: 0;
}
#mem-semantic-count { flex: 1; color: var(--text-dim); }
.mem-clear-btn {
  background: none;
  border: 1px solid var(--border);
  color: var(--text-dim);
  font-size: 10px;
  padding: 2px 8px;
  border-radius: 4px;
  cursor: pointer;
  transition: all 0.1s;
}
.mem-clear-btn:hover { background: rgba(200,60,60,0.1); border-color: #5a2a2a; color: #c05050; }

/* ── Workspace search overlay ── */
#search-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0,0,0,0.65);
  z-index: 1200;
  display: flex;
  align-items: flex-start;
  justify-content: center;
  padding-top: 80px;
  opacity: 0;
  pointer-events: none;
  transition: opacity 0.15s;
  backdrop-filter: blur(2px);
}
#search-overlay.open { opacity: 1; pointer-events: all; }
#search-card {
  background: var(--surface);
  border: 1px solid #444;
  border-radius: 10px;
  width: 580px;
  max-width: 94%;
  max-height: 460px;
  display: flex;
  flex-direction: column;
  box-shadow: 0 16px 48px rgba(0,0,0,0.6);
  transform: translateY(8px);
  transition: transform 0.15s;
}
#search-overlay.open #search-card { transform: translateY(0); }
#search-input-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 12px 16px;
  border-bottom: 1px solid var(--border);
  flex-shrink: 0;
}
#search-input-row .search-icon { font-size: 15px; color: var(--text-dim); flex-shrink: 0; }
#search-input {
  flex: 1;
  background: none;
  border: none;
  outline: none;
  color: var(--text);
  font-size: 15px;
  font-family: inherit;
}
#search-input::placeholder { color: var(--text-dim); }
#search-close {
  background: none;
  border: none;
  color: var(--text-dim);
  cursor: pointer;
  font-size: 16px;
  padding: 2px 4px;
  border-radius: 4px;
  flex-shrink: 0;
  line-height: 1;
  transition: color 0.1s;
}
#search-close:hover { color: var(--text); }
#search-results { flex: 1; overflow-y: auto; }
#search-results::-webkit-scrollbar { width: 4px; }
#search-results::-webkit-scrollbar-thumb { background: var(--border); border-radius: 2px; }
.search-result {
  display: flex;
  align-items: baseline;
  gap: 8px;
  padding: 8px 16px;
  cursor: pointer;
  border-bottom: 1px solid #1a1a1a;
  transition: background 0.1s;
}
.search-result:hover { background: var(--surface2); }
.search-result-file { font-size: 11px; font-family: monospace; color: var(--accent); flex-shrink: 0; }
.search-result-line { font-size: 10px; color: var(--text-dim); flex-shrink: 0; }
.search-result-preview { font-size: 12px; color: var(--text-muted); font-family: monospace; flex: 1; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; min-width: 0; }
.search-empty { padding: 24px 16px; font-size: 13px; color: var(--text-dim); text-align: center; }
#search-footer { padding: 6px 16px; font-size: 11px; color: var(--text-dim); border-top: 1px solid var(--border); flex-shrink: 0; }

/* ── Context attachment bar ── */
#context-bar {
  max-width: 720px;
  margin: 0 auto 6px;
  display: none;
  flex-wrap: wrap;
  gap: 6px;
  padding: 0 4px;
}
#context-bar.has-files { display: flex; }
.ctx-chip {
  display: flex;
  align-items: center;
  gap: 5px;
  background: rgba(90,120,200,0.15);
  border: 1px solid rgba(90,120,200,0.35);
  border-radius: 5px;
  padding: 3px 8px 3px 7px;
  font-size: 11px;
  color: #aac0f8;
  font-family: 'Cascadia Code', 'Consolas', monospace;
}
.ctx-chip-remove {
  background: none;
  border: none;
  color: #6080c0;
  cursor: pointer;
  padding: 0;
  font-size: 13px;
  line-height: 1;
  transition: color 0.12s;
}
.ctx-chip-remove:hover { color: #c07070; }

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
#jump-bottom {
  position: fixed;
  right: 18px;
  bottom: 84px;
  z-index: 900;
  display: none;
  background: rgba(30,30,30,0.92);
  border: 1px solid var(--border);
  color: var(--text);
  padding: 6px 10px;
  border-radius: 999px;
  font-size: 12px;
  cursor: pointer;
  box-shadow: 0 6px 20px rgba(0,0,0,0.35);
}
#jump-bottom:hover { border-color: var(--text-muted); }

.tab-badge {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  margin-left: 6px;
  min-width: 16px;
  height: 16px;
  padding: 0 5px;
  border-radius: 999px;
  font-size: 10px;
  line-height: 1;
  border: 1px solid var(--border);
  color: var(--text-muted);
  background: #141414;
}
.tab-badge.running { color: #caa05a; border-color: rgba(202,160,90,0.35); background: rgba(202,160,90,0.10); }
.tab-badge.ok      { color: #66b06f; border-color: rgba(102,176,111,0.35); background: rgba(102,176,111,0.10); }
.tab-badge.fail    { color: #d06a6a; border-color: rgba(208,106,106,0.35); background: rgba(208,106,106,0.10); }

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

/* Math blocks (KaTeX) */
.math-block {
  margin: 14px 0;
  text-align: center;
  overflow-x: auto;
}
.math-block .katex { font-size: 1.1em; }
.katex-inline .katex { font-size: 1em; }

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

/* qwen3 live thinking indicator */
.thinking-live {
  display: flex;
  align-items: center;
  gap: 6px;
  color: var(--text-dim);
  font-size: 13px;
  padding: 4px 0;
  animation: thinking-pulse 1.8s ease-in-out infinite;
}
@keyframes thinking-pulse {
  0%, 100% { opacity: 0.7; }
  50%       { opacity: 1.0; }
}
.thinking-count {
  font-size: 11px;
  opacity: 0.55;
  margin-left: 2px;
}
/* Collapsed CoT block after thinking finishes */
.thinking-details {
  margin: 2px 0 8px;
  font-size: 12px;
}
.thinking-details summary {
  cursor: pointer;
  color: var(--accent);
  font-size: 12px;
  padding: 2px 0;
  user-select: none;
  list-style: none;
  display: flex;
  align-items: center;
  gap: 5px;
}
.thinking-details summary::-webkit-details-marker { display: none; }
.thinking-details summary:hover { opacity: 0.8; }
.thinking-details[open] summary::before { content: '▾'; font-size: 10px; }
.thinking-details:not([open]) summary::before { content: '▸'; font-size: 10px; }
.thinking-trace {
  margin-top: 6px;
  padding: 8px 10px;
  background: var(--bg-input);
  border-radius: 4px;
  white-space: pre-wrap;
  font-size: 11px;
  line-height: 1.55;
  max-height: 280px;
  overflow-y: auto;
  color: var(--text-dim);
  border-left: 2px solid var(--accent);
}

/* Thinking indicator */
.thinking {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 14px;
  color: var(--text-muted);
}

/* Reasoning panel — live activity log */
.reasoning {
  margin-top: 8px;
  font-size: 12px;
  color: var(--text-muted);
  border-left: 2px solid var(--border);
  padding-left: 10px;
}
.reasoning-line {
  padding: 2px 0;
  line-height: 1.4;
  display: flex;
  align-items: baseline;
  gap: 6px;
}
.reasoning-line .r-icon { flex-shrink: 0; }
.reasoning-line .r-text { color: var(--text-dim); }
.reasoning-line.r-tool { color: #4a8abf; }
.reasoning-line.r-tool-detail { color: var(--text-dim); font-size: 11px; opacity: 0.85; }
.reasoning-line.r-tool-detail .r-text { word-break: break-all; }
.reasoning-line.r-tool-done-ok { color: #4a9a5a; }
.reasoning-line.r-tool-done-fail { color: #9a4a4a; }
.reasoning-line.r-plan { color: var(--text-muted); }
.reasoning-line.r-critic-pass { color: #4a9a5a; }
.reasoning-line.r-critic-fail { color: #c08030; }
.reasoning-line.r-repair  { color: #c08030; }
.reasoning-line.r-replan  { color: #9a5a9a; }
.reasoning-line.r-file-written { color: #4a7abf; }

/* Project card — shown during coding task execution */
.project-card {
  background: var(--bg-elevated, #1e2228);
  border: 1px solid var(--border, #333);
  border-left: 3px solid #4a8abf;
  border-radius: 6px;
  padding: 8px 12px;
  margin: 6px 0;
  font-size: 13px;
}
.pc-header { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
.pc-lang {
  background: #2a3a5a;
  color: #7ab0e0;
  border-radius: 3px;
  padding: 1px 6px;
  font-size: 11px;
  font-weight: 600;
  text-transform: uppercase;
}
.pc-status {
  border-radius: 3px;
  padding: 1px 6px;
  font-size: 11px;
  font-weight: 600;
  text-transform: uppercase;
}
.pc-success { background: #1a3a2a; color: #4a9a5a; }
.pc-partial  { background: #3a3010; color: #c09030; }
.pc-failed   { background: #3a1010; color: #c04a4a; }
.pc-building { background: #1a2a3a; color: #4a7abf; }
.pc-stats { color: var(--text-dim, #888); font-size: 12px; margin-top: 3px; }
.pc-stats a { color: #4a8abf; text-decoration: none; }
.pc-stats a:hover { text-decoration: underline; }

/* Collapsible reasoning toggle on completed turns */
.reasoning-toggle {
  font-size: 11px;
  color: var(--text-dim);
  cursor: pointer;
  margin-top: 6px;
  display: inline-flex;
  align-items: center;
  gap: 4px;
  user-select: none;
}
.reasoning-toggle:hover { color: var(--text-muted); }
.reasoning-full {
  display: none;
  margin-top: 6px;
  font-size: 12px;
  color: var(--text-dim);
  border-left: 2px solid var(--border);
  padding-left: 10px;
  white-space: pre-wrap;
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

/* ── Approval modal ── */
.approval-modal {
  position: fixed;
  top: 0; left: 0; right: 0; bottom: 0;
  background: rgba(0,0,0,0.7);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
}
.approval-card {
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: 12px;
  padding: 24px;
  max-width: 440px;
  width: 90%;
}
.approval-card h3 { font-size: 16px; margin-bottom: 8px; color: var(--text); }
.approval-card .risk-badge {
  display: inline-block;
  background: rgba(200,60,60,0.15);
  color: #c83c3c;
  border: 1px solid rgba(200,60,60,0.3);
  border-radius: 4px;
  padding: 2px 8px;
  font-size: 12px;
  margin-bottom: 12px;
}
.approval-card .action-desc {
  font-size: 13px;
  color: var(--text-muted);
  background: var(--surface2);
  border-radius: 6px;
  padding: 10px 12px;
  margin-bottom: 16px;
  font-family: monospace;
}
.approval-btns { display: flex; gap: 10px; justify-content: flex-end; }
.btn-approve {
  background: #2a5c2a;
  color: #7fcc7f;
  border: 1px solid #3a7c3a;
  border-radius: 6px;
  padding: 8px 18px;
  cursor: pointer;
  font-size: 14px;
}
.btn-approve:hover { background: #336633; }
.btn-reject {
  background: var(--surface2);
  color: var(--text-muted);
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 8px 18px;
  cursor: pointer;
  font-size: 14px;
}
.btn-reject:hover { background: var(--surface); color: var(--text); }

/* ── Terminal Panel ── */
#terminal-drawer {
  position: fixed;
  bottom: 0; left: 0; right: 0;
  height: 220px;
  background: #0d0d0d;
  border-top: 1px solid #2a2a2a;
  display: flex;
  flex-direction: column;
  transform: translateY(100%);
  transition: transform 0.2s ease;
  z-index: 200;
  font-family: 'Cascadia Code', 'Fira Code', 'Consolas', monospace;
  font-size: 12px;
}
#terminal-drawer.open { transform: translateY(0); }

#terminal-header {
  display: flex;
  align-items: center;
  padding: 6px 12px;
  background: #111;
  border-bottom: 1px solid #222;
  gap: 8px;
  flex-shrink: 0;
  cursor: pointer;
  user-select: none;
}
#terminal-header .term-title {
  color: #5a5a5a;
  font-size: 11px;
  letter-spacing: 0.05em;
  text-transform: uppercase;
  flex: 1;
}
#terminal-header .term-dots { display: flex; gap: 5px; }
#terminal-header .term-dots span {
  width: 10px; height: 10px; border-radius: 50%;
}
#terminal-header .term-dots .d-red { background: #3a1a1a; }
#terminal-header .term-dots .d-yellow { background: #3a3a1a; }
#terminal-header .term-dots .d-green { background: #1a3a1a; }
#terminal-header .term-dots.active .d-red { background: #ff5f57; }
#terminal-header .term-dots.active .d-yellow { background: #febc2e; }
#terminal-header .term-dots.active .d-green { background: #28c840; }

#terminal-body {
  flex: 1;
  overflow-y: auto;
  padding: 8px 12px;
  color: #c8c8c8;
  line-height: 1.5;
}
#terminal-body::-webkit-scrollbar { width: 4px; }
#terminal-body::-webkit-scrollbar-track { background: #0d0d0d; }
#terminal-body::-webkit-scrollbar-thumb { background: #2a2a2a; }

.term-cwd { color: #3a3a5a; font-size: 10px; padding-bottom: 1px; }
.term-cwd::before { content: "📁 "; }
.term-cmd { color: #50fa7b; }
.term-cmd::before { content: "$ "; color: #6272a4; }
.term-stdout { color: #c8c8c8; white-space: pre-wrap; word-break: break-all; }
.term-stderr { color: #ff5555; white-space: pre-wrap; word-break: break-all; }
.term-exit-ok { color: #50fa7b; font-size: 10px; }
.term-exit-fail { color: #ff5555; font-size: 10px; }
.term-sep { border: none; border-top: 1px solid #1a1a1a; margin: 6px 0; }

/* Terminal toggle button in top bar */
#term-toggle {
  background: none;
  border: 1px solid var(--border);
  color: var(--text-muted);
  cursor: pointer;
  padding: 4px 8px;
  border-radius: 6px;
  font-size: 11px;
  display: flex;
  align-items: center;
  gap: 5px;
  flex-shrink: 0;
  transition: all 0.15s;
  font-family: 'Cascadia Code', 'Consolas', monospace;
}
#term-toggle:hover { background: var(--surface2); color: var(--text); }
#term-toggle.has-output { border-color: #28c840; color: #28c840; }

/* Push chat area up when terminal is open */
body.terminal-open #chat-area { padding-bottom: 230px; }

/* ── Planning mode toggle ── */
#plan-toggle {
  background: none;
  border: 1px solid var(--border);
  color: var(--text-muted);
  cursor: pointer;
  padding: 4px 8px;
  border-radius: 6px;
  font-size: 11px;
  flex-shrink: 0;
  transition: all 0.15s;
  display: flex;
  align-items: center;
  gap: 5px;
}
#plan-toggle:hover { background: var(--surface2); }
#plan-toggle.active {
  background: rgba(90,120,200,0.15);
  border-color: rgba(90,120,200,0.5);
  color: #8aaaee;
}

/* ── Planning questionnaire overlay ── */
#plan-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0,0,0,0.75);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1100;
  backdrop-filter: blur(2px);
  opacity: 0;
  pointer-events: none;
  transition: opacity 0.18s ease;
}
#plan-overlay.open {
  opacity: 1;
  pointer-events: all;
}
#plan-card {
  background: var(--surface);
  border: 1px solid #3a3a4a;
  border-radius: 14px;
  padding: 28px 28px 22px;
  max-width: 520px;
  width: 92%;
  max-height: 80vh;
  overflow-y: auto;
  transform: translateY(12px);
  transition: transform 0.18s ease;
}
#plan-overlay.open #plan-card { transform: translateY(0); }
#plan-card h3 {
  font-size: 15px;
  font-weight: 600;
  color: var(--text);
  margin-bottom: 4px;
  display: flex;
  align-items: center;
  gap: 8px;
}
#plan-card .plan-subtitle {
  font-size: 12px;
  color: var(--text-muted);
  margin-bottom: 20px;
}
.plan-question {
  margin-bottom: 20px;
}
.plan-question-text {
  font-size: 13px;
  color: var(--text);
  margin-bottom: 10px;
  font-weight: 500;
}
.plan-options {
  display: flex;
  flex-wrap: wrap;
  gap: 7px;
}
.plan-option {
  padding: 6px 13px;
  border-radius: 20px;
  border: 1px solid var(--border);
  background: var(--surface2);
  color: var(--text-muted);
  font-size: 12px;
  cursor: pointer;
  transition: all 0.12s;
  user-select: none;
}
.plan-option:hover { border-color: #5a7abe; color: var(--text); }
.plan-option.selected {
  background: rgba(90,120,200,0.2);
  border-color: #5a7abe;
  color: #aac0f8;
}
.plan-custom-wrap {
  display: none;
  margin-top: 8px;
}
.plan-custom-wrap.visible { display: block; }
.plan-custom-input {
  width: 100%;
  background: var(--surface2);
  border: 1px solid var(--border);
  border-radius: 8px;
  color: var(--text);
  font-size: 13px;
  padding: 8px 12px;
  outline: none;
  font-family: inherit;
  transition: border-color 0.15s;
}
.plan-custom-input:focus { border-color: #5a7abe; }
.plan-custom-input::placeholder { color: var(--text-dim); }
.plan-actions {
  display: flex;
  gap: 10px;
  justify-content: flex-end;
  margin-top: 22px;
  padding-top: 16px;
  border-top: 1px solid var(--border);
}
.btn-plan-run {
  background: #2a3c7a;
  color: #aac0f8;
  border: 1px solid #3a4c9a;
  border-radius: 6px;
  padding: 9px 20px;
  cursor: pointer;
  font-size: 13px;
  font-weight: 500;
  transition: background 0.15s;
}
.btn-plan-run:hover { background: #354a9a; }
.btn-plan-run:disabled { opacity: 0.4; cursor: default; }
.btn-plan-cancel {
  background: var(--surface2);
  color: var(--text-muted);
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 9px 16px;
  cursor: pointer;
  font-size: 13px;
}
.btn-plan-cancel:hover { background: var(--surface); color: var(--text); }
.plan-loading {
  text-align: center;
  padding: 32px 0 24px;
  color: var(--text-muted);
  font-size: 14px;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 14px;
}

/* Admin mode toggle */
#admin-toggle {
  background: none;
  border: 1px solid var(--border);
  color: var(--text-muted);
  cursor: pointer;
  padding: 4px 8px;
  border-radius: 6px;
  font-size: 11px;
  flex-shrink: 0;
  transition: all 0.15s;
  display: flex;
  align-items: center;
  gap: 5px;
}
#admin-toggle:hover { background: var(--surface2); }
#admin-toggle.active {
  background: rgba(200,60,60,0.15);
  border-color: rgba(200,60,60,0.5);
  color: #e07070;
}
body.admin-active { outline: 2px solid rgba(200,60,60,0.3); }

/* ── File editor panel ── */
#file-editor {
  position: fixed;
  top: 0; right: 0;
  width: 520px;
  height: 100%;
  background: #111;
  border-left: 1px solid var(--border);
  z-index: 290;
  display: flex;
  flex-direction: column;
  transform: translateX(100%);
  transition: transform 0.22s ease;
}
#file-editor.open { transform: translateX(0); }
/* Nudge settings panel left when editor is also open */
#file-editor.open ~ #settings-panel { transform: translateX(calc(-520px)); }

#editor-header {
  padding: 10px 14px;
  border-bottom: 1px solid var(--border);
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
  background: #161616;
}
#editor-filename {
  font-size: 13px;
  color: var(--text);
  font-family: 'Cascadia Code', 'Consolas', monospace;
  flex: 1;
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
#editor-dirty {
  font-size: 13px;
  color: var(--accent);
  opacity: 0;
  transition: opacity 0.15s;
}
#editor-dirty.visible { opacity: 1; }
.editor-btn {
  background: none;
  border: 1px solid var(--border);
  color: var(--text-muted);
  cursor: pointer;
  padding: 4px 10px;
  border-radius: 5px;
  font-size: 12px;
  flex-shrink: 0;
  transition: all 0.12s;
}
.editor-btn:hover { background: var(--surface2); color: var(--text); }
.editor-btn.save {
  background: #1a3a1a;
  border-color: #2a5a2a;
  color: #5a9a5a;
}
.editor-btn.save:hover { background: #224422; }
.editor-btn.close { border: none; color: var(--text-dim); font-size: 15px; padding: 4px 8px; }

#editor-body {
  flex: 1;
  display: flex;
  overflow: hidden;
}
#editor-textarea {
  flex: 1;
  background: #0d0d0d;
  color: #c9c5be;
  border: none;
  outline: none;
  resize: none;
  font-family: 'Cascadia Code', 'Fira Code', 'Consolas', monospace;
  font-size: 13px;
  line-height: 1.6;
  padding: 14px 16px;
  tab-size: 2;
  white-space: pre;
  overflow-wrap: normal;
  overflow-x: auto;
}
#editor-statusbar {
  padding: 4px 14px;
  background: #161616;
  border-top: 1px solid var(--border);
  font-size: 11px;
  color: var(--text-dim);
  display: flex;
  gap: 14px;
  flex-shrink: 0;
  font-family: monospace;
}
#editor-statusbar .refreshed {
  color: #4a9a5a;
  opacity: 0;
  transition: opacity 0.3s;
}
#editor-statusbar .refreshed.show { opacity: 1; }

/* ── Settings panel ── */
#settings-toggle {
  background: none;
  border: 1px solid var(--border);
  color: var(--text-muted);
  cursor: pointer;
  padding: 4px 8px;
  border-radius: 6px;
  font-size: 13px;
  flex-shrink: 0;
  transition: all 0.15s;
  line-height: 1;
}
#settings-toggle:hover { background: var(--surface2); color: var(--text); }

#settings-panel {
  position: fixed;
  top: 0; right: 0;
  width: 340px;
  height: 100%;
  background: #161616;
  border-left: 1px solid var(--border);
  z-index: 300;
  display: flex;
  flex-direction: column;
  transform: translateX(100%);
  transition: transform 0.22s ease;
  overflow: hidden;
}
#settings-panel.open { transform: translateX(0); }

#settings-header {
  padding: 16px 18px 12px;
  border-bottom: 1px solid var(--border);
  display: flex;
  align-items: center;
  gap: 10px;
  flex-shrink: 0;
}
#settings-header h2 {
  font-size: 14px;
  font-weight: 600;
  color: var(--text);
  flex: 1;
}
#settings-close {
  background: none;
  border: none;
  color: var(--text-dim);
  cursor: pointer;
  font-size: 18px;
  padding: 2px 6px;
  border-radius: 4px;
  line-height: 1;
  transition: all 0.12s;
}
#settings-close:hover { background: var(--surface2); color: var(--text); }

#settings-body {
  flex: 1;
  overflow-y: auto;
  padding: 18px;
}
#settings-body::-webkit-scrollbar { width: 4px; }
#settings-body::-webkit-scrollbar-thumb { background: var(--border); border-radius: 2px; }

.settings-section {
  margin-bottom: 22px;
}
.settings-section-label {
  font-size: 10px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.08em;
  color: var(--text-dim);
  margin-bottom: 10px;
}
.settings-field {
  margin-bottom: 14px;
}
.settings-field label {
  display: block;
  font-size: 12px;
  color: var(--text-muted);
  margin-bottom: 5px;
}
.settings-field input[type="text"],
.settings-field input[type="number"] {
  width: 100%;
  background: var(--surface2);
  border: 1px solid var(--border);
  border-radius: 6px;
  color: var(--text);
  font-size: 13px;
  padding: 7px 10px;
  outline: none;
  font-family: 'Cascadia Code', 'Consolas', monospace;
  transition: border-color 0.15s;
}
.settings-field input:focus { border-color: #555; }
.settings-field input::placeholder { color: var(--text-dim); }
.settings-field .field-hint {
  font-size: 11px;
  color: var(--text-dim);
  margin-top: 4px;
}
.settings-field input[type="range"] {
  width: 100%;
  accent-color: var(--accent);
}
.settings-range-row {
  display: flex;
  align-items: center;
  gap: 10px;
}
.settings-range-val {
  font-size: 13px;
  color: var(--text);
  font-family: monospace;
  min-width: 20px;
  text-align: right;
}

#settings-footer {
  padding: 14px 18px;
  border-top: 1px solid var(--border);
  display: flex;
  gap: 10px;
  align-items: center;
  flex-shrink: 0;
}
.btn-settings-save {
  background: var(--accent);
  color: #fff;
  border: none;
  border-radius: 6px;
  padding: 8px 18px;
  font-size: 13px;
  cursor: pointer;
  transition: opacity 0.15s;
}
.btn-settings-save:hover { opacity: 0.85; }
.settings-save-msg {
  font-size: 12px;
  color: #5a9a5a;
  opacity: 0;
  transition: opacity 0.3s;
}
.settings-save-msg.visible { opacity: 1; }
</style>
<!-- KaTeX — math rendering for LaTeX expressions in AI responses -->
<link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/katex@0.16.11/dist/katex.min.css">
<script src="https://cdn.jsdelivr.net/npm/katex@0.16.11/dist/katex.min.js"></script>
</head>
<body>

<!-- Sidebar -->
<div id="sidebar">
  <div id="sidebar-header">
    <span class="logo">aihomeserver</span>
    <button id="new-chat" onclick="newChat()">+ New</button>
  </div>

  <!-- Tab bar -->
  <div id="sidebar-tabs">
    <button class="tab-btn active" id="tab-btn-chat"  onclick="showSidebarTab('chat')">Chat</button>
    <button class="tab-btn"        id="tab-btn-files" onclick="showSidebarTab('files')">Files</button>
    <button class="tab-btn"        id="tab-btn-git"   onclick="showSidebarTab('git')">Git</button>
    <button class="tab-btn"        id="tab-btn-mem"   onclick="showSidebarTab('mem')">Mem</button>
    <button class="tab-btn"        id="tab-btn-kb"    onclick="showSidebarTab('kb')">KB</button>
  </div>

  <!-- Chat tab -->
  <div id="tab-chat">
    <div id="sidebar-section">Recent</div>
    <div id="history-list"></div>
    <div class="sidebar-divider" id="archived-toggle" onclick="toggleArchived()">
      <svg class="chevron" viewBox="0 0 10 10" width="10" height="10" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M3 2l4 3-4 3"/></svg>
      Archived
    </div>
    <div id="archived-list" style="display:none"></div>
  </div>

  <!-- Files tab -->
  <div id="tab-files">
    <div id="file-tree-toolbar">
      <span class="tree-root-label" id="tree-root-label">workspace</span>
      <button class="tree-refresh-btn" onclick="showNewMenu(event)" title="New file or folder">+</button>
      <button class="tree-refresh-btn" onclick="openUploadPicker()" title="Upload files">⤒</button>
      <button class="tree-refresh-btn" onclick="runSelfCheck()" title="Diagnostics">✓</button>
      <button class="tree-refresh-btn" onclick="loadFileTree()" title="Refresh">↻</button>
    </div>
    <div id="file-tree">
      <div class="tree-empty">Switch to Files tab to browse your workspace</div>
    </div>
    <input id="upload-input" type="file" multiple style="display:none" />
  </div>

  <!-- Memory tab -->
  <div id="tab-mem">
    <div id="mem-toolbar">
      <input type="text" id="mem-search" placeholder="Filter tasks…" oninput="filterMemTasks()" />
      <button class="tree-refresh-btn" onclick="loadMemory()" title="Refresh">↻</button>
    </div>
    <div id="mem-list">
      <div class="mem-empty">Switch to Mem tab to browse task history.</div>
    </div>
    <div id="mem-footer">
      <span id="mem-semantic-count">0 in semantic index</span>
      <button class="mem-clear-btn" onclick="clearAllMemory()">Clear All</button>
    </div>
  </div>

  <!-- Git tab -->
  <div id="tab-git">
    <div id="git-branch-bar">
      <span id="git-branch-name">─ no repo ─</span>
      <button class="git-refresh-btn" onclick="loadGitStatus()" title="Refresh">↻</button>
    </div>
    <div id="git-body">
      <div id="git-empty">Switch to Git tab to view repository status.</div>
    </div>
    <div id="git-commit-form" style="display:none">
      <textarea id="git-commit-msg" rows="2" placeholder="Commit message…"></textarea>
      <div class="git-commit-row">
        <button class="git-commit-btn" id="git-commit-btn" onclick="commitChanges()">Commit</button>
        <span id="git-commit-status"></span>
      </div>
    </div>
  </div>

  <!-- Knowledge Base tab -->
  <div id="tab-kb">
    <div id="kb-toolbar">
      <input type="text" id="kb-search" placeholder="Filter topics…" oninput="filterKb()" />
      <button class="kb-add-btn" onclick="openKbModal(null)">+ Add</button>
      <button class="kb-add-btn" style="background:#2a2a2a;color:var(--text-muted);border:1px solid var(--border)" onclick="exportKbAll()">Export</button>
      <button class="tree-refresh-btn" onclick="loadKb()" title="Refresh">↻</button>
    </div>
    <div id="kb-list">
      <div style="padding:16px;color:var(--text-muted);font-size:12px;text-align:center">
        Switch to KB tab to browse stored research.
      </div>
    </div>
    <div id="kb-footer">
      <span id="kb-count">0 topics</span>
    </div>
  </div>

  <!-- Resize handle -->
  <div id="sidebar-resize" title="Drag to resize"></div>

  <div id="sidebar-footer">
    <div style="color:var(--text-muted);font-size:12px">Models</div>
    <div class="model-tag" id="footer-fast-model">— · fast</div>
    <div class="model-tag" id="footer-critic-model" style="margin-top:4px">— · critic</div>
  </div>
</div>

<!-- Main -->
<div id="main">
  <div id="top-bar">
    <button id="sidebar-toggle" onclick="toggleSidebar()" title="Toggle sidebar">
      <svg viewBox="0 0 18 18"><line x1="2" y1="4" x2="16" y2="4"/><line x1="2" y1="9" x2="16" y2="9"/><line x1="2" y1="14" x2="16" y2="14"/></svg>
    </button>
    <button id="plan-toggle" onclick="togglePlanMode()" title="Plan mode — ask questions before running">&#x1F9E0; Plan</button>
    <button id="term-toggle" onclick="toggleTerminal()" title="Toggle terminal">&#x2328; Terminal</button>
    <button id="admin-toggle" onclick="toggleAdmin()" title="Admin mode — bypasses approval gate">&#x26A1; Admin</button>
    <span class="title" id="top-title">New conversation</span>
    <span class="status-pill" id="status-pill">● online</span>
    <button id="search-btn" onclick="openSearch()" title="Search workspace (Ctrl+K)" style="background:none;border:1px solid var(--border);color:var(--text-muted);cursor:pointer;padding:4px 8px;border-radius:6px;font-size:12px;flex-shrink:0;transition:all .15s;display:flex;align-items:center;gap:4px">🔍 Search</button>
    <a id="learn-btn" href="/learn" title="Interview-mode Learn site" style="text-decoration:none;background:none;border:1px solid var(--border);color:var(--text-muted);cursor:pointer;padding:4px 8px;border-radius:6px;font-size:12px;flex-shrink:0;transition:all .15s;display:flex;align-items:center;gap:4px">📚 Learn</a>
    <button id="export-btn" onclick="exportConversation()" title="Export conversation as Markdown" style="background:none;border:1px solid var(--border);color:var(--text-muted);cursor:pointer;padding:4px 8px;border-radius:6px;font-size:13px;flex-shrink:0;transition:all .15s;line-height:1">⬇</button>
    <button id="settings-toggle" onclick="toggleSettings()" title="Settings">⚙</button>
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
  <button id="jump-bottom" onclick="jumpToBottom()">↓ New messages</button>

  <div id="input-wrap">
    <div id="context-bar"></div>
    <div id="input-box">
      <textarea id="input" rows="1" placeholder="Message aihomeserver…" onkeydown="handleKey(event)"></textarea>
      <button id="send-btn" onclick="send()" disabled>
        <svg viewBox="0 0 24 24"><line x1="12" y1="19" x2="12" y2="5"/><polyline points="5 12 12 5 19 12"/></svg>
      </button>
    </div>
    <div id="input-hint">Enter to send &nbsp;·&nbsp; Shift+Enter for new line</div>
  </div>
</div>

<!-- Planning questionnaire overlay -->
<div id="plan-overlay">
  <div id="plan-card">
    <h3>🧠 Plan before running</h3>
    <div class="plan-subtitle" id="plan-subtitle">Loading questions…</div>
    <div id="plan-body">
      <div class="plan-loading">
        <div class="dots"><span></span><span></span><span></span></div>
        Generating questions…
      </div>
    </div>
    <div class="plan-actions">
      <button class="btn-plan-cancel" onclick="closePlanOverlay()">Cancel</button>
      <button class="btn-plan-run" id="plan-run-btn" onclick="runWithPlan()" disabled>Run with plan ➜</button>
    </div>
  </div>
</div>

<!-- File editor panel -->
<div id="file-editor">
  <div id="editor-header">
    <span id="editor-filename">untitled</span>
    <span id="editor-dirty">●</span>
    <button class="editor-btn save" onclick="saveEditorFile()" title="Save (Ctrl+S)">Save</button>
    <button class="editor-btn close" onclick="closeFileEditor()">✕</button>
  </div>
  <div id="editor-body">
    <textarea id="editor-textarea" spellcheck="false" oninput="markEditorDirty()"></textarea>
  </div>
  <div id="editor-statusbar">
    <span id="editor-linecount">0 lines</span>
    <span id="editor-size">0 B</span>
    <span id="editor-refreshed" class="refreshed">↻ refreshed</span>
  </div>
</div>

<!-- Settings panel -->
<div id="settings-panel">
  <div id="settings-header">
    <h2>⚙ Settings</h2>
    <button id="settings-close" onclick="toggleSettings()">✕</button>
  </div>
  <div id="settings-body">
    <div class="settings-section">
      <div class="settings-section-label">Workspace</div>
      <div class="settings-field">
        <label>Workspace path</label>
        <input type="text" id="cfg-workspace" placeholder="C:\Users\you\workspace" />
        <div class="field-hint">Default directory for shell commands and file operations</div>
      </div>
    </div>
    <div class="settings-section">
      <div class="settings-section-label">Ollama</div>
      <div class="settings-field">
        <label>Ollama URL</label>
        <input type="text" id="cfg-ollama-url" placeholder="http://localhost:11434" />
      </div>
      <div class="settings-field">
        <label>Fast model <span style="color:var(--text-dim)">(planner · executor · repair)</span></label>
        <input type="text" id="cfg-fast-model" placeholder="qwen2.5:14b" />
      </div>
      <div class="settings-field">
        <label>Critic model <span style="color:var(--text-dim)">(high-risk validation)</span></label>
        <input type="text" id="cfg-critic-model" placeholder="qwen2.5:32b" />
      </div>
    </div>
    <div class="settings-section">
      <div class="settings-section-label">Performance</div>
      <div class="settings-field">
        <label>GPU layers <span style="color:var(--text-dim)">(num_gpu)</span></label>
        <input type="number" id="cfg-num-gpu" min="0" max="999" />
        <div class="field-hint">999 = all layers on GPU (max speed). 0 = CPU only.</div>
      </div>
      <div class="settings-field">
        <label>Context window <span style="color:var(--text-dim)">(tokens)</span></label>
        <input type="number" id="cfg-num-ctx" min="2048" max="65536" step="1024" />
        <div class="field-hint">4096 default (fast) · 8192 for deep research · needs VRAM to match</div>
      </div>
      <div class="settings-field">
        <label>Max output tokens <span style="color:var(--text-dim)">(num_predict)</span></label>
        <input type="number" id="cfg-num-predict" min="128" max="16384" step="128" />
        <div class="field-hint">Hard cap on response length. Raise this if answers feel shallow/cut off.</div>
      </div>
      <div class="settings-field">
        <label>Batch size <span style="color:var(--text-dim)">(num_batch)</span></label>
        <input type="number" id="cfg-num-batch" min="64" max="2048" step="64" />
        <div class="field-hint">512 default · higher = GPU processes more tokens per pass (needs VRAM)</div>
      </div>
      <div class="settings-field">
        <label>CPU threads <span style="color:var(--text-dim)">(0 = auto)</span></label>
        <input type="number" id="cfg-num-thread" min="0" max="128" />
        <div class="field-hint">0 lets Ollama pick the optimal thread count automatically</div>
      </div>
    </div>
    <div class="settings-section">
      <div class="settings-section-label">Search</div>
      <div class="settings-field">
        <label>Search engine URL <span style="color:var(--text-dim)">(optional)</span></label>
        <input type="text" id="cfg-search-url" placeholder="http://localhost:8080" />
        <div class="field-hint">SearXNG base URL — leave empty to use built-in DuckDuckGo scraping</div>
      </div>
    </div>
    <div class="settings-section">
      <div class="settings-section-label">Behaviour</div>
      <div class="settings-field">
        <label>Max steps per task</label>
        <input type="number" id="cfg-max-steps" min="1" max="50" />
        <div class="field-hint">Hard cap before the orchestrator forces finalization</div>
      </div>
      <div class="settings-field">
        <label>Risk gate threshold &nbsp;<span style="color:var(--text-dim)">— tasks scored ≥ this require approval</span></label>
        <div class="settings-range-row">
          <input type="range" id="cfg-risk-threshold" min="1" max="10" step="1"
            oninput="document.getElementById('cfg-risk-val').textContent=this.value" />
          <span class="settings-range-val" id="cfg-risk-val">8</span>
        </div>
        <div class="field-hint">1 = approve everything · 10 = never ask</div>
      </div>
      <div class="settings-field">
        <label>Auto-save to KB</label>
        <select id="cfg-auto-kb-mode" style="background:var(--surface2);border:1px solid var(--border);border-radius:6px;color:var(--text);padding:6px 8px;font-size:12px">
          <option value="off">Off</option>
          <option value="research">Research only</option>
          <option value="always">Always (substantial answers)</option>
        </select>
        <div class="field-hint">Saves useful outputs into KB after the answer returns (background).</div>
      </div>
      <div class="settings-field">
        <label>Auto-save min chars</label>
        <input type="number" id="cfg-auto-kb-min" min="200" max="50000" />
        <div class="field-hint">Skip auto-save when the content is shorter than this.</div>
      </div>
    </div>
  </div>
  <div id="settings-footer">
    <button class="btn-settings-save" onclick="saveSettings()">Save</button>
    <span class="settings-save-msg" id="settings-save-msg">✓ Saved</span>
  </div>
</div>

<!-- Knowledge Base add/edit modal -->
<div id="kb-modal">
  <div id="kb-modal-card">
    <h3 id="kb-modal-title">Add Knowledge Entry</h3>
    <input type="text"   id="kb-modal-topic"   placeholder="Topic (e.g. Kez Dota 2 hero)" />
    <input type="text"   id="kb-modal-tags"    placeholder="Tags, comma separated (e.g. dota2,gaming,kez)" />
    <input type="text"   id="kb-modal-summary" placeholder="Short summary (1-3 sentences)" />
    <textarea id="kb-modal-content" placeholder="Full research content…"></textarea>
    <input type="hidden" id="kb-modal-id" value="" />
    <div id="kb-modal-footer">
      <button class="kb-modal-cancel" onclick="closeKbModal()">Cancel</button>
      <button class="kb-modal-save"   onclick="saveKbEntry()">Save</button>
    </div>
  </div>
</div>

<!-- Workspace search overlay -->
<div id="search-overlay">
  <div id="search-card">
    <div id="search-input-row">
      <span class="search-icon">🔍</span>
      <input type="text" id="search-input" placeholder="Search workspace files…" oninput="doSearch()" autocomplete="off" spellcheck="false" />
      <button id="search-close" onclick="closeSearch()">✕</button>
    </div>
    <div id="search-results">
      <div class="search-empty">Type to search file contents</div>
    </div>
    <div id="search-footer">↵ open in editor &nbsp;·&nbsp; Esc close</div>
  </div>
</div>

<!-- New file/folder dropdown -->
<div id="new-menu" class="ctx-menu" style="display:none">
  <div class="ctx-menu-item" onclick="promptNewFile()">📄 New File</div>
  <div class="ctx-menu-item" onclick="promptNewFolder()">📁 New Folder</div>
</div>

<!-- File/folder context menu -->
<div id="ctx-menu" class="ctx-menu" style="display:none">
  <div class="ctx-menu-item" id="ctx-download" onclick="ctxDownload()">Download</div>
  <hr class="ctx-menu-sep" id="ctx-sep1">
  <div class="ctx-menu-item" onclick="ctxRename()">Rename</div>
  <hr class="ctx-menu-sep">
  <div class="ctx-menu-item danger" onclick="ctxDelete()">Delete</div>
</div>

<div id="terminal-drawer">
  <div id="terminal-header" onclick="toggleTerminal()">
    <div class="term-dots" id="term-dots">
      <span class="d-red"></span><span class="d-yellow"></span><span class="d-green"></span>
    </div>
    <span class="term-title">Terminal</span>
    <span style="color:#3a3a3a;font-size:11px" id="term-hint">click to open</span>
  </div>
  <div id="terminal-body"></div>
</div>

<script>
const msgsEl = document.getElementById('messages');
const jumpBottomBtn = document.getElementById('jump-bottom');

let autoScroll = true;
let activeSidebarTab = 'chat';

function isNearBottom() {
  const threshold = 90;
  return (msgsEl.scrollTop + msgsEl.clientHeight) >= (msgsEl.scrollHeight - threshold);
}

msgsEl.addEventListener('scroll', () => {
  autoScroll = isNearBottom();
  if (autoScroll && jumpBottomBtn) jumpBottomBtn.style.display = 'none';
});

function jumpToBottom() {
  autoScroll = true;
  if (jumpBottomBtn) jumpBottomBtn.style.display = 'none';
  scrollBottom(true);
}

function setTabBadge(tab, text, cls) {
  const btn = document.getElementById('tab-btn-' + tab);
  if (!btn) return;
  if (!btn.dataset.label) btn.dataset.label = btn.textContent.trim();
  // Clear
  if (!text) {
    btn.innerHTML = esc(btn.dataset.label);
    return;
  }
  const badge = `<span class="tab-badge ${cls||''}">${esc(text)}</span>`;
  btn.innerHTML = `${esc(btn.dataset.label)}${badge}`;
}
const inputEl = document.getElementById('input');
const sendBtn = document.getElementById('send-btn');
let busy = false;
let currentSessionId = null;  // tracks the active session across turns

// Per-session status badges for the sidebar list.
// key: session_id (string) -> { text, cls }
const sessionBadges = new Map();
let activeRunSessionId = null;

function setSessionBadge(sessionId, text, cls) {
  if (!sessionId) return;
  if (!text) sessionBadges.delete(sessionId);
  else sessionBadges.set(sessionId, { text, cls: cls || '' });

  // Update in-place if the item exists; otherwise it will be applied on next refresh.
  const item = document.querySelector(`.history-item[data-sid="${sessionId}"]`);
  if (!item) return;

  const existing = item.querySelector('.sess-badge');
  if (!text) {
    if (existing) existing.remove();
    return;
  }

  const el = existing || document.createElement('span');
  el.className = `sess-badge ${cls || ''}`.trim();
  el.textContent = text;
  if (!existing) {
    const actions = item.querySelector('.item-actions');
    if (actions) item.insertBefore(el, actions);
    else item.appendChild(el);
  }
}

// Run persistence + queueing:
// - Keep receiving stream events even if you navigate away from the running chat.
// - Re-render the in-progress turn when you come back.
// - Allow enqueueing messages while another run is busy (runs execute sequentially).
const runStates = new Map();      // runId -> state
const sessionActiveRun = new Map(); // sessionId -> runId
const sendQueue = [];            // FIFO: { session_id, text, answers, ctxPrefix }
let activeRunId = null;

function ensureRunTurnVisible(runId) {
  const st = runStates.get(runId);
  if (!st || !st.session_id) return null;
  if (currentSessionId !== st.session_id) return null;

  let el = document.querySelector(`.turn.ai[data-run-id="${runId}"]`);
  if (!el) {
    el = document.createElement('div');
    el.className = 'turn ai';
    el.dataset.runId = runId;
    el.innerHTML = `<div class="ai-label">aihomeserver</div><div class="ai-content"></div>`;
    msgsEl.appendChild(el);
  }
  return el;
}

function renderRunState(runId) {
  const st = runStates.get(runId);
  if (!st) return;

  const el = ensureRunTurnVisible(runId);
  if (!el) return;
  const content = el.querySelector('.ai-content');
  if (!content) return;

  const statusText = st.done
    ? ((st.success === false || st.error) ? '✗ failed' : '✓ done')
    : (st.error ? '✗ failed' : phaseLabel(st.phase || 'thinking'));
  const dotsHtml = st.done || st.error ? '' : '<div class="dots"><span></span><span></span><span></span></div>';
  const headerHtml = `<div class="thinking">${dotsHtml}${esc(statusText)}</div>`;

  let cotHtml = '';
  if (st.thinking_text) {
    const approxTok = Math.round(st.thinking_text.length / 4);
    cotHtml = `<details class="thinking-details"><summary>🧠 Thought for ~${approxTok} tokens</summary><div class="thinking-trace">${esc(st.thinking_text)}</div></details>`;
  }

  let reasoningHtml = '';
  if (st.reasoning_lines && st.reasoning_lines.length > 0) {
    const uid = `r_${runId}`;
    reasoningHtml = `
      <span class="reasoning-toggle" onclick="(function(){const p=document.getElementById('${uid}'); if(!p) return; const open=p.style.display==='block'; p.style.display=open?'none':'block';})()">
        <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M2 3l3 3 3-3"/></svg> Show reasoning
      </span>
      <div class="reasoning-full" id="${uid}" style="display:none">${esc(st.reasoning_lines.join('\n'))}</div>`;
  }

  const bodyHtml = st.done
    ? `<div>${renderMarkdown(st.final_answer || '')}</div>`
    : (st.streaming_started
        ? `<div class="streaming-content">${esc(st.full_answer || '')}</div>`
        : '');

  content.innerHTML = headerHtml + cotHtml + bodyHtml + reasoningHtml;
  scrollBottom();
}

function maybeShowActiveRunForCurrentSession() {
  if (!currentSessionId) return;
  const rid = sessionActiveRun.get(currentSessionId);
  if (rid) renderRunState(rid);
}

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
function phaseLabel(phase) {
  if (phase === 'planning') return '🧠 Planning...';
  if (phase.startsWith('executing')) return '⚙️ Executing...';
  if (phase === 'critic') return '🔍 Reviewing...';
  return '💭 Thinking...';
}

async function send() {
  const text = inputEl.value.trim();
  if (!text) return;
  if (busy && !planMode) {
    // Queue the request (sequential execution).
    const prefix = buildContextPrefix();
    const fullRequest = prefix ? prefix + text : text;
    const fileNames = Object.values(attachedFiles).map(f => f.name);
    addUserTurn(text, fileNames);
    attachedFiles = {};
    renderContextBar();
    inputEl.value = '';
    inputEl.style.height = 'auto';

    const qEl = addThinkingTurn();
    qEl.dataset.queue = '1';
    const qContent = qEl.querySelector('.ai-content');
    if (qContent) {
      qContent.innerHTML = `<div class="thinking">⏳ Queued…</div><div style="margin-top:6px;color:var(--text-dim);font-size:12px">Will run when the current task finishes.</div>`;
    }

    sendQueue.push({ session_id: currentSessionId, request: fullRequest, answers: null, queued_el: qEl });
    if (currentSessionId) setSessionBadge(currentSessionId, '⏳', 'running');
    return;
  }

  // In plan mode: show questionnaire before sending.
  // Do NOT clear the input yet — restore it if the user cancels.
  if (planMode) {
    busy = true;
    sendBtn.disabled = true;
    await openPlanOverlay(text);
    // Execution resumes inside runWithPlan() → sendWithAnswers()
    return;
  }

  await sendWithAnswers(text, null);
}

async function sendWithAnswers(text, answers) {
  busy = true;
  sendBtn.disabled = true;
  inputEl.value = '';
  inputEl.style.height = 'auto';

  const welcome = document.getElementById('welcome');
  if (welcome) welcome.remove();
  document.getElementById('top-title').textContent = text.slice(0, 50) + (text.length > 50 ? '…' : '');

  // Show attached file names in the user bubble if any
  const fileNames = Object.values(attachedFiles).map(f => f.name);
  addUserTurn(text, fileNames);
  // Clear attachments after sending
  attachedFiles = {};
  renderContextBar();
  const thinkEl = addThinkingTurn();
  const thinkContent = thinkEl.querySelector('.ai-content');
  setTabBadge('chat', '…', 'running');

  // Track which session is doing work so the user can switch chats while it runs.
  // For brand new chats, the session_id is assigned server-side; we update once we get `done`.
  const runSessionAtStart = currentSessionId;
  activeRunSessionId = runSessionAtStart;
  if (runSessionAtStart) setSessionBadge(runSessionAtStart, '…', 'running');

  // Persist state so switching chats doesn't visually "wipe" the in-progress run.
  const runId = 'run_' + Date.now() + '_' + Math.floor(Math.random() * 1e9);
  activeRunId = runId;
  thinkEl.dataset.runId = runId;
  runStates.set(runId, {
    run_id: runId,
    session_id: runSessionAtStart,
    phase: 'planning',
    success: null,
    reasoning_lines: [],
    thinking_text: '',
    streaming_started: false,
    full_answer: '',
    final_answer: '',
    done: false,
    error: false,
  });
  if (runSessionAtStart) sessionActiveRun.set(runSessionAtStart, runId);

  try {
    // Prepend any attached file contents to the request
    const prefix = buildContextPrefix();
    const fullRequest = prefix ? prefix + text : text;
    const st0 = runStates.get(runId);
    if (st0) st0.request = fullRequest;

    const body = { request: fullRequest };
    if (currentSessionId) body.session_id = currentSessionId;
    if (adminMode) body.admin = true;
    if (answers && Object.keys(answers).length > 0) body.answers = answers;

    const response = await fetch('/run/stream', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });
    if (!response.ok) throw new Error(`Server error ${response.status}`);

    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    let buffer = '';
    let currentEventType = null;
    let answerEl = null;
    let fullAnswer = '';
    let reasoningLines = [];   // collected during run
    let reasoningEl = null;    // live panel inside thinkEl
    let thinkingText = '';     // accumulated qwen3 chain-of-thought text
    let thinkingEl = null;     // live "🧠 Thinking…" indicator element
    let doneEventReceived = false;

    function getOrCreateReasoning() {
      if (!reasoningEl) {
        reasoningEl = document.createElement('div');
        reasoningEl.className = 'reasoning';
        thinkContent.appendChild(reasoningEl);
      }
      return reasoningEl;
    }

    function addReasoningLine(cls, icon, text) {
      const line = `<span class="r-icon">${icon}</span><span class="r-text">${esc(text)}</span>`;
      reasoningLines.push({ cls, icon, text });
      const st = runStates.get(runId);
      if (st) st.reasoning_lines.push(`${icon} ${text}`);
      const div = document.createElement('div');
      div.className = `reasoning-line ${cls}`;
      div.innerHTML = line;
      getOrCreateReasoning().appendChild(div);
      scrollBottom();
    }

    while (true) {
      const { done, value } = await reader.read();
      if (done) break;

      buffer += decoder.decode(value, { stream: true });
      const lines = buffer.split('\n');
      buffer = lines.pop() || '';

      for (const line of lines) {
        if (line.startsWith('event: ')) {
          currentEventType = line.slice(7).trim();
        } else if (line.startsWith('data: ')) {
          let data;
          try { data = JSON.parse(line.slice(6)); } catch (_) { continue; }

          if (currentEventType === 'status') {
            const statusText = phaseLabel(data.phase);
            const st = runStates.get(runId);
            if (st) st.phase = data.phase;
            // Update the main thinking label
            const dotsHtml = '<div class="dots"><span></span><span></span><span></span></div>';
            const firstChild = thinkContent.firstChild;
            if (firstChild && firstChild.className === 'thinking') {
              firstChild.innerHTML = dotsHtml + esc(statusText);
            } else {
              const thinkDiv = document.createElement('div');
              thinkDiv.className = 'thinking';
              thinkDiv.innerHTML = dotsHtml + esc(statusText);
              thinkContent.insertBefore(thinkDiv, thinkContent.firstChild);
            }
            renderRunState(runId);
          } else if (currentEventType === 'plan') {
            const steps = data.steps || [];
            const risk = data.risk || 0;
            const riskLabel = risk <= 3 ? '🟢' : risk <= 7 ? '🟡' : '🔴';
            addReasoningLine('r-plan', '📋', `Plan: ${steps.length} step${steps.length !== 1 ? 's' : ''} ${riskLabel}`);
            steps.forEach((s, i) => {
              addReasoningLine('r-plan', `  ${i+1}.`, s);
            });
            renderRunState(runId);
          } else if (currentEventType === 'tool_call') {
            addReasoningLine('r-tool', '🔧', `${data.tool}: ${data.action}`);
            // Show URL / query details if present (one line per entry)
            if (data.detail) {
              data.detail.split('\n').forEach(line => {
                if (line.trim()) addReasoningLine('r-tool-detail', '  ↳', line.trim());
              });
            }
            renderRunState(runId);
          } else if (currentEventType === 'tool_done') {
            const cls = data.success ? 'r-tool-done-ok' : 'r-tool-done-fail';
            const icon = data.success ? '✅' : '❌';
            addReasoningLine(cls, icon, `${data.tool} ${data.success ? 'done' : 'failed'}`);
            renderRunState(runId);
          } else if (currentEventType === 'needs_approval') {
            showApprovalModal(data);
          } else if (currentEventType === 'critic_result') {
            if (data.passed) {
              addReasoningLine('r-critic-pass', '✅', `Critic passed (${data.score?.toFixed(1) ?? '?'}/10)`);
            } else {
              addReasoningLine('r-critic-fail', '⚠️', `Critic flagged (${data.score?.toFixed(1) ?? '?'}/10)`);
              (data.issues || []).slice(0, 2).forEach(issue =>
                addReasoningLine('r-critic-fail', '  ↳', issue)
              );
            }
            renderRunState(runId);
          } else if (currentEventType === 'repair') {
            addReasoningLine('r-repair', '🔧', `Repairing (cycle ${data.cycle})`);
            (data.issues || []).slice(0, 2).forEach(issue =>
              addReasoningLine('r-repair', '  ↳', issue)
            );
            renderRunState(runId);
          } else if (currentEventType === 'replan') {
            addReasoningLine('r-replan', '🔄', `Replanning from scratch (attempt ${data.attempt})`);
            renderRunState(runId);
          } else if (currentEventType === 'file_written') {
            addReasoningLine('r-file-written', '💾', `Wrote ${data.path}`);
            // Auto-refresh the editor if this file is currently open
            if (editorCurrentPath === data.path) refreshEditorFile();
            // Keep the file tree updated as files are created/edited.
            scheduleFileTreeRefresh();
            renderRunState(runId);
          } else if (currentEventType === 'project_card') {
            // Coding task status card — shown in reasoning panel during project builds
            const statusClass = data.status === 'success' ? 'pc-success'
              : data.status === 'partial' ? 'pc-partial'
              : data.status === 'failed' ? 'pc-failed'
              : 'pc-building';
            const buildBadge = data.build_passed === true ? ' · Build ✓'
              : data.build_passed === false ? ' · Build ✗' : '';
            const downloadLink = data.package_path
              ? ` · <a href="/workspace/download?path=${encodeURIComponent(data.package_path)}" target="_blank">Download ZIP</a>`
              : '';
            const card = document.createElement('div');
            card.className = 'project-card';
            card.innerHTML = `<div class="pc-header"><span class="pc-lang">${esc(data.language||'')}</span> <strong>${esc(data.project_name||'')}</strong> <span class="pc-status ${statusClass}">${esc(data.status||'')}</span></div><div class="pc-stats">${data.files_written||0} files written${buildBadge}${downloadLink}</div>`;
            reasoningEl.appendChild(card);
            reasoningEl.scrollTop = reasoningEl.scrollHeight;
          } else if (currentEventType === 'terminal_cmd') {
            appendTerminalCmd(data.step, data.command, data.cwd);
          } else if (currentEventType === 'terminal_out') {
            appendTerminalOut(data.stdout, data.stderr, data.exit_code, data.success);
          } else if (currentEventType === 'thinking_token') {
            // qwen3 chain-of-thought — accumulate and show live indicator
            thinkingText += data.text;
            const st = runStates.get(runId);
            if (st) st.thinking_text = thinkingText;
            if (!thinkingEl) {
              thinkingEl = document.createElement('div');
              thinkingEl.className = 'thinking-live';
              thinkingEl.innerHTML = '🧠 <span class="thinking-label">Thinking…</span><span class="thinking-count"></span>';
              thinkContent.appendChild(thinkingEl);
            }
            const approxTok = Math.round(thinkingText.length / 4);
            thinkingEl.querySelector('.thinking-count').textContent = ` ~${approxTok} tokens`;
            scrollBottom();
            renderRunState(runId);
          } else if (currentEventType === 'token') {
            if (!answerEl) {
              // Finalize CoT block — build the collapsed details BEFORE wiping thinkContent
              let savedCoT = null;
              if (thinkingText) {
                const approxTok = Math.round(thinkingText.length / 4);
                const details = document.createElement('details');
                details.className = 'thinking-details';
                details.innerHTML = `<summary>🧠 Thought for ~${approxTok} tokens</summary><div class="thinking-trace">${esc(thinkingText)}</div>`;
                savedCoT = details;
                thinkingEl = null;
              }
              // Clear the working-indicator area
              thinkContent.innerHTML = '';
              // Re-attach CoT block so it persists above the answer
              if (savedCoT) thinkContent.appendChild(savedCoT);
              const div2 = document.createElement('div');
              div2.className = 'streaming-content';
              thinkContent.appendChild(div2);
              answerEl = div2;
              // Add reasoning toggle if we have lines
              if (reasoningLines.length > 0) {
                const uid = 'r' + Date.now();
                const toggle = document.createElement('span');
                toggle.className = 'reasoning-toggle';
                toggle.innerHTML = `<svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M2 3l3 3 3-3"/></svg> Show reasoning`;
                toggle.onclick = () => {
                  const panel = document.getElementById(uid);
                  const open = panel.style.display === 'block';
                  panel.style.display = open ? 'none' : 'block';
                  toggle.querySelector('path').setAttribute('d', open ? 'M2 3l3 3 3-3' : 'M2 7l3-3 3 3');
                };
                const panel = document.createElement('div');
                panel.className = 'reasoning-full';
                panel.id = uid;
                panel.textContent = reasoningLines.map(l => `${l.icon} ${l.text}`).join('\n');
                thinkContent.appendChild(toggle);
                thinkContent.appendChild(panel);
              }
            }
            fullAnswer += data.text;
            answerEl.textContent = fullAnswer;
            const st = runStates.get(runId);
            if (st) {
              st.streaming_started = true;
              st.full_answer = fullAnswer;
            }
            scrollBottom();
            renderRunState(runId);
          } else if (currentEventType === 'done') {
            doneEventReceived = true;
            if (!answerEl) {
              // Preserve CoT block if present (non-streaming path)
              let savedCoT2 = thinkContent.querySelector('.thinking-details');
              thinkContent.innerHTML = '';
              if (savedCoT2) thinkContent.appendChild(savedCoT2);
              const div2 = document.createElement('div');
              div2.innerHTML = renderMarkdown(data.answer);
              thinkContent.appendChild(div2);
            } else {
              answerEl.innerHTML = renderMarkdown(fullAnswer || data.answer);
            }

            // Replace the spinner label with a stable completion label.
            const doneLabel = data.success ? '✓ done' : '✗ failed';
            const firstChild = thinkContent.firstChild;
            if (firstChild && firstChild.className === 'thinking') {
              firstChild.textContent = doneLabel;
            } else {
              const doneDiv = document.createElement('div');
              doneDiv.className = 'thinking';
              doneDiv.textContent = doneLabel;
              thinkContent.insertBefore(doneDiv, thinkContent.firstChild);
            }
            // Add reasoning toggle to final turn
            if (reasoningLines.length > 0 && !thinkContent.querySelector('.reasoning-toggle')) {
              const uid = 'r' + Date.now();
              const toggle = document.createElement('span');
              toggle.className = 'reasoning-toggle';
              toggle.innerHTML = `<svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M2 3l3 3 3-3"/></svg> Show reasoning`;
              toggle.onclick = () => {
                const panel = document.getElementById(uid);
                const open = panel.style.display === 'block';
                panel.style.display = open ? 'none' : 'block';
                toggle.querySelector('path').setAttribute('d', open ? 'M2 3l3 3 3-3' : 'M2 7l3-3 3 3');
              };
              const panel = document.createElement('div');
              panel.className = 'reasoning-full';
              panel.id = uid;
              panel.textContent = reasoningLines.map(l => `${l.icon} ${l.text}`).join('\n');
              thinkContent.appendChild(toggle);
              thinkContent.appendChild(panel);
            }
            // Show total duration if provided by server
            if (data.duration_ms != null && !thinkContent.querySelector('.ai-meta')) {
              const meta = document.createElement('div');
              meta.className = 'ai-meta';
              meta.innerHTML = `<span class="meta-chip">took ${(data.duration_ms / 1000).toFixed(1)}s</span>`;
              thinkContent.appendChild(meta);
            }

            // The server assigns/returns the definitive session id here.
            const runSessionId = data.session_id;
            if (runSessionId) {
              // Move the running badge (if any) to the new id and mark completion.
              if (activeRunSessionId && activeRunSessionId !== runSessionId) {
                setSessionBadge(activeRunSessionId, null);
              }
              activeRunSessionId = runSessionId;
              setSessionBadge(runSessionId, data.success ? '✓' : '✗', data.success ? 'ok' : 'fail');
            }

            // Only switch the UI's active session if the user hasn't navigated elsewhere mid-run.
            if (currentSessionId === runSessionAtStart) {
              currentSessionId = runSessionId;
            }

            // Persist final run state for when the user returns to this session.
            const st = runStates.get(runId);
            if (st) {
              st.session_id = runSessionId || st.session_id;
              st.done = true;
              st.success = data.success;
              st.error = !data.success;
              st.final_answer = data.answer || (fullAnswer || '');
              if (data.failure) {
                const f = data.failure;
                const bits = [];
                bits.push(`Failure: step ${f.step}${f.tool ? ' (' + f.tool + ')' : ''}`);
                if (f.action) bits.push(`Action: ${f.action}`);
                if (f.artifact_key) bits.push(`Artifact: ${f.artifact_key}`);
                if (f.error_code) bits.push(`Error: ${f.error_code}`);
                if (f.trace) bits.push(`Trace: ${f.trace}`);
                st.reasoning_lines.push('⚠️ ' + bits.join(' · '));
              }
              if (st.session_id) sessionActiveRun.set(st.session_id, runId);
            }
            if (runSessionId) setSessionBadge(runSessionId, data.success ? '✓' : '✗', data.success ? 'ok' : 'fail');
            renderRunState(runId);
            // Once done, drop the "active run" pointer for this session after a short delay.
            if (st && st.session_id) {
              setTimeout(() => {
                // Keep the state briefly in case the user navigates back immediately.
                sessionActiveRun.delete(st.session_id);
                runStates.delete(runId);
              }, 5000);
            }
            setTimeout(refreshSessions, 500);
            setTimeout(loadMemory, 700);
            setTimeout(loadKb, 900);
            scrollBottom();
            // Desktop notification when tab is not visible
            const notifyText = data.answer ? data.answer.slice(0, 100) : 'Task complete.';
            maybeNotify('aihomeserver', notifyText + (data.answer && data.answer.length > 100 ? '…' : ''));

            if (activeSidebarTab !== 'chat') setTabBadge('chat', '✓', 'ok');
          } else if (currentEventType === 'error') {
            thinkEl.remove();
            addErrorTurn(data.message || 'Unknown error');
            if (activeSidebarTab !== 'chat') setTabBadge('chat', '✗', 'fail');
            if (activeRunSessionId) setSessionBadge(activeRunSessionId, '✗', 'fail');
            const st = runStates.get(runId);
            if (st) {
              st.error = true;
              st.done = true;
              st.final_answer = data.message || 'Unknown error';
              renderRunState(runId);
              if (st.session_id) {
                setTimeout(() => {
                  sessionActiveRun.delete(st.session_id);
                  runStates.delete(runId);
                }, 5000);
              }
            }
          }
          currentEventType = null;
        }
      }

      if (doneEventReceived) {
        try { await reader.cancel(); } catch (_) {}
        break;
      }
    }
  } catch (err) {
    thinkEl.remove();
    addErrorTurn('Connection error: ' + err.message);
    if (activeSidebarTab !== 'chat') setTabBadge('chat', '✗', 'fail');
    if (activeRunSessionId) setSessionBadge(activeRunSessionId, '✗', 'fail');
  }
  busy = false;
  sendBtn.disabled = !inputEl.value.trim();
  inputEl.focus();
  if (activeSidebarTab === 'chat') setTabBadge('chat', null);

  // Drain queued requests (sequential execution).
  if (sendQueue.length > 0) {
    setTimeout(() => {
      if (busy || sendQueue.length === 0) return;
      const next = sendQueue.shift();
      if (!next) return;
      // If the queued placeholder isn't in the DOM anymore (navigated away), drop it.
      const qEl = (next.queued_el && next.queued_el.isConnected) ? next.queued_el : null;
      sendQueuedRequest(next.request, next.answers, qEl, next.session_id);
    }, 0);
  }
}

// Internal helper for queued sends where the request already includes any context prefix.
async function sendQueuedRequest(fullRequest, answers, queuedEl, sessionId) {
  busy = true;
  sendBtn.disabled = true;

  // If the queued placeholder is still present, reuse it as the run turn.
  const thinkEl = queuedEl || addThinkingTurn();
  if (queuedEl) queuedEl.dataset.queue = '';
  const thinkContent = thinkEl.querySelector('.ai-content');
  setTabBadge('chat', '…', 'running');

  const runSessionAtStart = sessionId;
  activeRunSessionId = runSessionAtStart;
  if (runSessionAtStart) setSessionBadge(runSessionAtStart, '…', 'running');

  const runId = 'run_' + Date.now() + '_' + Math.floor(Math.random() * 1e9);
  activeRunId = runId;
  thinkEl.dataset.runId = runId;
  runStates.set(runId, {
    run_id: runId,
    session_id: runSessionAtStart,
    phase: 'planning',
    reasoning_lines: [],
    thinking_text: '',
    streaming_started: false,
    full_answer: '',
    final_answer: '',
    done: false,
    error: false,
    request: fullRequest,
  });
  if (runSessionAtStart) sessionActiveRun.set(runSessionAtStart, runId);

  try {
    const body = { request: fullRequest };
    if (sessionId) body.session_id = sessionId;
    if (adminMode) body.admin = true;
    if (answers && Object.keys(answers).length > 0) body.answers = answers;

    const response = await fetch('/run/stream', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });
    if (!response.ok) throw new Error(`Server error ${response.status}`);

    // Delegate to the same stream reader by calling sendWithAnswers' logic would be nicer,
    // but keep this minimal: just piggy-back by setting globals and letting the existing
    // loop continue in this function's scope via copy-paste would be too big here.
    // Instead, re-run through sendWithAnswers by reconstructing the visible text only.
    // (Queued sends are best-effort; advanced streaming persistence works for the active run.)
    // Fall back: if streaming is not supported, the server will still return 'done' soon.
    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    let buffer = '';
    let currentEventType = null;
    let doneEventReceived = false;

    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      const lines = buffer.split('\n');
      buffer = lines.pop() || '';
      for (const line of lines) {
        if (line.startsWith('event: ')) currentEventType = line.slice(7).trim();
        else if (line.startsWith('data: ')) {
          let data;
          try { data = JSON.parse(line.slice(6)); } catch (_) { continue; }
          const st = runStates.get(runId);
          if (currentEventType === 'status') { if (st) st.phase = data.phase; renderRunState(runId); }
          else if (currentEventType === 'thinking_token') { if (st) { st.thinking_text = (st.thinking_text||'') + (data.text||''); } renderRunState(runId); }
          else if (currentEventType === 'token') { if (st) { st.streaming_started = true; st.full_answer = (st.full_answer||'') + (data.text||''); } renderRunState(runId); }
          else if (currentEventType === 'done') {
            doneEventReceived = true;
            const runSessionId = data.session_id;
            if (st) {
              st.session_id = runSessionId || st.session_id;
              st.done = true;
              st.success = data.success;
              st.error = !data.success;
              st.final_answer = data.answer || st.full_answer || '';
              if (data.failure) {
                const f = data.failure;
                const bits = [];
                bits.push(`Failure: step ${f.step}${f.tool ? ' (' + f.tool + ')' : ''}`);
                if (f.action) bits.push(`Action: ${f.action}`);
                if (f.artifact_key) bits.push(`Artifact: ${f.artifact_key}`);
                if (f.error_code) bits.push(`Error: ${f.error_code}`);
                if (f.trace) bits.push(`Trace: ${f.trace}`);
                st.reasoning_lines = st.reasoning_lines || [];
                st.reasoning_lines.push('⚠️ ' + bits.join(' · '));
              }
              if (st.session_id) sessionActiveRun.set(st.session_id, runId);
            }
            if (runSessionId) setSessionBadge(runSessionId, data.success ? '✓' : '✗', data.success ? 'ok' : 'fail');
            renderRunState(runId);
            setTimeout(refreshSessions, 500);
            setTimeout(loadMemory, 700);
            setTimeout(loadKb, 900);
          } else if (currentEventType === 'error') {
            if (st) { st.error = true; st.done = true; st.final_answer = data.message || 'Unknown error'; }
            renderRunState(runId);
            doneEventReceived = true;
          }
          currentEventType = null;
        }
      }
      if (doneEventReceived) {
        try { await reader.cancel(); } catch (_) {}
        break;
      }
    }
  } catch (err) {
    thinkEl.remove();
    addErrorTurn('Connection error: ' + err.message);
    if (activeSidebarTab !== 'chat') setTabBadge('chat', '✗', 'fail');
    if (activeRunSessionId) setSessionBadge(activeRunSessionId, '✗', 'fail');
  }
  busy = false;
  sendBtn.disabled = !inputEl.value.trim();
  inputEl.focus();
  if (activeSidebarTab === 'chat') setTabBadge('chat', null);
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
function addUserTurn(text, attachedFileNames) {
  const div = document.createElement('div');
  div.className = 'turn user';
  let fileHtml = '';
  if (attachedFileNames && attachedFileNames.length > 0) {
    fileHtml = '<div style="margin-bottom:6px;display:flex;flex-wrap:wrap;gap:4px">'
      + attachedFileNames.map(n =>
          `<span style="font-size:10px;background:rgba(90,120,200,0.2);border:1px solid rgba(90,120,200,0.35);border-radius:4px;padding:1px 6px;color:#8aaaee;font-family:monospace">📎 ${esc(n)}</span>`
        ).join('')
      + '</div>';
  }
  div.innerHTML = `<div class="user-bubble">${fileHtml}${esc(text)}</div>`;
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
  const durChip = (data.duration_ms != null)
    ? `<span class="meta-chip">took ${(data.duration_ms / 1000).toFixed(1)}s</span>`
    : '';
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
    <div class="ai-meta">${statusChip}${stepChip}${durChip}${failChip}${repairChip}${toolResults}</div>
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
  const mathBlocks = [];

  function katexSafe(expr, display) {
    if (typeof katex === 'undefined') return `<code>${esc(expr)}</code>`;
    try {
      return katex.renderToString(expr.trim(), { displayMode: display, throwOnError: false });
    } catch(_) { return `<code>${esc(expr)}</code>`; }
  }

  // Display math $$...$$ — extract before everything else (may span lines)
  md = md.replace(/\$\$([\s\S]+?)\$\$/g, (_, math) => {
    mathBlocks.push(`<div class="math-block">${katexSafe(math, true)}</div>`);
    return `\x00mx${mathBlocks.length - 1}\x00`;
  });
  // Inline math $...$ — single line only, not $$
  md = md.replace(/\$([^\$\n]+?)\$/g, (_, math) => {
    mathBlocks.push(katexSafe(math, false));
    return `\x00mx${mathBlocks.length - 1}\x00`;
  });

  // Code blocks (protect from other transforms)
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
  // Restore math blocks
  let result = out.join('\n');
  result = result.replace(/\x00mx(\d+)\x00/g, (_, i) => mathBlocks[parseInt(i)]);
  return result;
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

function scrollBottom(force) {
  if (!force && !autoScroll) {
    if (jumpBottomBtn) jumpBottomBtn.style.display = 'block';
    return;
  }
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
  const badge = sessionBadges.get(s.session_id);
  const badgeHtml = badge ? `<span class="sess-badge ${esc(badge.cls)}">${esc(badge.text)}</span>` : '';

  item.innerHTML = `
    <span class="dot ${archived ? 'archived' : 'ok'}"></span>
    <span class="item-label">${esc(preview)}</span>
    ${badgeHtml}
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
    maybeShowActiveRunForCurrentSession();
    document.querySelectorAll('.history-item').forEach(el => {
      el.classList.toggle('active', el.dataset.sid === sessionId);
    });
  } catch (_) {}
}

// ── Approval modal ────────────────────────────────────────────
function showApprovalModal(data) {
  const existing = document.getElementById('approval-modal');
  if (existing) existing.remove();

  const toolStr = data.tool ? ` via ${data.tool}` : '';
  const modal = document.createElement('div');
  modal.className = 'approval-modal';
  modal.id = 'approval-modal';
  modal.innerHTML = `
    <div class="approval-card">
      <h3>⚠️ High-Risk Action — Approval Required</h3>
      <div class="risk-badge">Risk ${data.risk}/10</div>
      <div class="action-desc">Step ${data.step}${esc(toolStr)}: ${esc(data.action)}</div>
      <div class="approval-btns">
        <button class="btn-reject" onclick="handleApproval('${data.task_id}', false)">Reject</button>
        <button class="btn-approve" onclick="handleApproval('${data.task_id}', true)">Approve</button>
      </div>
    </div>`;
  document.body.appendChild(modal);
}

async function handleApproval(taskId, approved) {
  const modal = document.getElementById('approval-modal');
  if (modal) modal.remove();
  const endpoint = approved ? 'approve' : 'reject';
  await fetch(`/task/${taskId}/${endpoint}`, { method: 'POST' });
}

// ── Terminal panel ────────────────────────────────────────────
let terminalOpen = false;
let terminalHasOutput = false;

function toggleTerminal() {
  terminalOpen = !terminalOpen;
  const drawer = document.getElementById('terminal-drawer');
  const btn = document.getElementById('term-toggle');
  const hint = document.getElementById('term-hint');
  drawer.classList.toggle('open', terminalOpen);
  document.body.classList.toggle('terminal-open', terminalOpen);
  hint.textContent = terminalOpen ? 'click to close' : 'click to open';
  if (terminalOpen) scrollTerminal();
}

function appendTerminalCmd(step, command, cwd) {
  const body = document.getElementById('terminal-body');
  const dots = document.getElementById('term-dots');
  const btn = document.getElementById('term-toggle');
  dots.classList.add('active');
  btn.classList.add('has-output');
  terminalHasOutput = true;

  if (body.children.length > 0) {
    const sep = document.createElement('hr');
    sep.className = 'term-sep';
    body.appendChild(sep);
  }
  // Show working directory as a dim path header
  if (cwd) {
    const cwdLine = document.createElement('div');
    cwdLine.className = 'term-cwd';
    cwdLine.textContent = cwd;
    body.appendChild(cwdLine);
  }
  const line = document.createElement('div');
  line.className = 'term-cmd';
  line.textContent = command;
  body.appendChild(line);
  scrollTerminal();

  // Auto-open terminal when a command runs
  if (!terminalOpen) toggleTerminal();
}

function appendTerminalOut(stdout, stderr, exitCode, success) {
  const body = document.getElementById('terminal-body');
  if (stdout.trim()) {
    const out = document.createElement('div');
    out.className = 'term-stdout';
    out.textContent = stdout.trim();
    body.appendChild(out);
  }
  if (stderr.trim()) {
    const err = document.createElement('div');
    err.className = 'term-stderr';
    err.textContent = stderr.trim();
    body.appendChild(err);
  }
  const exit = document.createElement('div');
  exit.className = success ? 'term-exit-ok' : 'term-exit-fail';
  exit.textContent = `[exit ${exitCode}]`;
  body.appendChild(exit);
  scrollTerminal();
}

function scrollTerminal() {
  const body = document.getElementById('terminal-body');
  body.scrollTop = body.scrollHeight;
}

// ── Planning mode ─────────────────────────────────────────────
let planMode = false;
let pendingPlanRequest = null;
let planAnswers = {};
let planCustomValues = {};
let planQuestions = [];

function togglePlanMode() {
  planMode = !planMode;
  const btn = document.getElementById('plan-toggle');
  btn.classList.toggle('active', planMode);
  btn.title = planMode
    ? 'Plan mode ON — will ask questions before running'
    : 'Plan mode — ask questions before running';
}

async function openPlanOverlay(requestText) {
  pendingPlanRequest = requestText;
  planAnswers = {};
  planCustomValues = {};
  planQuestions = [];

  const overlay  = document.getElementById('plan-overlay');
  const body     = document.getElementById('plan-body');
  const subtitle = document.getElementById('plan-subtitle');
  const runBtn   = document.getElementById('plan-run-btn');

  subtitle.textContent = `"${requestText.slice(0,60)}${requestText.length>60?'…':''}"`;
  body.innerHTML = '<div class="plan-loading"><div class="dots"><span></span><span></span><span></span></div>Generating questions…</div>';
  runBtn.disabled = true;
  overlay.classList.add('open');

  // 45-second timeout so the overlay never spins forever on slow hardware
  const timeout = new Promise((_, rej) => setTimeout(() => rej(new Error('timeout')), 45000));
  try {
    const res = await Promise.race([
      fetch('/plan', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ request: requestText }),
      }),
      timeout,
    ]);
    if (!res.ok) throw new Error('server error');
    const data = await res.json();
    planQuestions = data.questions || [];
    renderPlanQuestions(planQuestions);
  } catch (e) {
    const msg = e.message === 'timeout'
      ? 'Timed out generating questions.'
      : 'Could not generate questions.';
    body.innerHTML = `<div style="color:var(--text-muted);font-size:13px;padding:16px 0">${msg} You can still run directly.</div>`;
    runBtn.disabled = false;  // allow running without answers
  }
}

function renderPlanQuestions(questions) {
  const body   = document.getElementById('plan-body');
  const runBtn = document.getElementById('plan-run-btn');
  body.innerHTML = '';

  if (questions.length === 0) {
    body.innerHTML = '<div style="color:var(--text-muted);font-size:13px;padding:8px 0">No questions — ready to run.</div>';
    runBtn.disabled = false;
    return;
  }

  for (const q of questions) {
    const qDiv = document.createElement('div');
    qDiv.className = 'plan-question';

    const label = document.createElement('div');
    label.className = 'plan-question-text';
    label.textContent = q.text;
    qDiv.appendChild(label);

    const optRow = document.createElement('div');
    optRow.className = 'plan-options';

    for (const opt of q.options) {
      const pill = document.createElement('div');
      pill.className = 'plan-option';
      pill.textContent = opt.label;
      pill.dataset.value = opt.value;
      pill.onclick = () => selectPlanOption(q.id, opt.value, pill, qDiv);
      optRow.appendChild(pill);
    }
    qDiv.appendChild(optRow);

    // Custom text input — shown only when "custom" selected
    const customWrap = document.createElement('div');
    customWrap.className = 'plan-custom-wrap';
    customWrap.id = `custom-wrap-${q.id}`;
    const customInput = document.createElement('input');
    customInput.type = 'text';
    customInput.className = 'plan-custom-input';
    customInput.placeholder = 'Type your preference…';
    customInput.id = `custom-input-${q.id}`;
    customInput.oninput = () => {
      planCustomValues[q.id] = customInput.value.trim();
    };
    customWrap.appendChild(customInput);
    qDiv.appendChild(customWrap);

    body.appendChild(qDiv);
  }

  // Questions are optional — enable Run immediately after they load
  runBtn.disabled = false;
}

function selectPlanOption(qid, value, pillEl, qDiv) {
  qDiv.querySelectorAll('.plan-option').forEach(p => p.classList.remove('selected'));
  pillEl.classList.add('selected');
  planAnswers[qid] = value;

  const wrap = document.getElementById(`custom-wrap-${qid}`);
  if (value === 'custom') {
    wrap.classList.add('visible');
    document.getElementById(`custom-input-${qid}`).focus();
  } else {
    wrap.classList.remove('visible');
    delete planCustomValues[qid];
  }
}

async function runWithPlan() {
  const finalAnswers = {};
  for (const q of planQuestions) {
    const raw = planAnswers[q.id];
    if (!raw) continue;
    finalAnswers[q.id] = raw === 'custom' ? (planCustomValues[q.id] || 'custom') : raw;
  }
  const req = pendingPlanRequest;
  pendingPlanRequest = null;  // clear before close so cancel-restore doesn't fire
  document.getElementById('plan-overlay').classList.remove('open');
  // Clear the input now that we're actually running
  inputEl.value = '';
  inputEl.style.height = 'auto';
  await sendWithAnswers(req, finalAnswers);
}

function closePlanOverlay() {
  document.getElementById('plan-overlay').classList.remove('open');
  // User hit Cancel — restore their message and release the busy lock
  if (pendingPlanRequest !== null) {
    inputEl.value = pendingPlanRequest;
    inputEl.dispatchEvent(new Event('input')); // resize + enable send btn
    if (busy) {
      busy = false;
      inputEl.focus();
    }
  }
  pendingPlanRequest = null;
}

// ── Admin mode ────────────────────────────────────────────────
let adminMode = false;

function toggleAdmin() {
  adminMode = !adminMode;
  const btn = document.getElementById('admin-toggle');
  btn.classList.toggle('active', adminMode);
  document.body.classList.toggle('admin-active', adminMode);
  btn.title = adminMode
    ? 'Admin mode ON — approval gate bypassed'
    : 'Admin mode — bypasses approval gate';
}

// ── Sidebar tabs ──────────────────────────────────────────────
function showSidebarTab(tab) {
  activeSidebarTab = tab;
  ['chat','files','git','mem','kb'].forEach(t => {
    document.getElementById('tab-' + t).style.display = tab === t ? 'flex' : 'none';
    document.getElementById('tab-btn-' + t).classList.toggle('active', tab === t);
  });
  if (tab === 'files' && !fileTreeLoaded) loadFileTree();
  if (tab === 'git')  loadGitStatus();
  if (tab === 'mem')  loadMemory();
  if (tab === 'kb')   loadKb();
  if (tab === 'chat') setTabBadge('chat', null);
}

// ── Sidebar resize ────────────────────────────────────────────
(function() {
  const handle  = document.getElementById('sidebar-resize');
  const sidebar  = document.getElementById('sidebar');
  const MIN = 180, MAX = 560;
  let dragging = false, startX = 0, startW = 0;

  // Restore saved width
  const saved = localStorage.getItem('sidebarW');
  if (saved) {
    const w = parseInt(saved);
    if (w >= MIN && w <= MAX) {
      document.documentElement.style.setProperty('--sidebar-w', w + 'px');
    }
  }

  handle.addEventListener('mousedown', e => {
    dragging = true;
    startX   = e.clientX;
    startW   = sidebar.getBoundingClientRect().width;
    handle.classList.add('dragging');
    document.body.style.userSelect = 'none';
    document.body.style.cursor     = 'col-resize';
    e.preventDefault();
  });

  document.addEventListener('mousemove', e => {
    if (!dragging) return;
    const delta = e.clientX - startX;
    const newW  = Math.max(MIN, Math.min(MAX, startW + delta));
    document.documentElement.style.setProperty('--sidebar-w', newW + 'px');
  });

  document.addEventListener('mouseup', () => {
    if (!dragging) return;
    dragging = false;
    handle.classList.remove('dragging');
    document.body.style.userSelect = '';
    document.body.style.cursor     = '';
    // Persist
    const w = parseInt(getComputedStyle(document.documentElement).getPropertyValue('--sidebar-w'));
    localStorage.setItem('sidebarW', w);
  });
})();

// ── File tree ──────────────────────────────────────────────────
let fileTreeLoaded = false;
let fileUploadInit = false;
let fileTreeRefreshTimer = null;

function scheduleFileTreeRefresh() {
  // Debounce to avoid hammering the server during multi-file writes.
  if (!fileTreeLoaded) return;
  if (fileTreeRefreshTimer) clearTimeout(fileTreeRefreshTimer);
  fileTreeRefreshTimer = setTimeout(() => {
    fileTreeRefreshTimer = null;
    loadFileTree();
  }, 350);
}

function openUploadPicker() {
  const input = document.getElementById('upload-input');
  if (!input) return;
  input.value = '';
  input.click();
}

async function runSelfCheck() {
  try {
    const full = confirm('Run FULL eval? (includes network + LLM checks; can take longer)\n\nOK = full, Cancel = quick');
    const body = full ? { mode: 'full', timeout_secs: 30 } : { mode: 'quick', timeout_secs: 15 };
    const res = await fetch('/eval/run', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });
    if (!res.ok) throw new Error(`${res.status}`);
    const data = await res.json();
    const lines = [];
    lines.push(`Eval: ${data.ok ? 'OK' : 'FAIL'} (${data.duration_ms}ms)  passed=${data.summary?.passed ?? 0} failed=${data.summary?.failed ?? 0} skipped=${data.summary?.skipped ?? 0}`);
    for (const c of (data.results || [])) {
      const mark = c.skipped ? '⏭' : (c.ok ? '✓' : '✗');
      lines.push(`${mark} ${c.id}`);
    }
    alert(lines.join('\n'));
  } catch (e) {
    alert('Self-check failed: ' + (e.message || e));
  }
}

async function uploadFileList(fileList) {
  const files = Array.from(fileList || []);
  if (files.length === 0) return;

  const dir = (prompt('Upload to folder (relative to workspace root). Leave blank for root:', '') || '').trim();
  const fd = new FormData();
  if (dir) fd.append('dir', dir);
  for (const f of files) {
    fd.append('files', f, f.webkitRelativePath || f.name);
  }

  try {
    const res = await fetch('/workspace/upload', { method: 'POST', body: fd });
    if (!res.ok) throw new Error(`${res.status}`);
    await res.json().catch(() => ({}));
    if (fileTreeLoaded) await loadFileTree();
    showSidebarTab('files');
  } catch (e) {
    alert('Upload failed: ' + (e.message || e));
  }
}

function initFileUploadUI() {
  if (fileUploadInit) return;
  fileUploadInit = true;

  const input = document.getElementById('upload-input');
  if (input) {
    input.addEventListener('change', async (e) => {
      const files = e.target.files;
      e.target.value = '';
      await uploadFileList(files);
    });
  }

  const tree = document.getElementById('file-tree');
  if (tree) {
    tree.addEventListener('dragover', (e) => {
      e.preventDefault();
      tree.classList.add('drop-target');
    });
    tree.addEventListener('dragleave', () => {
      tree.classList.remove('drop-target');
    });
    tree.addEventListener('drop', (e) => {
      e.preventDefault();
      tree.classList.remove('drop-target');
      if (e.dataTransfer && e.dataTransfer.files) uploadFileList(e.dataTransfer.files);
    });
  }
}
let attachedFiles = {};   // path → { path, name, content }

async function loadFileTree() {
  const treeEl = document.getElementById('file-tree');
  treeEl.innerHTML = '<div class="tree-empty"><div class="dots"><span></span><span></span><span></span></div></div>';
  try {
    const res = await fetch('/workspace/tree');
    if (!res.ok) throw new Error('server error');
    const root = await res.json();
    fileTreeLoaded = true;
    // Update root label
    const rootLabel = document.getElementById('tree-root-label');
    rootLabel.textContent = root.name || 'workspace';
    rootLabel.title = root.name || '';
    treeEl.innerHTML = '';
    if (!root.children || root.children.length === 0) {
      treeEl.innerHTML = '<div class="tree-empty">Workspace is empty.<br>Files you create will appear here.</div>';
      return;
    }
    for (const node of root.children) {
      treeEl.appendChild(buildTreeNode(node));
    }
  } catch (e) {
    treeEl.innerHTML = `<div class="tree-empty">Could not load workspace.<br><span style="font-size:10px;color:var(--text-dim)">${esc(e.message)}</span></div>`;
  }
}

function buildTreeNode(node) {
  if (node.type === 'dir') {
    const wrap = document.createElement('div');
    wrap.className = 'tree-dir';

    const row = document.createElement('div');
    row.className = 'tree-row';
    row.innerHTML = `<span class="tree-dir-icon">▶</span><span class="tree-dir-name">${esc(node.name)}</span>`;
    row.onclick = () => wrap.classList.toggle('open');
    row.oncontextmenu = (e) => showCtxMenu(e, node.path, node.name, true, row);
    wrap.appendChild(row);

    const children = document.createElement('div');
    children.className = 'tree-children';
    for (const child of (node.children || [])) {
      children.appendChild(buildTreeNode(child));
    }
    wrap.appendChild(children);
    return wrap;
  } else {
    // File node
    const el = document.createElement('div');
    el.className = 'tree-file' + (attachedFiles[node.path] ? ' attached' : '');
    el.dataset.path = node.path;
    const sizeStr = node.size < 1024
      ? `${node.size}B`
      : node.size < 1048576
        ? `${(node.size/1024).toFixed(1)}K`
        : `${(node.size/1048576).toFixed(1)}M`;
    const icon = fileIcon(node.ext);
    el.innerHTML = `<span class="tree-file-icon">${icon}</span><span class="tree-file-name">${esc(node.name)}</span><span class="tree-file-size">${sizeStr}</span><button class="tree-file-dl" title="Download" onclick="event.stopPropagation();downloadFile('${node.path.replace(/'/g,"\\'")}','${node.name.replace(/'/g,"\\'")}')">⬇</button><button class="tree-file-edit" title="Open in editor" onclick="event.stopPropagation();openFileEditor('${node.path.replace(/'/g,"\\'")}','${node.name.replace(/'/g,"\\'")}')">✎</button>`;
    el.onclick = () => toggleAttachFile(node.path, node.name, el);
    el.oncontextmenu = (e) => showCtxMenu(e, node.path, node.name, false, el);
    return el;
  }
}

function fileIcon(ext) {
  const icons = {
    rs:'🦀', js:'📜', ts:'📜', jsx:'📜', tsx:'📜',
    py:'🐍', go:'🐹', java:'☕', c:'⚙', cpp:'⚙', h:'⚙',
    html:'🌐', css:'🎨', json:'📋', toml:'📋', yaml:'📋', yml:'📋',
    md:'📝', txt:'📄', sh:'💲', ps1:'💲',
    png:'🖼', jpg:'🖼', gif:'🖼', svg:'🖼',
    sql:'🗄', db:'🗄',
  };
  return icons[ext?.toLowerCase()] || '📄';
}

async function toggleAttachFile(path, name, el) {
  if (attachedFiles[path]) {
    // Detach
    delete attachedFiles[path];
    el.classList.remove('attached');
    renderContextBar();
    return;
  }
  // Fetch content and attach
  el.style.opacity = '0.5';
  try {
    const res = await fetch(`/workspace/file?path=${encodeURIComponent(path)}`);
    if (!res.ok) throw new Error('could not read file');
    const data = await res.json();
    attachedFiles[path] = { path, name, content: data.content, truncated: data.truncated };
    el.classList.add('attached');
    renderContextBar();
  } catch (e) {
    console.warn('attach failed:', e);
  }
  el.style.opacity = '';
}

function renderContextBar() {
  const bar = document.getElementById('context-bar');
  bar.innerHTML = '';
  const files = Object.values(attachedFiles);
  if (files.length === 0) {
    bar.classList.remove('has-files');
    return;
  }
  bar.classList.add('has-files');
  for (const f of files) {
    const chip = document.createElement('div');
    chip.className = 'ctx-chip';
    chip.innerHTML = `📎 ${esc(f.name)}${f.truncated ? ' <span title="Truncated at 64KB" style="opacity:.6">(truncated)</span>' : ''}<button class="ctx-chip-remove" onclick="detachFile('${f.path.replace(/'/g,"\\'")}')">×</button>`;
    bar.appendChild(chip);
  }
}

function detachFile(path) {
  delete attachedFiles[path];
  // Also un-highlight the tree node if visible
  const el = document.querySelector(`.tree-file[data-path="${CSS.escape(path)}"]`);
  if (el) el.classList.remove('attached');
  renderContextBar();
}

function buildContextPrefix() {
  const files = Object.values(attachedFiles);
  if (files.length === 0) return '';
  const parts = files.map(f =>
    `[Attached file: ${f.path}${f.truncated ? ' (truncated at 64KB)' : ''}]\n\`\`\`\n${f.content}\n\`\`\``
  );
  return parts.join('\n\n') + '\n\n---\n\n';
}

// ── File editor ───────────────────────────────────────────────
let editorCurrentPath = null;
let editorDirty = false;

async function openFileEditor(path, name) {
  editorCurrentPath = path;
  editorDirty = false;

  const panel    = document.getElementById('file-editor');
  const textarea = document.getElementById('editor-textarea');
  const filename = document.getElementById('editor-filename');
  const dirty    = document.getElementById('editor-dirty');

  filename.textContent = name || path;
  filename.title = path;
  dirty.classList.remove('visible');
  textarea.value = '';
  updateEditorStatusBar('');
  panel.classList.add('open');

  try {
    const res = await fetch(`/workspace/file?path=${encodeURIComponent(path)}`);
    if (!res.ok) throw new Error(`${res.status}`);
    const data = await res.json();
    textarea.value = data.content;
    updateEditorStatusBar(data.content);
    if (data.truncated) {
      filename.textContent = (name || path) + ' (truncated)';
    }
  } catch (e) {
    textarea.value = `// Could not load file: ${e.message}`;
  }
  textarea.focus();
}

function closeFileEditor() {
  if (editorDirty && !confirm('You have unsaved changes. Close anyway?')) return;
  document.getElementById('file-editor').classList.remove('open');
  editorCurrentPath = null;
  editorDirty = false;
}

function markEditorDirty() {
  if (!editorDirty) {
    editorDirty = true;
    document.getElementById('editor-dirty').classList.add('visible');
  }
  const ta = document.getElementById('editor-textarea');
  updateEditorStatusBar(ta.value);
}

function updateEditorStatusBar(content) {
  const lines = content ? content.split('\n').length : 0;
  const bytes = new TextEncoder().encode(content).length;
  const sizeStr = bytes < 1024 ? `${bytes} B`
    : bytes < 1048576 ? `${(bytes/1024).toFixed(1)} KB`
    : `${(bytes/1048576).toFixed(1)} MB`;
  document.getElementById('editor-linecount').textContent = `${lines} line${lines !== 1 ? 's' : ''}`;
  document.getElementById('editor-size').textContent = sizeStr;
}

async function saveEditorFile() {
  if (!editorCurrentPath) return;
  const content = document.getElementById('editor-textarea').value;
  try {
    const res = await fetch('/workspace/file', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ path: editorCurrentPath, content }),
    });
    if (!res.ok) throw new Error(`${res.status}`);
    editorDirty = false;
    document.getElementById('editor-dirty').classList.remove('visible');
    // Brief flash on the filename
    const fn = document.getElementById('editor-filename');
    const orig = fn.style.color;
    fn.style.color = '#5a9a5a';
    setTimeout(() => fn.style.color = orig, 800);
    // Refresh tree so size updates
    if (fileTreeLoaded) loadFileTree();
  } catch (e) {
    alert('Save failed: ' + e.message);
  }
}

async function refreshEditorFile() {
  if (!editorCurrentPath) return;
  try {
    const res = await fetch(`/workspace/file?path=${encodeURIComponent(editorCurrentPath)}`);
    if (!res.ok) return;
    const data = await res.json();
    // Only refresh if not dirty — don't clobber user edits
    if (!editorDirty) {
      document.getElementById('editor-textarea').value = data.content;
      updateEditorStatusBar(data.content);
      const badge = document.getElementById('editor-refreshed');
      badge.classList.add('show');
      setTimeout(() => badge.classList.remove('show'), 2000);
    }
  } catch (_) {}
}

// Global keyboard shortcuts
document.addEventListener('keydown', e => {
  // Ctrl+S — save file editor
  if ((e.ctrlKey || e.metaKey) && e.key === 's') {
    if (editorCurrentPath) {
      e.preventDefault();
      saveEditorFile();
    }
  }
  // Ctrl+K — open workspace search
  if ((e.ctrlKey || e.metaKey) && e.key === 'k') {
    e.preventDefault();
    openSearch();
  }
  // Escape — close overlays in priority order
  if (e.key === 'Escape') {
    if (document.getElementById('kb-modal').classList.contains('open')) {
      closeKbModal();
    } else if (document.getElementById('search-overlay').classList.contains('open')) {
      closeSearch();
    } else if (document.getElementById('file-editor').classList.contains('open')) {
      closeFileEditor();
    }
  }
});

// ── Knowledge Base tab ────────────────────────────────────────
let kbEntries = [];

async function loadKb() {
  const list = document.getElementById('kb-list');
  list.innerHTML = '<div style="padding:16px;color:var(--text-muted);font-size:12px;text-align:center">Loading…</div>';
  try {
    const res  = await fetch('/knowledge');
    const data = await res.json();
    kbEntries  = data.entries || [];
    renderKb(kbEntries);
  } catch(e) {
    list.innerHTML = `<div style="padding:16px;color:#e05;font-size:12px">${e.message}</div>`;
  }
}

function renderKb(entries) {
  const list = document.getElementById('kb-list');
  document.getElementById('kb-count').textContent = entries.length + ' topic' + (entries.length !== 1 ? 's' : '');
  if (!entries.length) {
    list.innerHTML = '<div style="padding:16px;color:var(--text-muted);font-size:12px;text-align:center">No knowledge stored yet.<br>The AI saves research automatically.</div>';
    return;
  }
  list.innerHTML = entries.map(e => {
    const age   = kbAge(e.updated_at);
    const stale = kbDaysOld(e.updated_at) >= 14;
    const tags  = (e.tags || '').split(',').filter(Boolean).map(t =>
      `<span class="kb-tag">${t.trim()}</span>`).join('');
    return `
    <div class="kb-entry" id="kb-${e.id}">
      <div class="kb-entry-header" onclick="toggleKbEntry('${e.id}')">
        <div>
          <div class="kb-entry-topic">${esc(e.topic)}</div>
          <div class="kb-entry-summary">${esc(e.summary)}</div>
          ${tags ? `<div class="kb-entry-tags">${tags}</div>` : ''}
        </div>
        <div class="kb-entry-meta">${stale ? '⚠ ' : ''}${age} · v${e.version}</div>
      </div>
      <div class="kb-entry-full" id="kb-full-${e.id}"></div>
      <div class="kb-entry-actions" style="display:none">
        <button class="kb-btn" onclick="downloadKbEntry('${e.id}')">Download</button>
        <button class="kb-btn refresh-btn" onclick="refreshKbEntry('${e.id}','${esc(e.topic)}')">↻ Refresh</button>
        <button class="kb-btn" onclick="openKbModal('${e.id}')">Edit</button>
        <button class="kb-btn danger" onclick="deleteKbEntry('${e.id}')">Delete</button>
      </div>
    </div>`;
  }).join('');
}

function toggleKbEntry(id) {
  const el = document.getElementById('kb-' + id);
  el.classList.toggle('open');
  const actions = el.querySelector('.kb-entry-actions');
  actions.style.display = el.classList.contains('open') ? 'flex' : 'none';

  if (el.classList.contains('open')) {
    const entry = kbEntries.find(x => x.id === id);
    const full = document.getElementById('kb-full-' + id);
    if (entry && full && !full.dataset.rendered) {
      const contentMd = entry.content || '';
      let sourcesHtml = '';
      try {
        const src = JSON.parse(entry.sources || '[]');
        if (Array.isArray(src) && src.length) {
          const links = src.slice(0, 20).map(u => `<li><a href="${esc(u)}" target="_blank" rel="noreferrer">${esc(u)}</a></li>`).join('');
          sourcesHtml = `<hr style="border:none;border-top:1px solid var(--border);margin:10px 0" /><div style="font-size:11px;color:var(--text-muted);margin-bottom:6px">Sources</div><ul style="margin:0;padding-left:16px;line-height:1.5">${links}</ul>`;
        }
      } catch (_) {}
      full.innerHTML = renderMarkdown(contentMd) + sourcesHtml;
      full.dataset.rendered = '1';
    }
  }
}

function filterKb() {
  const q = document.getElementById('kb-search').value.toLowerCase();
  if (!q) { renderKb(kbEntries); return; }
  renderKb(kbEntries.filter(e =>
    e.topic.toLowerCase().includes(q) ||
    e.tags.toLowerCase().includes(q)  ||
    e.summary.toLowerCase().includes(q)
  ));
}

async function deleteKbEntry(id) {
  if (!confirm('Delete this knowledge entry?')) return;
  await fetch('/knowledge/' + id, { method: 'DELETE' });
  loadKb();
}

function openKbModal(id) {
  const modal = document.getElementById('kb-modal');
  document.getElementById('kb-modal-id').value = id || '';
  if (id) {
    const e = kbEntries.find(x => x.id === id);
    if (!e) return;
    document.getElementById('kb-modal-title').textContent   = 'Edit Knowledge Entry';
    document.getElementById('kb-modal-topic').value         = e.topic;
    document.getElementById('kb-modal-tags').value          = e.tags;
    document.getElementById('kb-modal-summary').value       = e.summary;
    document.getElementById('kb-modal-content').value       = e.content;
  } else {
    document.getElementById('kb-modal-title').textContent   = 'Add Knowledge Entry';
    document.getElementById('kb-modal-topic').value         = '';
    document.getElementById('kb-modal-tags').value          = '';
    document.getElementById('kb-modal-summary').value       = '';
    document.getElementById('kb-modal-content').value       = '';
  }
  modal.classList.add('open');
}

function closeKbModal() {
  document.getElementById('kb-modal').classList.remove('open');
}

async function saveKbEntry() {
  const topic   = document.getElementById('kb-modal-topic').value.trim();
  const tags    = document.getElementById('kb-modal-tags').value.trim();
  const summary = document.getElementById('kb-modal-summary').value.trim();
  const content = document.getElementById('kb-modal-content').value.trim();
  if (!topic || !content) { alert('Topic and content are required.'); return; }
  await fetch('/knowledge', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ topic, tags, summary, content }),
  });
  closeKbModal();
  loadKb();
}

async function refreshKbEntry(id, topic) {
  if (!confirm(`Re-research "${topic}" and update this entry?`)) return;
  // Send a research request to the AI
  const msg = `Research and update the knowledge entry for: ${topic}. Search for the latest information and save updated knowledge using save_knowledge.`;
  document.getElementById('user-input').value = msg;
  sendMessage();
  showSidebarTab('chat');
}

function downloadKbEntry(id) {
  // Trigger a file download; server returns `Content-Disposition: attachment`.
  window.open('/knowledge/' + id + '/download', '_blank');
}

function exportKbAll() {
  window.open('/knowledge/export', '_blank');
}

function kbDaysOld(ts) {
  if (!ts) return 0;
  return Math.floor((Date.now() - new Date(ts).getTime()) / 86400000);
}

function kbAge(ts) {
  const d = kbDaysOld(ts);
  if (d === 0) return 'today';
  if (d === 1) return '1d ago';
  if (d < 30)  return d + 'd ago';
  return Math.floor(d/30) + 'mo ago';
}

function esc(s) {
  return String(s||'').replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;');
}

// ── Settings panel ────────────────────────────────────────────
let settingsOpen = false;

function toggleSettings() {
  settingsOpen = !settingsOpen;
  document.getElementById('settings-panel').classList.toggle('open', settingsOpen);
  if (settingsOpen) loadSettings();
}

async function loadSettings() {
  try {
    const res = await fetch('/settings');
    if (!res.ok) return;
    const cfg = await res.json();
    document.getElementById('cfg-workspace').value    = cfg.workspace_path  || '';
    document.getElementById('cfg-ollama-url').value   = cfg.ollama_url      || '';
    document.getElementById('cfg-fast-model').value   = cfg.fast_model      || '';
    document.getElementById('cfg-critic-model').value = cfg.critic_model    || '';
    document.getElementById('cfg-max-steps').value    = cfg.max_steps       || 20;
    document.getElementById('cfg-num-gpu').value      = cfg.num_gpu    ?? 999;
    document.getElementById('cfg-num-ctx').value      = cfg.num_ctx    ?? 4096;
    document.getElementById('cfg-num-predict').value  = cfg.num_predict ?? 2048;
    document.getElementById('cfg-num-batch').value    = cfg.num_batch  ?? 512;
    document.getElementById('cfg-num-thread').value   = cfg.num_thread ?? 0;
    document.getElementById('cfg-search-url').value   = cfg.search_url      || '';
    document.getElementById('cfg-auto-kb-mode').value = cfg.auto_kb_mode    || 'off';
    document.getElementById('cfg-auto-kb-min').value  = cfg.auto_kb_min_chars ?? 1200;
    const threshold = cfg.risk_gate_threshold ?? 8;
    document.getElementById('cfg-risk-threshold').value = threshold;
    document.getElementById('cfg-risk-val').textContent  = threshold;
    // Keep sidebar model tags in sync
    updateModelTags(cfg.fast_model, cfg.critic_model);
  } catch (e) {
    console.warn('Failed to load settings:', e);
  }
}

async function saveSettings() {
  const cfg = {
    workspace_path:      document.getElementById('cfg-workspace').value.trim(),
    ollama_url:          document.getElementById('cfg-ollama-url').value.trim(),
    fast_model:          document.getElementById('cfg-fast-model').value.trim(),
    critic_model:        document.getElementById('cfg-critic-model').value.trim(),
    max_steps:       parseInt(document.getElementById('cfg-max-steps').value) || 20,
    risk_gate_threshold: parseInt(document.getElementById('cfg-risk-threshold').value) || 8,
    num_gpu:         parseInt(document.getElementById('cfg-num-gpu').value)    ?? 999,
    num_ctx:         parseInt(document.getElementById('cfg-num-ctx').value)    ?? 4096,
    num_predict:     parseInt(document.getElementById('cfg-num-predict').value) ?? 2048,
    num_batch:       parseInt(document.getElementById('cfg-num-batch').value)  ?? 512,
    num_thread:      parseInt(document.getElementById('cfg-num-thread').value) ?? 0,
    search_url:          document.getElementById('cfg-search-url').value.trim(),
    auto_kb_mode:        document.getElementById('cfg-auto-kb-mode').value,
    auto_kb_min_chars: parseInt(document.getElementById('cfg-auto-kb-min').value) || 1200,
  };
  try {
    const res = await fetch('/settings', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(cfg),
    });
    if (!res.ok) throw new Error(`${res.status}`);
    updateModelTags(cfg.fast_model, cfg.critic_model);
    const msg = document.getElementById('settings-save-msg');
    msg.classList.add('visible');
    setTimeout(() => msg.classList.remove('visible'), 2500);
  } catch (e) {
    alert('Failed to save settings: ' + e.message);
  }
}

function updateModelTags(fast, critic) {
  const f = document.getElementById('footer-fast-model');
  const c = document.getElementById('footer-critic-model');
  if (f && fast)   f.textContent = fast   + ' · fast';
  if (c && critic) c.textContent = critic + ' · critic';
}

// ── Git panel ─────────────────────────────────────────────────
let gitStatus = null;
let diffCounter = 0;

async function loadGitStatus() {
  const bodyEl = document.getElementById('git-body');
  const branchEl = document.getElementById('git-branch-name');
  const commitForm = document.getElementById('git-commit-form');
  bodyEl.innerHTML = '<div id="git-empty"><div class="dots"><span></span><span></span><span></span></div></div>';
  try {
    const res = await fetch('/git/status');
    if (!res.ok) throw new Error('not a git repo');
    gitStatus = await res.json();

    branchEl.textContent = '⎇ ' + (gitStatus.branch || 'HEAD');
    if (gitStatus.ahead || gitStatus.behind) {
      branchEl.textContent += ` ↑${gitStatus.ahead||0} ↓${gitStatus.behind||0}`;
    }
    bodyEl.innerHTML = '';

    const staged    = gitStatus.staged    || [];
    const unstaged  = gitStatus.unstaged  || [];
    const untracked = gitStatus.untracked || [];

    if (staged.length === 0 && unstaged.length === 0 && untracked.length === 0) {
      bodyEl.innerHTML = '<div id="git-empty">✓ Working tree clean</div>';
      commitForm.style.display = 'none';
      loadGitLog();
      return;
    }

    if (staged.length > 0) {
      const hdr = document.createElement('div');
      hdr.className = 'git-section-header';
      hdr.innerHTML = '<span>Staged</span>';
      bodyEl.appendChild(hdr);
      for (const f of staged) renderGitFileRow(bodyEl, f, true, false);
    }

    if (unstaged.length > 0 || untracked.length > 0) {
      const hdr = document.createElement('div');
      hdr.className = 'git-section-header';
      hdr.innerHTML = '<span>Changes</span><button class="git-stage-all-btn" onclick="stageAll()">Stage All</button>';
      bodyEl.appendChild(hdr);
      for (const f of unstaged)  renderGitFileRow(bodyEl, f, false, false);
      for (const f of untracked) renderGitFileRow(bodyEl, f, false, true);
    }

    commitForm.style.display = staged.length > 0 ? 'block' : 'none';
    loadGitLog();
  } catch (e) {
    bodyEl.innerHTML = `<div id="git-empty">No git repository found.<br><span style="font-size:10px">${esc(e.message)}</span></div>`;
    branchEl.textContent = '─ no repo ─';
    commitForm.style.display = 'none';
  }
}

function renderGitFileRow(container, file, staged, isUntracked) {
  const statusChar = isUntracked ? '?' : (file.status || 'M');
  const cls = { M:'s-M', A:'s-A', D:'s-D', R:'s-R', '?':'s-u' }[statusChar] || 's-M';
  const diffId = 'gdiff' + (++diffCounter);

  const row = document.createElement('div');
  row.className = 'git-file-row';
  row.dataset.diffid = diffId;
  row.innerHTML = `<span class="git-status-badge ${cls}">${statusChar}</span><span class="git-file-path" title="${esc(file.path)}">${esc(file.path)}</span>${!staged ? `<button class="git-stage-btn" onclick="event.stopPropagation();stageFile('${esc(file.path).replace(/'/g,"\\'")}')">+Stage</button>` : ''}`;
  row.onclick = () => toggleFileDiff(diffId, file.path, staged);
  container.appendChild(row);

  const diff = document.createElement('div');
  diff.className = 'git-diff-wrap';
  diff.id = diffId;
  container.appendChild(diff);
}

async function toggleFileDiff(diffId, filePath, staged) {
  const diffEl = document.getElementById(diffId);
  if (!diffEl) return;
  if (diffEl.classList.contains('open')) { diffEl.classList.remove('open'); return; }

  diffEl.innerHTML = '<div style="padding:8px 12px;color:var(--text-dim);font-size:11px">Loading…</div>';
  diffEl.classList.add('open');
  try {
    const res = await fetch(`/git/diff?path=${encodeURIComponent(filePath)}&staged=${staged}`);
    if (!res.ok) throw new Error('diff unavailable');
    const data = await res.json();
    renderDiff(diffEl, data.diff || '');
  } catch (e) {
    diffEl.innerHTML = `<div style="padding:8px;color:#9a5a5a;font-size:11px">${esc(e.message)}</div>`;
  }
}

function renderDiff(el, diff) {
  el.innerHTML = '';
  if (!diff.trim()) {
    el.innerHTML = '<div style="padding:8px 12px;color:var(--text-dim);font-size:11px">No diff available</div>';
    return;
  }
  for (const line of diff.split('\n')) {
    const d = document.createElement('div');
    d.className = 'diff-line';
    if (line.startsWith('+') && !line.startsWith('+++')) d.classList.add('diff-add');
    else if (line.startsWith('-') && !line.startsWith('---')) d.classList.add('diff-del');
    else if (line.startsWith('@@')) d.classList.add('diff-hunk');
    else if (/^(diff |index |--- |\+\+\+ )/.test(line)) d.classList.add('diff-meta');
    d.textContent = line;
    el.appendChild(d);
  }
}

async function stageFile(path) {
  try {
    await fetch('/git/stage', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ paths: [path] }),
    });
    loadGitStatus();
  } catch (e) { console.warn('stage failed:', e); }
}

async function stageAll() {
  const paths = (gitStatus?.unstaged  || []).map(f => f.path)
    .concat((gitStatus?.untracked || []).map(f => f.path));
  if (paths.length === 0) return;
  try {
    await fetch('/git/stage', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ paths }),
    });
    loadGitStatus();
  } catch (e) { console.warn('stage all failed:', e); }
}

async function commitChanges() {
  const msg = document.getElementById('git-commit-msg').value.trim();
  if (!msg) { document.getElementById('git-commit-msg').focus(); return; }
  const btn    = document.getElementById('git-commit-btn');
  const status = document.getElementById('git-commit-status');
  btn.disabled = true;
  status.textContent = 'Committing…';
  try {
    const res = await fetch('/git/commit', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ message: msg }),
    });
    const data = await res.json();
    if (data.success) {
      document.getElementById('git-commit-msg').value = '';
      status.textContent = '✓ ' + (data.hash ? data.hash.slice(0,7) : 'committed');
      setTimeout(() => { status.textContent = ''; }, 3000);
      loadGitStatus();
    } else {
      status.textContent = '✗ ' + (data.error || 'commit failed');
    }
  } catch (e) {
    status.textContent = '✗ ' + e.message;
  }
  btn.disabled = false;
}

async function loadGitLog() {
  const bodyEl = document.getElementById('git-body');
  try {
    const res = await fetch('/git/log?limit=10');
    if (!res.ok) return;
    const data = await res.json();
    const commits = data.commits || [];
    if (commits.length === 0) return;
    const hdr = document.createElement('div');
    hdr.className = 'git-section-header';
    hdr.innerHTML = '<span>Recent Commits</span>';
    bodyEl.appendChild(hdr);
    for (const c of commits) {
      const item = document.createElement('div');
      item.className = 'git-commit-item';
      item.innerHTML = `<div><span class="git-commit-hash">${esc(c.hash.slice(0,7))}</span><span class="git-commit-msg-text">${esc(c.message)}</span></div><div class="git-commit-meta">${esc(c.author)} · ${esc(c.date)}</div>`;
      bodyEl.appendChild(item);
    }
  } catch (_) {}
}

// ── File management ───────────────────────────────────────────
let ctxMenuTarget = null;

function showNewMenu(e) {
  e.stopPropagation();
  const menu = document.getElementById('new-menu');
  const rect = e.currentTarget.getBoundingClientRect();
  menu.style.left = rect.left + 'px';
  menu.style.top  = (rect.bottom + 4) + 'px';
  menu.style.display = 'block';
}

function hideNewMenu() {
  const m = document.getElementById('new-menu');
  if (m) m.style.display = 'none';
}

async function promptNewFile() {
  hideNewMenu();
  const name = prompt('New file name (e.g. notes.txt or subdir/file.rs):');
  if (!name || !name.trim()) return;
  try {
    const res = await fetch('/workspace/file', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ path: name.trim().replace(/\\/g,'/'), content: '' }),
    });
    if (!res.ok) { const d = await res.json().catch(()=>{}); throw new Error(d?.error || res.status); }
    loadFileTree();
  } catch (e) { alert('Could not create file: ' + e.message); }
}

async function promptNewFolder() {
  hideNewMenu();
  const name = prompt('New folder name:');
  if (!name || !name.trim()) return;
  try {
    const res = await fetch('/workspace/mkdir', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ path: name.trim() }),
    });
    if (!res.ok) { const d = await res.json().catch(()=>{}); throw new Error(d?.error || res.status); }
    loadFileTree();
  } catch (e) { alert('Could not create folder: ' + e.message); }
}

function showCtxMenu(e, path, name, isDir, nodeEl) {
  e.preventDefault();
  e.stopPropagation();
  ctxMenuTarget = { path, name, isDir, el: nodeEl };
  const menu = document.getElementById('ctx-menu');
  const dl = document.getElementById('ctx-download');
  const sep = document.getElementById('ctx-sep1');
  if (dl) dl.style.display = isDir ? 'none' : 'block';
  if (sep) sep.style.display = isDir ? 'none' : 'block';
  const x = Math.min(e.clientX, window.innerWidth - 160);
  const y = Math.min(e.clientY, window.innerHeight - 90);
  menu.style.left = x + 'px';
  menu.style.top  = y + 'px';
  menu.style.display = 'block';
}

function hideCtxMenu() {
  const m = document.getElementById('ctx-menu');
  if (m) m.style.display = 'none';
  ctxMenuTarget = null;
}

function downloadFile(path, name) {
  if (!path) return;
  const url = `/workspace/download?path=${encodeURIComponent(path)}`;
  const a = document.createElement('a');
  a.href = url;
  a.download = (name || path.split('/').pop() || 'download');
  document.body.appendChild(a);
  a.click();
  a.remove();
}

function ctxDownload() {
  if (!ctxMenuTarget || ctxMenuTarget.isDir) return;
  hideCtxMenu();
  downloadFile(ctxMenuTarget.path, ctxMenuTarget.name);
}

async function ctxRename() {
  if (!ctxMenuTarget) return;
  hideCtxMenu();
  const { path, name } = ctxMenuTarget;
  const newName = prompt(`Rename "${name}" to:`, name);
  if (!newName || !newName.trim() || newName.trim() === name) return;
  const parts = path.split('/');
  parts[parts.length - 1] = newName.trim();
  const newPath = parts.join('/');
  try {
    const res = await fetch('/workspace/rename', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ from: path, to: newPath }),
    });
    if (!res.ok) { const d = await res.json().catch(()=>{}); throw new Error(d?.error || res.status); }
    if (editorCurrentPath === path) closeFileEditor();
    loadFileTree();
  } catch (e) { alert('Rename failed: ' + e.message); }
}

async function ctxDelete() {
  if (!ctxMenuTarget) return;
  hideCtxMenu();
  const { path, name, isDir } = ctxMenuTarget;
  const msg = isDir
    ? `Delete folder "${name}" and everything inside? This cannot be undone.`
    : `Delete "${name}"? This cannot be undone.`;
  if (!confirm(msg)) return;
  try {
    const res = await fetch(`/workspace/file?path=${encodeURIComponent(path)}`, { method: 'DELETE' });
    if (!res.ok) { const d = await res.json().catch(()=>{}); throw new Error(d?.error || res.status); }
    if (editorCurrentPath === path) closeFileEditor();
    loadFileTree();
  } catch (e) { alert('Delete failed: ' + e.message); }
}

// Close menus on outside click
document.addEventListener('click', () => { hideCtxMenu(); hideNewMenu(); });

// ── Memory browser ───────────────────────────────────────────
let memTasksData = [];

async function loadMemory() {
  const listEl = document.getElementById('mem-list');
  listEl.innerHTML = '<div class="mem-empty"><div class="dots"><span></span><span></span><span></span></div></div>';
  try {
    const [tasksRes, semRes] = await Promise.all([
      fetch('/memory/tasks?limit=50'),
      fetch('/memory/semantic'),
    ]);
    const tasks   = tasksRes.ok ? await tasksRes.json() : [];
    const semList = semRes.ok   ? await semRes.json()   : [];

    document.getElementById('mem-semantic-count').textContent =
      `${semList.length} in semantic index`;

    memTasksData = Array.isArray(tasks) ? tasks : [];
    renderMemTasks(memTasksData);
  } catch (e) {
    listEl.innerHTML = `<div class="mem-empty">${esc(e.message)}</div>`;
  }
}

function renderMemTasks(tasks) {
  const listEl = document.getElementById('mem-list');
  listEl.innerHTML = '';
  if (tasks.length === 0) {
    listEl.innerHTML = '<div class="mem-empty">No tasks recorded yet.<br>Run a task to start building memory.</div>';
    return;
  }
  for (const t of tasks) {
    const item  = document.createElement('div');
    item.className = 'mem-task';
    const badge  = t.success ? '✅' : '❌';
    const dur    = t.duration_ms < 1000
      ? `${t.duration_ms}ms`
      : `${(t.duration_ms / 1000).toFixed(1)}s`;
    const age    = memFormatAge(t.created_at);
    const extras = [];
    if (t.failure_count  > 0) extras.push(`${t.failure_count} fail`);
    if (t.repair_cycles  > 0) extras.push(`${t.repair_cycles} repair`);
    const meta = [dur, ...extras].join(' · ');
    const uid  = 'mt' + t.task_id.replace(/-/g, '').slice(0, 8);
    const req  = t.user_request || '';
    const planText = t.plan_json
      ? (() => { try { return JSON.stringify(JSON.parse(t.plan_json), null, 2); } catch(_){ return t.plan_json; } })()
      : '— no plan recorded —';
    const detailText =
      `task_id: ${t.task_id}\n` +
      `success: ${!!t.success}\n` +
      `created_at: ${t.created_at || ''}\n` +
      `duration_ms: ${t.duration_ms}\n` +
      (t.user_request ? `\nrequest:\n${t.user_request}\n` : '') +
      `\nplan:\n${planText}`;

    item.innerHTML = `
      <div class="mem-task-header" id="${uid}-hdr">
        <span class="mem-task-badge">${badge}</span>
        <span class="mem-task-req" title="${esc(req)}">${esc(req.slice(0,55))}${req.length>55?'…':''}</span>
        <span class="mem-task-time">${age}</span>
        <button class="mem-delete-btn" title="Delete record" onclick="event.stopPropagation();deleteMemTask('${t.task_id}')">×</button>
      </div>
      <div class="mem-task-meta">${meta}</div>
      <div class="mem-task-detail" id="${uid}" onclick="event.stopPropagation()">
        <div style="display:flex;justify-content:flex-end;margin:-2px 0 6px 0">
          <button class="kb-btn" style="font-size:10px;padding:2px 8px" onclick="event.stopPropagation();copyMemText('${uid}')">Copy</button>
        </div>
        <div>${esc(detailText)}</div>
      </div>`;

    // Only toggle open/closed when clicking the header, so text selection in the detail
    // panel doesn't collapse it.
    const hdr = item.querySelector('#' + uid + '-hdr');
    if (hdr) hdr.onclick = () => document.getElementById(uid).classList.toggle('open');
    listEl.appendChild(item);
  }
}

async function copyMemText(uid) {
  const el = document.getElementById(uid);
  if (!el) return;
  // Copy the raw visible text (plan + request).
  const text = el.innerText.replace(/^\s*Copy\s*/,'').trim();
  try {
    await navigator.clipboard.writeText(text);
  } catch (_) {
    // Fallback for older browsers
    const ta = document.createElement('textarea');
    ta.value = text;
    document.body.appendChild(ta);
    ta.select();
    try { document.execCommand('copy'); } catch (_) {}
    document.body.removeChild(ta);
  }
}

function filterMemTasks() {
  const q = document.getElementById('mem-search').value.trim().toLowerCase();
  renderMemTasks(q
    ? memTasksData.filter(t => (t.user_request||'').toLowerCase().includes(q))
    : memTasksData
  );
}

async function deleteMemTask(taskId) {
  try {
    const r = await fetch(`/memory/tasks/${taskId}`, { method: 'DELETE' });
    if (r.ok || r.status === 204) loadMemory();
  } catch (_) {}
}

async function clearAllMemory() {
  if (!confirm('Delete all task records and semantic memory entries? This cannot be undone.')) return;
  try {
    await fetch('/memory/clear', { method: 'POST' });
    loadMemory();
  } catch (_) {}
}

function memFormatAge(iso) {
  if (!iso) return '';
  const diff = Date.now() - new Date(iso).getTime();
  const m = Math.floor(diff / 60000);
  const h = Math.floor(diff / 3600000);
  const d = Math.floor(diff / 86400000);
  if (m < 2)  return 'just now';
  if (m < 60) return `${m}m ago`;
  if (h < 24) return `${h}h ago`;
  return `${d}d ago`;
}

// ── Workspace search ──────────────────────────────────────────
let searchDebounceTimer = null;

function openSearch() {
  document.getElementById('search-overlay').classList.add('open');
  setTimeout(() => document.getElementById('search-input').focus(), 60);
}

function closeSearch() {
  document.getElementById('search-overlay').classList.remove('open');
  document.getElementById('search-input').value = '';
  document.getElementById('search-results').innerHTML = '<div class="search-empty">Type to search file contents</div>';
}

function doSearch() {
  clearTimeout(searchDebounceTimer);
  const q = document.getElementById('search-input').value.trim();
  if (!q) {
    document.getElementById('search-results').innerHTML = '<div class="search-empty">Type to search file contents</div>';
    return;
  }
  document.getElementById('search-results').innerHTML = '<div class="search-empty"><div class="dots"><span></span><span></span><span></span></div></div>';
  searchDebounceTimer = setTimeout(() => performSearch(q), 320);
}

async function performSearch(q) {
  try {
    const res = await fetch(`/workspace/search?q=${encodeURIComponent(q)}`);
    if (!res.ok) throw new Error('search failed');
    const data = await res.json();
    renderSearchResults(data.matches || [], q);
  } catch (e) {
    document.getElementById('search-results').innerHTML = `<div class="search-empty">${esc(e.message)}</div>`;
  }
}

function renderSearchResults(matches, query) {
  const el = document.getElementById('search-results');
  const footer = document.getElementById('search-footer');
  if (matches.length === 0) {
    el.innerHTML = '<div class="search-empty">No results found.</div>';
    footer.textContent = '0 results';
    return;
  }
  footer.textContent = `${matches.length} result${matches.length !== 1 ? 's' : ''}${matches.length === 200 ? ' (limit reached)' : ''} · ↵ open · Esc close`;
  el.innerHTML = '';
  for (const m of matches) {
    const row = document.createElement('div');
    row.className = 'search-result';
    // Highlight query term in preview
    const lower = m.preview.toLowerCase();
    const qi = lower.indexOf(query.toLowerCase());
    let previewHtml;
    if (qi >= 0) {
      const before = esc(m.preview.slice(0, qi));
      const match  = esc(m.preview.slice(qi, qi + query.length));
      const after  = esc(m.preview.slice(qi + query.length));
      previewHtml = `${before}<mark style="background:rgba(201,123,75,0.3);color:var(--text);border-radius:2px">${match}</mark>${after}`;
    } else {
      previewHtml = esc(m.preview);
    }
    row.innerHTML = `<span class="search-result-file">${esc(m.file)}</span><span class="search-result-line">:${m.line}</span><span class="search-result-preview">${previewHtml}</span>`;
    row.onclick = () => {
      closeSearch();
      showSidebarTab('files');
      const fname = m.file.split('/').pop();
      openFileEditor(m.file, fname);
    };
    el.appendChild(row);
  }
}

// ── Conversation export ───────────────────────────────────────
function exportConversation() {
  const turns = document.querySelectorAll('.turn');
  if (!turns.length) { alert('No conversation to export.'); return; }
  const date = new Date().toLocaleString();
  let md = `# aihomeserver — conversation export\n\n_Exported: ${date}_\n\n---\n\n`;
  for (const turn of turns) {
    if (turn.classList.contains('user')) {
      const bubble = turn.querySelector('.user-bubble');
      if (bubble) md += `**You:** ${bubble.innerText.trim()}\n\n`;
    } else if (turn.classList.contains('ai')) {
      const content = turn.querySelector('.ai-content');
      if (content) md += `**Assistant:** ${content.innerText.trim()}\n\n`;
      md += '---\n\n';
    }
  }
  const blob = new Blob([md], { type: 'text/markdown' });
  const a = document.createElement('a');
  a.href = URL.createObjectURL(blob);
  a.download = 'aihomeserver-' + new Date().toISOString().split('T')[0] + '.md';
  a.click();
  setTimeout(() => URL.revokeObjectURL(a.href), 10000);
}

// ── Desktop notifications ─────────────────────────────────────
if ('Notification' in window && Notification.permission === 'default') {
  // Silently request; browser won't prompt until user gesture, but primes the pipe
  Notification.requestPermission().catch(() => {});
}

function maybeNotify(title, body) {
  if (!('Notification' in window) || Notification.permission !== 'granted') return;
  if (!document.hidden) return;
  try {
    new Notification(title, { body, icon: '/favicon.ico', tag: 'aihomeserver-done' });
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
loadSettings();   // sync model tags in sidebar with actual config
initFileUploadUI();
inputEl.focus();
</script>
</body>
</html>
"#;
