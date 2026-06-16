const fs = require('node:fs/promises');
const fsSync = require('node:fs');
const crypto = require('node:crypto');
const { spawn } = require('node:child_process');
const path = require('node:path');

function resolveScriptPath() {
  if (process.env.AIHOMESERVER_HYPERV_SCRIPT) {
    return process.env.AIHOMESERVER_HYPERV_SCRIPT;
  }

  const packagedPath = path.join(process.resourcesPath || '', 'scripts', 'hyperv-worker.ps1');
  if (process.resourcesPath && fsSync.existsSync(packagedPath)) {
    return packagedPath;
  }

  return path.join(__dirname, '..', 'scripts', 'hyperv-worker.ps1');
}

async function runPowerShell(scriptPath, args, extraEnv = {}) {
  const child = spawn(
    'powershell.exe',
    [
      '-NoProfile',
      '-NonInteractive',
      '-ExecutionPolicy',
      'Bypass',
      '-File',
      scriptPath,
      ...args,
    ],
    {
      stdio: ['ignore', 'pipe', 'pipe'],
      windowsHide: true,
      env: { ...process.env, ...extraEnv },
    }
  );

  let stdout = '';
  let stderr = '';
  child.stdout.on('data', (chunk) => {
    stdout += chunk.toString('utf8');
  });
  child.stderr.on('data', (chunk) => {
    stderr += chunk.toString('utf8');
  });

  const exitCode = await new Promise((resolve) => {
    child.on('error', (error) => {
      resolve({ code: 1, error });
    });
    child.on('exit', (code) => {
      resolve({ code: code ?? 1 });
    });
  });

  if (exitCode.error) {
    throw exitCode.error;
  }

  if (exitCode.code !== 0) {
    // Hyper-V script contract: before treating a non-zero exit as failure,
    // check for {"ok":true} in stdout. PowerShell 5.1 propagates $LASTEXITCODE
    // from native commands (e.g. wsl --unmount in a finally block) even when
    // the script ran to completion. This override is intentional for the
    // hyperv-worker.ps1 contract only — do not reuse runPowerShell for other
    // scripts that do not emit this signal.
    const lastJsonLine = stdout
      .split(/\r?\n/)
      .map((l) => l.trim())
      .filter((l) => l.startsWith('{'))
      .at(-1);
    if (lastJsonLine) {
      try {
        const parsed = JSON.parse(lastJsonLine);
        if (parsed.ok === true) {
          return { stdout, stderr };
        }
      } catch (_) {}
    }
    const message = stderr.trim() || stdout.trim() || `PowerShell exited with code ${exitCode.code}`;
    throw new Error(message);
  }

  return { stdout, stderr };
}

function tokenFingerprint(token) {
  return `len=${token.length} fp=${token.slice(0, 8)}...`;
}

async function ensureWorkerToken(userDataDir) {
  const tokenPath = path.join(userDataDir, 'worker-token.txt');
  try {
    const token = (await fs.readFile(tokenPath, 'utf8')).trim();
    if (token) {
      console.log(`[token] loaded from ${tokenPath} (${tokenFingerprint(token)})`);
      return token;
    }
  } catch (_) {
    // Fall through and create a new token.
  }

  const token = crypto.randomBytes(24).toString('hex');
  await fs.mkdir(userDataDir, { recursive: true });
  await fs.writeFile(tokenPath, `${token}\n`, 'utf8');
  console.log(`[token] created new token at ${tokenPath} (${tokenFingerprint(token)})`);
  return token;
}

function lastJsonLine(stdout) {
  return stdout
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter((l) => l.startsWith('{'))
    .at(-1) ?? null;
}

function parseLastJson(stdout, label) {
  const line = lastJsonLine(stdout);
  if (!line) throw new Error(`${label} returned no JSON status line`);
  try {
    return JSON.parse(line);
  } catch {
    throw new Error(`Failed to parse ${label} output: ${line}`);
  }
}

async function bootstrapHyperV(options) {
  const scriptPath = resolveScriptPath();
  const args = [
    '-Action',
    'bootstrap',
    '-VmName',
    options.vmName,
    '-RepoUrl',
    options.repoUrl,
    '-Branch',
    options.branch,
    '-VmIp',
    options.vmIp,
    '-VmGateway',
    options.vmGateway,
    '-SwitchName',
    options.switchName,
    '-VmCpus',
    String(options.vmCpus),
    '-VmMemoryMb',
    String(options.vmMemoryMb),
    '-WorkerPort',
    String(options.workerPort),
    '-WorkerToken',
    options.workerToken,
    '-ImageVersion',
    options.imageVersion,
    '-WorkspacePath',
    options.workspacePath,
  ];

  if (options.rootDir) {
    args.push('-RootDir', options.rootDir);
  }

  const { stdout } = await runPowerShell(scriptPath, args, {
    AIHOMESERVER_VM_ROOT: options.rootDir,
  });

  return parseLastJson(stdout, 'Hyper-V bootstrap');
}

async function stopHyperV(vmName) {
  const scriptPath = resolveScriptPath();
  const { stdout } = await runPowerShell(scriptPath, ['-Action', 'stop', '-VmName', vmName]);
  return lastJsonLine(stdout) ? parseLastJson(stdout, 'Hyper-V stop') : { ok: false };
}

async function startHyperV(options) {
  const scriptPath = resolveScriptPath();
  const args = [
    '-Action', 'start',
    '-VmName', options.vmName,
    '-VmIp', options.vmIp,
    '-VmGateway', options.vmGateway,
    '-SwitchName', options.switchName,
    '-WorkerPort', String(options.workerPort),
  ];
  const { stdout } = await runPowerShell(scriptPath, args);
  return parseLastJson(stdout, 'Hyper-V start');
}

async function getHyperVStatus(options) {
  const scriptPath = resolveScriptPath();
  const args = [
    '-Action', 'status',
    '-VmName', options.vmName,
    '-VmIp', options.vmIp,
    '-WorkerPort', String(options.workerPort),
  ];
  const { stdout } = await runPowerShell(scriptPath, args);
  return lastJsonLine(stdout) ? parseLastJson(stdout, 'Hyper-V status') : { ok: false, vm_state: 'Unknown' };
}

async function exportHyperVLogs(options) {
  const scriptPath = resolveScriptPath();
  const args = [
    '-Action', 'export-logs',
    '-VmName', options.vmName,
    '-VmIp', options.vmIp,
    '-WorkerPort', String(options.workerPort),
  ];
  if (options.rootDir) {
    args.push('-RootDir', options.rootDir);
  }
  const { stdout } = await runPowerShell(scriptPath, args, {
    AIHOMESERVER_VM_ROOT: options.rootDir,
  });
  return parseLastJson(stdout, 'Hyper-V export-logs');
}

module.exports = {
  ensureWorkerToken,
  bootstrapHyperV,
  stopHyperV,
  startHyperV,
  getHyperVStatus,
  exportHyperVLogs,
  resolveScriptPath,
};
