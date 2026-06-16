param(
    [ValidateSet('bootstrap', 'start', 'stop', 'status', 'export-logs')]
    [string]$Action = 'status',

    [string]$VmName = 'AIHomeServerWorker',
    [string]$RepoUrl = 'https://github.com/kab102395/aihomeserver.git',
    [string]$Branch = 'main',
    [string]$VmIp = '192.168.250.10',
    [string]$VmGateway = '192.168.250.1',
    [string]$SwitchName = 'AIHomeServerSwitch',
    [int]$VmCpus = 4,
    [int]$VmMemoryMb = 8192,
    [int]$WorkerPort = 3031,
    [string]$WorkerToken = '',
    [string]$ImageVersion = '24.04',
    [string]$WorkspacePath = '/workspace',
    [string]$RootDir = ''
)

$ErrorActionPreference = 'Stop'

function Write-Log {
    param([string]$Message)
    [Console]::Error.WriteLine($Message)
}

function Write-JsonResult {
    param([hashtable]$Value)
    $Value | ConvertTo-Json -Depth 8 -Compress
}

if (-not ('ISOFile' -as [type])) {
    $isoTypeDefinition = @'
using System;
using System.IO;
using System.Runtime.InteropServices;
using System.Runtime.InteropServices.ComTypes;

public unsafe static class ISOFile {
    public static void Create(string path, object stream, int blockSize, int totalBlocks) {
        int bytes = 0;
        byte[] buffer = new byte[blockSize];
        IntPtr bytesPtr = (IntPtr)(&bytes);
        var input = stream as IStream;

        using (var output = File.Open(path, FileMode.Create, FileAccess.Write, FileShare.None)) {
            while (totalBlocks-- > 0) {
                input.Read(buffer, blockSize, bytesPtr);
                output.Write(buffer, 0, bytes);
            }
            output.Flush();
        }
    }
}
'@

    if ($PSVersionTable.PSVersion.Major -ge 7) {
        Add-Type -CompilerOptions '/unsafe' -TypeDefinition $isoTypeDefinition
    } else {
        $compilerParameters = New-Object System.CodeDom.Compiler.CompilerParameters
        $compilerParameters.CompilerOptions = '/unsafe'
        Add-Type -CompilerParameters $compilerParameters -TypeDefinition $isoTypeDefinition
    }
}

function Assert-HyperVAvailable {
    if (-not (Get-Command Get-VM -ErrorAction SilentlyContinue)) {
        throw 'Hyper-V PowerShell cmdlets are not available. Enable Hyper-V and run PowerShell as Administrator.'
    }
}

function Assert-Admin {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($identity)
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw 'Hyper-V provisioning requires an elevated PowerShell session.'
    }
}

function Assert-WslMountAvailable {
    if (-not (Get-Command wsl.exe -ErrorAction SilentlyContinue)) {
        throw 'WSL is not installed. Hyper-V worker bootstrap requires WSL 2 with `wsl --mount` support.'
    }

    $helpText = ((& wsl.exe --help 2>&1 | Out-String) -replace "`0", '')
    if ([string]::IsNullOrWhiteSpace($helpText)) {
        throw 'WSL is installed but did not return any help output.'
    }

    if ($helpText -notmatch '(?m)--mount\b') {
        throw 'The installed WSL does not support `wsl --mount`. Upgrade to a WSL 2 build that supports virtual disk mounting.'
    }
}

function Ensure-Directory {
    param([string]$Path)
    if (-not (Test-Path $Path)) {
        New-Item -ItemType Directory -Path $Path | Out-Null
    }
}

function Wait-FileUnlocked {
    param(
        [string]$Path,
        [int]$TimeoutSeconds = 30
    )

    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ((Get-Date) -lt $deadline) {
        try {
            $stream = [System.IO.File]::Open($Path, 'Open', 'ReadWrite', 'None')
            $stream.Close()
            return
        } catch {
            Start-Sleep -Seconds 1
        }
    }

    throw "Timed out waiting for $Path to be released"
}

function Remove-VmArtifacts {
    param([string]$VmName)

    $vm = Get-VM -Name $VmName -ErrorAction SilentlyContinue
    if ($vm) {
        try {
            Stop-VM -Name $VmName -TurnOff -ErrorAction SilentlyContinue | Out-Null
        } catch {
            Write-Log "Warning: unable to stop VM ${VmName} cleanly: $($_.Exception.Message)"
        }

        try {
            Get-VMCheckpoint -VMName $VmName -ErrorAction SilentlyContinue |
                ForEach-Object { Remove-VMCheckpoint -VMCheckpoint $_ -ErrorAction SilentlyContinue | Out-Null }
        } catch {
            Write-Log "Warning: unable to remove VM checkpoints for ${VmName}: $($_.Exception.Message)"
        }

        try {
            Remove-VM -Name $VmName -Force -ErrorAction SilentlyContinue | Out-Null
        } catch {
            Write-Log "Warning: unable to remove VM ${VmName} cleanly: $($_.Exception.Message)"
        }
    }

    # Always remove the per-VM working disk so the next bootstrap starts from a
    # clean copy of the base image with no stale cloud-init or token state.
    $vmDir = Join-Path (Get-RootDir) 'vm'
    $workingDisk = Join-Path $vmDir "$VmName-disk.vhdx"
    if (Test-Path $workingDisk) {
        try {
            Remove-Item -LiteralPath $workingDisk -Force
            Write-Log "Removed working disk: $workingDisk"
        } catch {
            Write-Log "Warning: unable to remove working disk ${workingDisk}: $($_.Exception.Message)"
        }
    }
}

function Invoke-Download {
    param(
        [string]$Url,
        [string]$OutFile
    )

    if (Test-Path $OutFile) {
        return
    }

    Write-Log "Downloading $Url"
    Invoke-WebRequest -Uri $Url -OutFile $OutFile -UseBasicParsing
}

function Get-ImageUrl {
    param([string]$Version)
    return "https://cloud-images.ubuntu.com/releases/$Version/release/ubuntu-$Version-server-cloudimg-amd64-azure.vhd.tar.gz"
}

function Get-StaticMacAddress {
    param([string]$VmName)

    $bytes = [System.Text.Encoding]::UTF8.GetBytes($VmName)
    $hash = [System.Security.Cryptography.SHA256]::Create().ComputeHash($bytes)
    $macBytes = @(
        0x00, 0x15, 0x5D,
        $hash[0],
        $hash[1],
        $hash[2]
    )
    return (($macBytes | ForEach-Object { $_.ToString('X2') }) -join '')
}

function Format-MacAddress {
    param([string]$MacAddress)

    return ($MacAddress -split '(.{2})' | Where-Object { $_ }) -join ':'
}

function Get-RootDir {
    if ($RootDir) {
        return $RootDir
    }
    if ($env:AIHOMESERVER_VM_ROOT) {
        return $env:AIHOMESERVER_VM_ROOT
    }
    return Join-Path $env:ProgramData 'AIHomeServer\hyperv'
}

