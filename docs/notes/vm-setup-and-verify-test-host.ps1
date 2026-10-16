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

Write-Host '--- 3b. proving the interactive session (the check this script was missing) ---'
# An earlier revision rebooted and went straight to the checkpoint: the reg
# value said auto-logon was on while no session ever existed. Now the session
# itself is the gate - explorer running, an Active console row, a non-empty
# desktop - and a checkpoint is only taken past this point.
$assertPath = Join-Path $PSScriptRoot 'vm-broker\guest\assert-interactive-session.ps1'
$deadline = (Get-Date).AddMinutes(10)
$proven = $false
while ((Get-Date) -lt $deadline) {
    try {
        Invoke-Command -VMName $VmName -Credential $Credential -FilePath $assertPath -ErrorAction Stop | Format-List
        $proven = $true
        break
    } catch {
        Write-Host ("    not yet: {0}" -f $_.Exception.Message)
        Start-Sleep -Seconds 15
    }
}
if (-not $proven) { throw 'no interactive session within 10 minutes of the reboot; no checkpoint is taken and nothing is promoted' }
Write-Host 'interactive session proven'

Write-Host '--- 4. baseline checkpoint ---'
if (Get-VMSnapshot -VMName $VmName -Name 'base-provisioned' -ErrorAction SilentlyContinue) {
    throw "checkpoint 'base-provisioned' already exists and holds the state from before this fix (no auto-logon password, no session); remove it first: .\vm-snapshot-test-host.ps1 -Remove base-provisioned"
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
