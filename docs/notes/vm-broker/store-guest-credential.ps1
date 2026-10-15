# Store the guest credential for the broker.  RUN ONCE, AS THE OWNER (no admin).
#
#   powershell -ExecutionPolicy Bypass -File docs\notes\vm-broker\store-guest-credential.ps1
#
# The password is encrypted with DPAPI in the *user* scope, so only this Windows
# account on this machine can read it back - the broker task runs as this user
# and can therefore use it for PowerShell Direct, and nothing else can.

$ErrorActionPreference = 'Stop'

$Root = 'D:\code\rustamp-vm\broker'
$CredPath = Join-Path $Root 'guest.cred'

New-Item -ItemType Directory -Force -Path $Root | Out-Null

$credential = Get-Credential -UserName 'rustamp' -Message 'Password of the guest local account (set during the Windows install)'
$credential | Export-Clixml -Path $CredPath

Write-Host "stored: $CredPath"
Write-Host 'encrypted with DPAPI (user scope) - only this account on this machine can read it.'
Write-Host 'delete the file to revoke it.'
