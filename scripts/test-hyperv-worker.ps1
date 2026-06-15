param(
    [string]$VmName = 'AIHomeServerWorker',
    [string]$RepoUrl = 'https://github.com/kab102395/aihomeserver.git',
    [string]$Branch = 'main',
    [string]$VmIp = '192.168.250.10',
    [string]$VmGateway = '192.168.250.1',
    [string]$SwitchName = 'AIHomeServerSwitch',
    [int]$VmCpus = 4,
    [int]$VmMemoryMb = 8192,
    [int]$WorkerPort = 3031,
    [string]$ImageVersion = '24.04',
    [string]$WorkspacePath = '/workspace',
    [switch]$Reset,
    [switch]$KeepRunning
)

$ErrorActionPreference = 'Stop'

function Write-Step {
    param([string]$Message)
    Write-Host "`n==> $Message"
}

function Assert-Admin {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($identity)
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw 'This test runner must be launched from an elevated PowerShell session.'
    }
}

function Invoke-WorkerScript {
    param(
        [Parameter(Mandatory = $true)]
        [string[]]$Arguments
    )

    $scriptPath = Join-Path $PSScriptRoot 'hyperv-worker.ps1'
    $output = & powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $scriptPath @Arguments 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw ($output | Out-String)
    }
    return $output
}

function Invoke-WorkerScriptTracked {
    param(
        [Parameter(Mandatory = $true)]
        [string[]]$Arguments,
        [Parameter(Mandatory = $true)]
        [string]$StepName
    )

    $scriptPath = Join-Path $PSScriptRoot 'hyperv-worker.ps1'
    $stdoutPath = [System.IO.Path]::GetTempFileName()
    $stderrPath = [System.IO.Path]::GetTempFileName()
    try {
        $argumentList = @(
            '-NoProfile',
            '-NonInteractive',
            '-ExecutionPolicy',
            'Bypass',
            '-File',
            $scriptPath
        ) + $Arguments
        $process = Start-Process powershell.exe -PassThru -WindowStyle Hidden -ArgumentList $argumentList -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath

        $lastLength = 0
        while (-not $process.HasExited) {
            Start-Sleep -Seconds 10
            $content = @()
            if (Test-Path $stdoutPath) {
                $content += Get-Content -LiteralPath $stdoutPath -ErrorAction SilentlyContinue
            }
            if (Test-Path $stderrPath) {
                $content += Get-Content -LiteralPath $stderrPath -ErrorAction SilentlyContinue
            }
            if ($content.Count -gt $lastLength) {
                $content[$lastLength..($content.Count - 1)] | ForEach-Object { Write-Host $_ }
                $lastLength = $content.Count
            }
            Write-Step "$StepName is still running..."
        }

        $stdout = if (Test-Path $stdoutPath) { Get-Content -LiteralPath $stdoutPath -Raw -ErrorAction SilentlyContinue } else { '' }
        $stderr = if (Test-Path $stderrPath) { Get-Content -LiteralPath $stderrPath -Raw -ErrorAction SilentlyContinue } else { '' }
        if ($process.ExitCode -ne 0) {
            throw (($stdout + "`n" + $stderr).Trim())
        }
        return $stdout
    } finally {
        Remove-Item -LiteralPath $stdoutPath, $stderrPath -Force -ErrorAction SilentlyContinue
    }
}

function Reset-WorkerState {
    param(
        [string]$VmName,
        [string]$SwitchName,
        [string]$VmGateway
    )

    Write-Step "Stopping any existing VM named $VmName"
    Invoke-WorkerScript -Arguments @('-Action', 'stop', '-VmName', $VmName) | Out-Host

    $root = 'C:\ProgramData\AIHomeServer\hyperv'
    Write-Step 'Removing disposable Hyper-V worker files while preserving the cached Ubuntu image'
    $seedIso = Join-Path $root 'seed\cloud-init-seed.iso'
    if (Test-Path $seedIso) {
        try {
            $deadline = (Get-Date).AddSeconds(60)
            while ((Get-Date) -lt $deadline) {
                try {
                    $stream = [System.IO.File]::Open($seedIso, 'Open', 'ReadWrite', 'None')
                    $stream.Close()
                    break
                } catch {
                    Start-Sleep -Seconds 2
                }
            }
        } catch {
            Write-Host "Warning: unable to confirm seed ISO unlock: $($_.Exception.Message)"
        }
    }
    foreach ($relative in @('vm', 'seed', 'repo', 'logs')) {
        Remove-Item -LiteralPath (Join-Path $root $relative) -Recurse -Force -ErrorAction SilentlyContinue
    }

    Write-Step 'Removing Hyper-V NAT and static host IP if present'
    try {
        Remove-NetNat -Name "$SwitchName-NAT" -Confirm:$false -ErrorAction SilentlyContinue | Out-Null
    } catch {
        Write-Host "Warning: unable to remove NAT cleanly: $($_.Exception.Message)"
    }

    try {
        $iface = "vEthernet ($SwitchName)"
        Get-NetIPAddress -InterfaceAlias $iface -ErrorAction SilentlyContinue |
            Where-Object { $_.IPAddress -eq $VmGateway } |
            Remove-NetIPAddress -Confirm:$false -ErrorAction SilentlyContinue | Out-Null
    } catch {
        Write-Host "Warning: unable to remove host IP cleanly: $($_.Exception.Message)"
    }
}

