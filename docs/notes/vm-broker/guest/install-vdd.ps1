# Install the signed Virtual Display Driver (MttVDD, an Indirect Display Driver)
# and report whether it presents a monitor.
#
# The package (staged at C:\rustamp\drivers\vdd) holds the INF, the UMDF DLL, a
# catalog and a vendor certificate. The INF declares a ROOT-enumerated device,
# `Root\MttVDD`, class Display, so it needs a devcon-style install step, not just
# `pnputil /add-driver`. The usbmmidd package already staged a devcon equivalent
# (`deviceinstaller64.exe install <inf> <hwid>`), which is reused here.
#
# Runs INSIDE the guest (allow-listed plain name). Owner-approved install.

$ErrorActionPreference = 'Continue'

$dir = 'C:\rustamp\drivers\vdd'
$inf = Get-ChildItem $dir -Recurse -Filter 'MttVDD.inf' -ErrorAction SilentlyContinue | Select-Object -First 1
$cer = Get-ChildItem $dir -Recurse -Filter '*.cer' -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $inf) { throw "MttVDD.inf not found under $dir" }
Write-Output ('inf: ' + $inf.FullName)
if ($cer) { Write-Output ('cer: ' + $cer.FullName) }

Write-Output '=== 1. trust the vendor certificate (installCert.bat, without the pause) ==='
if ($cer) {
    (& certutil -addstore -f root $cer.FullName 2>&1) | ForEach-Object { Write-Output ('  ' + $_) }
    (& certutil -addstore -f TrustedPublisher $cer.FullName 2>&1) | ForEach-Object { Write-Output ('  ' + $_) }
}

Write-Output '=== 2. is the user-mode driver framework there at all? ==='
Get-Service -ErrorAction SilentlyContinue | Where-Object { $_.Name -match 'Wudf|WUDF' } |
    ForEach-Object { Write-Output ("  service: {0} | {1} | {2}" -f $_.Status, $_.Name, $_.DisplayName) }
Get-Service -Name 'WudfRd' -ErrorAction SilentlyContinue | ForEach-Object { Write-Output ("  reflector: {0} | {1}" -f $_.Status, $_.Name) }

Write-Output '=== 3. add the driver to the store ==='
(& pnputil /add-driver $inf.FullName /install 2>&1) | ForEach-Object { Write-Output ('  ' + $_) }
Write-Output ('  exit: ' + $LASTEXITCODE)

Write-Output '=== 4. create the root-enumerated device (devcon-style) ==='
$devcon = Get-ChildItem 'C:\rustamp\drivers' -Recurse -Filter 'deviceinstaller64.exe' -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $devcon) { Write-Output '  no devcon equivalent found; the device was not created' }
else {
    Write-Output ('  using: ' + $devcon.FullName)
    Push-Location $inf.DirectoryName
    (& $devcon.FullName install $inf.Name 'Root\MttVDD' 2>&1) | ForEach-Object { Write-Output ('  ' + $_) }
    Write-Output ('  exit: ' + $LASTEXITCODE)
    Pop-Location
}

Write-Output '=== 5. the device, and the display ==='
Get-PnpDevice -ErrorAction SilentlyContinue | Where-Object { $_.InstanceId -match 'MttVDD|ROOT\\DISPLAY' -or $_.FriendlyName -match 'Mtt|Virtual Display' } |
    ForEach-Object { Write-Output ("  {0} | {1} | {2}" -f $_.Status, $_.FriendlyName, $_.InstanceId) }
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public class DD5 {
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
    $d = New-Object DD5+DISPLAY_DEVICE
    $d.cb = [System.Runtime.InteropServices.Marshal]::SizeOf($d)
    if (-not [DD5]::EnumDisplayDevices($null, $i, [ref]$d, 0)) { break }
    Write-Output ("  [{0}] {1} | {2} | attached={3}" -f $i, $d.DeviceName, $d.DeviceString, (($d.StateFlags -band 0x1) -ne 0))
    $i++
}
Write-Output ("  adapters: {0}" -f $i)
Write-Output '=== done ==='
