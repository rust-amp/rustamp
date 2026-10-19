# Compile and stage on the host (unelevated), run the suite in the guest's
# INTERACTIVE session by default (owner rule, 2026-10-18). Session 0 has no
# desktop: window, dialog, menu and plugin-host failures there are not product
# evidence. The same path handles normal, --ignored and --bless runs.
#
#   powershell -ExecutionPolicy Bypass -File docs\notes\vm-run-tests.ps1 -Reset
#   powershell -ExecutionPolicy Bypass -File docs\notes\vm-run-tests.ps1 -Filter rustamp_app
#   powershell ... -Reset            # restore base-provisioned first
#   powershell ... -Filter dsp_roundtrip -Repeat 20 -RepeatSummaryPath rate.json
#   powershell ... -DesktopFreeResidentFixtures -Filter '--ignored:pcm_process'
# The last form deliberately runs ONLY the desktop-free resident fixture tests
# (pcm_process or dsp_process); it is not a general session-0 escape hatch.
# Before guest copyin, stray staged plugin hosts are killed and awaited, not
# hidden by a copy retry. This adds one broker round trip, not a guest reboot.
# Repeats build/stage/copy once, then run sequentially in the same guest. -Reset
# restores once, not between repetitions. Wall times cover each guest request.
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
    [switch]$SkipBuild,
    [switch]$DesktopFreeResidentFixtures,
    [string]$SessionOut = 'D:\code\rustamp-vm\staging\out\gui-session',
    [ValidateRange(1, 2147483647)] [int]$Repeat = 1,
    [string]$RepeatSummaryPath = '',
    # Where the re-blessed golden images land on the host (only with --bless).
    [string]$BlessOut = 'D:\code\rustamp-vm\staging\out\bless-golden'
)

$ErrorActionPreference = 'Stop'
$broker = Join-Path $PSScriptRoot 'vm-broker\invoke-broker.ps1'
$runnerSource = Join-Path $PSScriptRoot 'vm-broker\guest\run-tests.ps1'
if ($DesktopFreeResidentFixtures -and $Filter -cnotin '--ignored:pcm_process', '--ignored:dsp_process') {
    throw '-DesktopFreeResidentFixtures requires exactly -Filter --ignored:pcm_process or --ignored:dsp_process; no other session-0 selection is allowed.'
}

function Step($t) { Write-Host "--- $t" }

