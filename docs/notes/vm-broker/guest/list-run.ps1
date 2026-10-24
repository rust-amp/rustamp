# Read-only: where did the staged harness run get to?
#
# Runs INSIDE the guest (allow-listed plain name). Changes nothing.

$ErrorActionPreference = 'Continue'

Write-Output '=== staged harness runs ==='
Get-ChildItem 'C:\rustamp\capture\tools\vm-capture\_runs' -Directory -ErrorAction SilentlyContinue |
    Sort-Object Name |
    ForEach-Object { Write-Output ("  " + $_.Name) }

Write-Output '=== newest run: DONE.json ==='
$r = Get-ChildItem 'C:\rustamp\capture\tools\vm-capture\_runs' -Directory -ErrorAction SilentlyContinue | Sort-Object Name -Descending | Select-Object -First 1
if ($r) {
    $d = Join-Path $r.FullName 'DONE.json'
    if (Test-Path $d) {
        $j = Get-Content $d -Raw | ConvertFrom-Json
        Write-Output ("  status={0} exit={1}" -f $j.status, $j.exit_code)
        Write-Output ("  gate={0}" -f $j.landmark_gate.verdict)
        $sr = $j.step_results
        Write-Output ("  steps={0} ok={1} skipped={2}" -f $sr.Count, (@($sr | Where-Object { $_.exit_code -eq 0 }).Count), (@($sr | Where-Object { $_.skipped }).Count))
        foreach ($f in @($j.failures | Select-Object -First 5)) { Write-Output ("  FAIL {0}" -f $f) }
    } else { Write-Output '  (no DONE.json yet)' }
    Write-Output '=== newest run: bootstrap.log tail ==='
    $b = Join-Path $r.FullName 'bootstrap.log'
    if (Test-Path $b) { Get-Content $b | Select-Object -Last 12 | ForEach-Object { Write-Output ('  ' + $_.Substring(0, [Math]::Min(150, $_.Length))) } }
}
Write-Output '=== done ==='
