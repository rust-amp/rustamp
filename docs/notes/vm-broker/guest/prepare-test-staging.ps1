# Allow-listed guest-only pre-copyin cleanup. A previous test can orphan the
# staged i686 host, which locks rustamp-plughost32.exe before any new test runs.
# Kill only that helper under the fixed guest staging tree; never touch a host
# process or a reference installation. Failure to inspect/kill/wait stays loud.

$ErrorActionPreference = 'Stop'
$stopped = 0
foreach ($helper in @(Get-CimInstance Win32_Process -Filter "Name = 'rustamp-plughost32.exe'")) {
    $path = $helper.ExecutablePath
    if (-not $path) { throw "cannot inspect plugin host PID $($helper.ProcessId); refusing to stage over an unknown lock" }
    if ($path.StartsWith('\\?\')) { $path = $path.Substring(4) }
    if (-not ($path.StartsWith('C:\rustamp\code\Rustamp\', [StringComparison]::OrdinalIgnoreCase) -or
              $path.StartsWith('D:\code\Rustamp\', [StringComparison]::OrdinalIgnoreCase))) { continue }
    Write-Host ("clearing stray guest plugin host PID {0}: {1}" -f $helper.ProcessId, $path)
    $proc = Get-Process -Id $helper.ProcessId -ErrorAction SilentlyContinue
    if (-not $proc) { continue }
    try {
        $proc.Kill()
        if (-not $proc.WaitForExit(10000)) { throw "plugin host PID $($helper.ProcessId) still holds $path after 10 s" }
        $stopped++
    } finally { $proc.Dispose() }
}
[pscustomobject]@{ stopped = $stopped }
