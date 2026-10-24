# Read-only inspection of the capture output directory and the session state.
#
# Runs INSIDE the guest (allow-listed plain name). Changes nothing.

$ErrorActionPreference = 'Continue'

Write-Output '=== session ==='
$p = Get-Process -Name explorer -ErrorAction SilentlyContinue | Select-Object -First 1
if ($p) { Write-Output ("  explorer session {0} as {1}" -f $p.SessionId, $env:USERNAME) }
else { Write-Output '  no explorer process' }

Write-Output '=== C:\rustamp\out tree ==='
Get-ChildItem 'C:\rustamp\out' -Recurse -ErrorAction SilentlyContinue |
    ForEach-Object { Write-Output ("  {0} {1}" -f $_.Length, $_.FullName) }

Write-Output '=== DONE.json if present ==='
$d = Get-ChildItem 'C:\rustamp\out' -Recurse -Filter 'DONE.json' -ErrorAction SilentlyContinue | Select-Object -First 1
if ($d) {
    $j = Get-Content $d.FullName -Raw | ConvertFrom-Json
    Write-Output ("  status={0} exit={1} steps={2}" -f $j.status, $j.exit_code, $j.step_results.Count)
} else { Write-Output '  (no DONE.json)' }
Write-Output '=== done ==='