$RepoRoot = [IO.Path]::GetFullPath($RepoRoot).TrimEnd('\')
$Stage = [IO.Path]::GetFullPath($Stage).TrimEnd('\')
$receiptPath = Join-Path $Stage 'stage-receipt.json'

# A receipt is written LAST, only after a successful full build and staging.
# Hashes prove completeness, not just a plausible count. Old stages without a
# receipt cannot prove anything and are deliberately refused (never rebuilt).
function Get-Inventory($Files, [string]$Root) {
    foreach ($file in @($Files | Sort-Object FullName -Unique)) {
        [pscustomobject]@{
            path = $file.FullName.Substring($Root.Length + 1)
            hash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash
        }
    }
}

function Get-WorkspaceStageInventory {
    # Include source/build inputs as well as current artifacts: a changed test
    # target or new crate must invalidate the old Cargo artifact selection.
    $files = @()
    foreach ($dir in 'crates', 'xtask', '.cargo', 'tests\golden') {
        $p = Join-Path $RepoRoot $dir
        if (Test-Path -LiteralPath $p) { $files += @(Get-ChildItem -LiteralPath $p -Recurse -File -Force) }
    }
    foreach ($name in 'Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'rust-toolchain') {
        $p = Join-Path $RepoRoot $name
        if (Test-Path -LiteralPath $p) { $files += Get-Item -LiteralPath $p }
    }
    foreach ($dir in 'target\debug', 'target\debug\deps', 'target\debug\examples', 'target\i686-pc-windows-msvc\debug', 'target\i686-pc-windows-msvc\debug\deps', 'target\i686-pc-windows-msvc\debug\examples') {
        $p = Join-Path $RepoRoot $dir
        if (Test-Path -LiteralPath $p) {
            $files += @(Get-ChildItem -LiteralPath $p -File | Where-Object { $_.Extension -in '.exe', '.dll' })
        }
    }
    Get-Inventory $files $RepoRoot
    [pscustomobject]@{ path = 'vm-test-runner\run-tests.ps1'; hash = (Get-FileHash -LiteralPath $runnerSource -Algorithm SHA256).Hash }
    foreach ($crt in 'vcruntime140.dll', 'vcruntime140_1.dll', 'msvcp140.dll') {
        $p = Join-Path $env:SystemRoot "System32\$crt"
        if (Test-Path -LiteralPath $p) {
            [pscustomobject]@{ path = "System32\$crt"; hash = (Get-FileHash -LiteralPath $p -Algorithm SHA256).Hash }
        }
    }
}

function Assert-Inventory($Expected, $Actual, [string]$Label) {
    $known = @{}
    foreach ($entry in $Actual) { $known[$entry.path] = $entry.hash }
    $problems = @()
    foreach ($entry in $Expected) {
        if (-not $known.ContainsKey($entry.path)) { $problems += "missing $Label file: $($entry.path)" }
        elseif ($known[$entry.path] -ne $entry.hash) { $problems += "changed $Label file: $($entry.path)" }
        $known.Remove($entry.path)
    }
    foreach ($name in $known.Keys) { $problems += "unexpected $Label file: $name" }
    if ($problems) {
        throw ("-SkipBuild refused: staging is incomplete or no longer matches the current workspace.`n    " +
            ($problems -join "`n    ") + "`nRun once without -SkipBuild to create a complete staging; no build or guest operation was attempted.")
    }
}

# The general-purpose plugin fixture of the P3 DLL acceptance is a cdylib
# (crates/rustamp-plughost/examples/gen_rustamp_fixture.rs) built for i686 by
# the test build below. The acceptance test loads it through the OS loader,
# so it has to travel with the test binaries; it is never linked into them.
$genFixtureDll = Join-Path $RepoRoot 'target\i686-pc-windows-msvc\debug\examples\gen_rustamp_fixture.dll'
$genFixtureStaged = $false

# The output plugin fixture (crates/rustamp-plughost/examples/out_rustamp_fixture.rs)
# is the same kind of artifact for the `out_` plugin kind: our own clean-room
# cdylib, built for i686 by the same test build, loaded through the OS loader.
$outFixtureDll = Join-Path $RepoRoot 'target\i686-pc-windows-msvc\debug\examples\out_rustamp_fixture.dll'
$outFixtureStaged = $false

# The input plugin fixture (crates/rustamp-plughost/examples/in_rustamp_fixture.rs)
# is the same kind of artifact for the `in_` plugin kind: our own clean-room
# cdylib, built for i686 by the same test build, loaded through the OS loader.
# It opens no window and no audio device.
$inFixtureDll = Join-Path $RepoRoot 'target\i686-pc-windows-msvc\debug\examples\in_rustamp_fixture.dll'
$inFixtureStaged = $false

# The visualization and DSP fixtures (crates/rustamp-plughost/examples/
# vis_rustamp_fixture.rs and dsp_rustamp_fixture.rs) are the same kind of
# artifact for the `vis_` and `dsp_` plugin kinds: own clean-room cdylibs,
# built for i686 by the same test build, loaded through the OS loader.
$visFixtureDll = Join-Path $RepoRoot 'target\i686-pc-windows-msvc\debug\examples\vis_rustamp_fixture.dll'
$visFixtureStaged = $false
$dspFixtureDll = Join-Path $RepoRoot 'target\i686-pc-windows-msvc\debug\examples\dsp_rustamp_fixture.dll'
$dspFixtureStaged = $false

# The crashing plugin fixture (crates/rustamp-plughost/examples/
# crash_rustamp_fixture.rs) is the same kind of artifact for the crash
# isolation acceptance (PLAN M3): our own clean-room cdylib, built for i686 by
# the same test build, loaded through the OS loader. It faults on purpose, so
# it must only ever run inside the guest.
$crashFixtureDll = Join-Path $RepoRoot 'target\i686-pc-windows-msvc\debug\examples\crash_rustamp_fixture.dll'
$crashFixtureStaged = $false

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
        if ($msg.reason -eq 'compiler-artifact' -and $msg.executable -and $msg.profile.test) {
            # rustamp-plughost is an i686 crate by design (RFC 0006 §8); its tests
            # are compiled for i686-pc-windows-msvc below to support 32-bit plugin DLL execution.
            if ($msg.package_id -match 'rustamp-plughost') { continue }
            $testExes += $msg.executable
        }
    }
    if (-not $testExes) { throw 'no test binaries were produced' }
    Write-Host ("    {0} test binaries" -f $testExes.Count)

    Step 'building mediahost resident fixture examples (decoder_fixture, dsp_fixture)'
    $prevPref = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    & cargo build --offline -p rustamp-mediahost --example decoder_fixture --example dsp_fixture 2>$null | Out-Null
    $fixtureBuildExit = $LASTEXITCODE
    $ErrorActionPreference = $prevPref
    if ($fixtureBuildExit -ne 0) { throw "cargo build examples decoder_fixture / dsp_fixture failed with exit $fixtureBuildExit" }

    Step 'building the real decoder resident (rustamp-dechost-symphonia)'
    # The engine adapter's two-process tests (crates/rustamp-audio/tests/
    # resident_process.rs) spawn the real resident. It is a workspace binary,
    # so it is built explicitly here: the test build only produces its test
    # harness, not the plain executable the test spawns.
    $prevPref = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    & cargo build --offline -p rustamp-dechost --bin rustamp-dechost-symphonia 2>$null | Out-Null
    $dechostBuildExit = $LASTEXITCODE
    $ErrorActionPreference = $prevPref
    if ($dechostBuildExit -ne 0) { throw "cargo build -p rustamp-dechost failed with exit $dechostBuildExit" }
} else {
    Step 'skipping the build (-SkipBuild); the previously staged binaries are reused'
    if (-not (Test-Path -LiteralPath $receiptPath -PathType Leaf)) {
        throw "-SkipBuild refused: missing $receiptPath; this staging has no completion receipt, so its completeness cannot be proved. Run once without -SkipBuild; no automatic rebuild or guest operation was attempted."
    }
    try { $receipt = Get-Content -LiteralPath $receiptPath -Raw | ConvertFrom-Json } catch {
        throw "-SkipBuild refused: unreadable completion receipt $receiptPath; staging completeness cannot be proved. Run once without -SkipBuild."
    }
    if ($receipt.version -ne 2 -or $receipt.repo -ne $RepoRoot -or -not $receipt.files -or -not $receipt.inputs -or -not $receipt.testExes -or -not $receipt.i686TestExes) {
        throw "-SkipBuild refused: invalid completion receipt $receiptPath for workspace $RepoRoot; staging completeness cannot be proved. Run once without -SkipBuild."
    }
    $stagedFiles = @(Get-ChildItem -LiteralPath $Stage -Recurse -File -Force | Where-Object { $_.FullName -ne $receiptPath })
    Assert-Inventory $receipt.files @(Get-Inventory $stagedFiles $Stage) 'staged'
    Assert-Inventory $receipt.inputs @(Get-WorkspaceStageInventory) 'workspace/runtime input'
    $testExes = @($receipt.testExes)
    $i686TestExes = @($receipt.i686TestExes)
    # The staging receipt covers the fixture file itself, so its presence in
    # the staged tree is the ground truth here.
    $genFixtureStaged = Test-Path (Join-Path $Stage 'target\debug\deps\gen_rustamp_fixture.dll')
    $outFixtureStaged = Test-Path (Join-Path $Stage 'target\debug\deps\out_rustamp_fixture.dll')
    $inFixtureStaged = Test-Path (Join-Path $Stage 'target\debug\deps\in_rustamp_fixture.dll')
    $crashFixtureStaged = Test-Path (Join-Path $Stage 'target\debug\deps\crash_rustamp_fixture.dll')
    Write-Host ("    verified {0} staged files against the complete receipt and current workspace/runtime inputs" -f $receipt.files.Count)
}

