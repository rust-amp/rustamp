# Compile and stage on the host (unelevated), run the ignored winit window
# tests in the guest's INTERACTIVE session, bring the results back.
#
#   powershell -ExecutionPolicy Bypass -File docs\notes\vm-run-gui-session.ps1 [-Filter eq_window]
#   powershell ... -Reset            # restore base-provisioned first
#
# Why a second runner instead of reusing vm-run-tests.ps1: that path executes
# the binaries in the broker's PowerShell Direct session (session 0, no
# desktop), where every winit window test panics. This path stages the exact
# same tree, then runs the allow-listed guest script `run-gui-session.ps1`,
# which registers a one-shot Scheduled Task as the auto-logon user with
# LogonType Interactive - i.e. on the console desktop - waits for it, proves
# which session it ran in, and writes per-test logs plus a session proof under
# C:\rustamp\out\gui-session\, which the broker `copyout` op brings back.
#
# The division of labour is the same as vm-run-tests.ps1 and equally
# security-relevant:
#   * HERE (unelevated): compile and stage. Compiling runs build scripts from
#     the repository, so it must never happen with administrator rights.
#   * THE BROKER (elevated, session 0): only touches the VM - eject the DVD,
#     start/restore, copy in over PowerShell Direct, run the allow-listed guest
#     script, copy the results back out. It NEVER builds.
#
# A run that lands in session 0 fails loudly (the guest script refuses) rather
# than reporting a pass it did not have.

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
$broker = Join-Path $PSScriptRoot 'vm-broker\invoke-broker.ps1'

function Step($t) { Write-Host "--- $t" }

# --- 1. compile on the host (builds, does not run) ---------------------------
# Identical to vm-run-tests.ps1 §1: the build must succeed and the binary count
# must be plausible, or a shrunk set would report a false green.
if (-not $SkipBuild) {
    Step 'compiling on the host (cargo test --no-run; nothing is executed)'
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $json = & cargo test --workspace --all-features --no-run --message-format=json 2>$null
    $buildExit = $LASTEXITCODE
    $ErrorActionPreference = $previous
    if ($buildExit -ne 0) { throw "cargo test --no-run failed with exit $buildExit; nothing is trustworthy until it builds" }
    $testExes = @()
    foreach ($line in $json) {
        if ($line -isnot [string]) { continue }
        try { $msg = $line | ConvertFrom-Json } catch { continue }
        if ($msg.reason -eq 'compiler-artifact' -and $msg.executable -and $msg.profile.test) { $testExes += $msg.executable }
    }
    if (-not $testExes) { throw 'no test binaries were produced' }
    Write-Host ("    {0} test binaries" -f $testExes.Count)
} else {
    Step 'skipping the build (-SkipBuild); the previously staged binaries are reused'
    $stagedDeps = Join-Path $Stage 'target\debug\deps'
    if (-not (Test-Path $stagedDeps)) { throw "nothing staged at $stagedDeps" }
    $testExes = @(Get-ChildItem -Path $stagedDeps -Filter '*.exe' | Select-Object -ExpandProperty FullName)
}

# --- 2. stage (identical tree to vm-run-tests.ps1 §2) --------------------------
# The window tests resolve the repository, the fixtures and rustamp.exe through
# compile-time host paths (D:\code\Rustamp\...); the guest script does
# `subst D: C:\rustamp` so those exact paths exist there.
Step "staging into $Stage"
if (Test-Path $Stage) { Remove-Item -Recurse -Force $Stage }
$null = New-Item -ItemType Directory -Force -Path (Join-Path $Stage 'target\debug\deps')
foreach ($exe in $testExes) { Copy-Item $exe (Join-Path $Stage 'target\debug\deps') -Force }
$debugDir = Join-Path $RepoRoot 'target\debug'
foreach ($dll in Get-ChildItem -Path (Join-Path $debugDir 'deps') -Filter '*.dll' -ErrorAction SilentlyContinue) {
    Copy-Item $dll.FullName (Join-Path $Stage 'target\debug\deps') -Force
}
if (Test-Path (Join-Path $debugDir 'rustamp.exe')) { Copy-Item (Join-Path $debugDir 'rustamp.exe') (Join-Path $Stage 'target\debug') -Force }

