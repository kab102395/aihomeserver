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
    [switch]$KeepRunning,

    # Token source (pick exactly one; default is standalone — generate fresh token):
    # -WorkerToken <value>   : use the supplied literal token
    # -TokenFile <path>      : read token from the given file (e.g. launcher's worker-token.txt)
    # -UseLauncherToken      : read from the default Electron app-data path
    [string]$WorkerToken = '',
    [string]$TokenFile = '',
    [switch]$UseLauncherToken
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
        $process = Start-Process powershell.exe -PassThru -WindowStyle Hidden -ArgumentList $argumentList -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath -Wait

        $stdout = if (Test-Path $stdoutPath) { Get-Content -LiteralPath $stdoutPath -Raw -ErrorAction SilentlyContinue } else { '' }
        $stderr = if (Test-Path $stderrPath) { Get-Content -LiteralPath $stderrPath -Raw -ErrorAction SilentlyContinue } else { '' }

        $lastJsonLine = ($stdout -split "`n" |
            Where-Object { $_.Trim().StartsWith('{') } |
            Select-Object -Last 1)
        $succeededByContract = $false
        if ($lastJsonLine) {
            try {
                $parsed = $lastJsonLine | ConvertFrom-Json
                $succeededByContract = ($parsed.ok -eq $true)
            } catch {}
        }

        if (-not $succeededByContract -and $process.ExitCode -ne 0) {
            throw (($stdout + "`n" + $stderr).Trim())
        }

        return (($stdout + "`n" + $stderr).Trim())
    } finally {
        Remove-Item -LiteralPath $stdoutPath, $stderrPath -Force -ErrorAction SilentlyContinue
    }
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

        $process.WaitForExit()

        $stdout = if (Test-Path $stdoutPath) { Get-Content -LiteralPath $stdoutPath -Raw -ErrorAction SilentlyContinue } else { '' }
        $stderr = if (Test-Path $stderrPath) { Get-Content -LiteralPath $stderrPath -Raw -ErrorAction SilentlyContinue } else { '' }

        # The contract signal is the last JSON line in stdout containing "ok":true.
        # Treat that as success regardless of exit code: PowerShell propagates
        # $LASTEXITCODE from native commands (e.g. wsl --unmount in a finally block)
        # as the process exit code even when the script itself ran to completion.
        $lastJsonLine = ($stdout -split "`n" |
            Where-Object { $_.Trim().StartsWith('{') } |
            Select-Object -Last 1)
        $succeededByContract = $false
        if ($lastJsonLine) {
            try {
                $parsed = $lastJsonLine | ConvertFrom-Json
                $succeededByContract = ($parsed.ok -eq $true)
            } catch {}
        }

        if (-not $succeededByContract -and $process.ExitCode -ne 0) {
            throw "Exit $($process.ExitCode)`n$(($stdout + "`n" + $stderr).Trim())"
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

function Invoke-WorkerPost {
    param(
        [string]$BaseUrl,
        [string]$Path,
        [hashtable]$Body,
        [string]$Token
    )

    $headers = @{ 'Content-Type' = 'application/json' }
    if ($Token) {
        $headers['Authorization'] = "Bearer $Token"
    }
    $json = $Body | ConvertTo-Json -Depth 8 -Compress
    $resp = Invoke-WebRequest -Uri "$BaseUrl$Path" -Method Post -Headers $headers -Body $json -UseBasicParsing -TimeoutSec 30
    if ($resp.StatusCode -lt 200 -or $resp.StatusCode -ge 300) {
        throw "$Path returned HTTP $($resp.StatusCode)"
    }
    return $resp.Content | ConvertFrom-Json
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

function Test-ShellRoundTrip {
    param(
        [string]$VmIp,
        [int]$WorkerPort,
        [string]$Token
    )

    $baseUrl = "http://$VmIp`:$WorkerPort"
    $sentinel = "aihomeserver-test-$([guid]::NewGuid().ToString('N').Substring(0, 8))"

    Write-Step "Shell round-trip: echo sentinel value"
    $result = Invoke-WorkerPost -BaseUrl $baseUrl -Path '/shell' -Token $Token -Body @{
        command      = "echo $sentinel"
        cwd          = '.'
        timeout_secs = 15
    }

    if (-not $result.success) {
        throw "Shell command failed: error_type=$($result.error_type) trace=$($result.trace)"
    }
    $stdout = $result.output.stdout.Trim()
    if ($stdout -ne $sentinel) {
        throw "Shell stdout mismatch: expected '$sentinel', got '$stdout'"
    }
    Write-Host "  stdout: $stdout  [ok]"

    Write-Step "Shell round-trip: exit code propagation"
    $failResult = Invoke-WorkerPost -BaseUrl $baseUrl -Path '/shell' -Token $Token -Body @{
        command      = 'exit 42'
        cwd          = '.'
        timeout_secs = 10
    }
    if ($failResult.success) {
        throw "Expected success=false for exit 42, got success=true"
    }
    if ($failResult.error_code -ne 'exit_42') {
        throw "Expected error_code=exit_42, got $($failResult.error_code)"
    }
    Write-Host "  exit_code propagation: exit_42  [ok]"
}