# --- 2. stage ----------------------------------------------------------------
if (-not $SkipBuild) {
Step "staging into $Stage"
if (Test-Path $Stage) { Remove-Item -Recurse -Force $Stage }
$null = New-Item -ItemType Directory -Force -Path (Join-Path $Stage 'target\debug\deps')
$null = New-Item -ItemType Directory -Force -Path (Join-Path $Stage 'target\debug\examples')
foreach ($exe in $testExes) { Copy-Item $exe (Join-Path $Stage 'target\debug\deps') -Force }
$debugDir = Join-Path $RepoRoot 'target\debug'
foreach ($dll in Get-ChildItem -Path (Join-Path $debugDir 'deps') -Filter '*.dll' -ErrorAction SilentlyContinue) {
    Copy-Item $dll.FullName (Join-Path $Stage 'target\debug\deps') -Force
}
if (Test-Path (Join-Path $debugDir 'rustamp.exe')) { Copy-Item (Join-Path $debugDir 'rustamp.exe') (Join-Path $Stage 'target\debug') -Force }

# Stage mediahost resident fixture examples (decoder_fixture, dsp_fixture)
$decoderFixtureExe = Join-Path $debugDir 'examples\decoder_fixture.exe'
$dspFixtureExe = Join-Path $debugDir 'examples\dsp_fixture.exe'
if (Test-Path $decoderFixtureExe) {
    Copy-Item $decoderFixtureExe (Join-Path $Stage 'target\debug\examples') -Force
    Copy-Item $decoderFixtureExe (Join-Path $Stage 'target\debug') -Force
    Write-Host '    staged decoder_fixture example'
}
if (Test-Path $dspFixtureExe) {
    Copy-Item $dspFixtureExe (Join-Path $Stage 'target\debug\examples') -Force
    Copy-Item $dspFixtureExe (Join-Path $Stage 'target\debug') -Force
    Write-Host '    staged dsp_fixture example'
}

# The real decoder resident (RFC 0006 §5). The engine adapter's two-process
# tests spawn it and look for it next to the test binaries, so it has to sit
# in target\debug beside them.
$dechostExe = Join-Path $debugDir 'rustamp-dechost-symphonia.exe'
if (Test-Path $dechostExe) {
    Copy-Item $dechostExe (Join-Path $Stage 'target\debug') -Force
    Write-Host '    staged the real decoder resident'
} else {
    Write-Host '    WARNING: no real decoder resident was built; the two-process engine test reports itself skipped'
}

# The plugin host is built for i686 — the app is 64-bit and the host 32-bit by
# design (RFC 0006 §8) — so a plain workspace test build produces none and the
# supervisor finds nothing to spawn. Build both the host binary and its test
# executables for i686-pc-windows-msvc with static CRT so the ignored end-to-end
# and DLL acceptance tests can exercise 32-bit plugin loading in the guest.
Step 'building the i686 plugin host and plughost test binaries'
$prevPref = $ErrorActionPreference
$ErrorActionPreference = 'Continue'
$prevRustflags = $env:RUSTFLAGS
$env:RUSTFLAGS = '-C target-feature=+crt-static'
& cargo build -p rustamp-plughost --target i686-pc-windows-msvc 2>$null | Out-Null
$plughostBuildExit = $LASTEXITCODE
$plughostTestJson = & cargo test -p rustamp-plughost --all-targets --all-features --no-run --target i686-pc-windows-msvc --message-format=json 2>$null
$plughostTestExit = $LASTEXITCODE
if ($null -eq $prevRustflags) { Remove-Item Env:\RUSTFLAGS -ErrorAction SilentlyContinue } else { $env:RUSTFLAGS = $prevRustflags }
$ErrorActionPreference = $prevPref
if ($plughostBuildExit -ne 0) { throw "cargo build -p rustamp-plughost --target i686-pc-windows-msvc failed with exit $plughostBuildExit" }
if ($plughostTestExit -ne 0) { throw "cargo test -p rustamp-plughost --no-run --target i686-pc-windows-msvc failed with exit $plughostTestExit" }

$hostExe = Join-Path $RepoRoot 'target\i686-pc-windows-msvc\debug\rustamp-plughost32.exe'
if (Test-Path $hostExe) {
    Copy-Item $hostExe (Join-Path $Stage 'target\debug\deps') -Force
    Copy-Item $hostExe (Join-Path $Stage 'target\debug') -Force
    $i686TargetDebug = Join-Path $Stage 'target\i686-pc-windows-msvc\debug'
    $null = New-Item -ItemType Directory -Force -Path $i686TargetDebug
    Copy-Item $hostExe $i686TargetDebug -Force
    Write-Host '    staged the i686 plugin host beside the binaries'
} else {
    Write-Host '    WARNING: no i686 plugin host was built; the end-to-end test will report it blocked'
}

$i686TestExes = @()
foreach ($line in $plughostTestJson) {
    if ($line -isnot [string]) { continue }
    try { $msg = $line | ConvertFrom-Json } catch { continue }
    if ($msg.reason -eq 'compiler-artifact' -and $msg.executable -and $msg.profile.test) {
        $i686TestExes += $msg.executable
    }
}
$i686TargetDeps = Join-Path $Stage 'target\i686-pc-windows-msvc\debug\deps'
$null = New-Item -ItemType Directory -Force -Path $i686TargetDeps
foreach ($exe in $i686TestExes) {
    Copy-Item $exe (Join-Path $Stage 'target\debug\deps') -Force
    Copy-Item $exe $i686TargetDeps -Force
}
if ($i686TestExes.Count -gt 0) {
    Write-Host ("    staged {0} i686 plughost test binaries beside the binaries" -f $i686TestExes.Count)
}

# The general-purpose plugin fixture travels with the test binaries: the
# acceptance test loads it through the OS loader, never linked into them.
if (Test-Path $genFixtureDll) {
    Copy-Item $genFixtureDll (Join-Path $Stage 'target\debug\deps') -Force
    Copy-Item $genFixtureDll $i686TargetDeps -Force
    $genFixtureStaged = $true
    Write-Host '    staged the i686 general-purpose plugin fixture beside the binaries'
} else {
    Write-Host '    WARNING: no i686 general-purpose plugin fixture was built; its acceptance test will report it blocked'
}

# The output plugin fixture travels with the test binaries for the same reason.
if (Test-Path $outFixtureDll) {
    Copy-Item $outFixtureDll (Join-Path $Stage 'target\debug\deps') -Force
    Copy-Item $outFixtureDll $i686TargetDeps -Force
    $outFixtureStaged = $true
    Write-Host '    staged the i686 output plugin fixture beside the binaries'
} else {
    Write-Host '    WARNING: no i686 output plugin fixture was built; its acceptance test will report it blocked'
}

# The input plugin fixture travels with the test binaries for the same reason.
if (Test-Path $inFixtureDll) {
    Copy-Item $inFixtureDll (Join-Path $Stage 'target\debug\deps') -Force
    Copy-Item $inFixtureDll $i686TargetDeps -Force
    $inFixtureStaged = $true
    Write-Host '    staged the i686 input plugin fixture beside the binaries'
} else {
    Write-Host '    WARNING: no i686 input plugin fixture was built; its acceptance test will report it blocked'
}

# The visualization and DSP plugin fixtures travel with the test binaries too.
if (Test-Path $visFixtureDll) {
    Copy-Item $visFixtureDll (Join-Path $Stage 'target\debug\deps') -Force
    Copy-Item $visFixtureDll $i686TargetDeps -Force
    $visFixtureStaged = $true
    Write-Host '    staged the i686 visualization plugin fixture beside the binaries'
} else {
    Write-Host '    WARNING: no i686 visualization plugin fixture was built; its acceptance test will report it blocked'
}
if (Test-Path $dspFixtureDll) {
    Copy-Item $dspFixtureDll (Join-Path $Stage 'target\debug\deps') -Force
    Copy-Item $dspFixtureDll $i686TargetDeps -Force
    $dspFixtureStaged = $true
    Write-Host '    staged the i686 DSP plugin fixture beside the binaries'
} else {
    Write-Host '    WARNING: no i686 DSP plugin fixture was built; its acceptance test will report it blocked'
}

# The crashing plugin fixture travels with the test binaries too: the crash
# isolation acceptance loads it through the OS loader and lets it fault.
if (Test-Path $crashFixtureDll) {
    Copy-Item $crashFixtureDll (Join-Path $Stage 'target\debug\deps') -Force
    Copy-Item $crashFixtureDll $i686TargetDeps -Force
    $crashFixtureStaged = $true
    Write-Host '    staged the i686 crashing plugin fixture beside the binaries'
} else {
    Write-Host '    WARNING: no i686 crashing plugin fixture was built; its acceptance test will report it blocked'
}

foreach ($crt in 'vcruntime140.dll', 'vcruntime140_1.dll', 'msvcp140.dll') {
    $p = Join-Path $env:SystemRoot "System32\$crt"
    if (Test-Path $p) {
        # Windows searches the DLLs in the EXE's own directory first, and the
        # test binaries live in deps\ - so the runtime has to be there too, not
        # only next to rustamp.exe. Missing it is 0xC0000135.
        Copy-Item $p (Join-Path $Stage 'target\debug') -Force
        Copy-Item $p (Join-Path $Stage 'target\debug\deps') -Force
        Copy-Item $p (Join-Path $Stage 'target\debug\examples') -Force
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
# The scheduled task reuses this exact runner, including its timeouts, fixture
# environment and result schema; never maintain a second, smaller suite here.
$runnerStage = Join-Path $Stage 'vm-test-runner'
$null = New-Item -ItemType Directory -Force -Path $runnerStage
Copy-Item -LiteralPath $runnerSource -Destination (Join-Path $runnerStage 'run-tests.ps1') -Force
$stagedFiles = @(Get-ChildItem -LiteralPath $Stage -Recurse -File -Force)
[pscustomobject]@{
    version = 2
    repo = $RepoRoot
    testExes = @($testExes)
    i686TestExes = @($i686TestExes)
    files = @(Get-Inventory $stagedFiles $Stage)
    inputs = @(Get-WorkspaceStageInventory)
} | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $receiptPath -Encoding UTF8
}

# Mirror the guest's binary-name filter and helper exclusions without launching
# any binary on the host. Keep the guest's ran=0 gate too: this is only preflight.
$binaryFilter = $Filter
if ($binaryFilter -like '--bless*') { $binaryFilter = $binaryFilter -replace '^--bless:?', '' }
if ($binaryFilter -like '--ignored*') { $binaryFilter = $binaryFilter -replace '^--ignored:?', '' }
$selected = @(Get-ChildItem -LiteralPath (Join-Path $Stage 'target\debug\deps') -Filter '*.exe' -File | Where-Object {
    $_.Name -notin 'rustamp-plughost32.exe', 'rustamp.exe' -and (-not $binaryFilter -or $_.Name -like "*$binaryFilter*")
})
if ($selected.Count -eq 0) {
    throw ("host preflight FAILED: filter '{0}' matches no staged test binary ({1} test artifacts); no guest operation was attempted." -f $Filter, ($testExes.Count + $i686TestExes.Count))
}
Write-Host ("    host selection: {0} test binaries (filter: '{1}')" -f $selected.Count, $Filter)

# --- 3. optional reset, then start (the broker ejects the DVD) --------------
if ($Reset) {
    Step "restoring $Baseline"
    & $broker -Op restore -Name $Baseline -TimeoutSeconds 900 | Out-Null
} else {
    Step 'starting the guest (the broker ejects the install DVD so the guest gets D:)'
    & $broker -Op start -TimeoutSeconds 900 | Out-Null
}

# --- 4. copy in and run ------------------------------------------------------
Step 'clearing stray staged plugin hosts in the guest before copyin'
$cleanupClock = [Diagnostics.Stopwatch]::StartNew()
try {
    & $broker -Op guest -Script 'prepare-test-staging.ps1' -TimeoutSeconds 120 | Out-Null
} finally { $cleanupClock.Stop() }
Write-Host ("    pre-staging cleanup: {0:F3} s (no reboot)" -f $cleanupClock.Elapsed.TotalSeconds)
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

# Real test plugins: staged read-only copies of the owner's testplugins directory
# (D:\code\rustamp-testplugins) into C:\rustamp\testplugins in the guest so
# DLL acceptance tests find the approved in_openmpt.dll plugin.
# $pluginsStaged records the host-side staging outcome so the report (step 5
# and the ignored summary in step 6) can say plainly whether the Group D
# fixture was staged; a missing fixture must never look like a pass.
$pluginsStaged = $false
$pluginsDir = 'D:\code\rustamp-testplugins'
# The classic DSP fixture (P8) travels inside the same wholesale copy, at
# dsp\dsp_optilab_core.dll. $dspHostState is what the host knows before the
# guest speaks: 'absent' (no file), 'wrong-hash' (a file that is not the pinned
# fixture - never staged as the approved one) or 'present'.
$dspFixtureName = 'dsp_optilab_core.dll'
$dspFixtureSha256 = '78A140C9114B60B361DC7E9A0203DD6D6F23D567E1F309396CD78BAEA7797874'
$dspHostFile = Join-Path $pluginsDir "dsp\$dspFixtureName"
$dspHostState = 'absent'
if (Test-Path $dspHostFile) {
    $dspHostHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $dspHostFile).Hash
    $dspHostState = if ($dspHostHash -eq $dspFixtureSha256) { 'present' } else { 'wrong-hash' }
}
if (Test-Path $pluginsDir) {
    Step 'copying test plugins into the guest (read-only copies)'
    $pluginsStage = Join-Path (Split-Path $Stage) 'testplugins'
    if (Test-Path $pluginsStage) { Remove-Item -Recurse -Force $pluginsStage }
    Copy-Item $pluginsDir $pluginsStage -Recurse -Force
    & $broker -Op copyin -Source $pluginsStage -Dest 'C:\rustamp\testplugins' -TimeoutSeconds 600 | Out-Null
    $pluginsStaged = $true
} else {
    Write-Host '    no test plugins on the host; plugin-dependent tests will skip/refuse'
}

function Invoke-GuestTestRun {
if ($DesktopFreeResidentFixtures) { Step 'running ONLY desktop-free resident fixtures in guest session 0' }
else { Step 'running the suite in the guest INTERACTIVE session' }
# The guest bounds each binary at 300 s (`$TestTimeoutSeconds` in
# docs\notes\vm-broker\guest\run-tests.ps1) and is meant to keep going after a
# hung one. This bound has to cover every binary in that worst case, or a run
# with several hangs would be cut off by the broker before the report exists -
# which is the very "the run reports nothing at all" failure the per-binary
# timeout exists to prevent. The two constants cannot share a value across the
# host/guest boundary (that is what the allow-list is), so they are kept in step
# by hand and named here.
$PerBinaryTimeoutSeconds = 300
$allBinariesCount = $testExes.Count + $i686TestExes.Count
$guestTimeoutSeconds = 180 + (($PerBinaryTimeoutSeconds + 60) * [math]::Max($allBinariesCount, 1))
Write-Host ("    per-binary timeout {0} s; broker bound {1} s for {2} binaries" -f $PerBinaryTimeoutSeconds, $guestTimeoutSeconds, $allBinariesCount)
# A single positional argument: `-ArgumentList` with an array binds only the
# first element to the script's parameter, which silently dropped the filter.
$guestClock = [Diagnostics.Stopwatch]::StartNew()
$guestScript = 'run-gui-session.ps1'
$guestFilter = $Filter
if ($DesktopFreeResidentFixtures) {
    $guestScript = 'run-tests.ps1'
    $guestFilter = '--desktop-free-residents:' + ($Filter -replace '^--ignored:', '')
}
try {
    $result = & $broker -Op guest -Script $guestScript -GuestArgs $guestFilter -TimeoutSeconds $guestTimeoutSeconds
} finally { $guestClock.Stop() }
# A guest run that produced no object produced no report at all: say which step
# lost it instead of printing empty columns below.
if (-not $result) { throw "STEP guest FAILED: the broker returned no result object for $guestScript (see D:\code\rustamp-vm\broker\broker.log)" }
if ($null -eq $result.results) { throw "STEP guest FAILED: the guest result object carries no results (see D:\code\rustamp-vm\broker\broker.log)" }
if (-not $DesktopFreeResidentFixtures) {
    if ($null -eq $result.session -or $result.session -le 0 -or -not $result.marker -or -not $result.screen) {
        throw 'STEP guest FAILED: no usable interactive-session proof; this run proves nothing.'
    }
    Step 'copying the session proof and per-binary logs back'
    if (Test-Path $SessionOut) { Remove-Item -Recurse -Force $SessionOut }
    $null = New-Item -ItemType Directory -Force -Path $SessionOut
    & $broker -Op copyout -Source 'C:\rustamp\out\gui-session' -Dest $SessionOut -TimeoutSeconds 600 | Out-Null
    # copyout nests the source directory inside the destination.
    $proofPath = Join-Path $SessionOut 'gui-session\session-proof.json'
    if (-not (Test-Path -LiteralPath $proofPath)) { throw 'no session-proof.json came back; nothing is promoted' }
    $proof = Get-Content -LiteralPath $proofPath -Raw | ConvertFrom-Json
    if ($proof.runId -ne $result.runId -or $proof.session -ne $result.session -or $proof.ran -ne $result.ran -or $proof.failed -ne $result.failed) {
        throw 'copied session proof does not match this guest run; nothing is promoted'
    }
    Write-Host ("    session {0}; screen {1}; proof and logs: {2}" -f $result.session, $result.screen, (Split-Path $proofPath))
}

# --- 5. results --------------------------------------------------------------
$hungCount = @($result.results | Where-Object { $_.hung }).Count
# Count repetitions with either the runner timeout or the resident's own escape
# as stalled. These remain failures, not a separate route around the exit gate.
$stallCount = @($result.results | Where-Object { $_.hung -or $_.stalled }).Count
Write-Host ''
Write-Host ("guest {0} as {1}; subst resolved: {2}; ran {3}, failed {4}, hung {5}" -f $result.host, $result.user, $result.subst, $result.ran, $result.failed, $hungCount)
# Plugin fixture status: the Group D loader tests prove nothing without the
# staged in_openmpt.dll, so the report names the staging outcome on every run,
# whatever the filter. Host-side staging throws on copy failure, so
# $pluginsStaged is ground truth for the host half; the guest half is confirmed
# when the guest script reports RUSTAMP_TESTPLUGINS back (older guest scripts
# predate that field and report nothing).
$guestPlugins = $result.testplugins
if ($pluginsStaged) {
    if ($guestPlugins) {
        Write-Host ("plugin fixture: staged (host D:\code\rustamp-testplugins copied; guest reports RUSTAMP_TESTPLUGINS={0})" -f $guestPlugins)
    } else {
        Write-Host 'plugin fixture: staged on the host, but the guest reported no RUSTAMP_TESTPLUGINS value; Group D tests cannot prove anything in this run'
    }
} else {
    Write-Host 'plugin fixture: NOT STAGED (no D:\code\rustamp-testplugins on the host, so RUSTAMP_TESTPLUGINS is unset); Group D tests refuse loudly and prove nothing in this run'
}
# DSP fixture status (P8), named on every run for the same reason: a missing or
# wrong DSP DLL must never read as a pass. The guest reports the path of the
# file it found and that file's SHA-256 (guest ground truth, after the copy).
$dspStaged = $false
$guestDspPath = $result.testplugindsp
$guestDspHash = $result.testplugindspsha256
if ($dspHostState -eq 'absent') {
    Write-Host ('dsp plugin fixture: NOT STAGED (no dsp\{0} under D:\code\rustamp-testplugins, so RUSTAMP_TESTPLUGIN_DSP is unset); the DSP acceptance test refuses loudly and proves nothing in this run' -f $dspFixtureName)
} elseif ($dspHostState -eq 'wrong-hash') {
    Write-Host ('dsp plugin fixture: WRONG FILE on the host (SHA-256 is not {0}); the DSP acceptance test refuses and proves nothing in this run' -f $dspFixtureSha256)
} elseif (-not $pluginsStaged) {
    Write-Host 'dsp plugin fixture: NOT STAGED (host staging did not run); the DSP acceptance test proves nothing in this run'
} elseif (-not $guestDspPath) {
    Write-Host 'dsp plugin fixture: staged on the host, but the guest reported no RUSTAMP_TESTPLUGIN_DSP value; the DSP acceptance test cannot prove anything in this run'
} elseif ($guestDspHash -ne $dspFixtureSha256) {
    Write-Host ('dsp plugin fixture: guest copy has SHA-256 {0}, not the pinned {1}; the DSP acceptance test proves nothing in this run' -f $guestDspHash, $dspFixtureSha256)
} else {
    $dspStaged = $true
    Write-Host ('dsp plugin fixture: staged and verified (guest RUSTAMP_TESTPLUGIN_DSP={0}, SHA-256 {1})' -f $guestDspPath, $guestDspHash)
}
# General-purpose plugin fixture status (P3): our own clean-room cdylib, built
# by the i686 test build and copied into the stage with the test binaries. It
# needs no third-party DLL, so it is staged whenever the build produced it.
if ($genFixtureStaged) {
    $genFixturePath = if (Test-Path $genFixtureDll) { $genFixtureDll } else { Join-Path $Stage 'target\debug\deps\gen_rustamp_fixture.dll' }
    Write-Host ('gen plugin fixture: staged (our own i686 cdylib, SHA-256 {0})' -f (Get-FileHash -LiteralPath $genFixturePath -Algorithm SHA256).Hash)
} else {
    Write-Host 'gen plugin fixture: NOT STAGED (the i686 build produced no general-purpose fixture); its acceptance test refuses loudly and proves nothing in this run'
}
# Output plugin fixture status (P3): the same kind of own cdylib, for the
# `out_` plugin kind. It needs no third-party DLL either.
if ($outFixtureStaged) {
    $outFixturePath = if (Test-Path $outFixtureDll) { $outFixtureDll } else { Join-Path $Stage 'target\debug\deps\out_rustamp_fixture.dll' }
    Write-Host ('out plugin fixture: staged (our own i686 cdylib, SHA-256 {0})' -f (Get-FileHash -LiteralPath $outFixturePath -Algorithm SHA256).Hash)
} else {
    Write-Host 'out plugin fixture: NOT STAGED (the i686 build produced no output fixture); its acceptance test refuses loudly and proves nothing in this run'
}
# Input plugin fixture status (P3): the same kind of own cdylib, for the
# `in_` plugin kind. It needs no third-party DLL either.
if ($inFixtureStaged) {
    $inFixturePath = if (Test-Path $inFixtureDll) { $inFixtureDll } else { Join-Path $Stage 'target\debug\deps\in_rustamp_fixture.dll' }
    Write-Host ('in plugin fixture: staged (our own i686 cdylib, SHA-256 {0})' -f (Get-FileHash -LiteralPath $inFixturePath -Algorithm SHA256).Hash)
} else {
    Write-Host 'in plugin fixture: NOT STAGED (the i686 build produced no input fixture); its acceptance test refuses loudly and proves nothing in this run'
}
# Crashing plugin fixture status (PLAN M3, crash isolation): the same kind of
# own cdylib, built by the i686 test build and copied into the stage with the
# test binaries. It faults on purpose, so it only ever runs in the guest.
if ($crashFixtureStaged) {
    $crashFixturePath = if (Test-Path $crashFixtureDll) { $crashFixtureDll } else { Join-Path $Stage 'target\debug\deps\crash_rustamp_fixture.dll' }
    Write-Host ('crash plugin fixture: staged (our own i686 cdylib, SHA-256 {0})' -f (Get-FileHash -LiteralPath $crashFixturePath -Algorithm SHA256).Hash)
} else {
    Write-Host 'crash plugin fixture: NOT STAGED (the i686 build produced no crashing fixture); its acceptance test refuses loudly and proves nothing in this run'
}
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
        $status = if ($_.exit -eq 0) {
            if ($_.summary -like 'LEFT OUT*') { 'SKIP' } else { 'ok  ' }
        } else { 'FAIL' }
        Write-Host ("  [{0}] {1,-46} {2}" -f $status, $_.exe, $_.summary)
        foreach ($f in $_.failed) { Write-Host ("         FAIL: $f") }
        foreach ($d in $_.diagnostic) { Write-Host ("         | $d") }
    }
}

