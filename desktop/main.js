const fs = require('node:fs');
const { app, BrowserWindow, dialog, ipcMain, shell } = require('electron');
const { spawn, spawnSync } = require('node:child_process');
const http = require('node:http');
const https = require('node:https');
const net = require('node:net');
const path = require('node:path');
const { bootstrapHyperV, stopHyperV, startHyperV, getHyperVStatus, exportHyperVLogs, ensureWorkerToken } = require('./hyperv');

const DEFAULT_URL = process.env.AIHOMESERVER_URL || 'http://127.0.0.1:3000';
const DEFAULT_COORDINATOR_HOST_PORT = Number(process.env.AIHOMESERVER_HOST_PORT || 3000);
const APP_NAME = 'AI Home Server';
const AUTO_START_DOCKER = process.env.AIHOMESERVER_AUTO_START_DOCKER !== '0';
const DEFAULT_BUNDLED_REPO_DIR = app.isPackaged
  ? path.join(process.resourcesPath, 'repo')
  : path.join(__dirname, '..');
const COMPOSE_DIR = process.env.AIHOMESERVER_COMPOSE_DIR || DEFAULT_BUNDLED_REPO_DIR;
const COMPOSE_FILES = (process.env.AIHOMESERVER_COMPOSE_FILES || 'docker-compose.yml,docker-compose.dev.yml')
  .split(',')
  .map((entry) => entry.trim())
  .filter(Boolean);
const DEFAULT_VM_PORT = Number(process.env.AIHOMESERVER_VM_PORT || 3031);
const DEFAULT_VM_IP = process.env.AIHOMESERVER_VM_IP || '192.168.250.10';
const DEFAULT_VM_GATEWAY = process.env.AIHOMESERVER_VM_GATEWAY || '192.168.250.1';
const DEFAULT_VM_SWITCH = process.env.AIHOMESERVER_VM_SWITCH || 'AIHomeServerSwitch';
const DEFAULT_VM_NAME = process.env.AIHOMESERVER_VM_NAME || 'AIHomeServerWorker';
const DEFAULT_VM_CPUS = Number(process.env.AIHOMESERVER_VM_CPUS || 4);
const DEFAULT_VM_MEMORY_MB = Number(process.env.AIHOMESERVER_VM_MEMORY_MB || 8192);
const DEFAULT_VM_IMAGE_VERSION = process.env.AIHOMESERVER_VM_IMAGE_VERSION || '24.04';
const DEFAULT_REPO_URL = process.env.AIHOMESERVER_REPO_URL || 'https://github.com/kab102395/aihomeserver.git';
const DEFAULT_REPO_BRANCH = process.env.AIHOMESERVER_REPO_BRANCH || 'main';
const RUNTIME_MODE = (process.env.AIHOMESERVER_RUNTIME || '').trim().toLowerCase();
const DEFAULT_HYPERV_ROOT =
  process.env.AIHOMESERVER_VM_ROOT ||
  path.join(process.env.ProgramData || 'C:\\ProgramData', 'AIHomeServer', 'hyperv');
const DEFAULT_HYPERV_IMAGE = path.join(
  DEFAULT_HYPERV_ROOT,
  'image',
  `ubuntu-${DEFAULT_VM_IMAGE_VERSION}`,
  `ubuntu-${DEFAULT_VM_IMAGE_VERSION}-server-cloudimg-amd64-base.vhdx`
);
const LAUNCHER_LOG_DIR_NAME = 'logs';

function launcherLogDir() {
  return path.join(app.getPath('userData'), LAUNCHER_LOG_DIR_NAME);
}

function ensureLauncherLogDir() {
  const dir = launcherLogDir();
  fs.mkdirSync(dir, { recursive: true });
  return dir;
}

function appendLauncherLog(line) {
  try {
    const stamp = new Date().toISOString();
    fs.appendFileSync(path.join(ensureLauncherLogDir(), 'launcher.log'), `[${stamp}] ${line}\n`);
  } catch (_) {}
}

function composeLogPaths() {
  const dir = ensureLauncherLogDir();
  return {
    stdout: path.join(dir, 'coordinator-compose.stdout.log'),
    stderr: path.join(dir, 'coordinator-compose.stderr.log'),
  };
}

function probeHealth(baseUrl, timeoutMs = 2000) {
  return new Promise((resolve) => {
    const target = new URL('/health', baseUrl);
    const transport = target.protocol === 'https:' ? https : http;
    const req = transport.request(
      target,
      { method: 'GET', headers: { 'User-Agent': 'aihomeserver-desktop' } },
      (res) => {
        res.resume();
        resolve(res.statusCode !== undefined && res.statusCode >= 200 && res.statusCode < 300);
      }
    );

    req.setTimeout(timeoutMs, () => {
      req.destroy(new Error('health probe timeout'));
    });
    req.on('error', () => resolve(false));
    req.end();
  });
}

