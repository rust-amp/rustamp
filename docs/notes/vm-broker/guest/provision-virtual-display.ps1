# Provision the PERSISTENT VIRTUAL DISPLAY in the headless test VM.  Allow-listed guest script.
#
# Called by the broker exactly like run-tests.ps1:
#
#   broker -Op guest -Script provision-virtual-display.ps1 -GuestArgs <Phase> <DriverInfPath> <AcknowledgeDriver>
#
# Or over PowerShell Direct from an elevated setup shell:
#
#   Invoke-Command -VMName rustamp-test -Credential $c -FilePath provision-virtual-display.ps1 -ArgumentList 'TestSigning', 'C:\rustamp\drivers\usbmmidd\usbmmidd.inf', '<acknowledgment>'
#
# Why two phases: an Indirect Display Driver from the community is UNSIGNED, so
# the guest must first allow test-signed drivers (`bcdedit /set testsigning on`,
# Phase TestSigning) and REBOOT before the driver can be installed (Phase
# Install). The orchestrator drives both phases through the broker; the reboots
# between them are broker stop/start pairs from the host side
# (docs\notes\vm-provision-virtual-display-host.ps1), because the broker has no
# reboot op and a self-reboot would drop the PowerShell Direct session.
#
# Division of labour (unchanged): the host stages the driver unelevated, the
# broker only touches the VM (start, copyin, guest, stop), the broker never
# builds, and this script only installs what is already staged. It NEVER
# downloads anything: the INF must already be in the guest (broker copyin).
#
# Approval: the owner approved this exact third-party driver install and the
# test-signing change for this guest (2026-10-16). -AcknowledgeDriver records
# that approval per run: it must equal the expected string below, which names
# the exact binary and source. Anything else throws and installs nothing.
#
# What each phase changes and how it is reverted:
#   TestSigning: sets the BCD `testsigning` flag on. After the reboot the
#     desktop shows a "Test Mode" watermark and Windows loads test-signed
#     drivers. Revert (elevated guest shell): `bcdedit /set testsigning off`
#     followed by a reboot; the watermark goes away.
#   Install: adds the staged driver package to the driver store
#     (`pnputil /add-driver ... /install`) and attaches one virtual monitor
#     (`deviceinstaller64 enableidd 1` from the staged driver directory, which
#     arrived in the guest with the same copyin). Revert (elevated guest
#     shell): `deviceinstaller64 enableidd 0`, then
#     `pnputil /delete-driver <oemNN.inf> /uninstall` (the published name is
#     printed by this script), then a reboot.
#
# Verification after Install: the display proof
# (assert-attached-display.ps1) with no viewer connected. The session proof
# (assert-interactive-session.ps1) and the display proof stay separate claims:
# session without display is the expected state before this step, session with
# display the expected state after it.

param(
    # 'TestSigning' enables test-signed drivers; 'Install' installs the driver.
    [string]$Phase = 'TestSigning',
    # Local path INSIDE the guest to the already-staged driver INF, e.g.
    # 'C:\rustamp\drivers\usbmmidd\usbmmidd.inf'. Only used by Phase Install.
    [string]$DriverInfPath = 'C:\rustamp\drivers\usbmmidd\usbmmidd.inf',
    # The owner's explicit approval for this exact third-party driver install.
    # Must equal $ExpectedAcknowledge below, verbatim.
    [string]$AcknowledgeDriver = ''
)

$ErrorActionPreference = 'Stop'

$ExpectedAcknowledge = 'I approve usbmmidd_v2.zip from https://www.amyuni.com/downloads/usbmmidd_v2.zip (Amyuni USB Mobile Monitor virtual display, Indirect Display Driver) in the rustamp-test guest, including bcdedit testsigning on'

Write-Output 'virtual-display provisioning: option B (IDD virtual monitor) - option A (Hyper-V synthetic video) is already provisioned and kept but insufficient alone'

if ($AcknowledgeDriver -ne $ExpectedAcknowledge) {
    throw ('third-party driver install NOT approved: nothing was changed. ' +
        'Pass -AcknowledgeDriver with exactly: ' +
        ("'{0}'. " -f $ExpectedAcknowledge) +
        'The host side (docs\notes\vm-provision-virtual-display-host.ps1) passes it; the broker copyin stages the INF first - this script never downloads.')
}

function Get-TestSigning {
    $entry = bcdedit /enum | Select-String '^\s*testsigning\s+(\S+)' | Select-Object -First 1
    if (-not $entry) { return '' }
    return $entry.Matches[0].Groups[1].Value
}

