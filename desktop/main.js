const { app, BrowserWindow, dialog } = require('electron');
const { spawn } = require('node:child_process');
const http = require('node:http');
const https = require('node:https');
const path = require('node:path');

const DEFAULT_URL = process.env.AIHOMESERVER_URL || 'http://127.0.0.1:3000';
const APP_NAME = 'AI Home Server';
const AUTO_START_DOCKER = process.env.AIHOMESERVER_AUTO_START_DOCKER !== '0';
const COMPOSE_DIR = process.env.AIHOMESERVER_COMPOSE_DIR || path.join(__dirname, '..');
const COMPOSE_FILES = (process.env.AIHOMESERVER_COMPOSE_FILES || 'docker-compose.yml,docker-compose.dev.yml')
  .split(',')
  .map((entry) => entry.trim())
  .filter(Boolean);

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

function composeCommand() {
  const args = ['compose'];
  for (const file of COMPOSE_FILES) {
    args.push('-f', file);
  }
  args.push('up', '-d', '--build');
  return { command: 'docker', args };
}

function startLocalDockerStack() {
  const { command, args } = composeCommand();
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: COMPOSE_DIR,
      detached: false,
      stdio: 'ignore',
      windowsHide: true,
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

async function waitForServerReady(timeoutMs = 180000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (await probeHealth(DEFAULT_URL, 1500)) {
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

function loadStartingPage(win, message, detail = 'The desktop app is waiting for the web server to come online.') {
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
        background: linear-gradient(135deg, #0e1117, #141b24 55%, #0b1320);
        color: #d7e1ee;
        font-family: Arial, Helvetica, sans-serif;
      }
      .card {
        width: min(620px, calc(100vw - 48px));
        padding: 32px 36px;
        border: 1px solid rgba(255, 255, 255, 0.08);
        border-radius: 20px;
        background: rgba(14, 18, 24, 0.86);
        box-shadow: 0 24px 72px rgba(0, 0, 0, 0.35);
      }
      h1 { margin: 0 0 12px; font-size: 28px; }
      p { margin: 0; line-height: 1.5; color: #a8b6c9; }
      code {
        display: inline-block;
        margin-top: 18px;
        padding: 10px 12px;
        border-radius: 12px;
        background: rgba(255, 255, 255, 0.06);
        color: #f0f6ff;
      }
    </style>
  </head>
  <body>
    <div class="card">
      <h1>${message}</h1>
      <p>${detail}</p>
      <code>${DEFAULT_URL}</code>
    </div>
  </body>
</html>`;

  return win.loadURL(`data:text/html;charset=utf-8,${encodeURIComponent(html)}`);
}

async function bootstrap() {
  const win = createWindow();
  win.hide();
  await loadStartingPage(
    win,
    `Starting ${APP_NAME}`,
    AUTO_START_DOCKER
      ? 'The desktop app is starting the local Docker stack and waiting for the web server to come online.'
      : 'The desktop app is waiting for an already-running web server to become available.'
  );
  win.show();

  const alreadyUp = await probeHealth(DEFAULT_URL);
  let stackStarted = false;
  if (!alreadyUp && AUTO_START_DOCKER) {
    try {
      await startLocalDockerStack();
      stackStarted = true;
    } catch (error) {
      dialog.showErrorBox(
        'Unable to start local Docker stack',
        `Failed to run docker compose from ${COMPOSE_DIR}.\n\n${error.message}`
      );
    }
  }

  const ready = await waitForServerReady(alreadyUp || !stackStarted ? 5000 : 180000);
  if (ready) {
    await win.loadURL(DEFAULT_URL);
    win.show();
    return;
  }

  await loadStartingPage(
    win,
    AUTO_START_DOCKER ? `${APP_NAME} is not responding yet` : `${APP_NAME} is not running`
  );
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
