# Compile and stage on the host (unelevated), run the tests in the guest.
#
#   powershell -ExecutionPolicy Bypass -File docs\notes\vm-run-tests.ps1 -Filter rustamp_app
#   powershell ... -Reset            # restore base-provisioned first
#
# The division of labour is deliberate and security-relevant:
#   * HERE (unelevated): compile and stage. Compiling runs build scripts from
#     the repository, so it must never happen with administrator rights.
#   * THE BROKER (elevated, session 0): only touches the VM - eject the DVD,
#     start, copy in over PowerShell Direct, run an allow-listed guest script.
#
# Why the guest can run the binaries unchanged: the tests resolve the repository,
# the fixtures and the built binary through compile-time paths
# (D:\code\Rustamp\...), and the guest script does `subst D: C:\rustamp`, so
# those exact paths exist in the guest too. The install DVD is ejected first,
# because it would otherwise hold D: in the guest.

[CmdletBinding()]
param(
    [string]$Filter = '',
    [switch]$Reset,
    [string]$Baseline = 'base-provisioned',
    [string]$RepoRoot = 'D:\code\Rustamp',
    [string]$Stage = 'D:\code\rustamp-vm\staging\code\Rustamp',
    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'
$broker = Join-Path $PSScriptRoot 'vm-broker\invoke-broker.ps1'

function Step($t) { Write-Host "--- $t" }

# --- 1. compile on the host (builds, does not run) ---------------------------
if (-not $SkipBuild) {
    Step 'compiling on the host (cargo test --no-run; nothing is executed)'
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    $json = & cargo test --workspace --all-features --no-run --message-format=json 2>$null
    $buildExit = $LASTEXITCODE
    $ErrorActionPreference = $previous
    # A build that fails part way still emits artifacts for what it managed to
    # compile, so a silent failure would quietly shrink the set of binaries that
    # get run - and a run that covers fewer tests would still report "green".
    # The build must therefore succeed, and the count must be plausible.
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

# --- 2. stage ----------------------------------------------------------------
Step "staging into $Stage"
if (Test-Path $Stage) { Remove-Item -Recurse -Force $Stage }
$null = New-Item -ItemType Directory -Force -Path (Join-Path $Stage 'target\debug\deps')
foreach ($exe in $testExes) { Copy-Item $exe (Join-Path $Stage 'target\debug\deps') -Force }
$debugDir = Join-Path $RepoRoot 'target\debug'
foreach ($dll in Get-ChildItem -Path (Join-Path $debugDir 'deps') -Filter '*.dll' -ErrorAction SilentlyContinue) {
    Copy-Item $dll.FullName (Join-Path $Stage 'target\debug\deps') -Force
}
if (Test-Path (Join-Path $debugDir 'rustamp.exe')) { Copy-Item (Join-Path $debugDir 'rustamp.exe') (Join-Path $Stage 'target\debug') -Force }

# The plugin host is built for i686 — the app is 64-bit and the host 32-bit by
# design (RFC 0006 §8) — so a plain workspace test build produces none and the
# supervisor finds nothing to spawn. Build it and stage it where the resolver
# looks first, next to the binary that will run, so the ignored end-to-end test
# can exercise the whole chain here, in the guest, which is the only place it
# may run. A missing host is reported, not hidden: the test then fails loudly
# rather than passing vacuously.
Step 'building the i686 plugin host for the end-to-end test'
$prevPref = $ErrorActionPreference
$ErrorActionPreference = 'Continue'
# The host links the C runtime statically here. It is a 32-bit binary that must
# sit beside a 64-bit application, and the two cannot share CRT DLLs in one
# directory (same name, different architecture), so a self-contained host is the
# only layout that works. Without this the host dies the moment it starts and the
# end-to-end test sees the channel close.
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
        # Windows searches the DLLs in the EXE's own directory first, and the
        # test binaries live in deps\ - so the runtime has to be there too, not
        # only next to rustamp.exe. Missing it is 0xC0000135.
        Copy-Item $p (Join-Path $Stage 'target\debug') -Force
        Copy-Item $p (Join-Path $Stage 'target\debug\deps') -Force
    }
}
Copy-Item (Join-Path $RepoRoot 'crates') (Join-Path $Stage 'crates') -Recurse -Force
# The workspace root files too: `rustamp-testkit`'s golden test walks up from its
# manifest directory looking for the workspace root, and without Cargo.toml it
# fails with "workspace root not found" (staged crates alone are not enough).
foreach ($rootFile in 'Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml') {
    $p = Join-Path $RepoRoot $rootFile
    if (Test-Path $p) { Copy-Item $p (Join-Path $Stage $rootFile) -Force }
}
# And the committed golden images: the lookalike cases compare against
# `tests/golden/lookalike/*.png` at the workspace root, so without this every
# golden test fails in the guest with "golden missing".
$golden = Join-Path $RepoRoot 'tests\golden'
if (Test-Path $golden) {
    $goldenDest = Join-Path $Stage 'tests\golden'
    New-Item -ItemType Directory -Force -Path (Split-Path $goldenDest) | Out-Null
    Copy-Item $golden $goldenDest -Recurse -Force
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

# --- 4. copy in and run ------------------------------------------------------
Step 'copying the staged tree into the guest'
& $broker -Op copyin -Source $Stage -Dest 'C:\rustamp\code\Rustamp' -TimeoutSeconds 1800 | Out-Null

# Real test media (copied once from the owner's archive, never modified there):
# if the directory exists on the host it is staged as-is, so tests that opt in
# via RUSTAMP_TESTDATA find real decodable files in the guest. Absent means
# those tests skip, exactly as on a machine without the archive. The broker
# only copies from under the staging root, so the media passes through a
# staging copy first (a blind copy like everything else).
$mediaDir = 'D:\code\rustamp-testmedia'
if (Test-Path $mediaDir) {
    Step 'copying real test media into the guest (read-only copies)'
    $mediaStage = Join-Path (Split-Path $Stage) 'testmedia'
    if (Test-Path $mediaStage) { Remove-Item -Recurse -Force $mediaStage }
    Copy-Item $mediaDir $mediaStage -Recurse -Force
    & $broker -Op copyin -Source $mediaStage -Dest 'C:\rustamp\testmedia' -TimeoutSeconds 600 | Out-Null
} else {
    Write-Host '    no real test media on the host; media-dependent tests will skip'
}

# Real test skins, the same contract as the media above: a directory of `.wsz`
# files and/or skin folders staged read-only, exposed to the guest as
# RUSTAMP_TESTSKINS. Tests that need a real classic skin opt in and skip when it
# is absent, so a checkout without it still passes.
$skinsDir = 'D:\code\rustamp-testskins'
if (Test-Path $skinsDir) {
    Step 'copying test skins into the guest (read-only copies)'
    $skinsStage = Join-Path (Split-Path $Stage) 'testskins'
    if (Test-Path $skinsStage) { Remove-Item -Recurse -Force $skinsStage }
    Copy-Item $skinsDir $skinsStage -Recurse -Force
    & $broker -Op copyin -Source $skinsStage -Dest 'C:\rustamp\testskins' -TimeoutSeconds 600 | Out-Null
} else {
    Write-Host '    no test skins on the host; skin-dependent tests will skip'
}

Step 'running the tests in the guest'
# The guest bounds each binary at 300 s (`$TestTimeoutSeconds` in
# docs\notes\vm-broker\guest\run-tests.ps1) and is meant to keep going after a
# hung one. This bound has to cover every binary in that worst case, or a run
# with several hangs would be cut off by the broker before the report exists -
# which is the very "the run reports nothing at all" failure the per-binary
# timeout exists to prevent. The two constants cannot share a value across the
# host/guest boundary (that is what the allow-list is), so they are kept in step
# by hand and named here.
$PerBinaryTimeoutSeconds = 300
$guestTimeoutSeconds = 60 + (($PerBinaryTimeoutSeconds + 60) * [math]::Max($testExes.Count, 1))
Write-Host ("    per-binary timeout {0} s; broker bound {1} s for {2} binaries" -f $PerBinaryTimeoutSeconds, $guestTimeoutSeconds, $testExes.Count)
# A single positional argument: `-ArgumentList` with an array binds only the
# first element to the script's parameter, which silently dropped the filter.
$result = & $broker -Op guest -Script 'run-tests.ps1' -GuestArgs $Filter -TimeoutSeconds $guestTimeoutSeconds
# A guest run that produced no object produced no report at all: say which step
# lost it instead of printing empty columns below.
if (-not $result) { throw "STEP guest FAILED: the broker returned no result object for run-tests.ps1 (see D:\code\rustamp-vm\broker\broker.log)" }
if ($null -eq $result.results) { throw "STEP guest FAILED: the guest result object carries no results (see D:\code\rustamp-vm\broker\broker.log)" }

# --- 5. results --------------------------------------------------------------
$hungCount = @($result.results | Where-Object { $_.hung }).Count
Write-Host ''
Write-Host ("guest {0} as {1}; subst resolved: {2}; ran {3}, failed {4}, hung {5}" -f $result.host, $result.user, $result.subst, $result.ran, $result.failed, $hungCount)
$result.results | ForEach-Object {
    # A hung binary must be impossible to mistake for a pass or for a missing binary.
    if ($_.hung) {
        # The seconds come from the result, not from $_.exit: the exit code of a
        # killed binary is a kill artefact, and printing it as the timeout would
        # name a timeout of "-1 seconds".
        $bound = if ($_.timeoutSeconds) { 'timeout of {0} seconds' -f $_.timeoutSeconds } else { 'a timeout whose length the guest did not report' }
        Write-Host ("  [HNG] {0,-46} {1}" -f $_.exe, $_.summary)
        Write-Host ("         HUNG after {0} - binary was killed, last output captured" -f $bound)
        foreach ($d in $_.diagnostic) { Write-Host ("         | $d") }
    } else {
        $status = if ($_.exit -eq 0) { 'ok  ' } else { 'FAIL' }
        Write-Host ("  [{0}] {1,-46} {2}" -f $status, $_.exe, $_.summary)
        foreach ($f in $_.failed) { Write-Host ("         $f") }
        foreach ($d in $_.diagnostic) { Write-Host ("         | $d") }
    }
}
# A stall must never be reported as a green run: the hung count is a second,
# independent gate on the exit code, so a counting drift cannot hide a hang.
if ($result.failed -ne 0 -or $hungCount -ne 0) { exit 1 }
