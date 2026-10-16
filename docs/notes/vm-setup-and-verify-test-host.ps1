# Start, provision, verify and baseline the headless test VM.
# RUN ONCE, IN AN ELEVATED SHELL.
#
#   powershell -ExecutionPolicy Bypass -File docs\notes\vm-setup-and-verify-test-host.ps1
#
# It asks for the guest's local account password (the one you set during the
# Windows install, e.g. rustamp). Nothing of this appears on the desktop: the
# guest is started with Start-VM, never opened with VMConnect, and everything
# inside it is driven over PowerShell Direct (VMBus, no network, no window).
#
# What it does, in order:
#   1. starts the guest and waits for PowerShell Direct,
#   2. runs vm-provision-test-host.ps1 inside it (auto-logon, 100 % scale, no
#      lock screen or sleep) and prints the guest's facts,
#   3. reboots it once so auto-logon and the scale take effect,
#   3b. proves the interactive SESSION (explorer outside session 0, screen
#      non-empty; the `query session` Active row is corroboration only),
#   3c. proves the ATTACHED DISPLAY separately (persistent virtual monitor;
#      session without display is the expected state until the virtual-display
#      driver from vm-provision-virtual-display.ps1 is owner-approved and
#      installed - the run says which of the two it has),
#   4. creates the `base-provisioned` checkpoint (via vm-snapshot-test-host.ps1,
#      which shuts the guest down cleanly first) and starts it again.
#
# The VM is the only place Rustamp tests and any GUI may run (docs/HANDOFF.md,
# the GUI rule). The host compiles; the guest runs.

$ErrorActionPreference = 'Stop'

$VmName = 'rustamp-test'
$Credential = Get-Credential -UserName 'rustamp' -Message 'Password of the guest local account (set during the Windows install)'

function Wait-PowerShellDirect {
    param([int]$TimeoutSeconds = 600)
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ((Get-Date) -lt $deadline) {
        try {
            Invoke-Command -VMName $VmName -Credential $Credential -ScriptBlock { $true } -ErrorAction Stop | Out-Null
            return $true
        } catch { Start-Sleep -Seconds 10 }
    }
    return $false
}

function Wait-GuestReady {
    param([int]$TimeoutSeconds = 600)
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ((Get-Date) -lt $deadline) {
        try {
            $up = Invoke-Command -VMName $VmName -Credential $Credential -ScriptBlock {
                (Get-CimInstance Win32_OperatingSystem).LastBootUpTime
            } -ErrorAction Stop
            if ($up) { return $true }
        } catch { }
        Start-Sleep -Seconds 10
    }
    return $false
}

if (-not (Get-VM -Name $VmName -ErrorAction SilentlyContinue)) { throw "VM '$VmName' does not exist." }

Write-Host '--- 1. starting the guest (headless) ---'
if ((Get-VM -Name $VmName).State -ne 'Running') { Start-VM -Name $VmName }
if (-not (Wait-PowerShellDirect)) { throw 'PowerShell Direct did not come up. Is the guest a Windows guest with this local account?' }

Write-Host '--- 2. provisioning ---'
# The credential travels as an object over the encrypted PowerShell Direct
# channel so the provision script can write Winlogon's DefaultPassword: that
# value is the load-bearing half of auto-logon, and the checkpoint baked
# without it is exactly why the guest had no interactive session.
Invoke-Command -VMName $VmName -Credential $Credential -FilePath (Join-Path $PSScriptRoot 'vm-provision-test-host.ps1') -ArgumentList $Credential

