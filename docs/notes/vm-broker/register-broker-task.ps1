# Register the RustampVmBroker task.  RUN ONCE, ELEVATED.
#
#   powershell -ExecutionPolicy Bypass -File docs\notes\vm-broker\register-broker-task.ps1
#
# Why: the orchestrator's shell has no administrator token, so it cannot call
# the Hyper-V cmdlets. It writes a request file and starts this task; the task
# runs vm-broker.ps1 with **highest privileges** and writes a response file.
# Starting a pre-registered task does not need elevation, which is exactly the
# property we want.
#
# The task runs only while this user is logged on (`Interactive`), as this user,
# with highest privileges. It is an allow-list worker, not a command runner:
# see the safety note in vm-broker.ps1.

$ErrorActionPreference = 'Stop'

$TaskName = 'RustampVmBroker'
$Broker = Join-Path $PSScriptRoot 'vm-broker.ps1'
if (-not (Test-Path $Broker)) { throw "broker script not found: $Broker" }

$action = New-ScheduledTaskAction -Execute 'powershell.exe' `
    -Argument ('-NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File "{0}"' -f $Broker)

$principal = New-ScheduledTaskPrincipal -UserId "$env:USERDOMAIN\$env:USERNAME" `
    -LogonType S4U -RunLevel Highest

$settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries `
    -ExecutionTimeLimit (New-TimeSpan -Hours 2) -MultipleInstances IgnoreNew

# A repeating trigger instead of on-demand: the orchestrator then never starts a
# process on the host. It writes request.json; this task picks it up within a
# minute, runs it, and writes response.json. Combined with S4U (session 0) that
# means no window can appear, whoever triggers what.
#
# The Task Scheduler refuses a repetition interval shorter than one minute
# ("Interval:PT30S" is out of range), so it is one minute.
$trigger = New-ScheduledTaskTrigger -Once -At (Get-Date).AddMinutes(1) `
    -RepetitionInterval (New-TimeSpan -Minutes 1) `
    -RepetitionDuration (New-TimeSpan -Days 3650)

Register-ScheduledTask -TaskName $TaskName -Action $action -Principal $principal `
    -Settings $settings -Trigger $trigger -Force | Out-Null

Write-Host "registered '$TaskName' (S4U, session 0, highest privileges, every 30 s)."
Write-Host 'It picks up request.json and writes response.json; it skips a request it has already run.'
Write-Host 'Remove it again with:  Unregister-ScheduledTask -TaskName RustampVmBroker -Confirm:$false'
