$ErrorActionPreference = "Stop"

param(
  [string]$BaseUrl = "http://localhost:3000"
)

Invoke-RestMethod -Method Get -Uri "$BaseUrl/metrics" | ConvertTo-Json -Depth 6