Step 'building the i686 plugin host for the end-to-end test'
$prevPref = $ErrorActionPreference
$ErrorActionPreference = 'Continue'
$prevRustflags = $env:RUSTFLAGS
$env:RUSTFLAGS = '-C target-feature=+crt-static'
& cargo build -p rustamp-plughost --target i686-pc-windows-msvc 2>$null | Out-Null
if ($null -eq $prevRustflags) { Remove-Item Env:\RUSTFLAGS -ErrorAction SilentlyContinue } else { $env:RUSTFLAGS = $prevRustflags }
$ErrorActionPreference = $prevPref
$hostExe = Join-Path $RepoRoot 'target\i686-pc-windows-msvc\debug\rustamp-plughost32.exe'
if (Test-Path $hostExe) {
    Copy-Item $hostExe (Join-Path $Stage 'target\debug\deps') -Force
    Copy-Item $hostExe (Join-Path $Stage 'target\debug') -Force
    Write-Host '    staged the i686 plugin host beside the binaries'
} else {
    Write-Host '    WARNING: no i686 plugin host was built; the end-to-end test will report it blocked'
}
foreach ($crt in 'vcruntime140.dll', 'vcruntime140_1.dll', 'msvcp140.dll') {
    $p = Join-Path $env:SystemRoot "System32\$crt"
    if (Test-Path $p) {
        Copy-Item $p (Join-Path $Stage 'target\debug') -Force
        Copy-Item $p (Join-Path $Stage 'target\debug\deps') -Force
    }
}
Copy-Item (Join-Path $RepoRoot 'crates') (Join-Path $Stage 'crates') -Recurse -Force
foreach ($rootFile in 'Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml') {
    $p = Join-Path $RepoRoot $rootFile
    if (Test-Path $p) { Copy-Item $p (Join-Path $Stage $rootFile) -Force }
}
Write-Host ("    staged {0} MB" -f [math]::Round(((Get-ChildItem $Stage -Recurse -File | Measure-Object Length -Sum).Sum / 1MB), 1))

# --- 3. optional reset, then start (the broker ejects the DVD) --------------
if ($Reset) {
    Step "restoring $Baseline"
    & $broker -Op restore -Name $Baseline -TimeoutSeconds 900 | Out-Null
} else {
    Step 'starting the guest (the broker ejects the install DVD so the guest gets D:)'
    & $broker -Op start -TimeoutSeconds 900 | Out-Null
}

# --- 4. copy in and run in the interactive session -----------------------------
Step 'copying the staged tree into the guest'
& $broker -Op copyin -Source $Stage -Dest 'C:\rustamp\code\Rustamp' -TimeoutSeconds 1800 | Out-Null

Step 'running the window tests in the guest interactive session'
# No filter (default '') runs all four ignored winit window tests; a substring
# (e.g. eq_window) runs just the matching one. The guest script proves the
# session and fails loudly on session 0.
$result = & $broker -Op guest -Script 'run-gui-session.ps1' -GuestArgs $Filter -TimeoutSeconds 1800

# --- 5. copy the proof and logs back; report ------------------------------------
Step 'copying the session proof and test logs back to the host'
$OutDir = Join-Path $StageRoot 'out\gui-session'
if (Test-Path $OutDir) { Remove-Item -Recurse -Force $OutDir }
$null = New-Item -ItemType Directory -Force -Path $OutDir
& $broker -Op copyout -Source 'C:\rustamp\out\gui-session' -Dest $OutDir -TimeoutSeconds 600 | Out-Null

$proofPath = Join-Path $OutDir 'session-proof.json'
if (-not (Test-Path $proofPath)) { throw 'no session-proof.json came back: the guest run did not finish its proof step; nothing is promoted' }
$proof = Get-Content -Path $proofPath -Raw | ConvertFrom-Json
if ($proof.session -eq 0) { throw 'the guest run landed in session 0, which has no desktop; this run proves nothing' }

Write-Host ''
Write-Host ("gui session {0} as {1} on {2}; screen {3}; marker staged tree: {4}; ran {5}, failed {6}" -f `
    $proof.session, $proof.user, $proof.computer, $proof.screen, $proof.marker, $proof.ran, $proof.failed)
foreach ($r in $proof.results) {
    $status = if ($r.exit -eq 0) { 'ok  ' } else { 'FAIL' }
    Write-Host ("  [{0}] {1,-16} {2} ({3})" -f $status, $r.stem, $r.summary, $r.test)
    $logPath = Join-Path $OutDir $r.log
    if (($r.exit -ne 0) -and (Test-Path $logPath)) {
        Write-Host ("         log: {0}" -f $logPath)
    }
}
Write-Host ("proof and logs in {0}" -f $OutDir)
if ($proof.failed -ne 0) { exit 1 }
