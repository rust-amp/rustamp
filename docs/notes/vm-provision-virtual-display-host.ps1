# Provision the persistent virtual display in the headless test VM, end to end.
# RUN UNELEVATED, FROM THE ORCHESTRATOR.  The single command is:
#
#   powershell -ExecutionPolicy Bypass -File docs\notes\vm-provision-virtual-display-host.ps1
#
# The division of labour is deliberate and security-relevant (same shape as
# docs\notes\vm-run-tests.ps1):
#   * HERE (unelevated): fetch nothing, download nothing - the orchestrator
#     fetches the driver zip on the host (owner asks before every download)
#     and this script only expands and stages what is already there.
#     Compiling would execute build scripts, so like the test runner this
#     script never builds either.
#   * THE BROKER (elevated, session 0): only touches the VM - start, copyin,
#     guest (an allow-listed script), stop. It NEVER builds and NEVER downloads.
#
# What it does, in order:
#   1. expands the already-fetched driver zip into the broker staging area
#      (unelevated),
#   2. starts the guest, copies the driver in (broker copyin),
#   3. runs the allow-listed guest script Phase TestSigning (bcdedit
#      testsigning on; owner-approved, see the guest script header),
#   4. reboots the guest (broker stop + start) so test signing is active,
#   5. runs the guest script Phase Install (pnputil + monitor attach),
#   6. reboots again so the virtual monitor enumerates,
#   7. waits for BOTH proofs - the session proof
#      (assert-interactive-session.ps1) and the display proof
#      (assert-attached-display.ps1) - and reports each LOUDLY.
#
# The driver (the orchestrator fetches this on the host; this script never does):
#   usbmmidd_v2.zip from https://www.amyuni.com/downloads/usbmmidd_v2.zip
#   (Amyuni USB Mobile Monitor virtual display, an Indirect Display Driver).
#   Expected layout inside the zip: a folder holding usbmmidd.inf (the INF that
#   pnputil needs), usbmmidd.sys, and deviceinstaller.exe/deviceinstaller64.exe
#   (the monitor attach tool, staged into the guest with the same copyin).
#   The script finds usbmmidd.inf wherever it sits under the expanded zip and
#   stages its whole directory, so a flat or a nested zip both work.
#
# Proofs: session without display is the expected state before the driver,
# session WITH display the expected state after it. The run prints both lines
# and exits 0 only when both are proven; it never reports a pass it did not
# have (exit 2 = session proven, display missing; exit 1 = no session proven).
#
# Checkpoint: the driver and the test-signing flag must be baked in like
# auto-logon. The broker can CREATE but not REMOVE a checkpoint, so:
#   * REMOVE the stale one in an ELEVATED host shell (owner):
#       .\docs\notes\vm-snapshot-test-host.ps1 -Remove base-provisioned
#   * then re-take it: either pass -TakeCheckpoint here (broker checkpoint op,
#     only after both proofs pass) or, elevated:
#       .\docs\notes\vm-snapshot-test-host.ps1 -Create base-provisioned
# Until then every restore loses the display again.

[CmdletBinding()]
param(
    # Host-local path of the already-fetched driver zip. Not downloaded here.
    [string]$DriverZip = 'D:\code\rustamp-vm\driver-src\usbmmidd_v2.zip',
    [string]$Stage = 'D:\code\rustamp-vm\staging\drivers\usbmmidd',
    [string]$GuestDrivers = 'C:\rustamp\drivers\usbmmidd',
    [string]$GuestInf = 'C:\rustamp\drivers\usbmmidd\usbmmidd.inf',
    [string]$VmName = 'rustamp-test',
    [string]$Baseline = 'base-provisioned',
    # Re-take the baseline through the broker after both proofs pass. Removal
    # of the stale checkpoint is still an elevated owner step (see header).
    [switch]$TakeCheckpoint,
    [int]$ProofMinutes = 15
)

$ErrorActionPreference = 'Stop'
$broker = Join-Path $PSScriptRoot 'vm-broker\invoke-broker.ps1'
# Must equal $ExpectedAcknowledge in vm-broker\guest\provision-virtual-display.ps1, verbatim.
$Acknowledge = 'I approve usbmmidd_v2.zip from https://www.amyuni.com/downloads/usbmmidd_v2.zip (Amyuni USB Mobile Monitor virtual display, Indirect Display Driver) in the rustamp-test guest, including bcdedit testsigning on'

function Step($t) { Write-Host "--- $t" }

# --- 1. expand and stage on the host (no download, no build) ----------------
Step 'staging the already-fetched driver (no download happens here)'
if (-not (Test-Path $DriverZip)) {
    throw ("driver zip not found at '{0}'. The orchestrator fetches it on the host (ask the owner first - every download needs approval): " -f $DriverZip +
        'usbmmidd_v2.zip from https://www.amyuni.com/downloads/usbmmidd_v2.zip ' +
        '(Amyuni USB Mobile Monitor virtual display). Verifiable origin: the vendor download page; ' +
        'the zip is expected to hold usbmmidd.inf + usbmmidd.sys + deviceinstaller64.exe. This script never downloads.')
}
$expand = Join-Path ([System.IO.Path]::GetTempPath()) 'rustamp-usbmmidd'
if (Test-Path $expand) { Remove-Item -Recurse -Force $expand }
$null = New-Item -ItemType Directory -Force -Path $expand
Expand-Archive -Path $DriverZip -DestinationPath $expand -Force
$inf = Get-ChildItem -Path $expand -Filter 'usbmmidd.inf' -Recurse -File | Select-Object -First 1
if (-not $inf) { throw ("usbmmidd.inf not found inside '{0}'; the zip does not hold the expected driver layout (usbmmidd.inf + usbmmidd.sys)" -f $DriverZip) }
Write-Host ("    INF: {0}" -f $inf.FullName)
if (Test-Path $Stage) { Remove-Item -Recurse -Force $Stage }
$null = New-Item -ItemType Directory -Force -Path $Stage
Copy-Item -Path (Join-Path $inf.DirectoryName '*') -Destination $Stage -Recurse -Force
Remove-Item -Recurse -Force $expand
Write-Host ("    staged into {0}" -f $Stage)

