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

# The group's name is localised; find it by its well-known SID suffix (-2352).
$group = Get-LocalGroup | Where-Object { $_.SID.Value -like '*-2352' } | Select-Object -First 1
if (-not $group) { throw 'Hyper-V Administrators group not found on this machine.' }

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