function probeWorkerAuth(token, workerUrl, timeoutMs = 8000) {
  return new Promise((resolve) => {
    const target = new URL('/shell', workerUrl);
    const body = JSON.stringify({
      command: 'echo aihomeserver-auth-probe',
      cwd: '.',
      timeout_secs: 5,
    });
    const transport = target.protocol === 'https:' ? https : http;
    const req = transport.request(
      target,
      {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'Content-Length': Buffer.byteLength(body),
          'User-Agent': 'aihomeserver-desktop',
          Authorization: `Bearer ${token}`,
        },
      },
      (res) => {
        res.resume();
        resolve({ ok: res.statusCode === 200, status: res.statusCode });
      }
    );
    req.setTimeout(timeoutMs, () => {
      req.destroy(new Error('auth probe timeout'));
    });
    req.on('error', (err) => resolve({ ok: false, status: 0, error: err.message }));
    req.write(body);
    req.end();
  });
}

function probeCacheStatus() {
  return {
    imageExists: fs.existsSync(DEFAULT_HYPERV_IMAGE),
    logsExist: fs.existsSync(path.join(DEFAULT_HYPERV_ROOT, 'logs')),
    root: DEFAULT_HYPERV_ROOT,
    imagePath: DEFAULT_HYPERV_IMAGE,
  };
}

function findAvailablePort(preferredPort) {
  function tryListen(port) {
    return new Promise((resolve, reject) => {
      const server = net.createServer();
      server.unref();
      server.on('error', reject);
      server.listen(port, '0.0.0.0', () => {
        const address = server.address();
        const chosenPort = typeof address === 'object' && address ? address.port : port;
        server.close(() => resolve(chosenPort));
      });
    });
  }

  return tryListen(preferredPort).catch(() => tryListen(0));
}

function isPortAllocationError(message) {
  const text = String(message || '').toLowerCase();
  return text.includes('port is already allocated') || text.includes('bind for') || text.includes('failed to set up container networking');
}

function isRunningAsAdministrator() {
  if (process.platform !== 'win32') {
    return true;
  }

  // Exit code 0 = admin, 1 = not admin. Avoids reading stdout with stdio:'ignore'.
  const check = spawnSync(
    'powershell.exe',
    [
      '-NoProfile',
      '-NonInteractive',
      '-Command',
      'if (([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { exit 0 } else { exit 1 }',
    ],
    { stdio: 'ignore' }
  );

  return check.status === 0;
}

function isHyperVAvailable() {
  if (process.platform !== 'win32') return false;
  // Get-VM lives in the Hyper-V module; if it's absent, Hyper-V isn't installed.
  const check = spawnSync(
    'powershell.exe',
    ['-NoProfile', '-NonInteractive', '-Command',
     'if (Get-Command Get-VM -ErrorAction SilentlyContinue) { exit 0 } else { exit 1 }'],
    { stdio: 'ignore' }
  );
  return check.status === 0;
}

function categorizeHyperVError(error) {
  const msg = (error.stack || error.message || '').toLowerCase();
  if (msg.includes('get-vm') && msg.includes('not recognized')) {
    return {
      title: 'Hyper-V is not available',
      body: 'The Hyper-V Windows feature is not enabled on this machine. Enable it via "Turn Windows features on or off" or run:\n\nEnable-WindowsOptionalFeature -Online -FeatureName Microsoft-Hyper-V -All\n\nThen restart and relaunch the app.',
    };
  }
  if (msg.includes('wsl is not installed') || msg.includes('wsl --mount') || msg.includes('the installed wsl does not support')) {
    return {
      title: 'WSL 2 mount support is required',
      body: 'The Hyper-V worker bootstrap now provisions the VM disk offline and requires WSL 2 with `wsl --mount` support. Install or upgrade WSL, then relaunch the app.\n\n' + (error.message || ''),
    };
  }
  if (msg.includes('did not report healthy') || msg.includes('worker_port_open')) {
    return {
      title: 'Worker VM did not come online',
      body: 'The VM started but the worker process did not respond to health checks in time. Check the worker logs for startup errors.\n\n' + (error.message || ''),
    };
  }
  if (msg.includes('did not become healthy and authenticated') || msg.includes('worker auth probe') || msg.includes('/shell returned 401')) {
    return {
      title: 'Worker bootstrap/service startup failed',
      body: 'The VM booted, but the installed worker service did not come up with the expected authenticated state. Check the exported worker logs for guest startup or token wiring errors.\n\n' + (error.message || ''),
    };
  }
  if (msg.includes('access is denied') || msg.includes('elevation') || msg.includes('administrator')) {
    return {
      title: 'Administrator rights required',
      body: 'The Hyper-V bootstrap requires elevation. Right-click the app and choose "Run as administrator".',
    };
  }
  return {
    title: 'Hyper-V worker unavailable, falling back to Docker worker',
    body: `The VM bootstrap failed.\n\n${error.stack || error.message}`,
  };
}

