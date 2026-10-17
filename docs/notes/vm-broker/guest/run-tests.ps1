# Run the Rustamp test binaries INSIDE the guest.  Allow-listed guest script.
#
# Called by the broker (`-Op guest -Script run-tests.ps1`). It expects the host
# to have compiled and staged the tree (see docs\notes\vm-run-tests.ps1) at
# C:\rustamp\code\Rustamp.
#
# The trick that makes "compile on the host, run in the guest" work without
# touching a single test: the tests locate the repository, the fixtures and the
# built binary through paths baked in at COMPILE time (the host's
# D:\code\Rustamp\...). `subst D: C:\rustamp` makes exactly those paths resolve
# inside the guest, so the same binaries run unchanged.
#
# Nothing graphical is involved: the test binaries are console programs, and
# this runs in a PowerShell Direct session (no desktop).

param([string]$Filter = '')

$ErrorActionPreference = 'Continue'

# 1. make the baked host paths resolve
if (-not (Test-Path 'D:\code\Rustamp')) {
    subst D: C:\rustamp
}
$repo = 'D:\code\Rustamp'
if (-not (Test-Path $repo)) { throw "the repository is not staged at $repo" }

# Enumerate the staged test binaries instead of reading a list file: the host
# stages exactly the test executables into target\debug\deps, and a list file is
# one more thing that can go stale (it did: an old list containing a joined
# string made this script try to run all 34 paths as one command).
$deps = Join-Path $repo 'target\debug\deps'
if (-not (Test-Path $deps)) { throw "no staged binaries at $deps" }
$exes = @(Get-ChildItem -Path $deps -Filter '*.exe' | Sort-Object Name | Select-Object -ExpandProperty FullName)
if (-not $exes) { throw "no test binaries in $deps" }

$filter = $Filter
$extra = @()
# An `--ignored` prefix asks the binaries to run their ignored tests as well.
# Those are exactly the tests that open a window or spawn a process: they are
# ignored by default because the host must stay windowless, and they may only
# run here, in the guest. `--ignored:name` also filters by binary name.
if ($filter -like '--ignored*') {
    $extra = @('--ignored')
    $filter = ($filter -replace '^--ignored:?', '')
}

$results = @()
# Real test media: the host runner stages read-only copies at C:\rustamp\testmedia
# when the owner's archive copy exists on the host. Point the documented
# RUSTAMP_TESTDATA variable at it so media-dependent tests opt in; without it
# they skip, exactly as on a machine without the archive.
if (Test-Path 'C:\rustamp\testmedia') { $env:RUSTAMP_TESTDATA = 'C:\rustamp\testmedia' }
# Real test skins, the same contract: the host runner stages read-only copies at
# C:\rustamp\testskins (`.wsz` files and/or skin folders) when they exist on the
# host. Point RUSTAMP_TESTSKINS at it so skin-dependent tests opt in; without it
# they skip.
if (Test-Path 'C:\rustamp\testskins') { $env:RUSTAMP_TESTSKINS = 'C:\rustamp\testskins' }
foreach ($exe in $exes) {
    $name = Split-Path $exe -Leaf
    if ($filter -and $name -notlike "*$filter*") { continue }
    if (-not (Test-Path $exe)) { $results += [pscustomobject]@{ exe = $name; exit = -1; summary = 'MISSING BINARY'; failed = @() }; continue }

    $raw = & $exe @extra 2>&1 | Out-String
    $exit = $LASTEXITCODE
    $lines = $raw -split "`r?`n"
    $summary = ($lines | Where-Object { $_ -match '^test result:' } | Select-Object -Last 1)
    $failedNames = @($lines | Where-Object { $_ -match '^test .* FAILED$' } | ForEach-Object { $_.Trim() })
    # A panic's values live on the lines AFTER the FAILED line (multi-line
    # assertion messages), so keep a window behind each failure: without it a
    # failure like "timeline stopped at X of Y frames" keeps its verdict but
    # loses X and Y, and every diagnosis costs another guest round trip.
    $failureWindows = @()
    for ($i = 0; $i -lt $lines.Count; $i++) {
        if ($lines[$i] -match '^test .* FAILED$|panicked at') {
            $failureWindows += @($lines | Select-Object -Skip $i -First 10)
        }
    }
    # When a binary produces no test summary it did not run its tests at all
    # (usually it could not start), so keep the first output lines: without them
    # a failure like this is a mystery.
    $diagnostic = @()
    if (-not $summary) { $diagnostic = @($lines | Where-Object { $_ -and $_.Trim() } | Select-Object -First 5) }
    # A failing test's own output is the only way to see *why* from the runner, and
    # a test that panics says so in one line. Keep those lines (with a little
    # context) so the failure is diagnosable without a second round trip.
    if ($failedNames.Count -gt 0) {
        $interesting = @($lines | Where-Object { $_ -match 'panicked|assertion|FAILED|BLOCKED|left:|right:|Error|error:' } | Select-Object -First 14)
        if ($interesting) { $diagnostic = @($diagnostic + $interesting) }
        if ($failureWindows) { $diagnostic = @($diagnostic + @($failureWindows | Select-Object -First 20)) }
    }
    $results += [pscustomobject]@{
        exe        = $name
        exit       = $exit
        summary    = $summary
        failed     = $failedNames
        diagnostic = $diagnostic
    }
}

[pscustomobject]@{
    host      = $env:COMPUTERNAME
    user      = $env:USERNAME
    repo      = $repo
    subst     = (Test-Path 'D:\code\Rustamp')
    ran       = $results.Count
    failed    = @($results | Where-Object { $_.exit -ne 0 }).Count
    results   = $results
}
