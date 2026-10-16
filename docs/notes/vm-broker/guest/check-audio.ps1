# Read-only check: which audio endpoints exist in the guest, and is a
# VB-CABLE setup still running?
#
# Runs INSIDE the guest (allow-listed plain name). Changes nothing.

$ErrorActionPreference = 'Continue'

Write-Output '=== audio endpoints ==='
Get-PnpDevice -Class AudioEndpoint -ErrorAction SilentlyContinue |
    ForEach-Object { Write-Output ("  {0} | {1}" -f $_.Status, $_.FriendlyName) }

Write-Output '=== VB-CABLE driver in store ==='
$p = (& pnputil /enum-drivers 2>&1) -join "`n"
$found = $false
foreach ($block in ($p -split "`r?`n`r?`n")) {
    if ($block -match 'vbMmeCable|VB-Audio|VBCABLE') { Write-Output ('  ' + ($block -replace "`r?`n", ' | ')); $found = $true }
}
if (-not $found) { Write-Output '  (no VB-CABLE driver in the store)' }

Write-Output '=== setup processes still alive ==='
Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.ProcessName -match 'VBCABLE|setup' } |
    ForEach-Object { Write-Output ("  {0} (id {1}, started {2})" -f $_.ProcessName, $_.Id, $_.StartTime) }
Write-Output '=== done ==='
