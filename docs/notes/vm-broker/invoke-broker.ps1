# Client for the RustampVmBroker task.  Runs unelevated, from the orchestrator.
#
# Flat parameters on purpose: a hashtable cannot survive `powershell -File`
# (PowerShell passes it as a string there), so every op takes named switches.
#
#   .\invoke-broker.ps1 -Op probe
#   .\invoke-broker.ps1 -Op checkpoint -Name base-provisioned
#   .\invoke-broker.ps1 -Op restore   -Name base-provisioned
#   .\invoke-broker.ps1 -Op guest     -Script run-tests.ps1 -GuestArgs '-Filter','rustamp-app'
#   .\invoke-broker.ps1 -Op copyin    -Source D:\code\rustamp-vm\staging\x -Dest C:\rustamp\x
#
# It writes request.json, starts the task, waits for response.json and returns
# the result as an object. Every operation is allow-listed inside vm-broker.ps1.

[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateSet('probe', 'start', 'stop', 'checkpoint', 'restore', 'list', 'guest', 'copyin')]
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

$arguments = @{}
if ($PSBoundParameters.ContainsKey('Name')) { $arguments['name'] = $Name }
if ($PSBoundParameters.ContainsKey('Script')) { $arguments['script'] = $Script }
if ($PSBoundParameters.ContainsKey('GuestArgs')) { $arguments['guestArgs'] = @($GuestArgs) }
if ($PSBoundParameters.ContainsKey('Source')) { $arguments['source'] = $Source }
if ($PSBoundParameters.ContainsKey('Dest')) { $arguments['dest'] = $Dest }

@{ op = $Op; args = $arguments } | ConvertTo-Json -Depth 6 | Set-Content -Path $RequestPath -Encoding UTF8
Remove-Item -Path $ResponsePath -ErrorAction SilentlyContinue

Start-ScheduledTask -TaskName $TaskName

$deadline = (Get-Date).AddSeconds($TimeoutSeconds)
while ((Get-Date) -lt $deadline -and -not (Test-Path $ResponsePath)) { Start-Sleep -Seconds 3 }

if (-not (Test-Path $ResponsePath)) { throw "no response from $TaskName within $TimeoutSeconds s (see $Root\broker.log)" }

$response = Get-Content -Path $ResponsePath -Raw | ConvertFrom-Json
if (-not $response.ok) { throw "broker op '$Op' failed: $($response.error) (see $Root\broker.log)" }
$response.result
