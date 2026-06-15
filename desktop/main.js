const fs = require('node:fs');
const { app, BrowserWindow, dialog, ipcMain, shell } = require('electron');
const { spawn, spawnSync } = require('node:child_process');
const http = require('node:http');
const https = require('node:https');
const path = require('node:path');
const { bootstrapHyperV, ensureWorkerToken } = require('./hyperv');

const DEFAULT_URL = process.env.AIHOMESERVER_URL || 'http://127.0.0.1:3000';
const APP_NAME = 'AI Home Server';
const AUTO_START_DOCKER = process.env.AIHOMESERVER_AUTO_START_DOCKER !== '0';
const COMPOSE_DIR = process.env.AIHOMESERVER_COMPOSE_DIR || path.join(__dirname, '..');
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
  `ubuntu-${DEFAULT_VM_IMAGE_VERSION}-server-cloudimg-amd64.vhdx`
);

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

function probeCacheStatus() {
  return {
    imageExists: fs.existsSync(DEFAULT_HYPERV_IMAGE),
    logsExist: fs.existsSync(path.join(DEFAULT_HYPERV_ROOT, 'logs')),
    root: DEFAULT_HYPERV_ROOT,
    imagePath: DEFAULT_HYPERV_IMAGE,
  };
}

function isRunningAsAdministrator() {
  if (process.platform !== 'win32') {
    return true;
  }

  const check = spawnSync(
    'powershell.exe',
    [
      '-NoProfile',
      '-NonInteractive',
      '-Command',
      '[bool]([Security.Principal.WindowsPrincipal] [Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)',
    ],
    { stdio: 'ignore' }
  );

  return check.status === 0;
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
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: COMPOSE_DIR,
      detached: false,
      stdio: 'ignore',
      windowsHide: true,
      env: { ...process.env, ...extraEnv },
    });

    child.on('error', reject);
    child.on('exit', (code) => {
      if (code === 0) {
        resolve();
        return;
      }
      reject(new Error(`docker compose exited with code ${code}`));
    });
  });
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
  await shell.openPath(target);
  return target;
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
        <button class="btn" type="button" onclick="window.aihomeserverLauncher.openWorkerFolder('logs')">Open worker logs</button>
        <button class="btn" type="button" onclick="window.aihomeserverLauncher.openWorkerFolder('root')">Open worker root</button>
      </div>
      <div class="footer">
        <code>${DEFAULT_URL}</code>
      </div>
    </div>
  </body>
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
  const initialRuntimeLabel = shouldTryHyperV ? 'hyperv' : AUTO_START_DOCKER ? 'docker' : 'manual';
  await loadStartingPage(win, {
    title: `Starting ${APP_NAME}`,
    detail: shouldTryHyperV
      ? 'The launcher is provisioning the VM, starting the coordinator, and then checking that both the app and worker respond.'
      : AUTO_START_DOCKER
        ? 'The launcher is starting the local Docker stack and waiting for the coordinator and worker to answer health checks.'
        : 'The launcher is waiting for an already-running server and worker to become available.',
    runtimeLabel: initialRuntimeLabel,
    coordinatorUrl: DEFAULT_URL,
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
  let workerHealthUrl = dockerWorkerUrl;
  let dockerEnv = {
    WORKER_TOKEN: workerToken,
    WORKER_URL: coordinatorWorkerUrl,
    EXECUTION_MODE: 'remote',
    COMPOSE_PROFILES: '',
  };
  let runtimeLabel = 'docker';

  if (shouldTryHyperV) {
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
        coordinatorUrl: DEFAULT_URL,
        workerUrl: workerHealthUrl,
        vmState: 'running',
        coordinatorState: 'starting',
        workerState: 'starting',
        readyState: 'pending',
      });
      dockerEnv = {
        WORKER_TOKEN: workerToken,
        WORKER_URL: workerHealthUrl,
        EXECUTION_MODE: 'remote',
        COMPOSE_PROFILES: '',
      };
      runtimeLabel = 'hyperv';
    } catch (error) {
      if (RUNTIME_MODE === 'hyperv') {
        throw error;
      }
      dialog.showErrorBox(
        'Hyper-V worker unavailable, falling back to Docker worker',
        `The VM bootstrap failed, so the desktop app is switching to the Docker worker fallback.\n\n${error.stack || error.message}`
      );
      await loadStartingPage(win, {
        title: `${APP_NAME} launcher`,
        detail: 'Hyper-V bootstrap failed, so the launcher is using the Docker worker fallback instead.',
        runtimeLabel: 'docker',
        coordinatorUrl: DEFAULT_URL,
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

  if (!AUTO_START_DOCKER) {
    const ready = await waitForServerReady(DEFAULT_URL, 5000);
    const workerReady = await waitForServerReady(workerHealthUrl, 5000);
    if (ready && workerReady) {
      await loadStartingPage(win, {
        title: `${APP_NAME} ready`,
        detail: 'The server and worker are already running and healthy.',
        runtimeLabel,
        coordinatorUrl: DEFAULT_URL,
        workerUrl: workerHealthUrl,
        vmState: runtimeLabel === 'hyperv' ? 'running' : 'manual',
        coordinatorState: 'running',
        workerState: 'running',
        readyState: 'ready',
      });
      await win.loadURL(DEFAULT_URL);
      win.show();
      return;
    }
    await loadStartingPage(win, {
      title: `${APP_NAME} is not running`,
      detail: `Expected a server at ${DEFAULT_URL} and a worker at ${workerHealthUrl}.`,
      runtimeLabel,
      coordinatorUrl: DEFAULT_URL,
      workerUrl: workerHealthUrl,
      vmState: runtimeLabel === 'hyperv' ? 'running' : 'manual',
      coordinatorState: 'failed',
      workerState: 'failed',
      readyState: 'failed',
    });
    return;
  }

  try {
    await startLocalDockerStack(dockerEnv);
  } catch (error) {
    throw new Error(`Failed to start the coordinator stack from ${COMPOSE_DIR}: ${error.message}`);
  }

  const serverReady = await waitForServerReady(DEFAULT_URL, runtimeLabel === 'hyperv' ? 1800000 : 300000);
  if (!serverReady) {
    await loadStartingPage(win, {
      title: `${APP_NAME} is not responding yet`,
      detail: `The coordinator did not come online at ${DEFAULT_URL}.`,
      runtimeLabel,
      coordinatorUrl: DEFAULT_URL,
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
      coordinatorUrl: DEFAULT_URL,
      workerUrl: workerHealthUrl,
      vmState: runtimeLabel === 'hyperv' ? 'running' : 'manual',
      coordinatorState: 'running',
      workerState: 'failed',
      readyState: 'failed',
    });
    return;
  }

  await loadStartingPage(win, {
    title: `${APP_NAME} ready`,
    detail: 'The coordinator and worker are healthy. The application is opening now.',
    runtimeLabel,
    coordinatorUrl: DEFAULT_URL,
    workerUrl: workerHealthUrl,
    vmState: runtimeLabel === 'hyperv' ? 'running' : 'manual',
    coordinatorState: 'running',
    workerState: 'running',
    readyState: 'ready',
  });
  await win.loadURL(DEFAULT_URL);
  win.show();
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
