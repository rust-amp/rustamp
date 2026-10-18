<#
.SYNOPSIS
  Smoke-prove the packaged player: start it in the VM, show its window, capture it.

.DESCRIPTION
  Takes a packaged folder (default: the newest under D:\code\rustamp-beta),
  stages the packaged bytes VERBATIM into the test-VM guest (nothing is
  rebuilt: the player and the bundled host are byte copies of the shipped
  files), starts the packaged player with a real window and the null audio
  device (`--no-audio`) in the guest's INTERACTIVE session, waits for the
  main window in a bounded way, screenshots it through the existing capture
  tool (`cargo xtask capture`), and brings the PNG plus the session proof
  back through the existing broker copyout path with hash verification.

  Division of labour (same as vm-run-tests.ps1, security-relevant):
    * HERE (unelevated): validate the package layout, build ONLY the capture
      tool (`cargo build -p xtask`: a host-side helper, never the subject),
      stage. Building the player/host here would void the proof, so the
      player and host binaries are copied byte-for-byte, never compiled.
    * THE BROKER (elevated, session 0): only touches the VM - start/restore,
      copy in over PowerShell Direct, run the allow-listed guest script
      `run-smoke.ps1`, copy the results back out. It NEVER builds.
    * THE GUEST: the allow-listed script runs the packaged player in the
      interactive session (the broker's session 0 has no desktop), waits for
      the window, checks the bundled host child, captures, proves.

  Nothing graphical runs on this host: the only window opens in the guest.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File docs\notes\vm-run-smoke.ps1
  powershell -ExecutionPolicy Bypass -File docs\notes\vm-run-smoke.ps1 -Reset
  powershell -ExecutionPolicy Bypass -File docs\notes\vm-run-smoke.ps1 -PackageDir D:\code\rustamp-beta\2026-10-17
#>
[CmdletBinding()]
param(
    [string]$PackageDir = '',
    [string]$BetaRoot = 'D:\code\rustamp-beta',
    [string]$RepoRoot = 'D:\code\Rustamp',
    [string]$StageRoot = 'D:\code\rustamp-vm\staging',
    [string]$Baseline = 'base-provisioned',
    [switch]$Reset,
    [int]$GuestTimeoutSeconds = 1800
)

$ErrorActionPreference = 'Stop'
$broker = Join-Path $PSScriptRoot 'vm-broker\invoke-broker.ps1'

function Step($t) { Write-Host "--- $t" }
function Fail([string]$step, [string]$detail) { throw "STEP $step FAILED: $detail" }

# --- 1. resolve and audit the package (read-only; nothing is executed) -------
Step 'resolving the packaged folder'
if ([string]::IsNullOrWhiteSpace($PackageDir)) {
    if (-not (Test-Path $BetaRoot)) { Fail 'package' "no package dir given and the beta root $BetaRoot does not exist" }
    $newest = @(Get-ChildItem -Path $BetaRoot -Directory | Sort-Object Name -Descending | Select-Object -First 1)
    if (-not $newest) { Fail 'package' "no packaged folder found under $BetaRoot" }
    $PackageDir = $newest[0].FullName
}
if (-not (Test-Path $PackageDir)) { Fail 'package' "packaged folder not found: $PackageDir" }
Write-Host ("    package: {0}" -f $PackageDir)

# The load-bearing binaries of the BETA-build layout: the player, the host
# bundled beside it (RFC 0006 section 8 packaged layout: the first
# supervise.rs candidate), and the standalone host. Licenses and manifests
# are warned about, not gated: their absence is a release-process finding,
# not a start-the-player finding.
Step 'auditing the package layout (binaries required, nothing executed)'
$playerExe = Join-Path $PackageDir 'player\rustamp.exe'
$bundledHost = Join-Path $PackageDir 'player\rustamp-plughost32.exe'
$standaloneHost = Join-Path $PackageDir 'host\rustamp-plughost32.exe'
foreach ($need in @($playerExe, $bundledHost, $standaloneHost)) {
    if (-not (Test-Path $need)) { Fail 'package' "missing load-bearing binary: $need" }
    if ((Get-Item $need).Length -eq 0) { Fail 'package' "empty binary (0 bytes): $need" }
}
foreach ($sidecar in @('player\LICENSE', 'player\PLUGIN-EXCEPTION.md', 'host\LICENSE-MIT', 'host\LICENSE-APACHE', 'manifest.json', 'player\manifest.json', 'host\manifest.json')) {
    if (-not (Test-Path (Join-Path $PackageDir $sidecar))) { Write-Host ("    WARNING: expected sidecar missing: {0}" -f $sidecar) }
}
$playerHash = (Get-FileHash -Path $playerExe -Algorithm SHA256).Hash
$bundledHash = (Get-FileHash -Path $bundledHost -Algorithm SHA256).Hash
$standaloneHash = (Get-FileHash -Path $standaloneHost -Algorithm SHA256).Hash
Write-Host ("    player\rustamp.exe                 {0} MB  {1}" -f ([math]::Round((Get-Item $playerExe).Length / 1MB, 2)), $playerHash)
Write-Host ("    player\rustamp-plughost32.exe      {0} MB  {1}" -f ([math]::Round((Get-Item $bundledHost).Length / 1MB, 2)), $bundledHash)
Write-Host ("    host\rustamp-plughost32.exe        {0} MB  {1}" -f ([math]::Round((Get-Item $standaloneHost).Length / 1MB, 2)), $standaloneHash)
if ($bundledHash -ne $standaloneHash) {
    Write-Host '    WARNING: the bundled host copy differs from the standalone host copy (same build should be byte-identical)'
}

# --- 2. build ONLY the capture tool (the subject is never rebuilt) ------------
# Rationale, stated so this step is never misread as rebuilding the player:
# the packaged rustamp.exe / rustamp-plughost32.exe above are staged
# byte-for-byte and are the only binaries under test. `cargo xtask capture`
# is the existing, reviewed window-capture tool; compiling it on the host
# (unelevated, like the existing runners compile their test binaries) does
# not touch the packaged subject.
Step 'building the capture tool on the host (cargo build -p xtask; the packaged player is NOT rebuilt)'
$previous = $ErrorActionPreference
$ErrorActionPreference = 'Continue'
& cargo build -p xtask --manifest-path (Join-Path $RepoRoot 'Cargo.toml') 2>$null | Out-Null
$buildExit = $LASTEXITCODE
$ErrorActionPreference = $previous
if ($buildExit -ne 0) { Fail 'build-tool' "cargo build -p xtask failed with exit $buildExit; nothing graphical can be captured until it builds" }
$toolExe = Join-Path $RepoRoot 'target\debug\xtask.exe'
if (-not (Test-Path $toolExe)) { Fail 'build-tool' "no capture tool was produced at $toolExe" }
Write-Host ("    capture tool: {0} ({1} MB)" -f $toolExe, ([math]::Round((Get-Item $toolExe).Length / 1MB, 2)))

# The repo's own committed fixture: the 1.5 s, 440 Hz FLAC the headless window
# test plays. A known-good file, so a decode failure names the package, not
# the media (contrast BETA-04's phantom-playback on bad input).
$fixtureSrc = Join-Path $RepoRoot 'crates\rustamp-decoders\tests\fixtures\sine.flac'
if (-not (Test-Path $fixtureSrc)) { Fail 'stage' "fixture missing (not committed?): $fixtureSrc" }

# --- 3. stage the package tree into the guest staging area --------------------
# Verbatim copies of the shipped bytes plus the tool and the fixture. The CRT
# DLLs travel only beside the dev-built capture tool (dynamic CRT, like the
# existing runners' test binaries); the packaged player and host are
# static-CRT (X1) and need nothing.
Step 'staging the packaged bytes (verbatim copies, no rebuild)'
# Mirror the test runner: the staged tree lives under the broker's staging
# root, and the copy-in hands the broker that ABSOLUTE path. It is resolved
# here (GetFullPath), so even a relative -StageRoot argument cannot produce a
# relative copy-in source; the broker's rule itself is verified again, on the
# host, right before the copy-in call (section 5).
$Stage = Join-Path $StageRoot 'smoke'
$Stage = [System.IO.Path]::GetFullPath($Stage)
if (Test-Path $Stage) { Remove-Item -Recurse -Force $Stage }
$null = New-Item -ItemType Directory -Force -Path (Join-Path $Stage 'player')
$null = New-Item -ItemType Directory -Force -Path (Join-Path $Stage 'tools')
$null = New-Item -ItemType Directory -Force -Path (Join-Path $Stage 'media')
Copy-Item $playerExe (Join-Path $Stage 'player\rustamp.exe') -Force
Copy-Item $bundledHost (Join-Path $Stage 'player\rustamp-plughost32.exe') -Force
Copy-Item $toolExe (Join-Path $Stage 'tools\xtask.exe') -Force
Copy-Item $fixtureSrc (Join-Path $Stage 'media\sine.flac') -Force
$crtCopied = 0
foreach ($crt in 'vcruntime140.dll', 'vcruntime140_1.dll', 'msvcp140.dll') {
    $p = Join-Path $env:SystemRoot "System32\$crt"
    if (Test-Path $p) { Copy-Item $p (Join-Path $Stage 'tools') -Force; $crtCopied++ }
}
if ($crtCopied -eq 0) { Write-Host '    WARNING: no host CRT DLLs found to stage beside the dev-built capture tool; it may die with 0xC0000135 in the guest' }
else { Write-Host ("    staged {0} CRT DLL(s) beside the capture tool" -f $crtCopied) }
Write-Host ("    staged {0} MB" -f [math]::Round(((Get-ChildItem $Stage -Recurse -File | Measure-Object Length -Sum).Sum / 1MB), 1))

# --- 4. optional reset, then start --------------------------------------------
if ($Reset) {
    Step "restoring $Baseline"
    & $broker -Op restore -Name $Baseline -TimeoutSeconds 900 | Out-Null
} else {
    Step 'starting the guest'
    & $broker -Op start -TimeoutSeconds 900 | Out-Null
}

# --- 5. copy in and run the smoke in the interactive session ------------------
# The broker's copyin rule (docs\notes\vm-broker\vm-broker.ps1, 'copyin' op):
# the source must resolve under the staging root it owns
# (D:\code\rustamp-vm\staging) and the destination under the guest root
# (C:\rustamp). Both are checked HERE, on the host, so a path mistake fails
# this runner under its own step name instead of inside the broker. The staged
# tree is handed over as one absolute path under that root - the same shape as
# the test runner's copyin (its $Stage is an absolute path under the same
# root).
$copyinSource = [System.IO.Path]::GetFullPath($Stage)
$copyinDest = 'C:\rustamp\smoke'
$stageRootFull = [System.IO.Path]::GetFullPath($StageRoot)
$guestRootFull = [System.IO.Path]::GetFullPath('C:\rustamp')
if (-not $copyinSource.StartsWith($stageRootFull, [System.StringComparison]::OrdinalIgnoreCase)) {
    Fail 'copyin' "the staged tree '$copyinSource' is not under the broker's staging root '$stageRootFull' (its copyin accepts sources from that root only)"
}
if (-not $copyinDest.StartsWith($guestRootFull, [System.StringComparison]::OrdinalIgnoreCase)) {
    Fail 'copyin' "the copy-in destination '$copyinDest' is not under the guest root '$guestRootFull' (its copyin accepts destinations from that root only)"
}
Step 'copying the staged package tree into the guest'
try {
    & $broker -Op copyin -Source $copyinSource -Dest $copyinDest -TimeoutSeconds 1800 | Out-Null
} catch {
    $brokerErr = $_.Exception.Message
    if ($brokerErr -match 'copyin (source|destination) must live under') {
        # The broker's own path-policy refusal: it names the root and the
        # offending path, so the runner's path expressions are the thing to
        # fix - report it as such.
        Fail 'copyin' "the broker refused a copy-in path (its rule, docs\notes\vm-broker\vm-broker.ps1 'copyin'): $brokerErr"
    }
    # The broker's path rule passed, so the failure is on the broker's side
    # of the call. The known case (broker.log 2026-10-17 20:38 and 2026-10-18
    # 08:03) is an access-denied naming a staged file: the guest could not
    # delete/overwrite it because a process there still holds it open. After
    # a smoke run whose guest payload failed before its kill step, the player
    # lives on to its --seconds backstop and its bundled host child can
    # outlive it (the recorded hostReaped: false finding), so
    # C:\rustamp\smoke\player\rustamp-plughost32.exe stays locked and the next
    # copy-in cannot replace it. Remedy: a fresh guest - re-run with -Reset
    # (restores the baseline checkpoint) or stop+start the guest. broker.log's
    # 'op=copyin args=' line records the source the broker actually received,
    # so the next diagnosis can settle this runner's -Source in one look
    # instead of guessing at it.
    Fail 'copyin' ("the broker's copy-in failed with this runner's source '$copyinSource' and destination '$copyinDest', both of which passed the broker's path rule: $brokerErr`n" +
        "    If this is an access-denied naming a staged file (the known case), the guest still holds that file open - a leftover rustamp.exe / rustamp-plughost32.exe process from a previous smoke run (the guest payload's failure paths exit without killing the player, and the host child can outlive it). Remedy: re-run with -Reset for a fresh guest (restores the '$Baseline' checkpoint), or stop+start the guest.")
}

Step 'running the packaged player smoke in the guest interactive session'
# The results directory is the copy-out destination, so the broker's copyout
# rule applies to it: it must resolve under the host results root
# (D:\code\rustamp-vm\staging\out). Resolved and verified here, once, so both
# copy-out calls below (the failure path and the success path) use the same
# absolute path.
$OutDir = Join-Path $StageRoot 'out\smoke'
$OutDir = [System.IO.Path]::GetFullPath($OutDir)
$stageOutRoot = Join-Path $StageRoot 'out'
$stageOutFull = [System.IO.Path]::GetFullPath($stageOutRoot)
if (-not $OutDir.StartsWith($stageOutFull, [System.StringComparison]::OrdinalIgnoreCase)) {
    Fail 'copyout' "the copy-out destination '$OutDir' is not under the broker's results root '$stageOutFull' (its copyout accepts destinations from that root only)"
}
try {
    $result = & $broker -Op guest -Script 'run-smoke.ps1' -TimeoutSeconds $GuestTimeoutSeconds
    if (-not $result) { Fail 'guest' 'the broker returned no result object for run-smoke.ps1 (see broker.log)' }
} catch {
    # Failure path still copies the guest's own transcript out, so the next
    # diagnosis does not need a manual copyout: the inner payload writes
    # `guest-transcript.txt` (window dump included) before it fails. On a
    # failed run that transcript is the payload's only: the outer half
    # appends its session-discovery trail and writes `hashes.json` only on
    # a successful run (in run-smoke.ps1 both sit after the failure
    # throws), so a failed run brings the transcript back without a
    # manifest and the hash leg below never runs for it. Best-effort: a
    # copyout failure must not mask the original guest error.
    $guestErr = $_.Exception.Message
    try {
        if (Test-Path $OutDir) { Remove-Item -Recurse -Force $OutDir }
        $null = New-Item -ItemType Directory -Force -Path $OutDir
        & $broker -Op copyout -Source 'C:\rustamp\out\smoke' -Dest $OutDir -TimeoutSeconds 600 | Out-Null
        Write-Host ("    guest outputs (incl. guest-transcript.txt) copied to {0} despite the failure" -f $OutDir)
    } catch {
        Write-Host ("    WARNING: failure-path copyout also failed: {0}" -f $_.Exception.Message)
    }
    throw "STEP guest FAILED: $guestErr"
}

# --- 6. copy the proof and outputs back; verify -------------------------------
Step 'copying the smoke proof and captures back to the host'
if (Test-Path $OutDir) { Remove-Item -Recurse -Force $OutDir }
$null = New-Item -ItemType Directory -Force -Path $OutDir
& $broker -Op copyout -Source 'C:\rustamp\out\smoke' -Dest $OutDir -TimeoutSeconds 600 | Out-Null

# `copyout` puts the source directory *inside* the destination, so the files
# arrive one level deeper than the naive path (same trap as
# vm-run-gui-session.ps1). Accept both layouts.
$proofPath = Join-Path $OutDir 'proof.json'
if (-not (Test-Path $proofPath)) {
    $nested = Join-Path $OutDir 'smoke\proof.json'
    if (Test-Path $nested) { $OutDir = Join-Path $OutDir 'smoke'; $proofPath = $nested }
}
if (-not (Test-Path $proofPath)) { Fail 'verify' 'no proof.json came back: the guest run did not finish its proof step; nothing is proven' }
$proof = Get-Content -Path $proofPath -Raw | ConvertFrom-Json
if ($proof.session -eq 0) { Fail 'verify' 'the guest run landed in session 0, which has no desktop; this run proves nothing' }
foreach ($flag in 'windowTitle', 'windowClient', 'hostChildPid', 'captureFile', 'stdoutMarker') {
    if ($null -eq $proof.$flag) { Fail 'verify' ("proof.json is missing the '{0}' fact; the guest run is incomplete" -f $flag) }
}
if ($proof.windowClient -ne '275x116') { Fail 'verify' ("main window client is {0}, want 275x116 (S01 rest size)" -f $proof.windowClient) }
if (-not $proof.stdoutMarker) { Fail 'verify' 'the player never reported the silent path (`output null (windowed; silent, ...)`) ' }
$pngPath = Join-Path $OutDir $proof.captureFile
if (-not (Test-Path $pngPath)) { Fail 'verify' ("proof names capture '{0}' but it did not come back" -f $proof.captureFile) }
$pngBytes = (Get-Item $pngPath).Length
if ($pngBytes -lt 512) { Fail 'verify' ("capture '{0}' is {1} bytes: not a plausible window PNG" -f $proof.captureFile, $pngBytes) }
$sig = [System.IO.File]::ReadAllBytes($pngPath)[0..3]
if (-not ($sig[0] -eq 0x89 -and $sig[1] -eq 0x50 -and $sig[2] -eq 0x4E -and $sig[3] -eq 0x47)) {
    Fail 'verify' ("capture '{0}' has no PNG signature; it is not an image" -f $proof.captureFile)
}

# Byte-identity: recompute every hash, compare against the guest manifest
# (same integrity leg as vm-run-capture.ps1 section 7). The guest writes
# that manifest once, after guest-transcript.txt is complete
# (run-smoke.ps1, outer half), so every entry - the completed transcript
# included - is verifiable byte-for-byte (X9).
Step 'verifying hashes (byte-identical or refused)'
$manifestPath = Join-Path $OutDir 'hashes.json'
if (-not (Test-Path $manifestPath)) { Fail 'verify' 'no hashes.json came back: the guest run did not finish its manifest step; nothing is promoted' }
$manifest = Get-Content -Path $manifestPath -Raw | ConvertFrom-Json
# The transcript is the proof's primary evidence (window dump, stop record,
# exit code), so it must be IN the byte-identical set: a manifest that
# does not cover it means the guest script regressed to hashing a live
# log - refuse instead of verifying a subset (X9).
if ($null -eq $manifest.PSObject.Properties['guest-transcript.txt']) {
    Fail 'verify' "the manifest does not cover 'guest-transcript.txt': the guest writes hashes.json after the transcript is complete, so an uncovered transcript is a guest-script regression, not a smaller proof"
}
$checked = 0
foreach ($entry in $manifest.PSObject.Properties) {
    $p = Join-Path $OutDir $entry.Name
    if (-not (Test-Path $p)) { Fail 'verify' ("manifest lists '{0}' but it did not come back" -f $entry.Name) }
    $actual = (Get-FileHash -Path $p -Algorithm SHA256).Hash
    if ($actual -ne [string]$entry.Value) { Fail 'verify' ("hash mismatch on '{0}': guest {1}, host {2}" -f $entry.Name, $entry.Value, $actual) }
    $checked++
}
if ($checked -eq 0) { Fail 'verify' 'manifest is empty: a smoke run with no outputs is a failed run, not an empty success' }

Write-Host ''
Write-Host ("smoke PASS: packaged player started in session {0} as {1} on {2}; screen {3}" -f $result.session, $result.user, $result.host, $result.screen)
Write-Host ("  player pid {0}, up {1} s; main window {2} {3} {4} ({5})" -f $result.playerPid, $result.upSeconds, $result.windowHwnd, $result.windowTitle, $result.windowClient, $result.windowRect)
Write-Host ("  bundled host child pid {0} (reaped with player: {1})" -f $result.hostChildPid, $result.hostReaped)
Write-Host ("  capture {0} ({1} bytes, window {2} {3} {4}); silent-path marker: {5}" -f $result.captureFile, $result.captureBytes, $result.captureHwnd, $result.captureTitle, $result.captureSize, $result.stdoutMarker)
Write-Host ("  {0} files verified byte-identical in {1}" -f $checked, $OutDir)
