# Diagnose the staged tree inside the guest. Allow-listed guest script.
# Reports facts only; changes nothing.

$ErrorActionPreference = 'Continue'

if (-not (Test-Path 'D:\code\Rustamp')) { subst D: C:\rustamp }

$report = [ordered]@{
    computer      = $env:COMPUTERNAME
    user          = $env:USERNAME
    powershell    = $PSVersionTable.PSVersion.ToString()
    d_letter      = (Test-Path 'D:\')
    subst_target  = (Test-Path 'D:\code\Rustamp')
    c_rustamp     = (Test-Path 'C:\rustamp')
    c_repo        = (Test-Path 'C:\rustamp\code\Rustamp')
    repo_top      = @(Get-ChildItem 'C:\rustamp\code\Rustamp' -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Name)
    nested_repo   = (Test-Path 'C:\rustamp\code\Rustamp\Rustamp')
    list_path     = 'D:\code\Rustamp\target\debug\test-list.json'
    list_exists   = (Test-Path 'D:\code\Rustamp\target\debug\test-list.json')
    deps_count    = @(Get-ChildItem 'D:\code\Rustamp\target\debug\deps' -Filter '*.exe' -ErrorAction SilentlyContinue).Count
    fixture       = (Test-Path 'D:\code\Rustamp\crates\rustamp-decoders\tests\fixtures\sine.flac')
    rustamp_exe   = (Test-Path 'D:\code\Rustamp\target\debug\rustamp.exe')
}

if ($report.list_exists) {
    $exes = @(Get-Content $report.list_path -Raw | ConvertFrom-Json)
    $report['list_count'] = $exes.Count
    $report['list_type'] = $exes.GetType().Name
    $report['list_first'] = [string]$exes[0]
    $report['list_first_exists'] = (Test-Path ([string]$exes[0]))
}

[pscustomobject]$report
