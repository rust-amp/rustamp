# Checkpoint and reset the headless test VM.  RUN IN AN ELEVATED SHELL.
#
# The test VM is disposable: a named checkpoint is the "clean" state, and every
# run that could leave the guest dirty starts from it. Restoring is how we get
# determinism without reinstalling anything.
#
#   # once, after provisioning: the baseline everything resets to
#   .\vm-snapshot-test-host.ps1 -Create base-provisioned
#
#   # before a run that must start clean
#   .\vm-snapshot-test-host.ps1 -Restore base-provisioned
#
#   .\vm-snapshot-test-host.ps1 -List
#   .\vm-snapshot-test-host.ps1 -Remove old-name
#
# Notes
#  * Checkpoints are MANUAL only (`AutomaticCheckpointsEnabled` is off, set by
#    vm-create-test-host.ps1), so the chain holds exactly what we name.
#  * Every checkpoint adds a differencing disk (AVHDX). A long chain slows the
#    guest down; keep a small named set and `-Remove` the stale ones.
#  * A restore DISCARDS everything written since the checkpoint, including a
#    copied repository or build tree. The test runner therefore copies the repo
#    and the `target` tree again after every `-Restore`.
#  * `-Create` shuts the guest down cleanly first, so the baseline is a
#    shut-down, consistent disk state; restoring it is a normal boot.

[CmdletBinding(DefaultParameterSetName = 'List')]
param(
    [Parameter(ParameterSetName = 'Create', Mandatory)][string]$Create,
    [Parameter(ParameterSetName = 'Restore', Mandatory)][string]$Restore,
    [Parameter(ParameterSetName = 'Remove', Mandatory)][string]$Remove,
    [Parameter(ParameterSetName = 'List')][switch]$List,
    [string]$VmName = 'rustamp-test',
    [PSCredential]$Credential,
    [switch]$Force
)

$ErrorActionPreference = 'Stop'

if (-not (Get-VM -Name $VmName -ErrorAction SilentlyContinue)) { throw "VM '$VmName' does not exist." }

function Show-Snapshots {
    $snaps = Get-VMSnapshot -VMName $VmName -ErrorAction SilentlyContinue
    if (-not $snaps) { Write-Host "no checkpoints on $VmName"; return }
    $snaps | Sort-Object CreationTime | Format-Table -AutoSize Name, SnapshotType, CreationTime, ParentSnapshotName
}

switch ($PSCmdlet.ParameterSetName) {
    'Create' {
        if (Get-VMSnapshot -VMName $VmName -Name $Create -ErrorAction SilentlyContinue) {
            throw "checkpoint '$Create' already exists (remove it first if you mean to replace it)."
        }
        if ((Get-VM -Name $VmName).State -ne 'Off') {
            Write-Host "shutting $VmName down cleanly for a consistent baseline"
            Stop-VM -Name $VmName -Shutdown -Force:$Force
            $deadline = (Get-Date).AddMinutes(3)
            while ((Get-VM -Name $VmName).State -ne 'Off' -and (Get-Date) -lt $deadline) { Start-Sleep -Seconds 3 }
            if ((Get-VM -Name $VmName).State -ne 'Off') { throw "guest did not shut down; use -Force" }
        }
        Checkpoint-VM -Name $VmName -SnapshotName $Create
        Write-Host "checkpoint '$Create' created"
        Show-Snapshots
        Start-VM -Name $VmName
    }
    'Restore' {
        if (-not (Get-VMSnapshot -VMName $VmName -Name $Restore -ErrorAction SilentlyContinue)) {
            throw "no checkpoint named '$Restore' on $VmName."
        }
        if ((Get-VM -Name $VmName).State -ne 'Off' -and -not $Force) {
            throw "guest is running; stop it first or pass -Force (which discards its state)."
        }
        Restore-VMSnapshot -VMName $VmName -Name $Restore -Confirm:$false
        Write-Host "restored '$Restore'; starting headless"
        Start-VM -Name $VmName
    }
    'Remove' {
        Remove-VMSnapshot -VMName $VmName -Name $Remove -Confirm:$false
        Write-Host "removed '$Remove'"
        Show-Snapshots
    }
    default { Show-Snapshots }
}
