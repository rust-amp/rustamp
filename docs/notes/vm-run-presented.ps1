# Stage one presented capture job on the host (unelevated), run it in the guest,
# bring the captures back and verify them.
#
#   powershell -ExecutionPolicy Bypass -File docs\notes\vm-run-presented.ps1 -Job <name> [-Baseline base-provisioned] [-DisplayPin Set1024]
#
# The division of labour is deliberate and security-relevant, same as
# vm-run-capture.ps1:
#   * HERE (unelevated): stage the harness directory, the one job file, our
#     built tree and the reference skin. These are copied blindly, never inspected.
#   * THE BROKER (elevated, session 0): restore the baseline checkpoint, start,
#     pin the virtual display (allow-listed `set-vdd-mode.ps1`), copy in over
#     PowerShell Direct, run the allow-listed guest script `run-presented.ps1`,
#     copy the results back out with the allow-listed `copyout` op.
#
# The display step is mandatory, not cosmetic: the checkpoints carry the
# virtual display driver at its 800x600 default, which extends the desktop to
# 1824x768, while the harness one-to-one gate expects 1024x768. The size step
# runs right after the restore (a restore brings the wrong size back) and
# verifies the size before anything is staged; the guest pre-gate inside
# run-presented.ps1 re-checks it right before our player starts.
# -DisplayPin Off disables the virtual display instead of resizing it.
#
# The broker never builds, and this script never writes into the research
# tree: verified captures land in <staging>\out\<job>\ with hashes.json and
# run-provenance.json, and moving them into the research run directory is the
# orchestrator's deliberate step.
#
# Takes the built tree from whatever vm-run-tests.ps1 already stages and reuses
# that code path - do not duplicate the build logic.

[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Job,
    [string]$Baseline = 'base-provisioned',
    [ValidateSet('Set1024', 'Off')][string]$DisplayPin = 'Set1024',
    [string]$HarnessDir = 'D:\code\rustamp-harness\tools\vm-capture',
    [string]$BuiltTree = 'D:\code\rustamp-vm\staging\code\Rustamp',   # staged by vm-run-tests.ps1
    [string]$SkinDir = 'D:\code\rustamp-testskins',                    # contains base-2.91.wsz
    [string]$StageRoot = 'D:\code\rustamp-vm\staging',
    [int]$TimeoutSeconds = 3600
)

$ErrorActionPreference = 'Stop'
if ($Job -match '[\\/]' -or $Job -notmatch '^[A-Za-z0-9._-]+$') { throw "job name '$Job' is not allowed (plain name, no path)" }
$broker = Join-Path $PSScriptRoot 'vm-broker\invoke-broker.ps1'

function Step($t) { Write-Host "--- $t" }

$Stage = Join-Path $StageRoot 'presented'
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

