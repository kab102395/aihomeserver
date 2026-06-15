param(
    [ValidateSet('bootstrap', 'start', 'stop', 'status')]
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
    if (-not $vm) {
        return
    }

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

function Get-CloudInitUserData {
    param(
        [string]$RepoUrl,
        [string]$Branch,
        [string]$WorkerToken,
        [string]$WorkspacePath,
        [int]$WorkerPort,
        [string]$VmMac
    )

    $template = @'
#cloud-config
package_update: false
package_upgrade: false
users:
  - default
  - name: aihome
    shell: /bin/bash
    groups: [adm, sudo]
    sudo: ALL=(ALL) NOPASSWD:ALL
    lock_passwd: false
    ssh_authorized_keys:
      - ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBdQ4ptyx1DiHKIegOJjXtJLlxDOUSLHEyWWa6ptO9Ke kab10@Kyle
write_files:
  - path: /usr/local/bin/aihomeserver-network-setup
    permissions: '0755'
    content: |
      #!/usr/bin/env bash
      set -euo pipefail

      target_mac="__VM_MAC_COLON__"
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
      ip addr add "$target_ip/24" dev "$iface"
      ip route replace default via "$target_gateway" dev "$iface"
      printf 'nameserver 1.1.1.1\nnameserver 8.8.8.8\n' > /etc/resolv.conf
  - path: /etc/netplan/99-aihomeserver.yaml
    permissions: '0644'
    content: |
      network:
        version: 2
        renderer: networkd
        ethernets:
          aihome0:
            match:
              macaddress: __VM_MAC_COLON__
            set-name: eth0
            dhcp4: false
            addresses:
              - __VM_IP__/24
            routes:
              - to: default
                via: __VM_GATEWAY__
            nameservers:
              addresses:
                - 1.1.1.1
                - 8.8.8.8
  - path: /usr/local/bin/aihomeserver-worker.py
    permissions: '0755'
    content: |
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

      def require_auth(handler):
          if not TOKEN:
              return True
          return handler.headers.get("Authorization", "") == f"Bearer {TOKEN}"

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
          pathlib.Path(WORKSPACE).mkdir(parents=True, exist_ok=True)
          httpd = ThreadingHTTPServer(("0.0.0.0", PORT), Handler)
          httpd.serve_forever()

      if __name__ == "__main__":
          main()
  - path: /etc/systemd/system/aihomeserver-worker.service
    permissions: '0644'
    content: |
      [Unit]
      Description=AI Home Server Worker
      After=network.target

      [Service]
      Type=simple
      User=aihome
      WorkingDirectory=/opt/aihomeserver
      Environment=WORKER_WORKSPACE=__WORKSPACE__
      Environment=WORKER_PORT=__PORT__
      Environment=WORKER_TOKEN=__TOKEN__
      ExecStartPre=/usr/local/bin/aihomeserver-network-setup
      ExecStart=/usr/bin/python3 /usr/local/bin/aihomeserver-worker.py
      Restart=always
      RestartSec=3

      [Install]
      WantedBy=multi-user.target
bootcmd:
  - [ bash, -lc, "netplan generate && netplan apply || true" ]
runcmd:
  - [ bash, -lc, "/usr/local/bin/aihomeserver-network-setup || true" ]
  - [ bash, -lc, "mkdir -p /workspace /opt/aihomeserver /var/lib/aihomeserver" ]
  - [ bash, -lc, "chown -R aihome:aihome /workspace /opt/aihomeserver /var/lib/aihomeserver" ]
  - [ bash, -lc, "systemctl daemon-reload && systemctl enable --now aihomeserver-worker.service" ]
'@

    return $template.
        Replace('__REPO__', $RepoUrl).
        Replace('__BRANCH__', $Branch).
        Replace('__TOKEN__', $WorkerToken).
        Replace('__WORKSPACE__', $WorkspacePath).
        Replace('__PORT__', $WorkerPort.ToString()).
        Replace('__VM_IP__', $VmIp).
        Replace('__VM_GATEWAY__', $VmGateway).
        Replace('__VM_MAC__', $VmMac).
        Replace('__VM_MAC_COLON__', (Format-MacAddress -MacAddress $VmMac).ToLowerInvariant())
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

    $tarPath = Join-Path $ImageDir "ubuntu-$Version-server-cloudimg-amd64-azure.vhd.tar.gz"
    $imageExtractDir = Join-Path $ImageDir "ubuntu-$Version"
    Ensure-Directory $imageExtractDir

    $vhdxPath = Join-Path $imageExtractDir "ubuntu-$Version-server-cloudimg-amd64.vhdx"

    if (-not (Test-Path $vhdxPath)) {
        Invoke-Download -Url (Get-ImageUrl -Version $Version) -OutFile $tarPath
        Write-Log "Extracting Ubuntu cloud image"
        tar -xzf $tarPath -C $imageExtractDir

        $sourceVhd = Get-ChildItem -Path $imageExtractDir -Recurse -Filter *.vhd | Select-Object -First 1
        if (-not $sourceVhd) {
            throw "Ubuntu cloud image was downloaded but no .vhd was found in $imageExtractDir"
        }

        try {
            fsutil sparse setflag $sourceVhd.FullName 0 | Out-Null
        } catch {
            Write-Log "Warning: unable to clear sparse flag on $($sourceVhd.FullName); continuing with conversion attempt"
        }

        Write-Log "Converting Ubuntu cloud image to VHDX"
        Convert-VHD -Path $sourceVhd.FullName -DestinationPath $vhdxPath -VHDType Dynamic | Out-Null
    }

    if (-not (Test-Path $vhdxPath)) {
        throw "Ubuntu cloud image conversion failed; expected VHDX at $vhdxPath"
    }
    return $vhdxPath
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
        Set-VMHardDiskDrive -VMName $VmName -ControllerType SCSI -ControllerNumber 0 -ControllerLocation 0 -Path $VmDisk | Out-Null        # Fix cloud-init datasource in the avhdx before boot
        $avhdx = (Get-VMHardDiskDrive -VMName $VmName).Path
        wsl --mount --vhd $avhdx --bare | Out-Null
        Start-Sleep -Seconds 2
        wsl -u root sh -c "mkdir -p /mnt3 && mount /dev/sdd1 /mnt3 2>/dev/null || mount /dev/sdc1 /mnt3 2>/dev/null && mkdir -p /mnt3/etc/cloud/cloud.cfg.d && echo 'datasource_list: [NoCloud, None]' > /mnt3/etc/cloud/cloud.cfg.d/99-nocloud.cfg && umount /mnt3" | Out-Null
        wsl --unmount $avhdx | Out-Null
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
        [int]$Port
    )

    $deadline = (Get-Date).AddMinutes(30)
    while ((Get-Date) -lt $deadline) {
        try {
            if (Test-NetConnection -ComputerName $VmIp -Port $Port -InformationLevel Quiet) {
                $resp = Invoke-WebRequest -Uri "http://$VmIp`:$Port/health" -TimeoutSec 10 -UseBasicParsing
                if ($resp.StatusCode -ge 200 -and $resp.StatusCode -lt 300) {
                    return $true
                }
            }
        } catch {
            Start-Sleep -Seconds 15
            continue
        }
        Start-Sleep -Seconds 15
    }
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

Assert-Admin
Assert-HyperVAvailable

$paths = Get-Paths
$workerUrl = "http://$VmIp`:$WorkerPort"

switch ($Action) {
    'status' {
        Write-Output (Write-JsonResult (Get-Status -VmName $VmName -VmIp $VmIp -WorkerPort $WorkerPort))
    }
    'stop' {
        Remove-VmArtifacts -VmName $VmName
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
        Write-Output (Write-JsonResult (Get-Status -VmName $VmName -VmIp $VmIp -WorkerPort $WorkerPort))
    }
    'bootstrap' {
        Remove-VmArtifacts -VmName $VmName
        Ensure-VMSwitchAndNat -SwitchName $SwitchName -VmGateway $VmGateway
        $imageVhd = Ensure-UbuntuImage -ImageDir $paths.ImageDir -Version $ImageVersion
        $seedStamp = [DateTime]::UtcNow.ToString('yyyyMMddHHmmssfff')
        $seedPath = Join-Path $paths.SeedDir ("cloud-init-seed-$seedStamp.iso")
        $instanceId = "aihomeserver-$VmName-$seedStamp"
        $vmMac = Get-StaticMacAddress -VmName $VmName
        $userData = Get-CloudInitUserData -RepoUrl $RepoUrl -Branch $Branch -WorkerToken $WorkerToken -WorkspacePath $WorkspacePath -WorkerPort $WorkerPort -VmMac $vmMac
        Ensure-SeedIso -SeedPath $seedPath -UserData $userData -VmIp $VmIp -VmGateway $VmGateway -VmMac $vmMac -InstanceId $instanceId
        Ensure-VM -VmName $VmName -VmDisk $imageVhd -SeedIso $seedPath -SwitchName $SwitchName -VmCpus $VmCpus -VmMemoryMb $VmMemoryMb
        Start-WorkerVM -VmName $VmName
        $ready = Wait-WorkerHealth -VmIp $VmIp -Port $WorkerPort
        if (-not $ready) {
            throw "Worker VM did not report healthy on $workerUrl"
        }
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












