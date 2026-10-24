# Pin the virtual display to the size the capture gate expects (1024x768).
#
# The reference measurements were taken on a 1024x768 screen, and the capture
# harness's one-to-one gate assumes that size. The signed virtual display
# driver defaults to 800x600, which extends the desktop to 1824x768 - fine for
# the window tests, wrong for a capture run.
#
# Called by the broker BEFORE the job, ordered by the host runner
# (docs\notes\vm-run-capture.ps1):
#
#   broker -Op guest -Script set-vdd-mode.ps1 -GuestArgs Set1024   # default
#   broker -Op guest -Script set-vdd-mode.ps1 -GuestArgs Off       # VDD off
#
# Modes:
#   Set1024 - write the 1024x768 setting the driver reads
#             (vdd_settings.xml under C:\rustamp\drivers\vdd, provisioned into
#             the checkpoint image, so it survives the restore) and restart the
#             device so the new mode is applied.
#   Off     - disable the virtual display device instead, so the desktop falls
#             back to the synthetic adapter's mode. Re-enable with
#             `pnputil /enable-device <instance>`; in the capture path no manual
#             re-enable is needed because the next run starts from a restore.
#
# This script is SELF-VERIFYING: after applying the mode it polls the virtual
# screen (up to ~90 s - a device restart applies asynchronously) and THROWS
# with the measured values unless the screen is exactly 1024x768. A capture
# run must never proceed on a wrong size.
#
# Runs INSIDE the guest (allow-listed plain name). Idempotent.
#
# Result contract: this script's result is parsed by the host runner, so it
# emits EXACTLY ONE object (the summary below) and nothing else. Progress
# detail goes into the throw message on failure, and the host echoes the
# summary fields to its own console. Extra Write-Output lines would come back
# as a mixed array and break the host's field reads.

param(
    [ValidateSet('Set1024', 'Off')]
    [string]$Mode = 'Set1024'
)

$ErrorActionPreference = 'Stop'

Add-Type -AssemblyName System.Windows.Forms

$deviceExit = $null
$deviceNote = ''

if ($Mode -eq 'Set1024') {
    # The driver reads vdd_settings.xml from its own directory (staged with the
    # package) and the device re-reads it when restarted.
    $dir = 'C:\rustamp\drivers\vdd'
    $xml = Get-ChildItem $dir -Recurse -Filter 'vdd_settings.xml' -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $xml) { throw "set-vdd-mode refused: vdd_settings.xml not found under $dir (is the driver in this checkpoint?)" }

    $content = @"
<?xml version='1.0' encoding='utf-8'?>
<vdd_settings>
  <monitors>
    <count>1</count>
  </monitors>
  <gpu>
    <friendlyname>default</friendlyname>
  </gpu>
  <resolutions>
    <resolution>
      <width>1024</width>
      <height>768</height>
      <refresh_rate>60</refresh_rate>
    </resolution>
  </resolutions>
</vdd_settings>
"@
    Set-Content -Path $xml.FullName -Value $content -Encoding UTF8

    $dev = Get-PnpDevice -ErrorAction SilentlyContinue | Where-Object { $_.InstanceId -match 'MttVDD|ROOT\\DISPLAY' } | Select-Object -First 1
    if (-not $dev) {
        $deviceNote = 'no display device found to restart; waiting to see whether the screen is already right'
    } else {
        $out = (& pnputil /restart-device $dev.InstanceId 2>&1) | Out-String
        $deviceExit = $LASTEXITCODE
        $deviceNote = ("restart-device {0} exit {1}: {2}" -f $dev.InstanceId, $deviceExit, $out.Trim())
        if ($deviceExit -ne 0) { $deviceNote = 'RESTART FAILED; ' + $deviceNote }
    }
} else {
    $dev = Get-PnpDevice -ErrorAction SilentlyContinue | Where-Object { $_.InstanceId -match 'MttVDD|ROOT\\DISPLAY' } | Select-Object -First 1
    if (-not $dev) {
        $deviceNote = 'no virtual display device found; it may already be off or absent from this checkpoint'
    } else {
        $out = (& pnputil /disable-device $dev.InstanceId 2>&1) | Out-String
        $deviceExit = $LASTEXITCODE
        $deviceNote = ("disable-device {0} exit {1}: {2}" -f $dev.InstanceId, $deviceExit, $out.Trim())
        if ($deviceExit -ne 0) { $deviceNote = 'DISABLE FAILED; ' + $deviceNote }
    }
}

# --- settle and verify: the size is the requirement, the device op was means --
$screen = [System.Windows.Forms.SystemInformation]::VirtualScreen
$deadline = (Get-Date).AddSeconds(90)
while (($screen.Width -ne 1024 -or $screen.Height -ne 768) -and (Get-Date) -lt $deadline) {
    Start-Sleep -Seconds 3
    $screen = [System.Windows.Forms.SystemInformation]::VirtualScreen
}

$logPixels = (Get-ItemProperty 'HKCU:\Control Panel\Desktop' -Name LogPixels -ErrorAction SilentlyContinue).LogPixels
$screens = @([System.Windows.Forms.Screen]::AllScreens | ForEach-Object { ("{0} primary={1} bounds={2}" -f $_.DeviceName, $_.Primary, $_.Bounds) })

if ($screen.Width -ne 1024 -or $screen.Height -ne 768) {
    throw ("set-vdd-mode refused: virtual screen is {0}x{1} after mode '{2}', want 1024x768 (logpixels {3}; screens: {4}; device: {5})" -f `
        $screen.Width, $screen.Height, $Mode, $logPixels, ($screens -join ' | '), $deviceNote)
}

[pscustomobject]@{
    mode       = $Mode
    screen     = $screen.ToString()
    screens    = @($screens)
    logPixels  = $logPixels
    deviceNote = $deviceNote
}