# --- 2. start the guest -------------------------------------------------------
Step 'starting the guest (the broker ejects the install DVD so the guest gets D:)'
& $broker -Op start -TimeoutSeconds 900 | Out-Null

# --- 3. copy the driver in ----------------------------------------------------
Step 'copying the staged driver into the guest'
& $broker -Op copyin -Source $Stage -Dest $GuestDrivers -TimeoutSeconds 600 | Out-Null

# --- 4. phase TestSigning -----------------------------------------------------
Step 'enabling test-signed drivers in the guest (owner-approved)'
& $broker -Op guest -Script 'provision-virtual-display.ps1' -GuestArgs @('TestSigning', $GuestInf, $Acknowledge) -TimeoutSeconds 600 | Out-Null

# --- 5. reboot so test signing is active --------------------------------------
Step 'rebooting the guest so test signing is active (broker stop/start)'
& $broker -Op stop -TimeoutSeconds 600 | Out-Null
& $broker -Op start -TimeoutSeconds 900 | Out-Null

# --- 6. phase Install ---------------------------------------------------------
Step 'installing the virtual display driver in the guest (owner-approved)'
& $broker -Op guest -Script 'provision-virtual-display.ps1' -GuestArgs @('Install', $GuestInf, $Acknowledge) -TimeoutSeconds 600 | Out-Null

# --- 7. reboot so the virtual monitor enumerates ------------------------------
Step 'rebooting the guest so the virtual monitor enumerates (broker stop/start)'
& $broker -Op stop -TimeoutSeconds 600 | Out-Null
& $broker -Op start -TimeoutSeconds 900 | Out-Null

# --- 8. prove the SESSION (loud, separate claim) ------------------------------
Step 'proving the interactive SESSION'
$sessionProven = $false
$deadline = (Get-Date).AddMinutes($ProofMinutes)
while ((Get-Date) -lt $deadline) {
    try {
        & $broker -Op guest -Script 'assert-interactive-session.ps1' -TimeoutSeconds 600 | Format-List
        $sessionProven = $true
        break
    } catch {
        Write-Host ("    session not yet: {0}" -f $_.Exception.Message)
        Start-Sleep -Seconds 30
    }
}
if ($sessionProven) {
    Write-Host 'SESSION proven: explorer runs outside session 0 and the desktop is non-empty'
} else {
    Write-Host 'SESSION missing: no interactive session within the deadline; no checkpoint is taken and nothing is promoted'
}

# --- 9. prove the DISPLAY (loud, separate claim) ------------------------------
Step 'proving the ATTACHED DISPLAY (separate claim from the session)'
$displayProven = $false
if ($sessionProven) {
    try {
        & $broker -Op guest -Script 'assert-attached-display.ps1' -TimeoutSeconds 600 | Format-List
        $displayProven = $true
    } catch {
        Write-Host ("    DISPLAY missing: {0}" -f $_.Exception.Message)
    }
} else {
    Write-Host 'DISPLAY not attempted: without a session the display claim is meaningless'
}
if ($displayProven) {
    Write-Host 'DISPLAY proven: a persistent attached display is present with no viewer connected'
} else {
    Write-Host 'DISPLAY missing: session proven, display missing - the expected state before the driver enumerates; install/attach did not take (see the guest script output above)'
}

Write-Host '--- proof summary ---'
Write-Host ("    session: {0}" -f ($(if ($sessionProven) { 'PROVEN' } else { 'MISSING' })))
Write-Host ("    display: {0}" -f ($(if ($displayProven) { 'PROVEN' } else { 'MISSING' })))

# --- 10. checkpoint (only past both proofs; broker can create, not remove) ----
if ($sessionProven -and $displayProven) {
    if ($TakeCheckpoint) {
        Step "re-taking checkpoint $Baseline through the broker"
        try {
            & $broker -Op checkpoint -Name $Baseline -TimeoutSeconds 900 | Out-Null
            Write-Host ("    checkpoint '{0}' re-taken: driver and test signing are baked in" -f $Baseline)
        } catch {
            Write-Warning ("checkpoint was NOT re-taken: {0}" -f $_.Exception.Message)
            Write-Host ("    If it already exists, remove the stale one in an ELEVATED host shell (owner): .\docs\notes\vm-snapshot-test-host.ps1 -Remove {0} " -f $Baseline +
                ("then re-take: broker checkpoint, or .\docs\notes\vm-snapshot-test-host.ps1 -Create {0}" -f $Baseline))
        }
    } else {
        Write-Host ("CHECKPOINT: remove the stale '{0}' in an ELEVATED host shell (owner): .\docs\notes\vm-snapshot-test-host.ps1 -Remove {0} " -f $Baseline +
            ("then re-take it (broker: -Op checkpoint -Name {0}, or -Create {0} on the same elevated script), or every restore loses the display" -f $Baseline))
    }
} else {
    Write-Host 'CHECKPOINT: not re-taken - only a run with session PROVEN and display PROVEN may become the new baseline'
}

if (-not $sessionProven) { exit 1 }
if (-not $displayProven) { exit 2 }
