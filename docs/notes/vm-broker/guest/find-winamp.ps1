# Read-only: is the reference binary running, in which session, with what windows?
#
# Runs INSIDE the guest (allow-listed plain name). Changes nothing.

$ErrorActionPreference = 'Continue'

Write-Output '=== winamp processes ==='
Get-Process -Name 'winamp*' -ErrorAction SilentlyContinue |
    ForEach-Object { Write-Output ("  {0} id={1} session={2} started={3}" -f $_.ProcessName, $_.Id, $_.SessionId, $_.StartTime) }
if (-not (Get-Process -Name 'winamp*' -ErrorAction SilentlyContinue)) { Write-Output '  (none running)' }

Write-Output '=== sessions ==='
qwinsta 2>&1 | ForEach-Object { Write-Output ('  ' + $_) }

Write-Output '=== top-level windows visible from here (this session) ==='
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public class EW {
    public delegate bool CB(IntPtr h, IntPtr p);
    [DllImport("user32.dll")] public static extern bool EnumWindows(CB cb, IntPtr p);
    [DllImport("user32.dll")] public static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
}
'@ -ErrorAction SilentlyContinue
$count = 0
[EW]::EnumWindows({ param($h, $p)
    $sb = New-Object Text.StringBuilder 256
    [void][EW]::GetWindowTextW($h, $sb, 256)
    $t = $sb.ToString()
    if ($t -match 'Winamp|winamp') {
        $pid = 0; [void][EW]::GetWindowThreadProcessId($h, [ref]$pid)
        Write-Output ("  winamp window: '{0}' visible={1} pid={2}" -f $t, [EW]::IsWindowVisible($h), $pid)
    }
    return $true
}, [IntPtr]::Zero) | Out-Null
Write-Output '=== done ==='