# --- 4. stage the harness, the job, our built tree and the reference skin ------
# Blind copies, never inspected. The landmark captures live beside the harness
# in the research tree (<root>\captures) and the harness reads them from
# <ResearchRoot>\captures, so they must be staged too: without them the 1:1
# gate cannot compare and refuses before any job step.
Step "staging into $Stage"
if (-not (Test-Path $HarnessDir)) { throw "harness directory not found: $HarnessDir" }
$jobFile = Join-Path $HarnessDir ("jobs\" + $Job + ".json")
if (-not (Test-Path $jobFile)) { throw "job file not found: $jobFile" }
if (-not (Test-Path $BuiltTree)) { throw "built tree not found: $BuiltTree (run vm-run-tests.ps1 first to stage it)" }
if (-not (Test-Path $SkinDir)) { throw "skin directory not found: $SkinDir (needs base-2.91.wsz)" }
$skinFile = Join-Path $SkinDir 'base-2.91.wsz'
if (-not (Test-Path $skinFile)) { throw "base-2.91.wsz not found in $SkinDir" }
$landmarkDir = Join-Path (Split-Path (Split-Path $HarnessDir)) 'captures'
if (-not (Test-Path $landmarkDir)) { throw "landmark capture directory not found: $landmarkDir" }
if (Test-Path $Stage) { Remove-Item -Recurse -Force $Stage }
$null = New-Item -ItemType Directory -Force -Path (Join-Path $Stage 'tools\vm-capture')
Copy-Item (Join-Path $HarnessDir '*') (Join-Path $Stage 'tools\vm-capture') -Recurse -Force
$null = New-Item -ItemType Directory -Force -Path (Join-Path $Stage 'captures')
Copy-Item (Join-Path $landmarkDir '*') (Join-Path $Stage 'captures') -Recurse -Force
# Only the built binaries go to <Stage>\built: the guest launches
# `C:\rustamp\built\rustamp.exe` directly, and the staged repo tree keeps its
# binaries under target\debug. Copying the whole tree (the first version did)
# left the guest with no exe at the path it launches and the job failed on a
# missing file instead of a measurement.
$builtBinaries = Join-Path $BuiltTree 'target\debug'
if (-not (Test-Path (Join-Path $builtBinaries 'rustamp.exe'))) {
    throw ("no rustamp.exe under {0} (run vm-run-tests.ps1 first, which builds and stages the tree)" -f $builtBinaries)
}
$null = New-Item -ItemType Directory -Force -Path (Join-Path $Stage 'built')
Copy-Item (Join-Path $builtBinaries '*') (Join-Path $Stage 'built') -Recurse -Force
$null = New-Item -ItemType Directory -Force -Path (Join-Path $Stage 'skins')
Copy-Item $skinFile (Join-Path $Stage 'skins\base-2.91.wsz') -Force

# --- 5. copy in and run one job -------------------------------------------------
Step 'copying the staged presented tree into the guest'
& $broker -Op copyin -Source $Stage -Dest 'C:\rustamp\capture' -TimeoutSeconds 1800 | Out-Null

# Real media for jobs that need a playing track (the owner's archive copy on
# the host, never modified there): staged as-is so a job can load a guest path
# like C:\rustamp\testmedia\jingle-rockdown.mp3. Absent means jobs that need
# media refuse loudly instead of measuring an empty list. The broker only
# copies from under the staging root, so the media passes through staging
# first (a blind copy like everything else).
$mediaDir = 'D:\code\rustamp-testmedia'
if (Test-Path $mediaDir) {
    Step 'copying real test media into the guest (read-only copies)'
    $mediaStage = Join-Path $StageRoot 'testmedia'
    if (Test-Path $mediaStage) { Remove-Item -Recurse -Force $mediaStage }
    Copy-Item $mediaDir $mediaStage -Recurse -Force
    & $broker -Op copyin -Source $mediaStage -Dest 'C:\rustamp\testmedia' -TimeoutSeconds 600 | Out-Null
} else {
    Write-Host '    no real test media on the host; media-dependent jobs will refuse'
}

Step "running presented job '$Job' in the guest"
$result = & $broker -Op guest -Script 'run-presented.ps1' -GuestArgs $Job -TimeoutSeconds $TimeoutSeconds

# --- 6. copy out ----------------------------------------------------------------
Step 'copying the captures back to the host'
if (Test-Path $OutDir) { Remove-Item -Recurse -Force $OutDir }
$null = New-Item -ItemType Directory -Force -Path $OutDir
& $broker -Op copyout -Source ("C:\rustamp\out\" + $Job) -Dest $OutDir -TimeoutSeconds 1800 | Out-Null

# The broker's copyout nests the source folder under the destination, so a job
# that succeeded every guest step still arrived as $OutDir\<job>\... and the
# manifest check below then refused it ("no hashes.json came back") while the
# guest's own DONE.json said status ok. Flatten that one level, keeping the
# name, so the layout the rest of this script (and the capture archive) reads
# is the one the guest wrote.
$nested = Join-Path $OutDir $Job
if ((-not (Test-Path (Join-Path $OutDir 'hashes.json'))) -and (Test-Path $nested)) {
    Get-ChildItem -LiteralPath $nested -Force | Move-Item -Destination $OutDir -Force
    Remove-Item -LiteralPath $nested -Recurse -Force
}

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