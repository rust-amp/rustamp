# Compatibility entry point for ignored window tests. The full suite runner now
# uses the guest INTERACTIVE session by default (owner rule, 2026-10-18): there
# is no second staging/reporting implementation and no session-0 GUI fallback.
#
#   powershell -ExecutionPolicy Bypass -File docs\notes\vm-run-gui-session.ps1
#   powershell ... -Filter eq_window -Reset
#
# No filter runs the general ignored pass, including all five winit tests (not
# only the old four-case list). Group E keeps its explicit filtered/reset runs.
# See vm-run-tests.ps1 for the full suite and the narrow desktop-free exception.

[CmdletBinding()]
param(
    [string]$Filter = '',
    [switch]$Reset,
    [string]$Baseline = 'base-provisioned',
    [string]$RepoRoot = 'D:\code\Rustamp',
    [string]$Stage = 'D:\code\rustamp-vm\staging\code\Rustamp',
    [string]$StageRoot = 'D:\code\rustamp-vm\staging',
    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'
$suiteFilter = '--ignored'
if ($Filter) { $suiteFilter += ':' + $Filter }
& (Join-Path $PSScriptRoot 'vm-run-tests.ps1') -Filter $suiteFilter -Reset:$Reset `
    -Baseline $Baseline -RepoRoot $RepoRoot -Stage $Stage -SkipBuild:$SkipBuild `
    -SessionOut (Join-Path $StageRoot 'out\gui-session')
exit $LASTEXITCODE