function psQuote(value) {
  return `'${String(value).replace(/'/g, "''")}'`;
}

function relaunchAsAdministrator() {
  const args = process.argv.slice(1).map(psQuote).join(', ');
  const command = `Start-Process -FilePath ${psQuote(process.execPath)} -ArgumentList @(${args}) -Verb RunAs`;
  const result = spawnSync(
    'powershell.exe',
    ['-NoProfile', '-NonInteractive', '-Command', command],
    { stdio: 'ignore' }
  );
  return result.status === 0;
}

function composeCommand() {
  const args = ['compose'];
  for (const file of COMPOSE_FILES) {
    args.push('-f', file);
  }
  args.push('up', '-d', '--build');
  return { command: 'docker', args };
}

function startLocalDockerStack(extraEnv = {}) {
  const { command, args } = composeCommand();
  const logs = composeLogPaths();
  fs.writeFileSync(logs.stdout, '', 'utf8');
  fs.writeFileSync(logs.stderr, '', 'utf8');
  appendLauncherLog(`Starting coordinator stack from ${COMPOSE_DIR} with command: ${command} ${args.join(' ')} (host port ${extraEnv.AIHOMESERVER_HOST_PORT || 'default'})`);
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: COMPOSE_DIR,
      detached: false,
      stdio: ['ignore', 'pipe', 'pipe'],
      windowsHide: true,
      env: { ...process.env, ...extraEnv },
    });
    const stdoutStream = fs.createWriteStream(logs.stdout, { flags: 'a' });
    const stderrStream = fs.createWriteStream(logs.stderr, { flags: 'a' });

    child.stdout.on('data', (chunk) => stdoutStream.write(chunk));
    child.stderr.on('data', (chunk) => stderrStream.write(chunk));

    child.on('error', reject);
    child.on('exit', (code) => {
      stdoutStream.end();
      stderrStream.end();
      appendLauncherLog(`docker compose exited with code ${code}`);
      if (code === 0) {
        resolve();
        return;
      }
      reject(
        new Error(
          `docker compose exited with code ${code}. Logs: ${logs.stdout} and ${logs.stderr}`
        )
      );
    });
  });
}

async function startLocalDockerStackWithRetry(baseEnv = {}, maxAttempts = 5) {
  let lastError = null;
  for (let attempt = 1; attempt <= maxAttempts; attempt += 1) {
    const chosenPort = baseEnv.AIHOMESERVER_HOST_PORT || String(await findAvailablePort(0));
    const env = { ...baseEnv, AIHOMESERVER_HOST_PORT: String(chosenPort) };
    appendLauncherLog(`Coordinator startup attempt ${attempt}/${maxAttempts} using host port ${env.AIHOMESERVER_HOST_PORT}`);
    try {
      await startLocalDockerStack(env);
      return { hostPort: Number(env.AIHOMESERVER_HOST_PORT) };
    } catch (error) {
      lastError = error;
      appendLauncherLog(`Coordinator startup attempt ${attempt} failed: ${error.message}`);
      if (!isPortAllocationError(error.message) || attempt === maxAttempts) {
        throw error;
      }
    }
  }
  throw lastError || new Error('Coordinator startup failed without a reported error');
}

async function waitForServerReady(baseUrl, timeoutMs = 180000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (await probeHealth(baseUrl, 1500)) {
      return true;
    }
    await new Promise((resolve) => setTimeout(resolve, 1500));
  }
  return false;
}

function createWindow() {
  const win = new BrowserWindow({
    width: 1600,
    height: 1000,
    backgroundColor: '#111111',
    title: APP_NAME,
    autoHideMenuBar: true,
    webPreferences: {
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
      preload: require('path').join(__dirname, 'preload.js'),
    },
  });

  return win;
}

