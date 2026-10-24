# Stage and run the transfer display measurement in the guest.
#
# Usage:
#   powershell -ExecutionPolicy Bypass -File docs\notes\vm-run-measure-transfer.ps1 [-Baseline base-provisioned]
#
# Runs three modes: normal, refused, cut
# Each mode runs as a separate guest script invocation via the broker.

[CmdletBinding()]
param(
    [string]$Baseline = 'base-provisioned',
    [ValidateSet('Set1024', 'Off')][string]$DisplayPin = 'Set1024',
    [string]$HarnessDir = 'D:\code\rustamp-harness\tools\vm-capture',
    # The reference copy the capture campaign already uses. Staging it (instead
    # of running the NSIS installer, which hangs in the interactive task) is the
    # difference between this measurement running and not running.
    [string]$RefDir = 'D:\code\rustamp-harness\refwinamp295',
    [string]$BuiltTree = 'D:\code\rustamp-vm\staging\code\Rustamp',
    [string]$SkinDir = 'D:\code\rustamp-testskins',
    [string]$StageRoot = 'D:\code\rustamp-vm\staging',
    [int]$TimeoutSeconds = 3600,
    [ValidateSet('normal', 'refused', 'cut', 'slow', 'refused-run', 'cut-run')][string[]]$Modes = @('normal', 'refused', 'cut')
)

$ErrorActionPreference = 'Stop'
$broker = Join-Path $PSScriptRoot 'vm-broker\invoke-broker.ps1'

function Step($t) { Write-Host "--- $t" }

$Stage = Join-Path $StageRoot 'measure-transfer'
$OutRoot = Join-Path $StageRoot 'out'

# --- 1. baseline provenance ---
Step "reading checkpoint catalogue (baseline: $Baseline)"
$listText = & $broker -Op list -TimeoutSeconds 300 | Out-String
$knownNames = @(
    ($listText -split "`r?`n") |
        Where-Object { $_ -match '^\s*(\S+)\s' } |
        ForEach-Object { $Matches[1] } |
        Where-Object { $_ -ne 'Name' -and $_ -notmatch '^-+$' }
)
if ($knownNames -notcontains $Baseline) { throw "no checkpoint named '$Baseline' (broker knows: $($knownNames -join ', '))" }
$baselineRow = @(($listText -split "`r?`n") | Where-Object { $_ -match ('^\s*' + [regex]::Escape($Baseline) + '\b') } | Select-Object -First 1)
$baselineCreated = 'unknown (unparsed list row)'
if ($baselineRow) {
    $rest = [regex]::Replace($baselineRow, ('^\s*' + [regex]::Escape($Baseline) + '\s+\S+\s+'), '')
    if ($rest.Trim()) { $baselineCreated = $rest.Trim() }
}

# --- 2. restore the baseline ---
Step "restoring $Baseline (discards everything since the checkpoint)"
& $broker -Op restore -Name $Baseline -TimeoutSeconds 900 | Out-Null
$restoreStamp = (Get-Date).ToString('o')

# --- 3. pin the virtual display ---
Step "pinning the virtual display to 1024x768 (mode: $DisplayPin)"
$display = & $broker -Op guest -Script 'set-vdd-mode.ps1' -GuestArgs $DisplayPin -TimeoutSeconds 600
$displaySetAt = (Get-Date).ToString('o')
Write-Host ("    screen {0} (logpixels {1})" -f $display.screen, $display.logPixels)
foreach ($s in @($display.screens)) { Write-Host ("    monitor {0}" -f $s) }

# --- 4. stage the guest scripts and test media ---
Step "staging into $Stage"
if (-not (Test-Path $HarnessDir)) { throw "harness directory not found: $HarnessDir" }
if (-not (Test-Path $BuiltTree)) { throw "built tree not found: $BuiltTree" }
if (-not (Test-Path $SkinDir)) { throw "skin directory not found: $SkinDir" }
$skinFile = Join-Path $SkinDir 'base-2.91.wsz'
if (-not (Test-Path $skinFile)) { throw "base-2.91.wsz not found in $SkinDir" }

if (Test-Path $Stage) { Remove-Item -Recurse -Force $Stage }
$null = New-Item -ItemType Directory -Force -Path (Join-Path $Stage 'tools\vm-capture\guest')
$null = New-Item -ItemType Directory -Force -Path (Join-Path $Stage 'tools\vm-capture\jobs')
$null = New-Item -ItemType Directory -Force -Path (Join-Path $Stage 'skins')

# Copy the guest scripts we need
Copy-Item (Join-Path $PSScriptRoot 'vm-broker\guest\measure-transfer.ps1') (Join-Path $Stage 'tools\vm-capture\guest\measure-transfer.ps1') -Force
Copy-Item (Join-Path $PSScriptRoot 'vm-broker\guest\set-vdd-mode.ps1') (Join-Path $Stage 'tools\vm-capture\guest\set-vdd-mode.ps1') -Force
Copy-Item (Join-Path $PSScriptRoot 'vm-broker\guest\assert-interactive-session.ps1') (Join-Path $Stage 'tools\vm-capture\guest\assert-interactive-session.ps1') -Force
Copy-Item (Join-Path $PSScriptRoot 'vm-broker\guest\assert-attached-display.ps1') (Join-Path $Stage 'tools\vm-capture\guest\assert-attached-display.ps1') -Force

