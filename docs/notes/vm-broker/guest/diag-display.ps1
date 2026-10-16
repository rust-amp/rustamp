# Diagnostic: why is there no attached display after the usbmmidd install?
#
# Runs INSIDE the guest (allow-listed name, plain .ps1). Read-only except for
# the vendor enable step, which is exactly what we are trying to understand.
# Prints everything a human needs to decide the next step; changes nothing.

$ErrorActionPreference = 'Continue'

Write-Output '=== 1. display devices (EnumDisplayDevices) ==='
try {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public class DD {
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
        $d = New-Object DD+DISPLAY_DEVICE
        $d.cb = [System.Runtime.InteropServices.Marshal]::SizeOf($d)
        if (-not [DD]::EnumDisplayDevices($null, $i, [ref]$d, 0)) { break }
        $attached = ($d.StateFlags -band 0x1) -ne 0
        $primary = ($d.StateFlags -band 0x4) -ne 0
        Write-Output ("  [{0}] {1} | {2} | attached_to_desktop={3} primary={4} flags=0x{5:X}" -f $i, $d.DeviceName, $d.DeviceString, $attached, $primary, $d.StateFlags)
        $i++
    }
    Write-Output ("  total: {0}" -f $i)
} catch { Write-Output ('  EnumDisplayDevices failed: ' + $_.Exception.Message) }

Write-Output '=== 2. screens (System.Windows.Forms) ==='
try {
    Add-Type -AssemblyName System.Windows.Forms -ErrorAction Stop
    $screens = [System.Windows.Forms.Screen]::AllScreens
    Write-Output ("  AllScreens: {0}" -f $screens.Count)
    foreach ($s in $screens) { Write-Output ("  {0} primary={1} bounds={2}" -f $s.DeviceName, $s.Primary, $s.Bounds) }
} catch { Write-Output ('  Screen enumeration failed: ' + $_.Exception.Message) }

Write-Output '=== 3. the driver in the driver store ==='
$p = & pnputil /enum-drivers 2>&1
$text = $p -join "`n"
foreach ($block in ($text -split "`r?`n`r?`n")) {
    if ($block -match 'usbmm|Idd|idd') { Write-Output ('  ' + ($block -replace "`r?`n", ' | ')) }
}
Write-Output '=== 4. monitor devices (Get-PnpDevice) ==='
Get-PnpDevice -Class Monitor -ErrorAction SilentlyContinue | ForEach-Object { Write-Output ("  {0} | {1} | {2}" -f $_.Status, $_.FriendlyName, $_.InstanceId) }
Get-PnpDevice -ErrorAction SilentlyContinue | Where-Object { $_.FriendlyName -match 'usbmm|Mobile Monitor|Idd' } | ForEach-Object { Write-Output ("  device: {0} | {1} | {2}" -f $_.Status, $_.FriendlyName, $_.InstanceId) }

Write-Output '=== 5. the vendor tool, and what it says ==='
$dir = 'C:\rustamp\drivers\usbmmidd'
$tool = Get-ChildItem $dir -Recurse -Filter 'deviceinstaller64.exe' -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $tool) { Write-Output "  deviceinstaller64.exe not found under $dir" }
else {
    Write-Output ('  tool: ' + $tool.FullName)
    Push-Location $tool.DirectoryName
    Write-Output '  --- enableidd 1 ---'
    (& $tool.FullName enableidd 1 2>&1) | ForEach-Object { Write-Output ('  ' + $_) }
    Write-Output ('  exit: ' + $LASTEXITCODE)
    Pop-Location
}
Write-Output '=== 6. test signing state ==='
(& bcdedit /enum '{current}' 2>&1) | Select-String -Pattern 'testsigning|integrityservices' | ForEach-Object { Write-Output ('  ' + $_.Line.Trim()) }
Write-Output '=== done ==='