# --- 5b. bless: bring the re-blessed golden images back (only with --bless) ---
# The guest staged them under C:\rustamp\out\bless (see run-tests.ps1). copyout
# nests the source folder inside the destination, so the files land at
# $BlessOut\bless\<skin>\<name>.png. The names must match the guest's $blessNames
# list; a missing image is a failure, not a silent partial promotion. Not run
# without the token: the default path is unchanged.
$blessNames = @('vis_stopped_tail', 'playlist_window_selection', 'playlist_window_mid_page')
$blessMissing = 0
if ($Filter -like '--bless*') {
    Step "copying the re-blessed golden images back to $BlessOut"
    if (Test-Path $BlessOut) { Remove-Item -Recurse -Force $BlessOut }
    $null = New-Item -ItemType Directory -Force -Path $BlessOut
    & $broker -Op copyout -Source 'C:\rustamp\out\bless' -Dest $BlessOut -TimeoutSeconds 600 | Out-Null
    foreach ($skin in 'lookalike', 'rust-theme') {
        foreach ($name in $blessNames) {
            $p = Join-Path $BlessOut "bless\$skin\$name.png"
            if (Test-Path $p) {
                Write-Host ("blessed: {0}  SHA-256 {1}" -f $p, (Get-FileHash -Algorithm SHA256 -LiteralPath $p).Hash)
            } else {
                $blessMissing++
                Write-Host ("  BLESS MISSING: {0} did not come back (the guest did not write it)" -f $p)
            }
        }
    }
}