# Copy the skin
Copy-Item $skinFile (Join-Path $Stage 'skins\base-2.91.wsz') -Force

# Copy test media
$mediaDir = 'D:\code\rustamp-testmedia'
if (Test-Path $mediaDir) {
    Step 'staging real test media'
    $mediaStage = Join-Path $StageRoot 'testmedia'
    if (Test-Path $mediaStage) { Remove-Item -Recurse -Force $mediaStage }
    Copy-Item $mediaDir $mediaStage -Recurse -Force
} else {
    Write-Host '    no real test media on host; generated WAV will be used'
}

# Stage the reference copy (the one the capture campaign runs). Not the NSIS
# installer: the installer needs a full interactive desktop and hangs inside the
# one-shot task, which is how the first attempt lost an hour and produced no
# measurement. The staged copy is a directory the harness already trusts.
if (-not (Test-Path $RefDir)) { throw "reference copy not found: $RefDir" }
Step "staging the reference copy from $RefDir"
$null = New-Item -ItemType Directory -Force -Path (Join-Path $Stage 'winamp295')
Copy-Item (Join-Path $RefDir '*') (Join-Path $Stage 'winamp295') -Recurse -Force

# --- 5. copy in ---
Step 'copying the staged tree into the guest'
& $broker -Op copyin -Source $Stage -Dest 'C:\rustamp\capture' -TimeoutSeconds 1800 | Out-Null

$mediaStage = Join-Path $StageRoot 'testmedia'
if (Test-Path $mediaStage) {
    Step 'copying real test media into the guest'
    & $broker -Op copyin -Source $mediaStage -Dest 'C:\rustamp\testmedia' -TimeoutSeconds 600 | Out-Null
}



# --- 6. Run three modes ---
$modes = @($Modes)
$allResults = @()
$allProvenance = @()

foreach ($mode in $modes) {
    $jobName = "transfer-$mode"
    Step "running measurement job '$jobName' (mode: $mode)"

    $result = & $broker -Op guest -Script 'measure-transfer.ps1' -GuestArgs $mode -TimeoutSeconds $TimeoutSeconds

    # --- 7. copy out ---
    $outDir = Join-Path $OutRoot $jobName
    Step "copying captures back for '$jobName'"
    if (Test-Path $outDir) { Remove-Item -Recurse -Force $outDir }
    $null = New-Item -ItemType Directory -Force -Path $outDir
    & $broker -Op copyout -Source ("C:\rustamp\out\" + $jobName) -Dest $outDir -TimeoutSeconds 1800 | Out-Null

    # Flatten nested directory if needed
    $nested = Join-Path $outDir $jobName
    if ((-not (Test-Path (Join-Path $outDir 'summary.json'))) -and (Test-Path $nested)) {
        Get-ChildItem -LiteralPath $nested -Force | Move-Item -Destination $outDir -Force
        Remove-Item -LiteralPath $nested -Recurse -Force
    }

    # Verify we got outputs
    $summaryPath = Join-Path $outDir 'summary.json'
    if (-not (Test-Path $summaryPath)) {
        Write-Host "WARNING: no summary.json for $jobName"
        continue
    }
    $summary = Get-Content -Path $summaryPath -Raw | ConvertFrom-Json
    $allResults += $summary

    $provenance = [ordered]@{
        job             = $jobName
        mode            = $mode
        baseline        = $Baseline
        baselineCreated = $baselineCreated
        restoreStamp    = $restoreStamp
        displayPin      = $DisplayPin
        displaySetAt    = $displaySetAt
        displaySet      = [string]$display.screen
        displayScreens  = @($display.screens)
        setLogPixels    = $display.logPixels
        guestBootTime   = [string]$result.bootTime
        guest           = [string]$result.host
        guestUser       = [string]$result.user
        durationSec     = [double]$summary.DurationSec
        observations    = [int]$summary.Observations
        outDir          = $outDir
    }
    $allProvenance += $provenance
    $provenance | ConvertTo-Json | Set-Content -Path (Join-Path $outDir 'run-provenance.json') -Encoding UTF8

    Write-Host ("  {0}: {1} observations in {2}s" -f $jobName, $summary.Observations, [math]::Round($summary.DurationSec,1))
}

# --- 8. Write combined provenance ---
$combinedProvenance = [ordered]@{
    baseline          = $Baseline
    baselineCreated   = $baselineCreated
    restoreStamp      = $restoreStamp
    displayPin        = $DisplayPin
    displaySetAt      = $displaySetAt
    displaySet        = [string]$display.screen
    displayScreens    = @($display.screens)
    setLogPixels      = $display.logPixels
    jobs              = $allProvenance
}
$combinedProvenance | ConvertTo-Json -Depth 5 | Set-Content -Path (Join-Path $OutRoot 'measure-transfer-provenance.json') -Encoding UTF8

Write-Host ''
Write-Host "All three modes completed. Results in $OutRoot"
foreach ($p in $allProvenance) {
    Write-Host ("  {0}: {1} obs, {2}s" -f $p.job, $p.observations, [math]::Round($p.durationSec,1))
}