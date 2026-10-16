# Diagnostic: is the indirect display's monitor list present, and does enabling
# it again change anything?
#
# The vendor instructions put the supported resolutions under
# HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\WUDF\Services\usbmmIdd\
# Parameters\Monitors, and say the monitor is activated with
# `deviceinstaller64 enableidd 1` (repeatable, up to four monitors). The device
# is present and OK but Windows enumerates no adapter, so the question is
# whether the monitor list exists at all and whether enabling it again helps.
#
# Runs INSIDE the guest (allow-listed plain name).

$ErrorActionPreference = 'Continue'

$key = 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\WUDF\Services\usbmmIdd\Parameters'
Write-Output '=== 1. the WUDF service key for the indirect display ==='
if (Test-Path $key) {
    Get-ItemProperty -Path $key -ErrorAction SilentlyContinue | Format-List | Out-String -Width 200 | ForEach-Object { Write-Output ('  ' + $_) }
    Get-ChildItem -Path $key -Recurse -ErrorAction SilentlyContinue | ForEach-Object { Write-Output ('  subkey: ' + $_.Name) }
    Get-ItemProperty -Path (Join-Path $key 'Monitors') -ErrorAction SilentlyContinue | Format-List | Out-String -Width 200 | ForEach-Object { Write-Output ('  Monitors: ' + $_) }
} else {
    Write-Output ("  MISSING: {0}" -f $key)
}
Write-Output '=== 2. the WUDF service itself ==='
Get-Service -Name 'WUDF*' -ErrorAction SilentlyContinue | ForEach-Object { Write-Output ("  {0} | {1} | {2}" -f $_.Status, $_.Name, $_.DisplayName) }

Write-Output '=== 3. enable the indirect display two more times ==='
$tool = 'C:\rustamp\drivers\usbmmidd\deviceinstaller64.exe'
Push-Location (Split-Path $tool)
foreach ($n in 1..2) {
    Write-Output ("  --- enableidd 1 (attempt {0}) ---" -f $n)
    (& $tool enableidd 1 2>&1) | ForEach-Object { Write-Output ('  ' + $_) }
    Write-Output ('  exit: ' + $LASTEXITCODE)
    Start-Sleep -Seconds 2
}
Pop-Location

Write-Output '=== 4. adapters after enabling again ==='
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public class DD4 {
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    public struct DISPLAY_DEVICE {
        public int cb;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string DeviceName;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)] public string DeviceString;
        public int StateFlags;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)] public string DeviceID;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)] public string DeviceKey;
    }
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern bool EnumDisplayDevices(string lpDevice, uint iDevNum, ref DISPLAY_DEVICE lpDisplayDevice, uint dwFlags);
}
'@ -ErrorAction SilentlyContinue
$i = 0
while ($true) {
    $d = New-Object DD4+DISPLAY_DEVICE
    $d.cb = [System.Runtime.InteropServices.Marshal]::SizeOf($d)
    if (-not [DD4]::EnumDisplayDevices($null, $i, [ref]$d, 0)) { break }
    Write-Output ("  [{0}] {1} | {2} | attached={3}" -f $i, $d.DeviceName, $d.DeviceString, (($d.StateFlags -band 0x1) -ne 0))
    $i++
}
Write-Output ("  adapters: {0}" -f $i)
Write-Output '=== 5. device problem code (if any) ==='
Get-PnpDevice -InstanceId 'ROOT\DISPLAY\0000' -ErrorAction SilentlyContinue | ForEach-Object {
    Write-Output ("  status={0} problem={1} problemcode={2}" -f $_.Status, $_.Problem, $_.ProblemDescription)
}
Write-Output '=== done ==='
