# Diagnose the staged tree and one test binary inside the guest. Facts only.

$ErrorActionPreference = 'Continue'

if (-not (Test-Path 'D:\code\Rustamp')) { subst D: C:\rustamp }

$deps = 'D:\code\Rustamp\target\debug\deps'
$one = Get-ChildItem -Path $deps -Filter 'rustamp_core-*.exe' -ErrorAction SilentlyContinue | Select-Object -First 1

$report = [ordered]@{
    subst_target   = (Test-Path 'D:\code\Rustamp')
    deps_exes      = @(Get-ChildItem $deps -Filter '*.exe' -ErrorAction SilentlyContinue).Count
    deps_dlls      = @(Get-ChildItem $deps -Filter '*.dll' -ErrorAction SilentlyContinue).Count
    debug_top      = @(Get-ChildItem 'D:\code\Rustamp\target\debug' -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Name)
    vcruntime_sys  = (Test-Path "$env:SystemRoot\System32\vcruntime140.dll")
    msvcp_sys      = (Test-Path "$env:SystemRoot\System32\msvcp140.dll")
    ucrt_sys       = (Test-Path "$env:SystemRoot\System32\ucrtbase.dll")
    test_exe       = if ($one) { $one.Name } else { 'none' }
}

if ($one) {
    $out = & $one.FullName --list 2>&1 | Out-String
    $report['run_exit'] = $LASTEXITCODE
    $report['run_exit_hex'] = ('0x{0:X8}' -f ($LASTEXITCODE -band 0xFFFFFFFFL))
    $report['run_output'] = @(($out -split "`r?`n") | Where-Object { $_.Trim() } | Select-Object -First 6)

    # Which DLLs does it want? The import table is the fastest answer.
    $bytes = [System.IO.File]::ReadAllBytes($one.FullName)
    $text = [System.Text.Encoding]::ASCII.GetString($bytes)
    $dlls = [regex]::Matches($text, '(?i)[A-Za-z0-9_\-\.]{3,40}\.dll') | ForEach-Object { $_.Value.ToLower() } | Sort-Object -Unique
    $report['imports_like'] = @($dlls | Where-Object { $_ -notmatch '^(api-ms|ext-ms)' } | Select-Object -First 12)
}

[pscustomobject]$report