function Get-Paths {
    $root = Get-RootDir
    Ensure-Directory $root
    $imageDir = Join-Path $root 'image'
    $vmDir = Join-Path $root 'vm'
    $seedDir = Join-Path $root 'seed'
    $repoDir = Join-Path $root 'repo'
    $logDir = Join-Path $root 'logs'
    Ensure-Directory $imageDir
    Ensure-Directory $vmDir
    Ensure-Directory $seedDir
    Ensure-Directory $repoDir
    Ensure-Directory $logDir
    [hashtable]@{
        Root = $root
        ImageDir = $imageDir
        VmDir = $vmDir
        SeedDir = $seedDir
        RepoDir = $repoDir
        LogDir = $logDir
    }
}

function New-IsoFile {
    param(
        [Parameter(Mandatory = $true)]
        [string]$SourceDir,
        [Parameter(Mandatory = $true)]
        [string]$OutFile,
        [string]$VolumeName = 'cidata'
    )

    $image = New-Object -ComObject IMAPI2FS.MsftFileSystemImage
    $image.VolumeName = $VolumeName
    $image.FileSystemsToCreate = 7
    $image.StrictFileSystemCompliance = $false
    Get-ChildItem -LiteralPath $SourceDir -Force | ForEach-Object {
        $image.Root.AddTree($_.FullName, $true) | Out-Null
    }

    $result = $image.CreateResultImage()
    [ISOFile]::Create($OutFile, $result.ImageStream, $result.BlockSize, $result.TotalBlocks)
}