# --- 6. ignored tests summary (W12 inventory) -------------------------------
if ($Filter -like '--ignored*') {
    Write-Host ''
    Write-Host '--- Ignored Tests Execution Summary (W12 Inventory) ---'

    $inventory = @(
        # Group A — VM GUI (8 tests)
        [pscustomobject]@{ Group = 'Group A (VM GUI)'; Name = 'the_menu_popup_opens_at_the_measured_size' },
        [pscustomobject]@{ Group = 'Group A (VM GUI)'; Name = 'the_windowed_player_opens_a_window_inside_the_test_vm' },
        [pscustomobject]@{ Group = 'Group A (VM GUI)'; Name = 'the_equalizer_window_opens_at_its_docked_size_and_position' },
        [pscustomobject]@{ Group = 'Group A (VM GUI)'; Name = 'the_main_menu_popup_opens_at_the_measured_client_size' },
        [pscustomobject]@{ Group = 'Group A (VM GUI)'; Name = 'the_playlist_window_opens_docked_below_the_main_window' },
        [pscustomobject]@{ Group = 'Group A (VM GUI)'; Name = 'the_new_dialogs_pin_their_os_window_parameters' },
        [pscustomobject]@{ Group = 'Group A (VM GUI)'; Name = 'opening_the_playlist_window_needs_an_os_window' },
        [pscustomobject]@{ Group = 'Group A (VM GUI)'; Name = 'opening_either_dialog_needs_an_os_window' },

        # Group B — VM real-process (3 tests)
        [pscustomobject]@{ Group = 'Group B (VM Real Process)'; Name = 'spawns_the_real_host_without_a_window' },
        [pscustomobject]@{ Group = 'Group B (VM Real Process)'; Name = 'app_published_state_reaches_the_host_window' },
        [pscustomobject]@{ Group = 'Group B (VM Real Process)'; Name = 'creates_the_real_host_window_vm_only' },

        # Group C — VM two-process resident fixtures (5 tests)
        [pscustomobject]@{ Group = 'Group C (Resident Fixtures)'; Name = 'named_pcm_two_process_transfer_and_seek' },
        [pscustomobject]@{ Group = 'Group C (Resident Fixtures)'; Name = 'pcm_process::host_exits_mid_stream_and_infinite_read_survives'; Search = 'host_exits_mid_stream_and_infinite_read_survives'; Binary = 'pcm_process' },
        [pscustomobject]@{ Group = 'Group C (Resident Fixtures)'; Name = 'pcm_process::killing_the_helper_mid_stream_is_a_loud_error_and_never_a_torn_frame'; Search = 'killing_the_helper_mid_stream_is_a_loud_error_and_never_a_torn_frame'; Binary = 'pcm_process' },
        [pscustomobject]@{ Group = 'Group C (Resident Fixtures)'; Name = 'named_dsp_two_process_gain_round_trip_then_sticky_eos' },
        [pscustomobject]@{ Group = 'Group C (Resident Fixtures)'; Name = 'dsp_process::host_exits_mid_stream_and_infinite_read_survives'; Search = 'host_exits_mid_stream_and_infinite_read_survives'; Binary = 'dsp_process' },

        # Group D — VM i686 plugin test + real approved DLL (2 tests)
        [pscustomobject]@{ Group = 'Group D (i686 DLL Acceptance)'; Name = 'loads_real_32bit_in_openmpt_dll_and_proves_host_interaction' },
        [pscustomobject]@{ Group = 'Group D (i686 DLL Acceptance)'; Name = 'approved_real_dll_maps_headlessly_from_verified_snapshot' },
        [pscustomobject]@{ Group = 'Group D (i686 DLL Acceptance)'; Name = 'real_dsp_plugin_loads_initializes_and_changes_audio_through_the_bridge'; Fixture = 'dsp' },
        [pscustomobject]@{ Group = 'Group D (i686 DLL Acceptance)'; Name = 'a_real_general_purpose_plugin_is_loaded_answers_from_its_own_code_and_quits_cleanly'; Fixture = 'gen' },
        [pscustomobject]@{ Group = 'Group D (i686 DLL Acceptance)'; Name = 'the_real_host_process_loads_a_general_purpose_dll_and_quits_it_cleanly'; Fixture = 'gen' },
        [pscustomobject]@{ Group = 'Group D (i686 DLL Acceptance)'; Name = 'a_real_input_plugin_is_loaded_filled_plays_and_quits_cleanly'; Fixture = 'in' },

        # Group E — Real audio endpoint (2 tests - left out pending decision)
        [pscustomobject]@{ Group = 'Group E (Real Audio Endpoint)'; Name = 'the_real_device_plays_a_reference_file_to_its_end'; Blocker = 'Real audio endpoint decision (WASAPI default device)' },
        [pscustomobject]@{ Group = 'Group E (Real Audio Endpoint)'; Name = 'the_cable_carries_the_fixture_tone'; Blocker = 'VB-Audio Virtual Cable installation in VM' }
    )

    $ranCount = 0
    $blockedCount = 0
    $missingFixtureCount = 0
    foreach ($item in $inventory) {
        $searchName = if ($item.Search) { $item.Search } else { $item.Name }
        $passed = $false
        $failed = $false
        foreach ($r in $result.results) {
            if ($item.Binary -and $r.exe -notlike "*$($item.Binary)*") { continue }
            if ($r.passed -and ($r.passed | Where-Object { $_ -like "*$searchName*" })) { $passed = $true }
            if ($r.failed -and ($r.failed | Where-Object { $_ -like "*$searchName*" })) { $failed = $true }
        }

        if ($passed) {
            $ranCount++
            Write-Host ("  [PASS] {0,-30} {1}" -f $item.Group, $item.Name)
        } elseif ($failed) {
            $ranCount++
            Write-Host ("  [FAIL] {0,-30} {1}" -f $item.Group, $item.Name)
        } elseif ($item.Blocker) {
            $blockedCount++
            Write-Host ("  [BLOCKED] {0,-27} {1} ({2})" -f $item.Group, $item.Name, $item.Blocker)
        } elseif ($item.Fixture -eq 'dsp' -and -not $dspStaged) {
            # The DSP fixture is missing, wrong or unconfirmed by the guest (see the
            # dsp plugin fixture line above): the test can only refuse, never pass.
            $missingFixtureCount++
            Write-Host ("  [MISSING FIXTURE] {0,-21} {1} (dsp plugin fixture not staged and verified; RUSTAMP_TESTPLUGIN_DSP)" -f $item.Group, $item.Name)
        } elseif ($item.Fixture -eq 'gen' -and -not $genFixtureStaged) {
            # Our own general-purpose fixture was not built for i686, so its
            # acceptance test can only refuse, never pass.
            $missingFixtureCount++
            Write-Host ("  [MISSING FIXTURE] {0,-21} {1} (the i686 general-purpose plugin fixture was not staged)" -f $item.Group, $item.Name)
        } elseif ($item.Fixture -eq 'in' -and -not $inFixtureStaged) {
            # Our own input fixture was not built for i686, so its acceptance
            # test can only refuse, never pass.
            $missingFixtureCount++
            Write-Host ("  [MISSING FIXTURE] {0,-21} {1} (the i686 input plugin fixture was not staged)" -f $item.Group, $item.Name)
        } elseif ($item.Group -like 'Group D*' -and -not $pluginsStaged) {
            # The host never staged D:\code\rustamp-testplugins, so RUSTAMP_TESTPLUGINS
            # is unset and these tests can only refuse loudly, never pass. Name that
            # plainly: a missing fixture must not read as a pass or a quiet skip.
            $missingFixtureCount++
            Write-Host ("  [MISSING FIXTURE] {0,-21} {1} (no D:\code\rustamp-testplugins on the host; RUSTAMP_TESTPLUGINS unset)" -f $item.Group, $item.Name)
        } else {
            Write-Host ("  [NOT RUN] {0,-27} {1}" -f $item.Group, $item.Name)
        }
    }
    Write-Host ("`nIgnored tests: {0} ran, {1} blocked pending host/VM decisions, {2} missing plugin fixture, {3} total in inventory" -f $ranCount, $blockedCount, $missingFixtureCount, $inventory.Count)
}

