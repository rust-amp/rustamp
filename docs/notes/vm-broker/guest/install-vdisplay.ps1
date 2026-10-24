# Create the virtual display DEVICE, then enable it - the step the first attempt
# missed.
#
# `pnputil /add-driver <inf> /install` only puts the driver in the store; the
# device that carries the Indirect Display Driver interface is created by the
# vendor's own install step. `deviceinstaller64 enableidd 1` then fails with
# "No active device interfaces found" until that device exists, which is exactly
# what the diagnostic showed.
#
# Runs INSIDE the guest (allow-listed plain name). Owner-approved install.

$ErrorActionPreference = 'Continue'

$dir = 'C:\rustamp\drivers\usbmmidd'
$tool = Get-ChildItem $dir -Recurse -Filter 'deviceinstaller64.exe' -ErrorAction SilentlyContinue | Select-Object -First 1
$inf = Get-ChildItem $dir -Recurse -Filter 'usbmm*.inf' -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $tool -or -not $inf) { throw "driver files not found under $dir (need deviceinstaller64.exe and the INF)" }
Write-Output ('tool: ' + $tool.FullName)
Write-Output ('inf : ' + $inf.FullName)

Push-Location $tool.DirectoryName

Write-Output '=== install the device (vendor install step) ==='
(& $tool.FullName install $inf.Name usbmmidd 2>&1) | ForEach-Object { Write-Output ('  ' + $_) }
Write-Output ('  exit: ' + $LASTEXITCODE)

Write-Output '=== enable the indirect display ==='
(& $tool.FullName enableidd 1 2>&1) | ForEach-Object { Write-Output ('  ' + $_) }
Write-Output ('  exit: ' + $LASTEXITCODE)

Pop-Location

Write-Output '=== display devices now ==='
try {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public class DD2 {
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
'@ -ErrorAction Stop
    $i = 0
    while ($true) {
        $d = New-Object DD2+DISPLAY_DEVICE
        $d.cb = [System.Runtime.InteropServices.Marshal]::SizeOf($d)
        if (-not [DD2]::EnumDisplayDevices($null, $i, [ref]$d, 0)) { break }
        $attached = ($d.StateFlags -band 0x1) -ne 0
        $primary = ($d.StateFlags -band 0x4) -ne 0
        Write-Output ("  [{0}] {1} | {2} | attached_to_desktop={3} primary={4} flags=0x{5:X}" -f $i, $d.DeviceName, $d.DeviceString, $attached, $primary, $d.StateFlags)
        $i++
    }
    Write-Output ("  total: {0}" -f $i)
} catch { Write-Output ('  EnumDisplayDevices failed: ' + $_.Exception.Message) }

Write-Output '=== monitors now ==='
Get-PnpDevice -Class Monitor -ErrorAction SilentlyContinue | ForEach-Object { Write-Output ("  {0} | {1}" -f $_.Status, $_.FriendlyName) }
Write-Output '=== done ==='
