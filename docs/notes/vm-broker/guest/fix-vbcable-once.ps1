# Keep exactly one VB-CABLE instance: remove all, install once, report.
#
# Runs INSIDE the guest (allow-listed plain name). Owner-approved install.

$ErrorActionPreference = 'Continue'

$devcon = Get-ChildItem 'C:\rustamp\drivers' -Recurse -Filter 'deviceinstaller64.exe' -ErrorAction SilentlyContinue | Select-Object -First 1
$inf = Get-ChildItem 'C:\rustamp\drivers\vbcable' -Recurse -Filter 'vbMmeCable64_win7.inf' -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $devcon -or -not $inf) { throw 'driver files missing' }

Write-Output '=== 1. remove all cable devices ==='
(& $devcon.FullName remove 'VBAudioVACWDM' 2>&1) | ForEach-Object { Write-Output ('  ' + $_) }
Start-Sleep -Seconds 3

Write-Output '=== 2. install exactly once ==='
Push-Location $inf.DirectoryName
(& $devcon.FullName install $inf.Name 'VBAudioVACWDM' 2>&1) | ForEach-Object { Write-Output ('  ' + $_) }
Pop-Location

Write-Output '=== 3. cable devices now ==='
Get-PnpDevice -ErrorAction SilentlyContinue | Where-Object { $_.FriendlyName -match 'VB-Audio|Virtual Cable' } |
    ForEach-Object { Write-Output ("  {0} | {1} | {2}" -f $_.Status, $_.FriendlyName, $_.InstanceId) }
Write-Output '=== done: reboot, then check AudioEndpoint ==='