ipcMain.handle('open-worker-folder', async (_event, kind) => {
  const target = kind === 'logs' ? path.join(DEFAULT_HYPERV_ROOT, 'logs') : DEFAULT_HYPERV_ROOT;
  if (kind === 'logs') {
    try {
      await exportHyperVLogs({
        vmName: DEFAULT_VM_NAME,
        vmIp: DEFAULT_VM_IP,
        workerPort: DEFAULT_VM_PORT,
        rootDir: DEFAULT_HYPERV_ROOT,
      });
    } catch (error) {
      fs.mkdirSync(target, { recursive: true });
      fs.writeFileSync(
        path.join(target, 'export-error.txt'),
        `${new Date().toISOString()}\n${error.stack || error.message}\n`,
        'utf8'
      );
    }
  }
  await shell.openPath(target);
  return target;
});

ipcMain.handle('open-launcher-log-folder', async () => {
  const target = ensureLauncherLogDir();
  await shell.openPath(target);
  return target;
});

ipcMain.handle('get-vm-state', async () => {
  try {
    const status = await getHyperVStatus({
      vmName: DEFAULT_VM_NAME,
      vmIp: DEFAULT_VM_IP,
      workerPort: DEFAULT_VM_PORT,
    });
    const workerHealthy = status.worker_port_open
      ? await probeHealth(`http://${DEFAULT_VM_IP}:${DEFAULT_VM_PORT}`, 2000)
      : false;
    return { ...status, worker_healthy: workerHealthy };
  } catch (error) {
    return { ok: false, vm_state: 'Error', worker_healthy: false, error: error.message };
  }
});

ipcMain.handle('stop-vm', async () => {
  try {
    return await stopHyperV(DEFAULT_VM_NAME);
  } catch (error) {
    return { ok: false, error: error.message };
  }
});

ipcMain.handle('start-vm', async () => {
  try {
    return await startHyperV({
      vmName: DEFAULT_VM_NAME,
      vmIp: DEFAULT_VM_IP,
      vmGateway: DEFAULT_VM_GATEWAY,
      switchName: DEFAULT_VM_SWITCH,
      workerPort: DEFAULT_VM_PORT,
    });
  } catch (error) {
    return { ok: false, error: error.message };
  }
});

