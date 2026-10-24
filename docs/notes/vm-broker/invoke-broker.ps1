# Client for the RustampVmBroker task.  Runs unelevated, from the orchestrator.
#
# The broker runs elevated, so it only ever touches the VM: eject-dvd, start,
# stop, checkpoint, restore, list, copyin, copyout, guest. It NEVER builds or stages -
# compiling would execute build scripts from the repository with administrator
# rights, which is a privilege-escalation hole. Building belongs here, on the
# unelevated host side.
#
# Flat parameters on purpose: a hashtable cannot survive `powershell -File`
# (PowerShell passes it as a string there), so every op takes named switches.
# Every request carries a nonce and the client waits for the response that
# echoes it, so a stale response can never be mistaken for a fresh one.

[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateSet('probe', 'eject-dvd', 'start', 'stop', 'checkpoint', 'restore', 'list', 'guest', 'copyin', 'copyout')]
    [string]$Op,
    [string]$Name,
    [string]$Script,
    [string[]]$GuestArgs,
    [string]$Source,
    [string]$Dest,
    [string]$TaskName = 'RustampVmBroker',
    [int]$TimeoutSeconds = 1800
)

$ErrorActionPreference = 'Stop'

$Root = 'D:\code\rustamp-vm\broker'
$RequestPath = Join-Path $Root 'request.json'
$ResponsePath = Join-Path $Root 'response.json'

New-Item -ItemType Directory -Force -Path $Root | Out-Null

$nonce = [guid]::NewGuid().ToString('N')
$arguments = @{ nonce = $nonce }
if ($PSBoundParameters.ContainsKey('Name')) { $arguments['name'] = $Name }
if ($PSBoundParameters.ContainsKey('Script')) { $arguments['script'] = $Script }
if ($PSBoundParameters.ContainsKey('GuestArgs')) { $arguments['guestArgs'] = @($GuestArgs) }
if ($PSBoundParameters.ContainsKey('Source')) { $arguments['source'] = $Source }
if ($PSBoundParameters.ContainsKey('Dest')) { $arguments['dest'] = $Dest }

# Wait for a broker run that is already in flight before writing: a trigger
# issued while the task is still running is queued, and the queued run reads
# whatever request.json holds at that moment. A client that was killed between
# writing its request and reading the answer (2026-10-20: a host session logoff
# killed the campaign together with such a client) leaves a stale request that
# the next queued run then executes, while the new client waits for its own
# nonce and never gets it. Waiting first removes that race.
$waitForIdle = (Get-Date).AddSeconds(120)
while ((Get-Date) -lt $waitForIdle) {
    $task = Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
    if (-not $task -or $task.State -ne 'Running') { break }
    Start-Sleep -Seconds 2
}

@{ op = $Op; args = $arguments } | ConvertTo-Json -Depth 6 | Set-Content -Path $RequestPath -Encoding UTF8
Remove-Item -Path $ResponsePath -ErrorAction SilentlyContinue

# Trigger now instead of waiting for the one-minute schedule. This is an
# unelevated cmdlet call; the orchestrator's shells are not on the interactive
# desktop, so nothing appears on the owner's screen.
Start-ScheduledTask -TaskName $TaskName

$deadline = (Get-Date).AddSeconds($TimeoutSeconds)
$response = $null
$attempt = 0
while ((Get-Date) -lt $deadline) {
    Start-Sleep -Seconds 3
    $attempt++
    # Every 15 s without an answer, rewrite the request and trigger again: a
    # lost or queued trigger must not turn into a silent five-minute timeout
    # (that is exactly how a stale request can strand a run).
    if ($attempt % 5 -eq 0) {
        @{ op = $Op; args = $arguments } | ConvertTo-Json -Depth 6 | Set-Content -Path $RequestPath -Encoding UTF8
        try { Start-ScheduledTask -TaskName $TaskName -ErrorAction Stop } catch { }
    }
    if (-not (Test-Path $ResponsePath)) { continue }
    try { $candidate = Get-Content -Path $ResponsePath -Raw | ConvertFrom-Json } catch { continue }
    if ($candidate.nonce -eq $nonce) { $response = $candidate; break }
}

if (-not $response) { throw "no response with nonce $nonce from $TaskName within $TimeoutSeconds s (see $Root\broker.log)" }
if (-not $response.ok) { throw "broker op '$Op' failed: $($response.error) (see $Root\broker.log)" }
$response.result