function Test-WorkspaceSync {
    param(
        [string]$VmIp,
        [int]$WorkerPort,
        [string]$Token
    )

    $baseUrl = "http://$VmIp`:$WorkerPort"
    $testContent = "hello from host $(Get-Date -Format 'o')"
    $contentsB64 = [Convert]::ToBase64String([System.Text.Encoding]::UTF8.GetBytes($testContent))

    Write-Step "Workspace sync: push a file to /workspace"
    $syncResult = Invoke-WorkerPost -BaseUrl $baseUrl -Path '/workspace/sync' -Token $Token -Body @{
        files = @(
            @{ path = 'e2e-test/probe.txt'; contents_b64 = $contentsB64 }
        )
    }
    if (-not $syncResult.ok) {
        throw "workspace/sync returned ok=false"
    }
    if ($syncResult.files_written -ne 1) {
        throw "Expected files_written=1, got $($syncResult.files_written)"
    }
    Write-Host "  files_written: $($syncResult.files_written)  [ok]"

    Write-Step "Workspace sync: verify file content via shell"
    $readResult = Invoke-WorkerPost -BaseUrl $baseUrl -Path '/shell' -Token $Token -Body @{
        command      = 'cat e2e-test/probe.txt'
        cwd          = '.'
        timeout_secs = 10
    }
    if (-not $readResult.success) {
        throw "cat probe.txt failed: $($readResult.trace)"
    }
    $actual = $readResult.output.stdout.Trim()
    if ($actual -ne $testContent) {
        throw "probe.txt content mismatch: expected '$testContent', got '$actual'"
    }
    Write-Host "  file content verified  [ok]"

    Write-Step "Workspace sync: collect artifact back via collect_paths"
    $collectResult = Invoke-WorkerPost -BaseUrl $baseUrl -Path '/shell' -Token $Token -Body @{
        command       = 'echo done'
        cwd           = '.'
        timeout_secs  = 10
        collect_paths = @('e2e-test/probe.txt')
    }
    if (-not $collectResult.success) {
        throw "collect_paths shell call failed: $($collectResult.trace)"
    }
    $artifacts = $collectResult.output.workspace.collected_artifacts
    if (-not $artifacts -or $artifacts.Count -eq 0) {
        throw "No artifacts returned in collect_paths response"
    }
    $artifact = $artifacts | Where-Object { $_.path -eq 'e2e-test/probe.txt' } | Select-Object -First 1
    if (-not $artifact) {
        throw "e2e-test/probe.txt not found in collected_artifacts"
    }
    $decoded = [System.Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($artifact.contents_b64))
    if ($decoded -ne $testContent) {
        throw "Artifact content mismatch: expected '$testContent', got '$decoded'"
    }
    Write-Host "  artifact round-trip verified  [ok]"
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

# ── Token resolution ──────────────────────────────────────────────────────────
# Standalone mode (default): generate a fresh token and fully own the bootstrap.
# Attached modes: read the persisted launcher token so auth matches the live app.
$resolvedToken = ''
if ($WorkerToken) {
    $resolvedToken = $WorkerToken.Trim()
    Write-Step "Token: supplied via -WorkerToken (fp=$($resolvedToken.Substring(0, [Math]::Min(8, $resolvedToken.Length)))...)"
} elseif ($TokenFile) {
    if (-not (Test-Path $TokenFile)) {
        throw "Token file not found: $TokenFile"
    }
    $resolvedToken = (Get-Content -LiteralPath $TokenFile -Raw).Trim()
    Write-Step "Token: loaded from $TokenFile (fp=$($resolvedToken.Substring(0, [Math]::Min(8, $resolvedToken.Length)))...)"
} elseif ($UseLauncherToken) {
    $launcherTokenPath = Join-Path $env:APPDATA 'aihomeserver\worker-token.txt'
    if (-not (Test-Path $launcherTokenPath)) {
        throw "Launcher token file not found: $launcherTokenPath (launch the app at least once to create it)"
    }
    $resolvedToken = (Get-Content -LiteralPath $launcherTokenPath -Raw).Trim()
    Write-Step "Token: loaded from launcher app-data (fp=$($resolvedToken.Substring(0, [Math]::Min(8, $resolvedToken.Length)))...)"
} else {
    $resolvedToken = ([guid]::NewGuid().ToString('N') + [guid]::NewGuid().ToString('N'))
    Write-Step "Token: generated fresh for standalone run (fp=$($resolvedToken.Substring(0, [Math]::Min(8, $resolvedToken.Length)))...)"
}

if (-not $resolvedToken) {
    throw 'Worker token is empty; cannot run authenticated tests'
}
$workerToken = $resolvedToken

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

Write-Step 'Verifying authenticated access (auth probe)'
$authProbeResult = Invoke-WorkerPost -BaseUrl "http://$VmIp`:$WorkerPort" -Path '/shell' -Token $workerToken -Body @{
    command      = 'echo aihomeserver-auth-probe'
    cwd          = '.'
    timeout_secs = 10
}
if (-not $authProbeResult.success) {
    $errorType = $authProbeResult.error_type
    $errorMsg  = $authProbeResult.trace
    throw "Auth probe FAILED (error_type=$errorType): $errorMsg`nToken fp=$($workerToken.Substring(0, [Math]::Min(8, $workerToken.Length)))..."
}
Write-Host "  Auth probe: success  [ok]"

Test-ShellRoundTrip -VmIp $VmIp -WorkerPort $WorkerPort -Token $workerToken

Test-WorkspaceSync -VmIp $VmIp -WorkerPort $WorkerPort -Token $workerToken

if (-not $KeepRunning) {
    Write-Step 'Stopping worker VM'
    Invoke-WorkerScript -Arguments @('-Action', 'stop', '-VmName', $VmName) | Out-Host
}

Write-Step 'End-to-end test complete'
