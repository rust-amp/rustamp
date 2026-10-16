# Stage one capture job on the host (unelevated), run it in the guest, bring the
# captures back and verify them.
#
#   powershell -ExecutionPolicy Bypass -File docs\notes\vm-run-capture.ps1 -Job <name> [-Baseline base-flat-main] [-DisplayPin Set1024]
#
# The division of labour is deliberate and security-relevant, same as
# vm-run-tests.ps1:
#   * HERE (unelevated): stage the harness directory, the one job file and the
#     reference copy. These are copied blindly, never inspected.
#   * THE BROKER (elevated, session 0): restore the baseline checkpoint, start,
#     pin the virtual display (allow-listed `set-vdd-mode.ps1`), copy in over
#     PowerShell Direct, run the allow-listed guest script `run-capture.ps1`,
#     copy the results back out with the allow-listed `copyout` op.
#
# The display step is mandatory, not cosmetic: the checkpoints carry the
# virtual display driver at its 800x600 default, which extends the desktop to
# 1824x768, while the harness one-to-one gate expects 1024x768. The size step
# runs right after the restore (a restore brings the wrong size back) and
# verifies the size before anything is staged; the guest pre-gate inside
# run-capture.ps1 re-checks it right before the reference binary starts.
# -DisplayPin Off disables the virtual display instead of resizing it.
#
# The broker never builds, and this script never writes into the research
# tree: verified captures land in <staging>\out\<job>\ with hashes.json and
# run-provenance.json, and moving them into the research run directory is the
# orchestrator's deliberate step.

[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Job,
    [string]$Baseline = 'base-provisioned',
    [ValidateSet('Set1024', 'Off')][string]$DisplayPin = 'Set1024',
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
# Two traps live here, both learned the hard way. First, the broker runs as a
# CHILD powershell.exe, so its stdout is formatted TEXT, not objects: the
# snapshot names must be parsed out of the text (a line that starts with a
# non-space token), because $_.Name on a string is always empty. Second,
# PowerShell variables are case-INsensitive, so a local called $baseline would
# overwrite the $Baseline parameter; the locals below are named so they cannot
# collide with any parameter.
Step "reading checkpoint catalogue (baseline: $Baseline)"
$listText = & $broker -Op list -TimeoutSeconds 300 | Out-String
$knownNames = @(
    ($listText -split "`r?`n") |
        Where-Object { $_ -match '^\s*(\S+)\s' } |
        ForEach-Object { $Matches[1] } |
        Where-Object { $_ -ne 'Name' -and $_ -notmatch '^-+$' }
)
if ($knownNames -notcontains $Baseline) { throw "no checkpoint named '$Baseline' (broker knows: $($knownNames -join ', '))" }
# The creation time is provenance, not a gate: parse it from the same text row
# that carried the name, and record it verbatim rather than pretending the
# child process returned an object. If the row cannot be parsed, say so.
$baselineRow = @(($listText -split "`r?`n") | Where-Object { $_ -match ('^\s*' + [regex]::Escape($Baseline) + '\b') } | Select-Object -First 1)
$baselineCreated = 'unknown (unparsed list row)'
if ($baselineRow) {
    $rest = [regex]::Replace($baselineRow, ('^\s*' + [regex]::Escape($Baseline) + '\s+\S+\s+'), '')
    if ($rest.Trim()) { $baselineCreated = $rest.Trim() }
}

# --- 2. restore the baseline: this is what "freshly booted" now means ---------
Step "restoring $Baseline (discards everything since the checkpoint)"
& $broker -Op restore -Name $Baseline -TimeoutSeconds 900 | Out-Null
$restoreStamp = (Get-Date).ToString('o')

# --- 3. pin the virtual display, then verify it (before anything is staged) ----
# The restore above brought the checkpoint's display back with it - the
# virtual monitor at its 800x600 default, i.e. an 1824x768 desktop - so the
# size step runs on every run, not once. set-vdd-mode.ps1 applies the mode
# and refuses itself unless the screen settles at 1024x768; a refusal here
# aborts before staging, with the measured values in the broker error.
Step "pinning the virtual display to 1024x768 (mode: $DisplayPin)"
$display = & $broker -Op guest -Script 'set-vdd-mode.ps1' -GuestArgs $DisplayPin -TimeoutSeconds 600
$displaySetAt = (Get-Date).ToString('o')
Write-Host ("    screen {0} (logpixels {1})" -f $display.screen, $display.logPixels)
foreach ($s in @($display.screens)) { Write-Host ("    monitor {0}" -f $s) }

# --- 4. stage the harness, the job and the reference copy (blind copies) -------
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

# --- 5. copy in and run one job -------------------------------------------------
Step 'copying the staged capture tree into the guest'
& $broker -Op copyin -Source $Stage -Dest 'C:\rustamp\capture' -TimeoutSeconds 1800 | Out-Null

Step "running capture job '$Job' in the guest"
$result = & $broker -Op guest -Script 'run-capture.ps1' -GuestArgs $Job -TimeoutSeconds $TimeoutSeconds

# --- 6. copy out ----------------------------------------------------------------
Step 'copying the captures back to the host'
if (Test-Path $OutDir) { Remove-Item -Recurse -Force $OutDir }
$null = New-Item -ItemType Directory -Force -Path $OutDir
& $broker -Op copyout -Source ("C:\rustamp\out\" + $Job) -Dest $OutDir -TimeoutSeconds 1800 | Out-Null

# --- 7. verify: recompute every hash, compare against the guest manifest --------
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

# --- 8. provenance ---------------------------------------------------------------
# Guest scripts whose result the host parses emit exactly one object (their
# progress lives in guest-transcript.txt inside the outputs, not on stdout),
# so $result and $display below are single objects with readable fields.
$provenance = [ordered]@{
    job               = $Job
    baseline          = $Baseline
    baselineCreated   = $baselineCreated
    restoreStamp      = $restoreStamp
    displayPin        = $DisplayPin
    displaySetAt      = $displaySetAt
    displaySet        = [string]$display.screen
    displaySetScreens = @($display.screens)
    setLogPixels      = $display.logPixels
    guestBootTime     = [string]$result.bootTime
    guest             = [string]$result.host
    guestUser         = [string]$result.user
    displayFacts      = [string]$result.screen
    displayScreens    = @($result.screens)
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