function escapeHtml(value) {
  return String(value)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

function renderStatusBadge(state) {
  const label = String(state || 'pending');
  return `<span class="badge badge-${escapeHtml(label)}">${escapeHtml(label)}</span>`;
}

function renderStatusRow(label, state, detail) {
  return `
    <div class="row">
      <div class="row-label">${escapeHtml(label)}</div>
      <div class="row-state">${renderStatusBadge(state)}</div>
      <div class="row-detail">${escapeHtml(detail || '')}</div>
    </div>`;
}

function renderCacheRow(cache) {
  const state = cache.imageExists ? 'warm' : 'cold';
  const label = cache.imageExists ? 'Warm cache' : 'Cold cache';
  const detail = cache.imageExists
    ? `Ubuntu image cached at ${cache.imagePath}`
    : `Ubuntu image will download to ${cache.imagePath}`;
  return `
    <div class="row">
      <div class="row-label">${escapeHtml(label)}</div>
      <div class="row-state">${renderStatusBadge(state)}</div>
      <div class="row-detail">${escapeHtml(detail)}</div>
    </div>`;
}

function loadStartingPage(win, options) {
  const {
    title = `Starting ${APP_NAME}`,
    detail = 'The desktop app is waiting for services to come online.',
    runtimeLabel = 'docker',
    coordinatorUrl = DEFAULT_URL,
    workerUrl = 'pending',
    vmName = DEFAULT_VM_NAME,
    vmIp = DEFAULT_VM_IP,
    vmState = 'pending',
    coordinatorState = 'pending',
    workerState = 'pending',
    readyState = 'pending',
    cache = probeCacheStatus(),
  } = options || {};
  const html = `<!doctype html>
<html>
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>${APP_NAME}</title>
    <style>
      body {
        margin: 0;
        min-height: 100vh;
        display: grid;
        place-items: center;
        background:
          radial-gradient(circle at top left, rgba(91, 141, 255, 0.18), transparent 32%),
          radial-gradient(circle at bottom right, rgba(33, 193, 136, 0.14), transparent 28%),
          linear-gradient(135deg, #0e1117, #141b24 55%, #0b1320);
        color: #d7e1ee;
        font-family: "Segoe UI", Arial, Helvetica, sans-serif;
      }
      .card {
        width: min(860px, calc(100vw - 48px));
        padding: 32px 34px 28px;
        border: 1px solid rgba(255, 255, 255, 0.08);
        border-radius: 20px;
        background: rgba(14, 18, 24, 0.86);
        box-shadow: 0 24px 72px rgba(0, 0, 0, 0.35);
      }
      .eyebrow {
        display: inline-flex;
        align-items: center;
        gap: 8px;
        padding: 6px 10px;
        margin-bottom: 16px;
        border-radius: 999px;
        background: rgba(255, 255, 255, 0.06);
        color: #9eb4d1;
        font-size: 12px;
        letter-spacing: 0.08em;
        text-transform: uppercase;
      }
      h1 { margin: 0 0 12px; font-size: 30px; line-height: 1.1; }
      .detail { margin: 0 0 18px; line-height: 1.5; color: #a8b6c9; }
      .meta {
        display: flex;
        flex-wrap: wrap;
        gap: 10px;
        margin-bottom: 22px;
        color: #8fa3bd;
        font-size: 13px;
      }
      .meta span {
        padding: 8px 10px;
        border-radius: 12px;
        background: rgba(255, 255, 255, 0.05);
      }
      .grid {
        display: grid;
        gap: 10px;
        margin-bottom: 18px;
      }
      .row {
        display: grid;
        grid-template-columns: 140px 110px 1fr;
        align-items: center;
        gap: 16px;
        padding: 14px 16px;
        border-radius: 14px;
        background: rgba(255, 255, 255, 0.04);
        border: 1px solid rgba(255, 255, 255, 0.06);
      }
      .row-label { font-weight: 600; color: #edf3fb; }
      .row-detail { color: #9db0c7; }
      .badge {
        display: inline-flex;
        align-items: center;
        justify-content: center;
        min-width: 86px;
        padding: 6px 10px;
        border-radius: 999px;
        font-size: 12px;
        font-weight: 700;
        text-transform: uppercase;
        letter-spacing: 0.06em;
      }
      .badge-pending { background: rgba(247, 197, 72, 0.14); color: #f7cb6a; }
      .badge-starting { background: rgba(91, 141, 255, 0.16); color: #95bbff; }
      .badge-running { background: rgba(33, 193, 136, 0.16); color: #63e0aa; }
      .badge-warm { background: rgba(33, 193, 136, 0.16); color: #63e0aa; }
      .badge-cold { background: rgba(247, 197, 72, 0.14); color: #f7cb6a; }
      .badge-manual { background: rgba(149, 163, 184, 0.16); color: #d4dde8; }
      .badge-ready { background: rgba(33, 193, 136, 0.16); color: #63e0aa; }
      .badge-failed { background: rgba(235, 87, 87, 0.16); color: #ff8e8e; }
      .actions {
        display: flex;
        flex-wrap: wrap;
        gap: 10px;
        margin-top: 10px;
      }
      .btn {
        border: 0;
        border-radius: 12px;
        padding: 10px 14px;
        background: rgba(255, 255, 255, 0.08);
        color: #f0f6ff;
        font-weight: 600;
        cursor: pointer;
      }
      .btn:hover {
        background: rgba(255, 255, 255, 0.14);
      }
      .footer {
        display: flex;
        flex-wrap: wrap;
        gap: 10px;
        margin-top: 18px;
        color: #92a4bb;
        font-size: 12px;
      }
      code {
        display: inline-block;
        padding: 10px 12px;
        border-radius: 12px;
        background: rgba(255, 255, 255, 0.06);
        color: #f0f6ff;
      }
      @media (max-width: 720px) {
        .row {
          grid-template-columns: 1fr;
          gap: 8px;
        }
      }
    </style>
  </head>
  <body>
    <div class="card">
      <div class="eyebrow">AI Home Server Launcher</div>
      <h1>${escapeHtml(title)}</h1>
      <p class="detail">${escapeHtml(detail)}</p>
      <div class="meta">
        <span>Runtime: ${escapeHtml(runtimeLabel)}</span>
        <span>Coordinator: ${escapeHtml(coordinatorUrl)}</span>
        <span>Worker: ${escapeHtml(workerUrl)}</span>
      </div>
      <div class="grid">
        ${renderStatusRow('VM', vmState, `${vmName} at ${vmIp}`)}
        ${renderStatusRow('Coordinator', coordinatorState, coordinatorUrl)}
        ${renderStatusRow('Worker', workerState, workerUrl)}
        ${renderCacheRow(cache)}
        ${renderStatusRow('Ready', readyState, 'Launcher can open the app when all systems are healthy')}
      </div>
      <div class="actions">
        <button class="btn" type="button" onclick="window.aihomeserverLauncher.openWorkerFolder('logs')">Open exported worker logs</button>
        <button class="btn" type="button" onclick="window.aihomeserverLauncher.openWorkerFolder('root')">Open worker root</button>
        <button class="btn" type="button" onclick="window.aihomeserverLauncher.openLauncherLogFolder()">Open coordinator logs</button>
        <button class="btn" id="btn-stop-vm" type="button" onclick="vmAction('stop')" style="display:none">Stop VM</button>
        <button class="btn" id="btn-start-vm" type="button" onclick="vmAction('start')" style="display:none">Start VM</button>
      </div>
      <div class="footer">
        <code>${escapeHtml(coordinatorUrl)}</code>
      </div>
    </div>
  </body>
  <script>
    const vmRow = document.querySelector('.row:nth-child(1) .row-state');
    const workerRow = document.querySelector('.row:nth-child(3) .row-state');
    const btnStop = document.getElementById('btn-stop-vm');
    const btnStart = document.getElementById('btn-start-vm');

    function makeBadge(state) {
      const label = String(state || 'pending');
      return '<span class="badge badge-' + label + '">' + label + '</span>';
    }

    function applyVmStatus(status) {
      const vmState = (status.vm_state || 'unknown').toLowerCase();
      vmRow.innerHTML = makeBadge(vmState);
      workerRow.innerHTML = makeBadge(status.worker_healthy ? 'running' : 'unreachable');
      btnStop.style.display = vmState === 'running' ? 'inline-flex' : 'none';
      btnStart.style.display = (vmState === 'off' || vmState === 'stopped' || vmState === 'missing') ? 'inline-flex' : 'none';
    }

    async function vmAction(action) {
      btnStop.disabled = true;
      btnStart.disabled = true;
      try {
        if (action === 'stop') {
          vmRow.innerHTML = makeBadge('stopping');
          await window.aihomeserverLauncher.stopVm();
        } else {
          vmRow.innerHTML = makeBadge('starting');
          await window.aihomeserverLauncher.startVm();
        }
      } catch (e) {
        console.error('VM action failed', e);
      }
      btnStop.disabled = false;
      btnStart.disabled = false;
    }

    async function pollVmState() {
      try {
        const status = await window.aihomeserverLauncher.getVmState();
        applyVmStatus(status);
      } catch (_) {}
    }

    pollVmState();
    setInterval(pollVmState, 5000);
  </script>
</html>`;

  return win.loadURL(`data:text/html;charset=utf-8,${encodeURIComponent(html)}`);
}

async function bootstrap() {
  const shouldTryHyperV = process.platform === 'win32' && RUNTIME_MODE !== 'docker';
  if (shouldTryHyperV && !isRunningAsAdministrator()) {
    const relaunched = relaunchAsAdministrator();
    if (!relaunched) {
      dialog.showErrorBox(
        `${APP_NAME} needs administrator rights`,
        'The launcher requires elevation to provision the Hyper-V worker VM. Right-click the app and choose Run as administrator.'
      );
    }
    app.quit();
    return;
  }

  const win = createWindow();
  win.hide();
  const coordinatorPort = process.env.AIHOMESERVER_URL
    ? Number(new URL(DEFAULT_URL).port || (new URL(DEFAULT_URL).protocol === 'https:' ? 443 : 80))
    : await findAvailablePort(0);
  let coordinatorUrl = process.env.AIHOMESERVER_URL || `http://127.0.0.1:${coordinatorPort}`;
  const initialRuntimeLabel = shouldTryHyperV ? 'hyperv' : AUTO_START_DOCKER ? 'docker' : 'manual';
  await loadStartingPage(win, {
    title: `Starting ${APP_NAME}`,
    detail: shouldTryHyperV
      ? 'The launcher is provisioning the VM, starting the coordinator, and then checking that both the app and worker respond.'
      : AUTO_START_DOCKER
        ? 'The launcher is starting the local Docker stack and waiting for the coordinator and worker to answer health checks.'
        : 'The launcher is waiting for an already-running server and worker to become available.',
    runtimeLabel: initialRuntimeLabel,
    coordinatorUrl,
    workerUrl: 'pending',
    vmState: shouldTryHyperV ? 'starting' : 'running',
    coordinatorState: 'pending',
    workerState: 'pending',
    readyState: 'pending',
  });
  win.show();

  const workerToken = await ensureWorkerToken(app.getPath('userData'));
  const dockerWorkerUrl = 'http://127.0.0.1:3031';
  const coordinatorWorkerUrl = 'http://worker:3031';
  // When the VM worker is active, the coordinator container reaches it via the
  // host portproxy (netsh portproxy set by the PS script). Docker containers on
  // Windows can reach the Windows host at host.docker.internal, which the
  // portproxy then forwards to the VM on the Hyper-V internal switch.
  const hypervCoordinatorWorkerUrl = `http://host.docker.internal:${DEFAULT_VM_PORT}`;
  let workerHealthUrl = dockerWorkerUrl;
  let dockerEnv = {
    AIHOMESERVER_HOST_PORT: String(coordinatorPort),
    WORKER_TOKEN: workerToken,
    WORKER_URL: coordinatorWorkerUrl,
    EXECUTION_MODE: 'remote',
    COMPOSE_PROFILES: '',
  };
  let runtimeLabel = 'docker';

  if (shouldTryHyperV && !isHyperVAvailable()) {
    const hypervMsg = 'The Hyper-V Windows feature is not available on this machine.\n\nEnable it via "Turn Windows features on or off" → "Hyper-V", then restart.\n\nFalling back to the Docker worker.';
    if (RUNTIME_MODE === 'hyperv') {
      throw new Error(hypervMsg);
    }
    dialog.showErrorBox('Hyper-V not available', hypervMsg);
    await loadStartingPage(win, {
      title: `${APP_NAME} launcher`,
      detail: 'Hyper-V is not available on this machine. Using the Docker worker instead.',
      runtimeLabel: 'docker',
      coordinatorUrl,
      workerUrl: dockerWorkerUrl,
      vmState: 'failed',
      coordinatorState: 'starting',
      workerState: 'starting',
      readyState: 'pending',
    });
    dockerEnv.COMPOSE_PROFILES = 'worker';
  } else if (shouldTryHyperV) {
    try {
      const vm = await bootstrapHyperV({
        vmName: DEFAULT_VM_NAME,
        repoUrl: DEFAULT_REPO_URL,
        branch: DEFAULT_REPO_BRANCH,
        vmIp: DEFAULT_VM_IP,
        vmGateway: DEFAULT_VM_GATEWAY,
        switchName: DEFAULT_VM_SWITCH,
        vmCpus: DEFAULT_VM_CPUS,
        vmMemoryMb: DEFAULT_VM_MEMORY_MB,
        workerPort: DEFAULT_VM_PORT,
        workerToken,
        imageVersion: DEFAULT_VM_IMAGE_VERSION,
        workspacePath: '/workspace',
      });
      workerHealthUrl = vm.worker_url || `http://${DEFAULT_VM_IP}:${DEFAULT_VM_PORT}`;
      await loadStartingPage(win, {
        title: `${APP_NAME} launcher`,
        detail: 'The VM bootstrapped successfully. The launcher is now starting the coordinator stack and waiting for health checks.',
        runtimeLabel: 'hyperv',
        coordinatorUrl,
        workerUrl: workerHealthUrl,
        vmState: 'running',
        coordinatorState: 'starting',
        workerState: 'starting',
        readyState: 'pending',
      });
      dockerEnv = {
        AIHOMESERVER_HOST_PORT: String(coordinatorPort),
        WORKER_TOKEN: workerToken,
        // Coordinator container reaches the VM via the host portproxy; the
        // raw VM IP (192.168.250.10) is unreachable from inside Docker on Windows.
        WORKER_URL: hypervCoordinatorWorkerUrl,
        EXECUTION_MODE: 'remote',
        COMPOSE_PROFILES: '',
      };
      runtimeLabel = 'hyperv';
    } catch (error) {
      if (RUNTIME_MODE === 'hyperv') {
        throw error;
      }
      const { title, body } = categorizeHyperVError(error);
      dialog.showErrorBox(title, body + '\n\nFalling back to the Docker worker.');
      await loadStartingPage(win, {
        title: `${APP_NAME} launcher`,
        detail: 'Hyper-V bootstrap failed, so the launcher is using the Docker worker fallback instead.',
        runtimeLabel: 'docker',
        coordinatorUrl,
        workerUrl: dockerWorkerUrl,
        vmState: 'failed',
        coordinatorState: 'starting',
        workerState: 'starting',
        readyState: 'pending',
      });
      dockerEnv.COMPOSE_PROFILES = 'worker';
    }
  } else {
    dockerEnv.COMPOSE_PROFILES = 'worker';
  }

  // Helper: run the auth probe and open the app, or surface a clear auth-failure
  // page. Used by every "services already running" fast-path so none can bypass
  // the authenticated execution check.
  async function openIfAuthOk(detail) {
    const authProbe = await probeWorkerAuth(workerToken, workerHealthUrl);
    if (!authProbe.ok) {
      const authDetail = authProbe.status === 401
        ? `Worker /shell returned 401 — token mismatch. Fingerprint: ${workerToken.slice(0, 8)}...`
        : `Worker auth probe failed (HTTP ${authProbe.status || 0}): ${authProbe.error || 'no response'}`;
      await loadStartingPage(win, {
        title: `${APP_NAME} worker auth failed`,
        detail: authDetail,
        runtimeLabel,
        coordinatorUrl,
        workerUrl: workerHealthUrl,
        vmState: runtimeLabel === 'hyperv' ? 'running' : 'manual',
        coordinatorState: 'running',
        workerState: 'failed',
        readyState: 'failed',
      });
      return false;
    }
    await loadStartingPage(win, {
      title: `${APP_NAME} ready`,
      detail,
      runtimeLabel,
      coordinatorUrl,
      workerUrl: workerHealthUrl,
      vmState: runtimeLabel === 'hyperv' ? 'running' : 'manual',
      coordinatorState: 'running',
      workerState: 'running',
      readyState: 'ready',
    });
    await win.loadURL(coordinatorUrl);
    win.show();
    return true;
  }

  const existingServerReady = await waitForServerReady(coordinatorUrl, 5000);
  const existingWorkerReady = await waitForServerReady(workerHealthUrl, 5000);
  if (existingServerReady && existingWorkerReady) {
    await openIfAuthOk('The coordinator and worker are already running. Verified authenticated access.');
    return;
  }

  if (!AUTO_START_DOCKER) {
    const ready = await waitForServerReady(coordinatorUrl, 5000);
    const workerReady = await waitForServerReady(workerHealthUrl, 5000);
    if (ready && workerReady) {
      await openIfAuthOk('The server and worker are already running. Verified authenticated access.');
      return;
    }
    await loadStartingPage(win, {
      title: `${APP_NAME} is not running`,
      detail: `Expected a server at ${coordinatorUrl} and a worker at ${workerHealthUrl}.`,
      runtimeLabel,
      coordinatorUrl,
      workerUrl: workerHealthUrl,
      vmState: runtimeLabel === 'hyperv' ? 'running' : 'manual',
      coordinatorState: 'failed',
      workerState: 'failed',
      readyState: 'failed',
    });
    return;
  }

  try {
    const composeStart = await startLocalDockerStackWithRetry(dockerEnv);
    if (!process.env.AIHOMESERVER_URL && composeStart?.hostPort) {
      coordinatorUrl = `http://127.0.0.1:${composeStart.hostPort}`;
    }
  } catch (error) {
    throw new Error(`Failed to start the coordinator stack from ${COMPOSE_DIR}: ${error.message}`);
  }

  const serverReady = await waitForServerReady(coordinatorUrl, runtimeLabel === 'hyperv' ? 1800000 : 300000);
  if (!serverReady) {
    await loadStartingPage(win, {
      title: `${APP_NAME} is not responding yet`,
      detail: `The coordinator did not come online at ${coordinatorUrl}.`,
      runtimeLabel,
      coordinatorUrl,
      workerUrl: workerHealthUrl,
      vmState: runtimeLabel === 'hyperv' ? 'running' : 'manual',
      coordinatorState: 'failed',
      workerState: 'starting',
      readyState: 'failed',
    });
    return;
  }

  const workerReady = await waitForServerReady(workerHealthUrl, runtimeLabel === 'hyperv' ? 1800000 : 300000);
  if (!workerReady) {
    await loadStartingPage(win, {
      title: `${APP_NAME} worker is not responding yet`,
      detail: `The coordinator is up, but the worker has not answered health checks at ${workerHealthUrl}.`,
      runtimeLabel,
      coordinatorUrl,
      workerUrl: workerHealthUrl,
      vmState: runtimeLabel === 'hyperv' ? 'running' : 'manual',
      coordinatorState: 'running',
      workerState: 'failed',
      readyState: 'failed',
    });
    return;
  }

  await openIfAuthOk('The coordinator and worker are healthy and authenticated. The application is opening now.');
}

app.whenReady().then(() => {
  bootstrap().catch((error) => {
    dialog.showErrorBox(`${APP_NAME} desktop launcher failed`, error.stack || error.message);
  });

  app.on('activate', () => {
    if (BrowserWindow.getAllWindows().length === 0) {
      bootstrap().catch((error) => {
        dialog.showErrorBox(`${APP_NAME} desktop launcher failed`, error.stack || error.message);
      });
    }
  });
});

app.on('window-all-closed', () => {
  if (process.platform !== 'darwin') {
    app.quit();
  }
});
