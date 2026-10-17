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
# Mirror the test runner: use an absolute path under the broker's staging root
# so the broker's copyin rule (StartsWith check) always passes.
$Stage = Join-Path $StageRoot 'smoke'
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
Step 'copying the staged package tree into the guest'
& $broker -Op copyin -Source $Stage -Dest 'C:\rustamp\smoke' -TimeoutSeconds 1800 | Out-Null

Step 'running the packaged player smoke in the guest interactive session'
$result = & $broker -Op guest -Script 'run-smoke.ps1' -TimeoutSeconds $GuestTimeoutSeconds
if (-not $result) { Fail 'guest' 'the broker returned no result object for run-smoke.ps1 (see broker.log)' }

# --- 6. copy the proof and outputs back; verify -------------------------------
Step 'copying the smoke proof and captures back to the host'
$OutDir = Join-Path $StageRoot 'out\smoke'
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
# (same integrity leg as vm-run-capture.ps1 section 7).
Step 'verifying hashes (byte-identical or refused)'
$manifestPath = Join-Path $OutDir 'hashes.json'
if (-not (Test-Path $manifestPath)) { Fail 'verify' 'no hashes.json came back: the guest run did not finish its manifest step; nothing is promoted' }
$manifest = Get-Content -Path $manifestPath -Raw | ConvertFrom-Json
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
