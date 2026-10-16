# Install VB-Audio Virtual Cable (CABLE Input + CABLE Output) in the guest.
#
# VB-CABLE is a signed virtual audio device: playing to "CABLE Input" makes the
# same samples appear on "CABLE Output", which a test can record and compare.
# That is how the project verifies real playback without a microphone.
#
# Why not the vendor setup exe: VBCABLE_Setup_x64.exe opens its installer GUI
# even with a switch and waits for clicks that can never come in session 0, so
# the install hangs forever. The INF route below uses only pnputil plus a
# devcon-equivalent that is already staged in the guest, and never shows UI.
#
# Runs INSIDE the guest (allow-listed plain name). Owner-approved install.

$ErrorActionPreference = 'Continue'

$dir = 'C:\rustamp\drivers\vbcable'
$inf = Get-ChildItem $dir -Recurse -Filter 'vbMmeCable64_win7.inf' -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $inf) { throw "vbMmeCable64_win7.inf not found under $dir (stage it with the broker copyin op first)" }
Write-Output ('inf: ' + $inf.FullName)

$devcon = Get-ChildItem 'C:\rustamp\drivers' -Recurse -Filter 'deviceinstaller64.exe' -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $devcon) { throw 'deviceinstaller64.exe not found under C:\rustamp\drivers (stage the usbmmidd package first)' }

Write-Output '=== 1. endpoints before ==='
Get-PnpDevice -Class AudioEndpoint -ErrorAction SilentlyContinue |
    ForEach-Object { Write-Output ("  {0} | {1}" -f $_.Status, $_.FriendlyName) }

Write-Output '=== 2. trust the signer (the store rejects an unknown publisher) ==='
$cat = Get-ChildItem $inf.DirectoryName -Filter 'vbaudio_cable64_win7.cat' -ErrorAction SilentlyContinue | Select-Object -First 1
if ($cat) {
    $sig = Get-AuthenticodeSignature -FilePath $cat.FullName
    Write-Output ('  catalog status: ' + $sig.Status)
    if ($sig.SignerCertificate) {
        $cerPath = Join-Path $inf.DirectoryName 'vbcable-signer.cer'
        [System.IO.File]::WriteAllBytes($cerPath, $sig.SignerCertificate.Export([System.Security.Cryptography.X509Certificates.X509ContentType]::Cert))
        (& certutil -addstore -f root $cerPath 2>&1) | Select-Object -Last 1 | ForEach-Object { Write-Output ('  root: ' + $_) }
        (& certutil -addstore -f TrustedPublisher $cerPath 2>&1) | Select-Object -Last 1 | ForEach-Object { Write-Output ('  publisher: ' + $_) }
    } else { Write-Output '  no signer certificate on the catalog' }
} else { Write-Output '  catalog not found next to the INF' }

Write-Output '=== 3. add the driver to the store ==='
(& pnputil /add-driver $inf.FullName /install 2>&1) | ForEach-Object { Write-Output ('  ' + $_) }
Write-Output ('  exit: ' + $LASTEXITCODE)

Write-Output '=== 3. create the device node (devcon-style, no UI) ==='
Push-Location $inf.DirectoryName
(& $devcon.FullName install $inf.Name 'VBAudioVACWDM' 2>&1) | ForEach-Object { Write-Output ('  ' + $_) }
Write-Output ('  exit: ' + $LASTEXITCODE)
Pop-Location

Write-Output '=== 4. audio endpoints now ==='
Get-PnpDevice -Class AudioEndpoint -ErrorAction SilentlyContinue |
    ForEach-Object { Write-Output ("  {0} | {1}" -f $_.Status, $_.FriendlyName) }
Write-Output '=== 5. the cable device ==='
Get-PnpDevice -ErrorAction SilentlyContinue | Where-Object { $_.FriendlyName -match 'VB-Audio|Virtual Cable|VBAudio' } |
    ForEach-Object { Write-Output ("  {0} | {1} | {2}" -f $_.Status, $_.FriendlyName, $_.InstanceId) }
Write-Output '=== done: reboot the guest so the endpoints enumerate ==='