function Ensure-HealthyWorker {
    param(
        [string]$VmIp,
        [int]$WorkerPort
    )

    Write-Step "Checking TCP reachability on $VmIp`:$WorkerPort"
    $tcp = Test-NetConnection -ComputerName $VmIp -Port $WorkerPort
    if (-not $tcp.TcpTestSucceeded) {
        throw "TCP connect failed to $VmIp`:$WorkerPort"
    }

    Write-Step "Fetching worker health from http://$VmIp`:$WorkerPort/health"
    $health = Invoke-WebRequest -Uri "http://$VmIp`:$WorkerPort/health" -UseBasicParsing -TimeoutSec 15
    if ($health.StatusCode -lt 200 -or $health.StatusCode -ge 300) {
        throw "Health endpoint returned HTTP $($health.StatusCode)"
    }

    Write-Host $health.Content
}

function Write-Diagnostics {
    param(
        [string]$VmName,
        [string]$VmIp,
        [int]$WorkerPort
    )

    Write-Step 'Collecting VM diagnostics'
    try {
        $status = Invoke-WorkerScript -Arguments @(
            '-Action', 'status',
            '-VmName', $VmName,
            '-VmIp', $VmIp,
            '-WorkerPort', $WorkerPort.ToString()
        )
        Write-Host $status
    } catch {
        Write-Host "Status lookup failed: $($_.Exception.Message)"
    }

    try {
        $vm = Get-VM -Name $VmName -ErrorAction SilentlyContinue
        if ($vm) {
            Write-Host ("VM state: {0}" -f $vm.State)
        }
        $adapter = Get-VMNetworkAdapter -VMName $VmName -ErrorAction SilentlyContinue
        if ($adapter) {
            Write-Host ("Adapter IPs: {0}" -f (($adapter | Select-Object -ExpandProperty IPAddresses) -join ', '))
            Write-Host ("Adapter MAC: {0}" -f ($adapter | Select-Object -ExpandProperty MacAddress -First 1))
        }
    } catch {
        Write-Host "Hyper-V diagnostics failed: $($_.Exception.Message)"
    }

    $seedDir = 'C:\ProgramData\AIHomeServer\hyperv\seed'
    if (Test-Path $seedDir) {
        Write-Host 'Seed files:'
        Get-ChildItem -LiteralPath $seedDir -Force | Select-Object Name,Length,LastWriteTime | Format-Table -AutoSize | Out-String | Write-Host
    }
}

Assert-Admin

if ($Reset) {
    Reset-WorkerState -VmName $VmName -SwitchName $SwitchName -VmGateway $VmGateway
}

$workerToken = ([guid]::NewGuid().ToString('N') + [guid]::NewGuid().ToString('N'))

Write-Step 'Bootstrapping worker VM'
    try {
        $bootstrapOutput = Invoke-WorkerScriptTracked -StepName 'Bootstrapping worker VM' -Arguments @(
        '-Action', 'bootstrap',
        '-VmName', $VmName,
    '-RepoUrl', $RepoUrl,
    '-Branch', $Branch,
    '-VmIp', $VmIp,
    '-VmGateway', $VmGateway,
    '-SwitchName', $SwitchName,
    '-VmCpus', $VmCpus.ToString(),
    '-VmMemoryMb', $VmMemoryMb.ToString(),
    '-WorkerPort', $WorkerPort.ToString(),
    '-WorkerToken', $workerToken,
    '-ImageVersion', $ImageVersion,
    '-WorkspacePath', $WorkspacePath
)
        Write-Host $bootstrapOutput
    } catch {
        Write-Host $_.Exception.Message
        Write-Diagnostics -VmName $VmName -VmIp $VmIp -WorkerPort $WorkerPort
        throw
    }

Write-Step 'Reading worker status'
$statusOutput = Invoke-WorkerScript -Arguments @(
    '-Action', 'status',
    '-VmName', $VmName,
    '-VmIp', $VmIp,
    '-WorkerPort', $WorkerPort.ToString()
)
Write-Host $statusOutput

Ensure-HealthyWorker -VmIp $VmIp -WorkerPort $WorkerPort

if (-not $KeepRunning) {
    Write-Step 'Stopping worker VM'
    Invoke-WorkerScript -Arguments @('-Action', 'stop', '-VmName', $VmName) | Out-Host
}

Write-Step 'End-to-end test complete'
