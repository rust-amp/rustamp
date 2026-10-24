# Provision a PERSISTENT VIRTUAL DISPLAY in the headless test VM.  Runs INSIDE the guest.
#
# The orchestrator runs this over PowerShell Direct from an elevated host shell,
# after docs\notes\vm-provision-test-host.ps1, so nothing appears on the owner's
# desktop:
#
#   $c = Get-Credential            # the guest's local account, e.g. rustamp
#   Invoke-Command -VMName rustamp-test -Credential $c `
#     -FilePath docs\notes\vm-provision-virtual-display.ps1 `
#     -ArgumentList 'C:\rustamp\drivers\usbmmidd\usbmmidd.inf', '<acknowledgment>'
#
# The runnable orchestrator path is docs\notes\vm-provision-virtual-display-host.ps1:
# a single unelevated host command that stages the driver, provisions the guest
# through the broker (allow-listed guest script
# vm-broker\guest\provision-virtual-display.ps1, same two steps as below),
# reboots, waits for both proofs, and reports. Prefer it over this script.
#
# The two options considered, and which one this script implements:
#
#   Option A (platform-only, no new software): the Hyper-V synthetic video
#   adapter (Microsoft Hyper-V Video) with the pinned mode from
#   vm-provision-test-host.ps1 (1024x768, LogPixels 96, power settings off).
#   This is already provisioned and stays: it is inbox, needs no install and
#   no approval. But it is NOT sufficient on its own. With no viewer attached
#   the console is Disconnected, Windows reports no
#   DISPLAY_DEVICE_ATTACHED_TO_DESKTOP, and the graphics subsystem has no
#   presentable surface - a GUI only "wakes up" when somebody looks at the
#   console. A viewer (VMConnect, Enhanced Session, RDP) would wake it, but a
#   viewer opens a GUI on the host, which the project forbids. So option A is
#   kept and documented as insufficient.
#
#   Option B (CHOSEN): an Indirect Display Driver (IDD) virtual monitor - the
#   concrete driver is the Amyuni USB Mobile Monitor virtual display
#   (usbmmidd_v2.zip from https://www.amyuni.com/downloads/usbmmidd_v2.zip:
#   usbmmidd.inf + usbmmidd.sys + deviceinstaller64.exe, installed with
#   pnputil), with Microsoft's IddSampleDriver as the reference implementation
#   of the same IDD class. The driver package is UNSIGNED, so the guest must
#   first allow test-signed drivers (`bcdedit /set testsigning on`, §0 below;
#   owner-approved 2026-10-16 together with the install) and reboot before the
#   install can succeed.
#   An IDD presents a monitor Windows believes is physically attached, so
#   EnumDisplayDevices reports DISPLAY_DEVICE_ATTACHED_TO_DESKTOP, WinForms
#   sees a primary screen, and DWM composition stays enabled even with the
#   console disconnected and no viewer ever opened. That is the persistent
#   display the headless guest is missing. It requires installing third-party
#   driver software in the guest, so it needs the owner's explicit approval
#   (see below) and, once installed, it MUST be baked into the checkpoint
#   like auto-logon: the old `base-provisioned` (taken without the driver)
#   must be removed and re-taken, or every restore loses the display again.
#
# Approval gate: this script installs NOTHING unless the run records the owner's
# approval for the exact driver binary and source (approved 2026-10-16: the
# Amyuni usbmmidd_v2 driver install AND the `bcdedit /set testsigning on`
# change in this guest). Pass -AcknowledgeThirdPartyDriver with
# -DriverInfPath pointing at an INF already present in the guest (copied in
# with the broker `copyin` op - this script NEVER downloads anything). Without
# both, it throws and names exactly what must be approved. No download and no
# install happen in this lane.
#
# Verification after install: run the display proof
# (vm-broker\guest\assert-attached-display.ps1) with no viewer connected. The
# session proof (assert-interactive-session.ps1) and the display proof stay
# separate claims: session without display is the expected state before this
# step, session with display the expected state after it.