function Convert-ToWslPath {
    param([string]$WindowsPath)

    $resolved = [System.IO.Path]::GetFullPath($WindowsPath)
    $drive = $resolved.Substring(0, 1).ToLowerInvariant()
    $rest = $resolved.Substring(2).TrimStart('\') -replace '\\', '/'
    if ([string]::IsNullOrWhiteSpace($rest)) {
        return "/mnt/$drive"
    }
    return "/mnt/$drive/$rest"
}

function Get-PosixDirectoryName {
    param([string]$PosixPath)

    $index = $PosixPath.LastIndexOf('/')
    if ($index -le 0) {
        return '/'
    }
    return $PosixPath.Substring(0, $index)
}

function Get-WorkerPublicKey {
    return 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBdQ4ptyx1DiHKIegOJjXtJLlxDOUSLHEyWWa6ptO9Ke kab10@Kyle'
}

function Get-WorkerNetworkSetupScript {
    param(
        [string]$VmIp,
        [string]$VmGateway,
        [string]$VmMac
    )

    $template = @'
#!/usr/bin/env bash
set -euo pipefail

target_mac="__VM_MAC__"
target_ip="__VM_IP__"
target_gateway="__VM_GATEWAY__"
iface=""

for candidate in /sys/class/net/*; do
  [ -e "$candidate/address" ] || continue
  current_mac="$(tr '[:lower:]' '[:upper:]' < "$candidate/address")"
  if [ "$current_mac" = "$target_mac" ]; then
    iface="$(basename "$candidate")"
    break
  fi
done

if [ -z "$iface" ]; then
  exit 0
fi

ip link set "$iface" up || true
ip addr flush dev "$iface" || true
ip addr add "$target_ip/24" dev "$iface" || true
ip route replace default via "$target_gateway" dev "$iface" || true
printf 'nameserver 1.1.1.1\nnameserver 8.8.8.8\n' > /etc/resolv.conf
'@
    return $template.
        Replace('__VM_MAC__', (Format-MacAddress -MacAddress $VmMac).ToUpperInvariant()).
        Replace('__VM_IP__', $VmIp).
        Replace('__VM_GATEWAY__', $VmGateway)
}

function Get-WorkerNetplanConfig {
    param(
        [string]$VmIp,
        [string]$VmGateway,
        [string]$VmMac
    )

    $template = @'
network:
  version: 2
  ethernets:
    eth0:
      match:
        macaddress: "__VM_MAC__"
      set-name: eth0
      dhcp4: false
      dhcp6: false
      addresses:
        - __VM_IP__/24
      routes:
        - to: default
          via: __VM_GATEWAY__
      nameservers:
        addresses:
          - 1.1.1.1
          - 8.8.8.8
'@
    return $template.
        Replace('__VM_MAC__', (Format-MacAddress -MacAddress $VmMac).ToLowerInvariant()).
        Replace('__VM_IP__', $VmIp).
        Replace('__VM_GATEWAY__', $VmGateway)
}

function Get-WorkerNetworkBootstrapService {
@'
[Unit]
Description=AI Home Server static network bootstrap
DefaultDependencies=no
After=local-fs.target systemd-udev-settle.service
Before=network-pre.target network.target network-online.target ssh.service sshd.service
Wants=network-pre.target

[Service]
Type=oneshot
ExecStart=/usr/local/bin/aihomeserver-network-setup
RemainAfterExit=yes

[Install]
WantedBy=multi-user.target
'@
}

function Get-WorkerPythonScript {
@'
#!/usr/bin/env python3
import base64
import datetime
import html
import json
import os
import pathlib
import re
import shutil
import subprocess
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

WORKSPACE = os.environ.get("WORKER_WORKSPACE", "/workspace")
TOKEN = os.environ.get("WORKER_TOKEN", "").strip()
PORT = int(os.environ.get("WORKER_PORT", "3031"))

FP = TOKEN[:8] if TOKEN else ""

def require_auth(handler):
    if not TOKEN:
        return True
    auth_header = handler.headers.get("Authorization", "")
    if not auth_header:
        import sys
        print(f"[worker] auth rejected: no Authorization header (token fp={FP}...)", file=sys.stderr, flush=True)
        return False
    if auth_header != f"Bearer {TOKEN}":
        import sys
        print(f"[worker] auth rejected: token mismatch (token fp={FP}...)", file=sys.stderr, flush=True)
        return False
    return True

def json_response(handler, code, payload):
    body = json.dumps(payload).encode("utf-8")
    handler.send_response(code)
    handler.send_header("Content-Type", "application/json")
    handler.send_header("Content-Length", str(len(body)))
    handler.end_headers()
    handler.wfile.write(body)

def resolve_path(root, requested):
    requested = (requested or ".").strip()
    candidate = pathlib.Path(root) if requested in ("", ".") else pathlib.Path(root) / requested
    resolved = candidate.resolve()
    root_resolved = pathlib.Path(root).resolve()
    if root_resolved != resolved and root_resolved not in resolved.parents:
        raise ValueError("path escapes worker workspace")
    return resolved

def clear_directory_contents(dir_path):
    if not dir_path.exists():
        dir_path.mkdir(parents=True, exist_ok=True)
        return
    for entry in dir_path.iterdir():
        if entry.is_dir():
            shutil.rmtree(entry)
        else:
            entry.unlink()

def write_sync_files(root, files):
    count = 0
    for file in files:
        rel = pathlib.PurePosixPath(file.get("path", ""))
        if rel.is_absolute() or ".." in rel.parts:
            continue
        target = pathlib.Path(root) / pathlib.Path(*rel.parts)
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(base64.b64decode(file.get("contents_b64", "")))
        count += 1
    return count

def collect_paths(root, paths):
    out = []
    for rel in paths:
        rel_path = pathlib.PurePosixPath(rel)
        if rel_path.is_absolute() or ".." in rel_path.parts:
            continue
        full = pathlib.Path(root) / pathlib.Path(*rel_path.parts)
        if not full.exists() or not full.is_file():
            continue
        data = full.read_bytes()
        out.append({
            "path": str(rel_path),
            "contents_b64": base64.b64encode(data).decode("ascii"),
            "size": len(data),
            "truncated": False,
        })
    return out

def run_shell(command, cwd, timeout_secs):
    proc = subprocess.run(
        ["bash", "-lc", command],
        cwd=cwd,
        capture_output=True,
        text=True,
        timeout=timeout_secs,
    )
    return proc.returncode, proc.stdout, proc.stderr

class Handler(BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):
        return

    def do_GET(self):
        if self.path == "/health":
            json_response(self, 200, {"ok": True, "workspace": WORKSPACE})
            return
        self.send_error(404)

    def do_POST(self):
        if not require_auth(self):
            json_response(self, 401, {"error": "unauthorized"})
            return

        length = int(self.headers.get("Content-Length", "0"))
        payload = json.loads(self.rfile.read(length).decode("utf-8") or "{}")

        if self.path == "/workspace/sync":
            prefix = (payload.get("prefix") or ".").strip().lstrip("./")
            target_root = resolve_path(WORKSPACE, prefix)
            clear_directory_contents(target_root)
            written = write_sync_files(target_root, payload.get("files", []))
            json_response(self, 200, {"ok": True, "workspace": WORKSPACE, "files_written": written})
            return

        if self.path == "/shell":
            cwd = resolve_path(WORKSPACE, payload.get("cwd"))
            cwd.mkdir(parents=True, exist_ok=True)
            timeout_secs = int(payload.get("timeout_secs") or 30)
            try:
                code, stdout, stderr = run_shell(payload.get("command", ""), cwd, timeout_secs)
            except subprocess.TimeoutExpired:
                json_response(self, 200, {
                    "success": False,
                    "error_type": "timeout",
                    "error_code": "command_timeout",
                    "trace": f"timed out after {timeout_secs}s",
                    "output": None,
                    "checkpoint": None,
                    "observed_state_hash": None,
                    "timestamp": datetime.datetime.utcnow().isoformat() + "Z",
                })
                return
            artifacts = collect_paths(WORKSPACE, payload.get("collect_paths", []))
            base = {
                "stdout": stdout,
                "stderr": stderr,
                "exit_code": code,
                "command": payload.get("command", ""),
                "cwd": str(cwd),
                "task_id": payload.get("task_id"),
                "workspace": {"root": WORKSPACE, "collected_artifacts": artifacts},
            }
            if code == 0:
                json_response(self, 200, {
                    "success": True,
                    "error_type": "none",
                    "error_code": None,
                    "trace": None,
                    "output": base,
                    "checkpoint": {
                        "type": "worker_shell",
                        "cwd": str(cwd),
                        "exit_code": code,
                        "collected_artifact_count": len(payload.get("collect_paths", [])),
                    },
                    "observed_state_hash": None,
                    "timestamp": datetime.datetime.utcnow().isoformat() + "Z",
                })
            else:
                json_response(self, 200, {
                    "success": False,
                    "error_type": "tool",
                    "error_code": f"exit_{code}",
                    "trace": stderr,
                    "output": base,
                    "checkpoint": {
                        "type": "worker_shell",
                        "cwd": str(cwd),
                        "exit_code": code,
                        "collected_artifact_count": len(payload.get("collect_paths", [])),
                    },
                    "observed_state_hash": None,
                    "timestamp": datetime.datetime.utcnow().isoformat() + "Z",
                })
            return

        if self.path == "/browser/fetch":
            url = payload.get("url", "")
            max_chars = int(payload.get("max_chars") or 12000)
            try:
                with urllib.request.urlopen(url, timeout=30) as resp:
                    html_text = resp.read().decode("utf-8", "replace")
                    final_url = resp.geturl()
                    status = getattr(resp, "status", 200)
            except Exception as e:
                json_response(self, 200, {
                    "success": False,
                    "error_type": "tool",
                    "error_code": "browser_fetch_error",
                    "trace": str(e),
                    "output": None,
                    "checkpoint": None,
                    "observed_state_hash": None,
                    "timestamp": datetime.datetime.utcnow().isoformat() + "Z",
                })
                return
            title = ""
            m = re.search(r"<title[^>]*>(.*?)</title>", html_text, re.I | re.S)
            if m:
                title = html.unescape(re.sub(r"\s+", " ", m.group(1)).strip())
            text = re.sub(r"<script.*?</script>|<style.*?</style>", " ", html_text, flags=re.I | re.S)
            text = re.sub(r"<[^>]+>", " ", text)
            text = html.unescape(re.sub(r"\s+", " ", text)).strip()
            json_response(self, 200, {
                "success": True,
                "error_type": "none",
                "error_code": None,
                "trace": None,
                "output": {
                    "url": url,
                    "final_url": final_url,
                    "status": status,
                    "title": title,
                    "text": text[:max_chars],
                },
                "checkpoint": {"type": "browser_fetch", "url": url},
                "observed_state_hash": None,
                "timestamp": datetime.datetime.utcnow().isoformat() + "Z",
            })
            return

        self.send_error(404)

def main():
    import sys
    auth_desc = f"configured (len={len(TOKEN)}, fp={FP}...)" if TOKEN else "none (open access)"
    print(f"[worker] auth: {auth_desc}", file=sys.stderr, flush=True)
    pathlib.Path(WORKSPACE).mkdir(parents=True, exist_ok=True)
    httpd = ThreadingHTTPServer(("0.0.0.0", PORT), Handler)
    httpd.serve_forever()

if __name__ == "__main__":
    main()
'@
}

function Get-WorkerEnvFile {
    param(
        [string]$WorkerToken,
        [string]$WorkspacePath,
        [int]$WorkerPort
    )

    @"
WORKER_WORKSPACE=$WorkspacePath
WORKER_PORT=$WorkerPort
WORKER_TOKEN=$WorkerToken
"@
}

function Get-WorkerServiceUnit {
@'
[Unit]
Description=AI Home Server Worker
After=aihomeserver-static-network.service network-online.target
Wants=aihomeserver-static-network.service network-online.target

[Service]
Type=simple
User=ubuntu
WorkingDirectory=/workspace
EnvironmentFile=/etc/aihomeserver-worker.env
ExecStart=/usr/bin/python3 /usr/local/bin/aihomeserver-worker.py
Restart=always
RestartSec=3

[Install]
WantedBy=multi-user.target
'@
}

function Get-SudoersRule {
@'
ubuntu ALL=(ALL) NOPASSWD:ALL
'@
}

function Indent-MultilineText {
    param(
        [string]$Text,
        [int]$Spaces
    )

    $prefix = ' ' * $Spaces
    (($Text -split "`r?`n") | ForEach-Object { "$prefix$_" }) -join "`n"
}

function Get-CloudInitUserData {
    param(
        [string]$RepoUrl,
        [string]$Branch,
        [string]$WorkerToken,
        [string]$WorkspacePath,
        [int]$WorkerPort,
        [string]$VmMac
    )

    $publicKey = Get-WorkerPublicKey
    $networkScript = Indent-MultilineText -Text (Get-WorkerNetworkSetupScript -VmIp $VmIp -VmGateway $VmGateway -VmMac $VmMac) -Spaces 6
    $netplan = Indent-MultilineText -Text (Get-WorkerNetplanConfig -VmIp $VmIp -VmGateway $VmGateway -VmMac $VmMac) -Spaces 6
    $workerScript = Indent-MultilineText -Text (Get-WorkerPythonScript) -Spaces 6
    $networkService = Indent-MultilineText -Text (Get-WorkerNetworkBootstrapService) -Spaces 6
    $serviceUnit = Indent-MultilineText -Text (Get-WorkerServiceUnit) -Spaces 6
    $sudoersRule = Indent-MultilineText -Text (Get-SudoersRule) -Spaces 6
    $envFile = Indent-MultilineText -Text (Get-WorkerEnvFile -WorkerToken $WorkerToken -WorkspacePath $WorkspacePath -WorkerPort $WorkerPort) -Spaces 6

    @"
#cloud-config
package_update: false
package_upgrade: false
users:
  - default
write_files:
  - path: /usr/local/bin/aihomeserver-network-setup
    permissions: '0755'
    content: |
$networkScript
  - path: /etc/netplan/99-aihomeserver.yaml
    permissions: '0644'
    content: |
$netplan
  - path: /usr/local/bin/aihomeserver-worker.py
    permissions: '0755'
    content: |
$workerScript
  - path: /etc/systemd/system/aihomeserver-static-network.service
    permissions: '0644'
    content: |
$networkService
  - path: /etc/systemd/system/aihomeserver-worker.service
    permissions: '0644'
    content: |
$serviceUnit
  - path: /etc/aihomeserver-worker.env
    permissions: '0600'
    content: |
$envFile
  - path: /etc/sudoers.d/90-aihomeserver-ubuntu
    permissions: '0440'
    content: |
$sudoersRule
bootcmd:
  - [ bash, -lc, "netplan generate && netplan apply || true" ]
runcmd:
  - [ bash, -lc, "mkdir -p /workspace /opt/aihomeserver /var/lib/aihomeserver /home/ubuntu/.ssh" ]
  - [ bash, -lc, "printf '$publicKey\n' > /home/ubuntu/.ssh/authorized_keys && chmod 700 /home/ubuntu/.ssh && chmod 600 /home/ubuntu/.ssh/authorized_keys && chown -R ubuntu:ubuntu /home/ubuntu/.ssh" ]
  - [ bash, -lc, "systemctl daemon-reload && systemctl enable aihomeserver-static-network.service && systemctl start aihomeserver-static-network.service || true" ]
  - [ bash, -lc, "chown -R ubuntu:ubuntu /workspace /opt/aihomeserver /var/lib/aihomeserver" ]
  - [ bash, -lc, "systemctl daemon-reload && systemctl enable aihomeserver-worker.service && systemctl restart aihomeserver-worker.service" ]
"@
}


function Ensure-SeedIso {
    param(
        [string]$SeedPath,
        [string]$UserData,
        [string]$VmIp,
        [string]$VmGateway,
        [string]$VmMac,
        [string]$InstanceId
    )

    $seedDir = Split-Path -Parent $SeedPath
    $seedSourceDir = Join-Path $seedDir 'cidata-src'
    Ensure-Directory $seedDir
    if (Test-Path $seedSourceDir) {
        Remove-Item -LiteralPath $seedSourceDir -Recurse -Force -ErrorAction SilentlyContinue
    }
    Ensure-Directory $seedSourceDir

    $metaData = @"
instance-id: $InstanceId
local-hostname: aihomeserver-worker
"@
    $networkData = @"
version: 2
ethernets:
  eth0:
    match:
      macaddress: $(Format-MacAddress -MacAddress $VmMac)
    set-name: eth0
    dhcp4: false
    addresses:
      - $VmIp/24
    gateway4: $VmGateway
    nameservers:
      addresses:
        - 1.1.1.1
        - 8.8.8.8
"@
    Set-Content -Path (Join-Path $seedSourceDir 'user-data') -Value $UserData -Encoding Ascii
    Set-Content -Path (Join-Path $seedSourceDir 'meta-data') -Value $metaData -Encoding Ascii
    Set-Content -Path (Join-Path $seedSourceDir 'network-config') -Value $networkData -Encoding Ascii
    Write-Log "Seed ISO prepared for $(Format-MacAddress -MacAddress $VmMac) with static IP $VmIp"

    if (Test-Path $SeedPath) {
        Wait-FileUnlocked -Path $SeedPath -TimeoutSeconds 60
        Remove-Item -LiteralPath $SeedPath -Force
    }

    New-IsoFile -SourceDir $seedSourceDir -OutFile $SeedPath -VolumeName 'cidata'
}

function Ensure-UbuntuImage {
    param(
        [string]$ImageDir,
        [string]$Version
    )

    $imageExtractDir = Join-Path $ImageDir "ubuntu-$Version"
    Ensure-Directory $imageExtractDir
    $tarPath = Join-Path $ImageDir "ubuntu-$Version-server-cloudimg-amd64-azure.vhd.tar.gz"
    $legacyTarCandidates = @(
        (Join-Path $imageExtractDir "ubuntu-$Version-azure.vhd.tar.gz"),
        (Join-Path $imageExtractDir "ubuntu-$Version-server-cloudimg-amd64-azure.vhd.tar.gz")
    )

    # Dedicated immutable-ish base image path. Older revisions booted directly
    # from ubuntu-<version>-server-cloudimg-amd64.vhdx, so that legacy path may
    # already contain stale cloud-init, service, or token state.
    $vhdxPath = Join-Path $imageExtractDir "ubuntu-$Version-server-cloudimg-amd64-base.vhdx"
    $legacyVhdxPath = Join-Path $imageExtractDir "ubuntu-$Version-server-cloudimg-amd64.vhdx"

    if (-not (Test-Path $vhdxPath)) {
        $existingArchives = @()
        $allCandidates = @($tarPath) + $legacyTarCandidates
        foreach ($candidate in $allCandidates) {
            if ((Test-Path $candidate) -and ((Get-Item $candidate).Length -gt 0)) {
                $existingArchives += Get-Item $candidate
            }
        }

        $archivePath = $null
        if ($existingArchives.Count -gt 0) {
            $selected = $existingArchives | Sort-Object Length -Descending | Select-Object -First 1
            $archivePath = $selected.FullName
            Write-Log "Using existing Ubuntu archive cache: $archivePath ($($selected.Length) bytes)"
            if ((Test-Path $tarPath) -and ($archivePath -ne $tarPath)) {
                Write-Log "Ignoring smaller or stale archive at $tarPath ($((Get-Item $tarPath).Length) bytes)"
            }
        }

        if (-not $archivePath) {
            Invoke-Download -Url (Get-ImageUrl -Version $Version) -OutFile $tarPath
            $archivePath = $tarPath
        }

        $sourceStamp = [DateTime]::UtcNow.ToString('yyyyMMddHHmmssfff')
        $sourceExtractDir = Join-Path $imageExtractDir "source-$sourceStamp"
        Ensure-Directory $sourceExtractDir

        Write-Log "Extracting Ubuntu cloud image into clean source directory"
        tar -xzf $archivePath -C $sourceExtractDir

        $sourceVhd = Get-ChildItem -Path $sourceExtractDir -Recurse -Filter *.vhd | Select-Object -First 1
        if (-not $sourceVhd) {
            throw "Ubuntu cloud image was downloaded but no .vhd was found in $sourceExtractDir"
        }

        try {
            fsutil sparse setflag $sourceVhd.FullName 0 | Out-Null
        } catch {
            Write-Log "Warning: unable to clear sparse flag on $($sourceVhd.FullName); continuing with conversion attempt"
        }

        if (Test-Path $legacyVhdxPath) {
            Write-Log "Ignoring legacy VM base path $legacyVhdxPath and rebuilding a clean base image"
        }

        Write-Log "Converting Ubuntu cloud image to dedicated base VHDX"
        Convert-VHD -Path $sourceVhd.FullName -DestinationPath $vhdxPath -VHDType Dynamic | Out-Null
    }

    if (-not (Test-Path $vhdxPath)) {
        throw "Ubuntu cloud image conversion failed; expected VHDX at $vhdxPath"
    }
    return $vhdxPath
}

function New-WorkingDisk {
    param(
        [string]$BaseVhdx,
        [string]$VmDir,
        [string]$VmName
    )

    Ensure-Directory $VmDir
    $workingDisk = Join-Path $VmDir "$VmName-disk.vhdx"

    if (Test-Path $workingDisk) {
        Write-Log "Removing stale working disk: $workingDisk"
        Remove-Item -LiteralPath $workingDisk -Force
    }

    Write-Log "Copying base image to working disk: $workingDisk"
    Copy-Item -LiteralPath $BaseVhdx -Destination $workingDisk -Force
    return $workingDisk
}

function Ensure-VMSwitchAndNat {
    param(
        [string]$SwitchName,
        [string]$VmGateway
    )

    $switch = Get-VMSwitch -Name $SwitchName -ErrorAction SilentlyContinue
    if (-not $switch) {
        New-VMSwitch -Name $SwitchName -SwitchType Internal | Out-Null
    }

    $interfaceAlias = "vEthernet ($SwitchName)"
    $ip = Get-NetIPAddress -InterfaceAlias $interfaceAlias -ErrorAction SilentlyContinue | Where-Object { $_.IPAddress -eq $VmGateway } | Select-Object -First 1
    if (-not $ip) {
        try {
            New-NetIPAddress -InterfaceAlias $interfaceAlias -IPAddress $VmGateway -PrefixLength 24 | Out-Null
        } catch {
            if ($_.Exception.Message -notmatch 'already exists') {
                throw
            }
        }
    }

    $natName = "$SwitchName-NAT"
    $nat = Get-NetNat -Name $natName -ErrorAction SilentlyContinue
    if (-not $nat) {
        try {
            New-NetNat -Name $natName -InternalIPInterfaceAddressPrefix "$VmGateway/24" | Out-Null
        } catch {
            if ($_.Exception.Message -notmatch 'already exists') {
                throw
            }
        }
    }
}

function Set-WorkerPortProxy {
    param(
        [string]$VmIp,
        [int]$WorkerPort
    )
    # Forward host 0.0.0.0:<port> → VM <ip>:<port> so Docker containers that
    # reach the Windows host via host.docker.internal can route to the VM worker.
    netsh interface portproxy delete v4tov4 listenport=$WorkerPort listenaddress=0.0.0.0 2>$null | Out-Null
    netsh interface portproxy add v4tov4 listenport=$WorkerPort listenaddress=0.0.0.0 connectport=$WorkerPort connectaddress=$VmIp | Out-Null
    Write-Log "Port proxy: 0.0.0.0:$WorkerPort -> $VmIp`:$WorkerPort"
}

function Remove-WorkerPortProxy {
    param([int]$WorkerPort)
    netsh interface portproxy delete v4tov4 listenport=$WorkerPort listenaddress=0.0.0.0 2>$null | Out-Null
    Write-Log "Port proxy removed for port $WorkerPort"
}

function Resolve-WslGuestRoot {
    param(
        [string]$MountPoint,
        [int]$TimeoutSeconds = 15
    )

    $mountCandidates = @($MountPoint)
    if ($MountPoint -like '/mnt/wsl/*') {
        $mountCandidates += ($MountPoint -replace '^/mnt/wsl/', '/mnt/host/wsl/')
    } elseif ($MountPoint -like '/mnt/host/wsl/*') {
        $mountCandidates += ($MountPoint -replace '^/mnt/host/wsl/', '/mnt/wsl/')
    }

    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ((Get-Date) -lt $deadline) {
        foreach ($candidate in ($mountCandidates | Select-Object -Unique)) {
            $probe = wsl -u root sh -c "[ -d '$candidate/etc' ] && [ -d '$candidate/home' ]" 2>&1
            if ($LASTEXITCODE -eq 0) {
                return $candidate
            }

            $childRoots = wsl -u root sh -c "find '$candidate' -mindepth 1 -maxdepth 1 -type d 2>/dev/null" 2>&1
            if ($LASTEXITCODE -eq 0) {
                foreach ($childRoot in ((($childRoots | Out-String) -replace "`0", '').Trim() -split "`r?`n")) {
                    if ([string]::IsNullOrWhiteSpace($childRoot)) {
                        continue
                    }

                    $verify = wsl -u root sh -c "[ -d '$childRoot/etc' ] && [ -d '$childRoot/home' ]" 2>&1
                    if ($LASTEXITCODE -eq 0) {
                        return $childRoot.Trim()
                    }
                }
            }
        }
        Start-Sleep -Seconds 1
    }

    throw "Mounted WSL path $MountPoint did not expose a Linux filesystem root within ${TimeoutSeconds}s"
}

function Get-WslMountPoint {
    param(
        [string]$VhdxPath,
        [int]$TimeoutSeconds = 30
    )

    $mountOutput = wsl --mount --vhd $VhdxPath --partition 1 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "wsl --mount failed (exit $LASTEXITCODE): $mountOutput"
    }

    $mountOutputText = (($mountOutput | Out-String) -replace "`0", '').Trim()
    $mountPoint = $null
    if ($mountOutputText -match "mounted as '([^']+)'") {
        $mountPoint = $matches[1]
    }

    if (-not $mountPoint) {
        # Fallback to polling /proc/mounts if WSL did not print the mount path.
        $beforeMounts = @(wsl -- cat /proc/mounts 2>$null)
        $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
        while ((Get-Date) -lt $deadline) {
            $afterMounts = @(wsl -- cat /proc/mounts 2>$null)
            $mountPoint = (Compare-Object $beforeMounts $afterMounts |
                Where-Object { $_.SideIndicator -eq '=>' } |
                Select-Object -ExpandProperty InputObject |
                ForEach-Object { ($_ -split '\s+')[1] } |
                Where-Object { $_ -and $_ -notmatch '^/(proc|sys|dev|run)' } |
                Select-Object -First 1)
            if ($mountPoint) { break }
            Start-Sleep -Seconds 2
        }

        if (-not $mountPoint) {
            throw "wsl --mount did not produce a new /proc/mounts entry within ${TimeoutSeconds}s for $VhdxPath"
        }
    }

    $resolvedMount = Resolve-WslGuestRoot -MountPoint $mountPoint
    return [pscustomobject]@{ MountPoint = $resolvedMount; Vhdx = $VhdxPath }
}

function Set-CloudInitDatasource {
    param([string]$VhdxPath)

    $mount = $null
    try {
        $mount = Get-WslMountPoint -VhdxPath $VhdxPath
        Write-Log "Patching cloud-init datasource at $($mount.MountPoint)"
        $patchOutput = wsl -u root sh -c "mkdir -p '$($mount.MountPoint)/etc/cloud/cloud.cfg.d' && printf 'datasource_list: [NoCloud, None]\n' > '$($mount.MountPoint)/etc/cloud/cloud.cfg.d/90_dpkg.cfg' && printf 'datasource_list: [NoCloud, None]\n' > '$($mount.MountPoint)/etc/cloud/cloud.cfg.d/99-nocloud.cfg'" 2>&1
        if ($LASTEXITCODE -ne 0) {
            throw "Failed to write cloud-init patch to $($mount.MountPoint)`: $patchOutput"
        }
    } finally {
        if ($mount) {
            wsl --unmount $VhdxPath 2>$null | Out-Null
            # Reset $LASTEXITCODE so a failed unmount cannot propagate as the
            # script's process exit code in PowerShell 5.1.
            $global:LASTEXITCODE = 0
        }
    }
}

function Invoke-WslRootCommand {
    param([string]$Command)

    $output = wsl -u root sh -c $Command 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "WSL root command failed: $output"
    }
    return $output
}

function Write-MountedGuestFile {
    param(
        [string]$MountPoint,
        [string]$GuestPath,
        [string]$Content,
        [string]$Mode,
        [string]$Owner = ''
    )

    $dest = ($MountPoint.TrimEnd('/') + $GuestPath)
    $destDir = Get-PosixDirectoryName $dest
    $encoded = [Convert]::ToBase64String([System.Text.UTF8Encoding]::new($false).GetBytes($Content))
    $commands = @(
        "mkdir -p '$destDir'"
        "printf '%s' '$encoded' | base64 -d > '$dest'"
        "chmod $Mode '$dest'"
    )
    if ($Owner) {
        $commands += "chown $Owner '$dest'"
    }
    Invoke-WslRootCommand -Command ($commands -join ' && ') | Out-Null
}

function Provision-WorkerDiskOffline {
    param(
        [string]$VhdxPath,
        [string]$VmIp,
        [string]$VmGateway,
        [string]$VmMac,
        [string]$WorkerToken,
        [string]$WorkspacePath,
        [int]$WorkerPort
    )

    $mount = $null
    try {
        $mount = Get-WslMountPoint -VhdxPath $VhdxPath
        $root = $mount.MountPoint.TrimEnd('/')
        Write-Log "Provisioning worker disk offline at $root"

        Invoke-WslRootCommand -Command @"
rm -f '$root/etc/netplan/90-hotplug-azure.yaml'
rm -rf '$root/var/lib/cloud'
mkdir -p '$root/workspace' '$root/opt/aihomeserver' '$root/var/lib/aihomeserver'
mkdir -p '$root/home/ubuntu/.ssh' '$root/etc/systemd/system/multi-user.target.wants' '$root/etc/sudoers.d'
chown -R 1000:1000 '$root/workspace' '$root/opt/aihomeserver' '$root/var/lib/aihomeserver' '$root/home/ubuntu/.ssh'
chmod 700 '$root/home/ubuntu/.ssh'
"@ | Out-Null

        Write-MountedGuestFile -MountPoint $root -GuestPath '/etc/netplan/99-aihomeserver.yaml' -Content (Get-WorkerNetplanConfig -VmIp $VmIp -VmGateway $VmGateway -VmMac $VmMac) -Mode '0644'
        Write-MountedGuestFile -MountPoint $root -GuestPath '/usr/local/bin/aihomeserver-network-setup' -Content (Get-WorkerNetworkSetupScript -VmIp $VmIp -VmGateway $VmGateway -VmMac $VmMac) -Mode '0755'
        Write-MountedGuestFile -MountPoint $root -GuestPath '/usr/local/bin/aihomeserver-worker.py' -Content (Get-WorkerPythonScript) -Mode '0755'
        Write-MountedGuestFile -MountPoint $root -GuestPath '/etc/systemd/system/aihomeserver-worker.service' -Content (Get-WorkerServiceUnit) -Mode '0644'
        Write-MountedGuestFile -MountPoint $root -GuestPath '/etc/aihomeserver-worker.env' -Content (Get-WorkerEnvFile -WorkerToken $WorkerToken -WorkspacePath $WorkspacePath -WorkerPort $WorkerPort) -Mode '0600'
        Write-MountedGuestFile -MountPoint $root -GuestPath '/etc/sudoers.d/90-aihomeserver-ubuntu' -Content (Get-SudoersRule) -Mode '0440'
        Write-MountedGuestFile -MountPoint $root -GuestPath '/home/ubuntu/.ssh/authorized_keys' -Content ("$(Get-WorkerPublicKey)`n") -Mode '0600' -Owner '1000:1000'
        Write-MountedGuestFile -MountPoint $root -GuestPath '/etc/cloud/cloud.cfg.d/90_dpkg.cfg' -Content "datasource_list: [NoCloud, None]`n" -Mode '0644'
        Write-MountedGuestFile -MountPoint $root -GuestPath '/etc/cloud/cloud.cfg.d/99-nocloud.cfg' -Content "datasource_list: [NoCloud, None]`n" -Mode '0644'

        Invoke-WslRootCommand -Command @"
ln -sf ../aihomeserver-worker.service '$root/etc/systemd/system/multi-user.target.wants/aihomeserver-worker.service'
chmod 600 '$root/home/ubuntu/.ssh/authorized_keys'
chown -R 1000:1000 '$root/home/ubuntu/.ssh'
"@ | Out-Null
    } finally {
        if ($mount) {
            wsl --unmount $VhdxPath 2>$null | Out-Null
            $global:LASTEXITCODE = 0
        }
    }
}

function Ensure-VM {
    param(
        [string]$VmName,
        [string]$VmDisk,
        [string]$SeedIso,
        [string]$SwitchName,
        [int]$VmCpus,
        [int]$VmMemoryMb
    )

    $vmMemoryBytes = [Int64]$VmMemoryMb * 1MB
    $staticMac = Get-StaticMacAddress -VmName $VmName
    $vm = Get-VM -Name $VmName -ErrorAction SilentlyContinue
    if (-not $vm) {
        New-VM -Name $VmName -Generation 2 -MemoryStartupBytes $vmMemoryBytes -NoVHD -SwitchName $SwitchName | Out-Null
        Add-VMHardDiskDrive -VMName $VmName -ControllerType SCSI -ControllerNumber 0 -ControllerLocation 0 | Out-Null
        Set-VMHardDiskDrive -VMName $VmName -ControllerType SCSI -ControllerNumber 0 -ControllerLocation 0 -Path $VmDisk | Out-Null
        $vm = Get-VM -Name $VmName
        Set-VMProcessor -VMName $VmName -Count $VmCpus | Out-Null
        Set-VMMemory -VMName $VmName -DynamicMemoryEnabled $false -StartupBytes $vmMemoryBytes | Out-Null
        Set-VMFirmware -VMName $VmName -EnableSecureBoot Off | Out-Null
        Set-VMNetworkAdapter -VMName $VmName -StaticMacAddress $staticMac | Out-Null
        Add-VMDvdDrive -VMName $VmName -Path $SeedIso | Out-Null
        return
    }

    Set-VMProcessor -VMName $VmName -Count $VmCpus | Out-Null
    Set-VMMemory -VMName $VmName -DynamicMemoryEnabled $false -StartupBytes $vmMemoryBytes | Out-Null
    Set-VMFirmware -VMName $VmName -EnableSecureBoot Off | Out-Null
    Set-VMNetworkAdapter -VMName $VmName -StaticMacAddress $staticMac | Out-Null

    $dvdAttached = Get-VMDvdDrive -VMName $VmName -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $SeedIso } | Select-Object -First 1
    if (-not $dvdAttached) {
        $attempts = 0
        while ($true) {
            try {
                $dvd = Get-VMDvdDrive -VMName $VmName -ErrorAction SilentlyContinue | Select-Object -First 1
                if ($dvd) {
                    Set-VMDvdDrive -VMName $VmName -Path $SeedIso | Out-Null
                } else {
                    Add-VMDvdDrive -VMName $VmName -Path $SeedIso | Out-Null
                }
                break
            } catch {
                $attempts += 1
                if ($attempts -ge 30) {
                    throw
                }
                Start-Sleep -Seconds 2
            }
        }
    }
}

function Start-WorkerVM {
    param([string]$VmName)
    $vm = Get-VM -Name $VmName -ErrorAction Stop
    if ($vm.State -ne 'Running') {
        Start-VM -Name $VmName | Out-Null
    }
}

function Wait-WorkerHealth {
    param(
        [string]$VmIp,
        [int]$Port,
        [string]$Token = ''
    )

    $baseUrl = "http://$VmIp`:$Port"
    $deadline = (Get-Date).AddMinutes(30)

    # Phase 1: wait for the unauthenticated /health endpoint to respond.
    $healthPassed = $false
    while (-not $healthPassed -and (Get-Date) -lt $deadline) {
        try {
            if (Test-NetConnection -ComputerName $VmIp -Port $Port -InformationLevel Quiet) {
                $resp = Invoke-WebRequest -Uri "$baseUrl/health" -TimeoutSec 10 -UseBasicParsing
                if ($resp.StatusCode -ge 200 -and $resp.StatusCode -lt 300) {
                    $healthPassed = $true
                    $fp = if ($Token) { $Token.Substring(0, [Math]::Min(8, $Token.Length)) } else { '' }
                    Write-Log "Worker /health OK (token fp=$fp...)"
                }
            }
        } catch {}
        if (-not $healthPassed) { Start-Sleep -Seconds 15 }
    }
    if (-not $healthPassed) { return $false }

    # Phase 2: if a token was supplied, wait for the authenticated /shell probe
    # to pass. The cloud-init runcmd restarts the service after writing the new
    # service file, so the first worker that answers /health may still be running
    # with the old token. Gating here ensures bootstrap only returns success when
    # the correct token is active.
    if (-not $Token) { return $true }

    $authDeadline = (Get-Date).AddMinutes(5)
    while ((Get-Date) -lt $authDeadline) {
        try {
            $headers = @{ Authorization = "Bearer $Token"; 'Content-Type' = 'application/json' }
            $body    = '{"command":"echo auth-ok","cwd":".","timeout_secs":10}'
            $probe   = Invoke-WebRequest -Uri "$baseUrl/shell" -Method Post -Headers $headers -Body $body -TimeoutSec 15 -UseBasicParsing
            if ($probe.StatusCode -eq 200) {
                Write-Log "Worker auth probe OK"
                return $true
            }
            if ($probe.StatusCode -eq 401) {
                Write-Log "Worker auth probe 401 - service may be restarting with new token, retrying..."
            }
        } catch {
            Write-Log "Worker auth probe error: $($_.Exception.Message)"
        }
        Start-Sleep -Seconds 5
    }

    Write-Log "Worker auth probe timed out after health passed"
    return $false
}

function Get-Status {
    param(
        [string]$VmName,
        [string]$VmIp,
        [int]$WorkerPort
    )

    $vm = Get-VM -Name $VmName -ErrorAction SilentlyContinue
    $adapter = Get-VMNetworkAdapter -VMName $VmName -ErrorAction SilentlyContinue | Select-Object -First 1
    $portOpen = $false
    try {
        $portOpen = Test-NetConnection -ComputerName $VmIp -Port $WorkerPort -InformationLevel Quiet
    } catch {
        $portOpen = $false
    }
    $workerUrl = "http://$VmIp`:$WorkerPort"
    return [hashtable]@{
        ok = [bool]$vm
        vm_name = $VmName
        vm_state = if ($vm) { $vm.State.ToString() } else { 'Missing' }
        ip_addresses = if ($adapter) { @($adapter.IPAddresses) } else { @() }
        worker_port_open = $portOpen
        worker_url = $workerUrl
        backend = 'hyperv'
    }
}

function Get-SshArguments {
    param([string]$VmIp)

    $args = @(
        '-o', 'BatchMode=yes',
        '-o', 'StrictHostKeyChecking=no',
        '-o', 'UserKnownHostsFile=NUL',
        '-o', 'ConnectTimeout=10'
    )

    $preferredKey = Join-Path $env:USERPROFILE '.ssh\aihomeserver'
    if (Test-Path $preferredKey) {
        $args += @('-i', $preferredKey)
    }

    $args += "ubuntu@$VmIp"
    return $args
}

function Invoke-GuestSsh {
    param(
        [string]$VmIp,
        [string]$Command
    )

    $sshArgs = @(Get-SshArguments -VmIp $VmIp)
    $allArgs = $sshArgs + @($Command)

    # Use Start-Process with file redirection to keep stderr (SSH host-key
    # warnings) out of the PowerShell error stream. 2>&1 in PS 5.1 wraps
    # stderr lines as ErrorRecord objects and sets $? = false even when ssh
    # exits 0, which causes false failures on "Permanently added ... to known hosts."
    $stdoutFile = [System.IO.Path]::GetTempFileName()
    $stderrFile = [System.IO.Path]::GetTempFileName()
    try {
        $proc = Start-Process ssh.exe -ArgumentList $allArgs -NoNewWindow -PassThru `
            -RedirectStandardOutput $stdoutFile -RedirectStandardError $stderrFile -Wait
        $stdout = Get-Content -LiteralPath $stdoutFile -Raw -ErrorAction SilentlyContinue
        $stderr = Get-Content -LiteralPath $stderrFile -Raw -ErrorAction SilentlyContinue
        if ($proc.ExitCode -ne 0) {
            throw "SSH command failed (exit $($proc.ExitCode)) for ${VmIp}: $stderr"
        }
        return $stdout
    } finally {
        Remove-Item -LiteralPath $stdoutFile, $stderrFile -Force -ErrorAction SilentlyContinue
    }
}

function Export-WorkerLogs {
    param(
        [string]$VmName,
        [string]$VmIp,
        [int]$WorkerPort,
        [hashtable]$Paths
    )

    Ensure-Directory $Paths.LogDir
    $timestamp = [DateTime]::UtcNow.ToString('yyyyMMddHHmmss')
    $prefix = "worker-$timestamp"

    $status = Get-Status -VmName $VmName -VmIp $VmIp -WorkerPort $WorkerPort
    if (-not $status.ok) {
        throw "VM $VmName is missing; cannot export worker logs"
    }

    $journalPath = Join-Path $Paths.LogDir "$prefix-journalctl.log"
    $servicePath = Join-Path $Paths.LogDir "$prefix-service-status.log"
    $networkPath = Join-Path $Paths.LogDir "$prefix-network.log"
    $summaryPath = Join-Path $Paths.LogDir "$prefix-summary.txt"

    $journalCmd = @'
sudo journalctl -u aihomeserver-worker --no-pager -n 400
'@
    $serviceCmd = @'
bash -lc 'sudo systemctl status aihomeserver-worker --no-pager --full || sudo systemctl cat aihomeserver-worker --no-pager'
'@
    $networkCmd = @'
bash -lc 'printf "== ip addr ==\n"; ip addr show; printf "\n== ip route ==\n"; ip route show; printf "\n== token fingerprint ==\n"; sudo journalctl -u aihomeserver-worker --no-pager -n 50 | grep -m1 "\[worker\] auth:" || echo unavailable'
'@

    $journal = Invoke-GuestSsh -VmIp $VmIp -Command $journalCmd
    $service = Invoke-GuestSsh -VmIp $VmIp -Command $serviceCmd
    $network = Invoke-GuestSsh -VmIp $VmIp -Command $networkCmd

    Set-Content -LiteralPath $journalPath -Value $journal -Encoding UTF8
    Set-Content -LiteralPath $servicePath -Value $service -Encoding UTF8
    Set-Content -LiteralPath $networkPath -Value $network -Encoding UTF8

    $summary = @(
        "vm_name=$VmName"
        "vm_ip=$VmIp"
        "worker_url=http://$VmIp`:$WorkerPort"
        "exported_utc=$([DateTime]::UtcNow.ToString('o'))"
        "journal=$journalPath"
        "service=$servicePath"
        "network=$networkPath"
    ) -join [Environment]::NewLine
    Set-Content -LiteralPath $summaryPath -Value $summary -Encoding UTF8

    return @{
        ok = $true
        vm_name = $VmName
        vm_state = $status.vm_state
        worker_url = "http://$VmIp`:$WorkerPort"
        backend = 'hyperv'
        log_dir = $Paths.LogDir
        exported_files = @($summaryPath, $journalPath, $servicePath, $networkPath)
    }
}

Assert-Admin
Assert-HyperVAvailable

$paths = Get-Paths
$workerUrl = "http://$VmIp`:$WorkerPort"

switch ($Action) {
    'status' {
        Write-Output (Write-JsonResult (Get-Status -VmName $VmName -VmIp $VmIp -WorkerPort $WorkerPort))
    }
    'export-logs' {
        Write-Output (Write-JsonResult (Export-WorkerLogs -VmName $VmName -VmIp $VmIp -WorkerPort $WorkerPort -Paths $paths))
    }
    'stop' {
        Remove-VmArtifacts -VmName $VmName
        Remove-WorkerPortProxy -WorkerPort $WorkerPort
        Write-Output (Write-JsonResult @{
            ok = $true
            vm_name = $VmName
            vm_state = 'Stopped'
            worker_url = $workerUrl
            backend = 'hyperv'
        })
    }
    'start' {
        Ensure-VMSwitchAndNat -SwitchName $SwitchName -VmGateway $VmGateway
        Start-WorkerVM -VmName $VmName
        Set-WorkerPortProxy -VmIp $VmIp -WorkerPort $WorkerPort
        Write-Output (Write-JsonResult (Get-Status -VmName $VmName -VmIp $VmIp -WorkerPort $WorkerPort))
    }
    'bootstrap' {
        Assert-WslMountAvailable
        Remove-VmArtifacts -VmName $VmName
        Ensure-VMSwitchAndNat -SwitchName $SwitchName -VmGateway $VmGateway
        $imageVhd = Ensure-UbuntuImage -ImageDir $paths.ImageDir -Version $ImageVersion
        # Copy base image to a disposable per-VM working disk so each bootstrap
        # starts from a clean Ubuntu state with no stale cloud-init or token data.
        $workingVhd = New-WorkingDisk -BaseVhdx $imageVhd -VmDir $paths.VmDir -VmName $VmName
        $seedStamp = [DateTime]::UtcNow.ToString('yyyyMMddHHmmssfff')
        $seedPath = Join-Path $paths.SeedDir ("cloud-init-seed-$seedStamp.iso")
        $instanceId = "aihomeserver-$VmName-$seedStamp"
        $vmMac = Get-StaticMacAddress -VmName $VmName
        $tokenFp = if ($WorkerToken) { $WorkerToken.Substring(0, [Math]::Min(8, $WorkerToken.Length)) } else { '(none)' }
        Write-Log "Bootstrap token fp=$tokenFp... instance=$instanceId disk=$workingVhd"
        Provision-WorkerDiskOffline -VhdxPath $workingVhd -VmIp $VmIp -VmGateway $VmGateway -VmMac $vmMac -WorkerToken $WorkerToken -WorkspacePath $WorkspacePath -WorkerPort $WorkerPort
        $userData = Get-CloudInitUserData -RepoUrl $RepoUrl -Branch $Branch -WorkerToken $WorkerToken -WorkspacePath $WorkspacePath -WorkerPort $WorkerPort -VmMac $vmMac
        Ensure-SeedIso -SeedPath $seedPath -UserData $userData -VmIp $VmIp -VmGateway $VmGateway -VmMac $vmMac -InstanceId $instanceId
        Ensure-VM -VmName $VmName -VmDisk $workingVhd -SeedIso $seedPath -SwitchName $SwitchName -VmCpus $VmCpus -VmMemoryMb $VmMemoryMb
        Start-WorkerVM -VmName $VmName
        $ready = Wait-WorkerHealth -VmIp $VmIp -Port $WorkerPort -Token $WorkerToken
        if (-not $ready) {
            throw "Worker VM did not become healthy and authenticated on $workerUrl (token fp=$tokenFp...)"
        }
        Set-WorkerPortProxy -VmIp $VmIp -WorkerPort $WorkerPort
        Write-Output (Write-JsonResult @{
            ok = $true
            vm_name = $VmName
            vm_state = 'Running'
            worker_url = $workerUrl
            backend = 'hyperv'
            vm_ip = $VmIp
            vm_gateway = $VmGateway
            cpu = $VmCpus
            memory_mb = $VmMemoryMb
        })
    }
}

exit 0