if ($Phase -eq 'TestSigning') {
    Write-Output '--- phase TestSigning: allow test-signed drivers (owner-approved) ---'
    $current = Get-TestSigning
    Write-Output ("    BCD testsigning is '{0}'" -f $current)
    if ($current -eq 'Yes') {
        Write-Output 'test signing is already on; reboot the guest so it is active, then run Phase Install'
        return
    }
    # Fails while Secure Boot is on; the fix is an elevated HOST step, never
    # something this script can do: Set-VMFirmware -VMName rustamp-test
    # -EnableSecureBoot Off (the VM is a disposable test VM), then re-run.
    & bcdedit /set testsigning on
    if ($LASTEXITCODE -ne 0) {
        throw ('bcdedit refused testsigning (exit {0}); if it names Secure Boot, the owner must disable it on the VM first: ' -f $LASTEXITCODE +
            'Set-VMFirmware -VMName rustamp-test -EnableSecureBoot Off (elevated host shell), then re-run this phase')
    }
    $current = Get-TestSigning
    Write-Output ("    BCD testsigning is now '{0}' (takes effect at boot)" -f $current)
    if ($current -ne 'Yes') { throw 'bcdedit reported success but testsigning is not Yes; refusing to claim a change that is not there' }
    Write-Output 'REBOOT the guest (broker stop/start from the host side) so test signing is active, then run Phase Install'
    Write-Output 'REVERT with: bcdedit /set testsigning off, then reboot (watermark goes away)'
    return
}

if ($Phase -eq 'Install') {
    Write-Output '--- phase Install: install the staged virtual display driver (owner-approved) ---'
    $signing = Get-TestSigning
    Write-Output ("    BCD testsigning is '{0}'" -f $signing)
    if ($signing -ne 'Yes') {
        throw 'test signing is not active (no reboot since Phase TestSigning, or it was turned back off); installing an unsigned driver now would fail, so nothing was installed - reboot first, then re-run this phase'
    }
    if ([string]::IsNullOrWhiteSpace($DriverInfPath)) { throw 'approved but no -DriverInfPath was given: point it at the staged INF inside the guest (this script never downloads)' }
    if (-not (Test-Path $DriverInfPath)) { throw ("staged driver INF not found at '{0}'; copy it in with the broker copyin op first (no download from here)" -f $DriverInfPath) }

    Write-Output ("installing virtual display driver from '{0}' (owner-approved)" -f $DriverInfPath)
    & pnputil /add-driver $DriverInfPath /install
    if ($LASTEXITCODE -ne 0) {
        $known = pnputil /enum-drivers | Select-String 'usbmmidd' | Select-Object -First 1
        if ($known) {
            Write-Output ("    pnputil exited {0}, but the driver is already in the store ({1}); continuing to the monitor attach" -f $LASTEXITCODE, $known.Line.Trim())
        } else {
            throw ("pnputil refused the driver with exit {0}; nothing was baked in" -f $LASTEXITCODE)
        }
    } else {
        Write-Output 'driver package added to the store'
    }
    $published = pnputil /enum-drivers | Select-String 'Published Name:\s*(\S+)' -Context 0, 4 | Where-Object { $_.Context.PostContext -match 'usbmmidd' } | ForEach-Object { $_.Matches[0].Groups[1].Value } | Select-Object -First 1
    if ($published) { Write-Output ("    published as '{0}' (needed for the revert: pnputil /delete-driver {0} /uninstall)" -f $published) }

    # Attaching the monitor is separate from installing the driver package: the
    # installer binary arrived in the guest with the same copyin, so using it
    # adds no new software. If it is not beside the INF, only warn - the
    # display proof afterwards decides whether this was enough.
    $installer = Join-Path (Split-Path $DriverInfPath -Parent) 'deviceinstaller64.exe'
    if (Test-Path $installer) {
        Write-Output 'attaching one virtual monitor (deviceinstaller64 enableidd 1)'
        & $installer enableidd 1
        Write-Output ("    enableidd exited {0}" -f $LASTEXITCODE)
        if ($LASTEXITCODE -ne 0) { throw ("enableidd refused with exit {0}; the driver is stored but no monitor is attached" -f $LASTEXITCODE) }
    } else {
        Write-Warning ("no installer beside the INF at '{0}'; skipping the monitor attach - the display proof decides" -f $installer)
    }
    Write-Output 'driver installed; reboot the guest so the virtual monitor enumerates, then run the display proof with no viewer connected'
    Write-Output 'CHECKPOINT: this change must be baked in like auto-logon - remove checkpoint base-provisioned (elevated host step, owner) and re-take it after the reboot + display proof pass, or every restore loses the display'
    Write-Output 'REVERT with (elevated guest shell): deviceinstaller64 enableidd 0, then pnputil /delete-driver <published oemNN.inf> /uninstall, then reboot; plus bcdedit /set testsigning off + reboot to leave test signing too'
    return
}

throw ("unknown -Phase '{0}': use 'TestSigning' or 'Install'" -f $Phase)