# A stall must never be reported as a green run: the hung count is a second,
# independent gate on the exit code, so a counting drift cannot hide a hang.
$runFailed = ($result.failed -ne 0 -or $hungCount -ne 0 -or $blessMissing -ne 0 -or $stallCount -ne 0)

# A run that executed no binary at all proves nothing, so it must never exit
# zero. Without this gate a filter that matches nothing - or a staging that lost
# the binaries - prints "ran 0, failed 0, hung 0" and reads exactly like a pass.
if ($null -eq $result.ran -or $result.ran -eq 0) {
    Write-Host ("  FAILED: the guest executed no test binary at all, so this run proves nothing (filter: '{0}'; {1} binaries were staged)." -f $Filter, $allBinariesCount)
    $runFailed = $true
}
if ($null -eq $result.testsRun -or $result.testsRun -eq 0) {
    Write-Host '  FAILED: no executed test case was reported; starting binaries that run zero tests proves nothing.'
    $runFailed = $true
}
return [pscustomobject]@{
    passed = -not $runFailed
    stalled = ($stallCount -ne 0)
    wallSeconds = $guestClock.Elapsed.TotalSeconds
    ran = $result.ran
    failedBinaries = $result.failed
    hungBinaries = $hungCount
    stalledBinaries = $stallCount
    error = $null
}
}

