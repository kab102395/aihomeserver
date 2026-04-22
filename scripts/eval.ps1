$ErrorActionPreference = "Stop"

param(
  [ValidateSet("quick","full")]
  [string]$Mode = "quick",
  [string]$BaseUrl = "http://localhost:3000"
)

$body = @{
  mode = $Mode
  timeout_secs = if ($Mode -eq "full") { 30 } else { 15 }
} | ConvertTo-Json

Write-Host "POST $BaseUrl/eval/run ($Mode)"
$res = Invoke-RestMethod -Method Post -Uri "$BaseUrl/eval/run" -ContentType "application/json" -Body $body

Write-Host ("ok={0} duration_ms={1} passed={2} failed={3} skipped={4}" -f `
  $res.ok, $res.duration_ms, $res.summary.passed, $res.summary.failed, $res.summary.skipped)

foreach ($r in $res.results) {
  $mark = if ($r.skipped) { "SKIP" } elseif ($r.ok) { "PASS" } else { "FAIL" }
  Write-Host ("[{0}] {1} ({2}ms)" -f $mark, $r.id, $r.duration_ms)
}

if (-not $res.ok) { exit 1 }

