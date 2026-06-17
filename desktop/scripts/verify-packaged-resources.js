const fs = require('fs');
const path = require('path');

function assertContains(filePath, needle) {
  const content = fs.readFileSync(filePath, 'utf8');
  if (!content.includes(needle)) {
    throw new Error(`Expected ${filePath} to contain ${needle}`);
  }
}

function main() {
  const root = path.resolve(__dirname, '..');
  const unpacked = path.join(root, 'dist', 'win-unpacked');
  const embeddedRepo = path.join(unpacked, 'resources', 'repo');
  const embeddedWorkerScript = path.join(unpacked, 'resources', 'scripts', 'hyperv-worker.ps1');
  const embeddedRustWorker = path.join(embeddedRepo, 'src', 'bin', 'worker.rs');
  const embeddedServer = path.join(embeddedRepo, 'src', 'api', 'server.rs');

  for (const required of [unpacked, embeddedRepo, embeddedWorkerScript, embeddedRustWorker, embeddedServer]) {
    if (!fs.existsSync(required)) {
      throw new Error(`Missing packaged resource: ${required}`);
    }
  }

  assertContains(embeddedWorkerScript, '/capabilities');
  assertContains(embeddedWorkerScript, '/filesystem/write');
  assertContains(embeddedWorkerScript, 'Get-WorkerRuntimeBootstrapScript');

  assertContains(embeddedRustWorker, '.route("/capabilities", get(capabilities))');
  assertContains(embeddedRustWorker, '.route("/filesystem/write", post(filesystem_write))');

  assertContains(embeddedServer, '"worker_capabilities"');
  assertContains(embeddedServer, 'derive_run_success');

  console.log('Embedded package verification passed.');
}

main();
