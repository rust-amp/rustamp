# Stage one capture job on the host (unelevated), run it in the guest, bring the
# captures back and verify them.
#
#   powershell -ExecutionPolicy Bypass -File docs\notes\vm-run-capture.ps1 -Job <name> [-Baseline base-flat-main]
#
# The division of labour is deliberate and security-relevant, same as
# vm-run-tests.ps1:
#   * HERE (unelevated): stage the harness directory, the one job file and the
#     reference copy. These are copied blindly, never inspected.
#   * THE BROKER (elevated, session 0): restore the baseline checkpoint, start,
#     copy in over PowerShell Direct, run the allow-listed guest script
#     `run-capture.ps1`, copy the results back out with the allow-listed
#     `copyout` op.
#
# The broker never builds, and this script never writes into the research
# tree: verified captures land in <staging>\out\<job>\ with hashes.json and
# run-provenance.json, and moving them into the research run directory is the
# orchestrator's deliberate step.

[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Job,
    [string]$Baseline = 'base-provisioned',
    [string]$HarnessDir = 'D:\code\rustamp-research\tools\vm-capture',
    [string]$RefDir = 'D:\code\rustamp-sandbox\winamp295',
    [string]$StageRoot = 'D:\code\rustamp-vm\staging',
    [int]$TimeoutSeconds = 3600
)

$ErrorActionPreference = 'Stop'
if ($Job -match '[\\/]' -or $Job -notmatch '^[A-Za-z0-9._-]+$') { throw "job name '$Job' is not allowed (plain name, no path)" }
$broker = Join-Path $PSScriptRoot 'vm-broker\invoke-broker.ps1'

function Step($t) { Write-Host "--- $t" }

$Stage = Join-Path $StageRoot 'capture'
$OutDir = Join-Path $StageRoot ("out\" + $Job)

# --- 1. baseline provenance (recorded before the restore) ---------------------
Step "reading checkpoint catalogue (baseline: $Baseline)"
$snaps = & $broker -Op list -TimeoutSeconds 300
$baseline = @($snaps | Where-Object { $_.Name -eq $Baseline })
if (-not $baseline) { throw "no checkpoint named '$Baseline' (broker knows: $(($snaps | Select-Object -ExpandProperty Name) -join ', '))" }
$baselineCreated = [string]$baseline[0].CreationTime

# --- 2. restore the baseline: this is what "freshly booted" now means ---------
Step "restoring $Baseline (discards everything since the checkpoint)"
& $broker -Op restore -Name $Baseline -TimeoutSeconds 900 | Out-Null
$restoreStamp = (Get-Date).ToString('o')

# --- 3. stage the harness, the job and the reference copy (blind copies) -------
Step "staging into $Stage"
if (-not (Test-Path $HarnessDir)) { throw "harness directory not found: $HarnessDir" }
$jobFile = Join-Path $HarnessDir ("jobs\" + $Job + ".json")
if (-not (Test-Path $jobFile)) { throw "job file not found: $jobFile" }
if (-not (Test-Path $RefDir)) { throw "reference copy not found: $RefDir (running the reference in the guest needs the owner's confirmation)" }
if (Test-Path $Stage) { Remove-Item -Recurse -Force $Stage }
$null = New-Item -ItemType Directory -Force -Path (Join-Path $Stage 'tools\vm-capture')
Copy-Item (Join-Path $HarnessDir '*') (Join-Path $Stage 'tools\vm-capture') -Recurse -Force
$null = New-Item -ItemType Directory -Force -Path (Join-Path $Stage 'winamp295')
Copy-Item (Join-Path $RefDir '*') (Join-Path $Stage 'winamp295') -Recurse -Force

# --- 4. copy in and run one job -------------------------------------------------
Step 'copying the staged capture tree into the guest'
& $broker -Op copyin -Source $Stage -Dest 'C:\rustamp\capture' -TimeoutSeconds 1800 | Out-Null

Step "running capture job '$Job' in the guest"
$result = & $broker -Op guest -Script 'run-capture.ps1' -GuestArgs $Job -TimeoutSeconds $TimeoutSeconds

# --- 5. copy out ----------------------------------------------------------------
Step 'copying the captures back to the host'
if (Test-Path $OutDir) { Remove-Item -Recurse -Force $OutDir }
$null = New-Item -ItemType Directory -Force -Path $OutDir
& $broker -Op copyout -Source ("C:\rustamp\out\" + $Job) -Dest $OutDir -TimeoutSeconds 1800 | Out-Null

# --- 6. verify: recompute every hash, compare against the guest manifest --------
Step 'verifying hashes (byte-identical or refused)'
$manifestPath = Join-Path $OutDir 'hashes.json'
if (-not (Test-Path $manifestPath)) { throw 'no hashes.json came back: the guest run did not finish its manifest step; nothing is promoted' }
$manifest = Get-Content -Path $manifestPath -Raw | ConvertFrom-Json
$checked = 0
foreach ($entry in $manifest.PSObject.Properties) {
    $p = Join-Path $OutDir $entry.Name
    if (-not (Test-Path $p)) { throw ("manifest lists '{0}' but it did not come back" -f $entry.Name) }
    $actual = (Get-FileHash -Path $p -Algorithm SHA256).Hash
    if ($actual -ne [string]$entry.Value) { throw ("hash mismatch on '{0}': guest {1}, host {2}" -f $entry.Name, $entry.Value, $actual) }
    $checked++
}
if ($checked -eq 0) { throw 'manifest is empty: a capture job with no outputs is a failed job, not an empty success' }

# --- 7. provenance ---------------------------------------------------------------
$provenance = [ordered]@{
    job               = $Job
    baseline          = $Baseline
    baselineCreated   = $baselineCreated
    restoreStamp      = $restoreStamp
    guestBootTime     = [string]$result.bootTime
    guest             = [string]$result.host
    guestUser         = [string]$result.user
    displayFacts      = [string]$result.screen
    logPixels         = $result.logPixels
    harnessExit       = $result.harnessExit
    files             = $checked
}
$provenance | ConvertTo-Json | Set-Content -Path (Join-Path $OutDir 'run-provenance.json') -Encoding UTF8

# The restore-then-boot proof: the guest must have booted after we restored.
$booted = [datetime]$result.bootTime
$restored = [datetime]$restoreStamp
if ($booted -lt $restored.AddMinutes(-2)) {
    throw ("provenance refused: guest boot time {0} predates the restore {1} - this run did not start from '{2}'" -f $result.bootTime, $restoreStamp, $Baseline)
}

Write-Host ''
Write-Host ("job {0}: {1} files verified byte-identical in {2}" -f $Job, $checked, $OutDir)
Write-Host ("baseline '{0}' (created {1}), restored {2}, guest booted {3}" -f $Baseline, $baselineCreated, $restoreStamp, $result.bootTime)
