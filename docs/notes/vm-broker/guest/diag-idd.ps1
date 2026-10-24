# Diagnostic: what does the usbmmidd package contain, and what state is the
# indirect display device in?
#
# An IDD (Indirect Display Driver) is not a standalone display: the kernel part
# reports a monitor, but a USER-MODE client usually has to feed it and keep the
# monitor "present". This probe lists what the package actually shipped, whether
# anything of it is running, and what Windows thinks of the device - so the next
# step is decided from evidence rather than guessed.
#
# Runs INSIDE the guest (allow-listed plain name). Read-only.

$ErrorActionPreference = 'Continue'

$dir = 'C:\rustamp\drivers\usbmmidd'
Write-Output '=== 1. what the package contains ==='
Get-ChildItem $dir -Recurse -File -ErrorAction SilentlyContinue |
    ForEach-Object { Write-Output ("  {0,10}  {1}" -f $_.Length, $_.FullName.Replace($dir, '')) }

Write-Output '=== 2. the indirect display device (Get-PnpDevice) ==='
Get-PnpDevice -ErrorAction SilentlyContinue |
    Where-Object { $_.InstanceId -match 'usbmm|Idd|ROOT' -or $_.FriendlyName -match 'usbmm|Mobile|Idd' } |
    ForEach-Object { Write-Output ("  {0} | {1} | {2}" -f $_.Status, $_.FriendlyName, $_.InstanceId) }

Write-Output '=== 3. drivers in the store matching the package ==='
$p = (& pnputil /enum-drivers 2>&1) -join "`n"
foreach ($block in ($p -split "`r?`n`r?`n")) {
    if ($block -match 'usbmm|Amyuni') { Write-Output ('  ' + ($block -replace "`r?`n", ' | ')) }
}

Write-Output '=== 4. services and processes that could be the client ==='
Get-Service -ErrorAction SilentlyContinue | Where-Object { $_.Name -match 'usbmm|idd|amyuni' -or $_.DisplayName -match 'usbmm|Mobile Monitor|Indirect' } |
    ForEach-Object { Write-Output ("  service: {0} | {1} | {2}" -f $_.Status, $_.Name, $_.DisplayName) }
Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.ProcessName -match 'usbmm|idd|amyuni|deviceinstaller' } |
    ForEach-Object { Write-Output ("  process: {0} (id {1})" -f $_.ProcessName, $_.Id) }

Write-Output '=== 5. driver files actually loaded ==='
(& driverquery /v /fo csv 2>&1) | Select-String -Pattern 'usbmm|idd' | Select-Object -First 5 |
    ForEach-Object { Write-Output ('  ' + $_.Line.Substring(0, [Math]::Min(160, $_.Line.Length))) }

Write-Output '=== 6. the display state one more time ==='
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public class DD3 {
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
    $d = New-Object DD3+DISPLAY_DEVICE
    $d.cb = [System.Runtime.InteropServices.Marshal]::SizeOf($d)
    if (-not [DD3]::EnumDisplayDevices($null, $i, [ref]$d, 0)) { break }
    Write-Output ("  [{0}] {1} | {2} | attached={3} flags=0x{4:X}" -f $i, $d.DeviceName, $d.DeviceString, (($d.StateFlags -band 0x1) -ne 0), $d.StateFlags)
    $i++
}
Write-Output ("  adapters: {0}" -f $i)
Write-Output '=== done ==='
