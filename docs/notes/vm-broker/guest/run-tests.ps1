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

param([string[]]$GuestArgs)

$ErrorActionPreference = 'Continue'

# 1. make the baked host paths resolve
if (-not (Test-Path 'D:\code\Rustamp')) {
    subst D: C:\rustamp
}
$repo = 'D:\code\Rustamp'
if (-not (Test-Path $repo)) { throw "the repository is not staged at $repo" }

$listPath = Join-Path $repo 'target\debug\test-list.json'
if (-not (Test-Path $listPath)) { throw "no test list at $listPath - did the host stage it?" }
$exes = @(Get-Content $listPath -Raw | ConvertFrom-Json)

$filter = ''
if ($GuestArgs -and $GuestArgs.Count -ge 2 -and $GuestArgs[0] -eq '-Filter') { $filter = $GuestArgs[1] }

$results = @()
foreach ($exe in $exes) {
    $name = Split-Path $exe -Leaf
    if ($filter -and $name -notlike "*$filter*") { continue }
    if (-not (Test-Path $exe)) { $results += [pscustomobject]@{ exe = $name; exit = -1; summary = 'MISSING BINARY'; failed = @() }; continue }

    $raw = & $exe 2>&1 | Out-String
    $exit = $LASTEXITCODE
    $lines = $raw -split "`r?`n"
    $summary = ($lines | Where-Object { $_ -match '^test result:' } | Select-Object -Last 1)
    $failedNames = @($lines | Where-Object { $_ -match '^test .* FAILED$' } | ForEach-Object { $_.Trim() })
    $results += [pscustomobject]@{
        exe     = $name
        exit    = $exit
        summary = $summary
        failed  = $failedNames
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
