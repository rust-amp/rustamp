# Assert the guest holds a PERSISTENT ATTACHED DISPLAY.  Allow-listed guest script.
#
# Called by the broker exactly like assert-interactive-session.ps1:
#
#   broker -Op guest -Script assert-attached-display.ps1
#
# Or over PowerShell Direct from the setup script:
#
#   Invoke-Command -VMName rustamp-test -Credential $c -FilePath docs\notes\vm-broker\guest\assert-attached-display.ps1
#
# Why this is a SEPARATE proof from the session proof: the session proof
# (assert-interactive-session.ps1) shows Winlogon logged on and a desktop
# exists (explorer outside session 0, non-empty virtual screen). A headless
# Hyper-V guest can hold that session while having NO attached display: the
# synthetic video adapter exposes a mode, but with no monitor attached the
# graphics subsystem has no presentable surface and a GUI only "wakes up" when
# somebody looks at the console (VMConnect/Enhanced Session). The fix is a
# persistent virtual display - a driver-backed monitor Windows believes is
# physically attached - so DWM always has a present target even with the
# console disconnected. See docs\notes\vm-provision-virtual-display.ps1 for
# the provisioning step (third-party driver, owner approval required).
#
# What this script checks (all read-only, no install, session-0 safe):
#   1. Win32 reports at least one display device with
#      DISPLAY_DEVICE_ATTACHED_TO_DESKTOP (EnumDisplayDevices, StateFlags 0x1).
#      This is the "a monitor is attached" claim; a bare Hyper-V synthetic
#      adapter with no viewer may expose a mode without this flag.
#   2. System.Windows.Forms.Screen.AllScreens is non-empty with a primary
#      screen and a non-empty virtual screen. This is the "there is somewhere
#      to present" claim; it must agree with the session proof's screen.
#   3. DWM composition is enabled (DwmIsCompositionEnabled). Without a
#      presentable surface composition can be off even when a session exists.
#
# Any of the three missing throws: the run has a session but no display, and
# says exactly that. A viewer (VMConnect, Enhanced Session, RDP) would also
# make these pass while it is attached, but a viewer is NOT the fix - it opens
# a GUI on the host, which the project forbids - so this proof is only
# meaningful with no viewer connected (the setup script never opens one).

$ErrorActionPreference = 'Stop'

Add-Type -AssemblyName System.Windows.Forms

# --- 1. at least one device attached to the desktop -----------------------------
$displaySource = @'
using System;
using System.Runtime.InteropServices;
public sealed class DisplayProbe {
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Auto)]
    public struct DISPLAY_DEVICE {
        public int cb;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string DeviceName;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)] public string DeviceString;
        public int StateFlags;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)] public string DeviceID;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)] public string DeviceKey;
    }
    [DllImport("user32.dll", CharSet = CharSet.Auto)]
    public static extern bool EnumDisplayDevices(string lpDevice, uint iDevNum, ref DISPLAY_DEVICE lpDisplayDevice, uint dwFlags);
    public const int ATTACHED_TO_DESKTOP = 0x1;
}
'@
Add-Type -TypeDefinition $displaySource -ErrorAction Stop
$attached = @()
$index = [uint32]0
while ($true) {
    $device = New-Object DisplayProbe+DISPLAY_DEVICE
    $device.cb = [Runtime.InteropServices.Marshal]::SizeOf($device)
    if (-not [DisplayProbe]::EnumDisplayDevices($null, $index, [ref]$device, 0)) { break }
    if (($device.StateFlags -band [DisplayProbe]::ATTACHED_TO_DESKTOP) -ne 0) {
        $attached += [pscustomobject]@{
            name  = $device.DeviceName
            model = $device.DeviceString
        }
    }
    $index++
    if ($index -gt 64) { break }
}
if ($attached.Count -eq 0) { throw 'no attached display: no display device reports DISPLAY_DEVICE_ATTACHED_TO_DESKTOP; the guest has a session but no monitor (see vm-provision-virtual-display.ps1)' }
Write-Output ("attached displays: {0}" -f $attached.Count)
foreach ($display in $attached) { Write-Output ("  attached: {0} ({1})" -f $display.name, $display.model) }

# --- 2. WinForms sees somewhere to present ---------------------------------------
$screens = [System.Windows.Forms.Screen]::AllScreens
if (-not $screens -or $screens.Count -eq 0) { throw 'no attached display: Screen.AllScreens is empty; nothing can present even though a session may exist' }
$primary = @($screens | Where-Object { $_.Primary }) | Select-Object -First 1
if (-not $primary) { throw 'no attached display: Screen.AllScreens reports screens but none is primary' }
$virtual = [System.Windows.Forms.SystemInformation]::VirtualScreen
if ($virtual.Width -le 0 -or $virtual.Height -le 0) { throw 'no attached display: the virtual screen is empty, so no window can present here' }
Write-Output ("screens: {0}; primary {1}; virtual {2}" -f $screens.Count, $primary.Bounds.ToString(), $virtual.ToString())

# --- 3. DWM composition is on ------------------------------------------------------
$dwmSource = @'
using System;
using System.Runtime.InteropServices;
public sealed class DwmProbe {
    [DllImport("dwmapi.dll")]
    public static extern int DwmIsCompositionEnabled(out bool enabled);
}
'@
Add-Type -TypeDefinition $dwmSource -ErrorAction Stop
$composition = $false
$compositionResult = [DwmProbe]::DwmIsCompositionEnabled([ref]$composition)
if ($compositionResult -ne 0) { throw ("no attached display: DwmIsCompositionEnabled failed with HRESULT 0x{0:X8}; composition state is unknown" -f $compositionResult) }
if (-not $composition) { throw 'no attached display: DWM composition is off; the session exists but has no presentable surface (no persistent display driver)' }
Write-Output 'dwm composition: enabled'

[pscustomobject]@{
    attached_count  = $attached.Count
    attached        = @($attached | ForEach-Object { ("{0} ({1})" -f $_.name, $_.model) })
    screens         = $screens.Count
    primary         = $primary.Bounds.ToString()
    virtual_screen  = $virtual.ToString()
    dwm_composition = $composition
}
