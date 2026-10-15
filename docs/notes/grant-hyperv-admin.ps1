# Grant the current user Hyper-V management rights.  RUN ONCE, ELEVATED.
#
#   powershell -ExecutionPolicy Bypass -File docs\notes\grant-hyperv-admin.ps1
#
# "Hyper-V-Administrators" is the group Windows provides for exactly this: full
# Hyper-V management without a full administrator token. It is how the
# orchestrator can start the test VM, take and restore checkpoints and use
# PowerShell Direct without asking for an elevated shell every time.
#
# Scope: Hyper-V only. It does not make the account a local administrator and it
# grants nothing else. Remove it again with:
#
#   Remove-LocalGroupMember -Group 'Hyper-V-Administrators' -Member $env:USERNAME

$ErrorActionPreference = 'Stop'

# The group's name is localised; find it by its well-known RID instead: the
# Hyper-V Administrators group is S-1-5-32-578 (the Administrators group is -544).
$group = Get-LocalGroup | Where-Object { $_.SID.Value -like '*-578' } | Select-Object -First 1
if (-not $group) {
    # Fall back to a name match, and say what was actually there so a failure is
    # diagnosable instead of mysterious.
    $group = Get-LocalGroup | Where-Object { $_.Name -like '*Hyper-V*' } | Select-Object -First 1
}
if (-not $group) {
    Write-Host 'local groups on this machine:'
    Get-LocalGroup | Sort-Object SID | Format-Table -AutoSize Name, SID
    throw 'Hyper-V Administrators group not found (looked for SID *-578 and a name matching *Hyper-V*).'
}

$me = "$env:USERDOMAIN\$env:USERNAME"
$members = Get-LocalGroupMember -Group $group | ForEach-Object { $_.Name }

Write-Host "group:   $($group.Name)"
Write-Host "account: $me"

if ($members -contains $me) {
    Write-Host 'already a member - nothing to do.'
} else {
    Add-LocalGroupMember -Group $group -Member $me
    Write-Host 'added. Log off and back on (or reboot) so the group membership is in your token.'
}

Write-Host @'

Afterwards the orchestrator checks with:

    Get-VM -Name rustamp-test

If that still fails from an unelevated shell, the group is not enough on this
build and the orchestrator will register a small, allow-listed Task Scheduler
broker instead (a task with "highest privileges" that the user can trigger
without elevation).
'@