Write-Host '--- 2b. guest facts ---'
Invoke-Command -VMName $VmName -Credential $Credential -ScriptBlock {
    Add-Type -AssemblyName System.Windows.Forms
    [pscustomobject]@{
        user       = $env:USERNAME
        computer   = $env:COMPUTERNAME
        os         = (Get-CimInstance Win32_OperatingSystem).Caption
        build      = (Get-CimInstance Win32_OperatingSystem).BuildNumber
        powershell = $PSVersionTable.PSVersion.ToString()
        screen     = [System.Windows.Forms.SystemInformation]::VirtualScreen.ToString()
        logpixels  = (Get-ItemProperty 'HKCU:\Control Panel\Desktop' -Name LogPixels -ErrorAction SilentlyContinue).LogPixels
        autologon  = (Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon' -Name AutoAdminLogon -ErrorAction SilentlyContinue).AutoAdminLogon
    }
} | Format-List

Write-Host '--- 3. reboot so auto-logon and the scale take effect ---'
Invoke-Command -VMName $VmName -Credential $Credential -ScriptBlock { Restart-Computer -Force } -ErrorAction SilentlyContinue
Start-Sleep -Seconds 20
if (-not (Wait-GuestReady)) { throw 'the guest did not come back after the reboot' }
Write-Host 'guest is back'

Write-Host '--- 3b. proving the interactive SESSION (the check this script was missing) ---'
# An earlier revision rebooted and went straight to the checkpoint: the reg
# value said auto-logon was on while no session ever existed. Now the session
# itself is the gate - explorer running outside session 0 plus a non-empty
# desktop - and a checkpoint is only taken past this point. The `query session`
# Active row is corroboration only: a headless console with no viewer attached
# is Disconnected, not Active, so demanding Active refused the healthy session
# the owner's 2026-10-16 run showed (explorer in session 1, no Active row).
# The display is a SEPARATE claim proven in §3c, so this gate says SESSION.
$assertSessionPath = Join-Path $PSScriptRoot 'vm-broker\guest\assert-interactive-session.ps1'
$deadline = (Get-Date).AddMinutes(10)
$sessionProven = $false
while ((Get-Date) -lt $deadline) {
    try {
        Invoke-Command -VMName $VmName -Credential $Credential -FilePath $assertSessionPath -ErrorAction Stop | Format-List
        $sessionProven = $true
        break
    } catch {
        Write-Host ("    session not yet: {0}" -f $_.Exception.Message)
        Start-Sleep -Seconds 15
    }
}
if (-not $sessionProven) { throw 'SESSION: no interactive session within 10 minutes of the reboot; no checkpoint is taken and nothing is promoted' }
Write-Host 'SESSION proven: explorer runs outside session 0 and the desktop is non-empty'

Write-Host '--- 3c. proving the ATTACHED DISPLAY (separate claim from the session) ---'
# A headless Hyper-V guest can hold the session above with NO attached display:
# the synthetic video adapter exposes a mode but nothing reports
# DISPLAY_DEVICE_ATTACHED_TO_DESKTOP, and the graphics subsystem waits for a
# presentable surface until somebody looks at the console. The fix is the
# persistent virtual display from vm-provision-virtual-display.ps1 (IDD driver,
# owner-approved install, baked into the checkpoint). Until that install this
# proof is EXPECTED to fail - and the run must say so distinctly instead of
# confusing it with the session: "session proven, display missing".
$assertDisplayPath = Join-Path $PSScriptRoot 'vm-broker\guest\assert-attached-display.ps1'
$displayProven = $false
try {
    Invoke-Command -VMName $VmName -Credential $Credential -FilePath $assertDisplayPath -ErrorAction Stop | Format-List
    $displayProven = $true
} catch {
    Write-Host ("    DISPLAY missing: {0}" -f $_.Exception.Message)
}
if ($displayProven) {
    Write-Host 'DISPLAY proven: a persistent attached display is present with no viewer connected'
} else {
    Write-Host 'DISPLAY missing: session proven, display missing - the expected state before the virtual-display driver is owner-approved and installed (see vm-provision-virtual-display.ps1); the checkpoint below is session-only until then'
}

Write-Host '--- proof summary ---'
Write-Host ("    session: {0}" -f ($(if ($sessionProven) { 'PROVEN' } else { 'MISSING' })))
Write-Host ("    display: {0}" -f ($(if ($displayProven) { 'PROVEN' } else { 'MISSING' })))

Write-Host '--- 4. baseline checkpoint ---'
if (Get-VMSnapshot -VMName $VmName -Name 'base-provisioned' -ErrorAction SilentlyContinue) {
    throw "checkpoint 'base-provisioned' already exists and holds the state from before this fix (no auto-logon password, no session); remove it first: .\vm-snapshot-test-host.ps1 -Remove base-provisioned"
}
if (-not $displayProven) {
    Write-Host 'NOTE: the display proof is missing, so this checkpoint is SESSION-ONLY (auto-logon baked in, virtual display not). After the owner approves and installs the virtual-display driver, remove this checkpoint and re-take it so the driver is baked in too.'
}
& (Join-Path $PSScriptRoot 'vm-snapshot-test-host.ps1') -Create base-provisioned -Credential $Credential

Write-Host @'

Done. The test VM is provisioned and `base-provisioned` is the state every run
resets to:

    .\vm-snapshot-test-host.ps1 -Restore base-provisioned

Next from the orchestrator: compile on the host, copy the repository and the
`target` tree into the guest over PowerShell Direct, run the test binaries there,
and copy the results back. No toolchain is installed in the guest.
'@
