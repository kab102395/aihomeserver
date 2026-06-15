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
    const message = stderr.trim() || stdout.trim() || `PowerShell exited with code ${exitCode.code}`;
    throw new Error(message);
  }

  return { stdout, stderr };
}

async function ensureWorkerToken(userDataDir) {
  const tokenPath = path.join(userDataDir, 'worker-token.txt');
  try {
    const token = (await fs.readFile(tokenPath, 'utf8')).trim();
    if (token) {
      return token;
    }
  } catch (_) {
    // Fall through and create a new token.
  }

  const token = crypto.randomBytes(24).toString('hex');
  await fs.mkdir(userDataDir, { recursive: true });
  await fs.writeFile(tokenPath, `${token}\n`, 'utf8');
  return token;
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

  const line = stdout
    .split(/\r?\n/)
    .map((entry) => entry.trim())
    .filter(Boolean)
    .pop();
  if (!line) {
    throw new Error('Hyper-V bootstrap returned no status');
  }

  try {
    return JSON.parse(line);
  } catch (error) {
    throw new Error(`Failed to parse Hyper-V bootstrap output: ${line}`);
  }
}

async function stopHyperV(vmName) {
  const scriptPath = resolveScriptPath();
  const { stdout } = await runPowerShell(scriptPath, ['-Action', 'stop', '-VmName', vmName]);
  const line = stdout
    .split(/\r?\n/)
    .map((entry) => entry.trim())
    .filter(Boolean)
    .pop();
  return line ? JSON.parse(line) : { ok: false };
}

module.exports = {
  ensureWorkerToken,
  bootstrapHyperV,
  stopHyperV,
  resolveScriptPath,
};
