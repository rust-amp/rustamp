# Set the virtual display driver's monitor to the size the project assumes.
#
# The reference measurements were taken on a 1024x768 screen, and the capture
# harness's one-to-one gate assumes that size. The signed virtual display driver
# defaults to 800x600, which extends the desktop to 1824x768 - fine for the
# window tests, wrong for a capture run. This writes the setting the driver
# reads and restarts the device so the new mode is applied.
#
# Runs INSIDE the guest (allow-listed plain name). Idempotent.

$ErrorActionPreference = 'Continue'

$target = 1024, 768
Write-Output ('target mode: {0}x{1}' -f $target[0], $target[1])

# The driver reads vdd_settings.xml from its own directory (staged with the
# package) and the device re-reads it when restarted.
$dir = 'C:\rustamp\drivers\vdd'
$xml = Get-ChildItem $dir -Recurse -Filter 'vdd_settings.xml' -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $xml) { throw "vdd_settings.xml not found under $dir" }
Write-Output ('settings: ' + $xml.FullName)

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
Write-Output 'settings written'

Write-Output '=== restart the display device so the mode is applied ==='
$dev = Get-PnpDevice -ErrorAction SilentlyContinue | Where-Object { $_.InstanceId -match 'MttVDD|ROOT\\DISPLAY' } | Select-Object -First 1
if (-not $dev) { Write-Output '  no display device found to restart' }
else {
    Write-Output ('  device: {0} | {1}' -f $dev.Status, $dev.InstanceId)
    (& pnputil /restart-device $dev.InstanceId 2>&1) | ForEach-Object { Write-Output ('  ' + $_) }
    Write-Output ('  exit: ' + $LASTEXITCODE)
}

Write-Output '=== screens now ==='
Add-Type -AssemblyName System.Windows.Forms -ErrorAction SilentlyContinue
[System.Windows.Forms.Screen]::AllScreens | ForEach-Object { Write-Output ("  {0} primary={1} bounds={2}" -f $_.DeviceName, $_.Primary, $_.Bounds) }
Write-Output '=== done ==='