$showRepeatSummary = $PSBoundParameters.ContainsKey('Repeat') -or $RepeatSummaryPath
$runs = @()
for ($iteration = 1; $iteration -le $Repeat; $iteration++) {
    if ($showRepeatSummary) { Step ("repetition {0}/{1}" -f $iteration, $Repeat) }
    $attemptClock = [Diagnostics.Stopwatch]::StartNew()
    try {
        $run = Invoke-GuestTestRun
    } catch {
        if (-not $showRepeatSummary) { throw }
        # After an infrastructure failure the guest may still be running. Do
        # NOT send another request: record the incomplete measurement and stop.
        $run = [pscustomobject]@{
            passed = $false
            stalled = $false
            wallSeconds = $attemptClock.Elapsed.TotalSeconds
            ran = $null
            failedBinaries = $null
            hungBinaries = $null
            stalledBinaries = $null
            error = $_.Exception.Message
        }
    } finally { $attemptClock.Stop() }
    $run | Add-Member -NotePropertyName iteration -NotePropertyValue $iteration
    $runs += $run
    if ($run.error) { break }
}

$passedRuns = @($runs | Where-Object { $_.passed }).Count
$failedRuns = @($runs | Where-Object { -not $_.passed }).Count
$stalledRuns = @($runs | Where-Object { $_.stalled }).Count
$notAttempted = $Repeat - $runs.Count
if ($showRepeatSummary) {
    Write-Host ''
    Step 'repeat summary (stall count is a subset of failures; no reset between repetitions)'
    foreach ($run in $runs) {
        $status = if ($run.passed) { 'PASS' } elseif ($run.stalled) { 'STALL' } else { 'FAIL' }
        Write-Host ("  run {0}/{1}: {2}; wall {3:F3} s; ran {4}; failed binaries {5}; hung {6}" -f $run.iteration, $Repeat, $status, $run.wallSeconds, $run.ran, $run.failedBinaries, $run.hungBinaries)
        if ($run.error) { Write-Host ("    infrastructure failure: {0}" -f $run.error) }
    }
    $rateLine = "RATE filter='{0}': {1}/{2} passed ({3:F1}%); {4}/{2} failed; {5}/{2} stalled; requested {6}, not attempted {7}" -f $Filter, $passedRuns, $runs.Count, (100.0 * $passedRuns / $runs.Count), $failedRuns, $stalledRuns, $Repeat, $notAttempted
    Write-Host $rateLine
    if ($RepeatSummaryPath) {
        [pscustomobject]@{
            filter = $Filter
            requested = $Repeat
            attempted = $runs.Count
            passed = $passedRuns
            failed = $failedRuns
            stalled = $stalledRuns
            notAttempted = $notAttempted
            resetBetweenRuns = $false
            rate = $rateLine
            runs = @($runs)
        } | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $RepeatSummaryPath -Encoding UTF8
    }
}
if ($failedRuns -ne 0 -or $notAttempted -ne 0) { exit 1 }
exit 0