param(
    # Local path INSIDE the guest to an already-staged driver INF, e.g.
    # 'C:\rustamp\drivers\usbmmidd\usbmmidd.inf'. The file must already be
    # there (broker copyin); this script never fetches it from the network.
    [string]$DriverInfPath = '',
    # The owner's explicit approval for this third-party driver install.
    # Without it the script refuses rather than installing anything.
    [switch]$AcknowledgeThirdPartyDriver
)

$ErrorActionPreference = 'Stop'

Write-Output 'virtual-display provisioning: option B (IDD virtual monitor) - option A (Hyper-V synthetic video) is already provisioned and kept but insufficient alone; see header'

if (-not $AcknowledgeThirdPartyDriver) {
    throw ('third-party driver install NOT approved: nothing was installed. ' +
        'To approve, the owner names the exact driver binary and source ' +
        '(driver: usbmmidd_v2.zip from https://www.amyuni.com/downloads/usbmmidd_v2.zip - Amyuni USB Mobile Monitor virtual display, ' +
        'usbmmidd.inf + usbmmidd.sys + deviceinstaller64.exe, an Indirect Display Driver; ' +
        'reference: Microsoft IddSampleDriver), stages its INF into the guest with the broker copyin op, ' +
        'then re-runs this script with -DriverInfPath <guest-local INF path> -AcknowledgeThirdPartyDriver. ' +
        'After a successful install the display proof must pass with no viewer connected, and checkpoint ' +
        "base-provisioned' must be removed and re-taken so the driver is baked in.")
}
if ([string]::IsNullOrWhiteSpace($DriverInfPath)) { throw 'approved but no -DriverInfPath was given: point it at the staged INF inside the guest (this script never downloads)' }
if (-not (Test-Path $DriverInfPath)) { throw ("staged driver INF not found at '{0}'; copy it in with the broker copyin op first (no download from here)" -f $DriverInfPath) }

# --- §0. allow test-signed drivers (owner-approved 2026-10-16) ----------------
# The IDD driver package is unsigned, so pnputil refuses it while signature
# enforcement is on. What this changes: the BCD `testsigning` flag; after the
# reboot the desktop shows a "Test Mode" watermark and Windows loads
# test-signed drivers. Revert (elevated guest shell): `bcdedit /set
# testsigning off` followed by a reboot; the watermark goes away. The flag
# takes effect only at boot, so a fresh enable means reboot and re-run: the
# driver install below runs only once test signing is ACTIVE.
$signing = (bcdedit /enum | Select-String '^\s*testsigning\s+(\S+)' | Select-Object -First 1)
$signingValue = if ($signing) { $signing.Matches[0].Groups[1].Value } else { '' }
Write-Output ("BCD testsigning is '{0}'" -f $signingValue)
if ($signingValue -ne 'Yes') {
    & bcdedit /set testsigning on
    if ($LASTEXITCODE -ne 0) {
        throw ('bcdedit refused testsigning; if it names Secure Boot, the owner must disable it on the VM first: ' +
            'Set-VMFirmware -VMName rustamp-test -EnableSecureBoot Off (elevated host shell), then re-run')
    }
    Write-Output 'test signing enabled; REBOOT the guest and re-run this script with the same flags - the driver install below runs only once test signing is active'
    Write-Output 'REVERT with: bcdedit /set testsigning off, then reboot'
    return
}

Write-Output ("installing virtual display driver from '{0}' (owner-approved)" -f $DriverInfPath)
& pnputil /add-driver $DriverInfPath /install
if ($LASTEXITCODE -ne 0) { throw ("pnputil refused the driver with exit {0}; nothing was baked in" -f $LASTEXITCODE) }
Write-Output 'driver installed; reboot the guest so the virtual monitor enumerates, then run the display proof with no viewer connected'

Write-Output 'CHECKPOINT: this change must be baked in like auto-logon - remove checkpoint base-provisioned and re-take it after the reboot + display proof pass, or every restore loses the display'
