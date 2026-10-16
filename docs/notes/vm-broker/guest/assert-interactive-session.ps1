# Assert the guest holds a real interactive session.  Allow-listed guest script.
#
# Called by the broker exactly like run-tests.ps1:
#
#   broker -Op guest -Script assert-interactive-session.ps1
#
# Or over PowerShell Direct from the setup script:
#
#   Invoke-Command -VMName rustamp-test -Credential $c -FilePath docs\notes\vm-broker\guest\assert-interactive-session.ps1
#
# This is the check the GUI runner was missing: run-gui-session.ps1 refuses
# loudly when there is no session, but nothing verified the session exists
# BEFORE a run. Three independent signals must all hold, or this throws
# instead of reporting a pass:
#   1. explorer.exe is running (it owns the console desktop),
#   2. `query session` shows an Active console row,
#   3. the virtual screen is non-empty (there is a desktop to open a window on).
#
# It needs no staged tree and no desktop of its own: it inspects system-wide
# state, so running it from the broker's session-0 PowerShell Direct session
# is fine. It proves the session; it does not use it.

$ErrorActionPreference = 'Stop'

# --- 1. explorer owns the console desktop -------------------------------------
$explorer = Get-Process -Name explorer -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $explorer) { throw 'no interactive session: explorer.exe is not running; the guest has no desktop to run a window on' }
$owner = (Get-CimInstance Win32_Process -Filter ("ProcessId = " + $explorer.Id) | Invoke-CimMethod -MethodName GetOwner)
Write-Output ("explorer session {0} as {1}\{2}" -f $explorer.SessionId, $owner.Domain, $owner.User)

# --- 2. an Active console row ---------------------------------------------------
$activeSession = $null
$activeUser = $null
foreach ($line in (query session 2>$null)) {
    # Active console row looks like: ">console  rustamp  1  Active"
    if ($line -match '^\s*>?\s*console\s+(\S+)\s+(\d+)\s+Active') {
        $activeUser = $Matches[1]
        $activeSession = [int]$Matches[2]
        break
    }
}
if ($null -eq $activeSession) { throw 'no interactive session: `query session` shows no Active console row' }
Write-Output ("active console session {0} as {1}" -f $activeSession, $activeUser)

if ($explorer.SessionId -ne $activeSession) {
    throw ("explorer sits in session {0} but the Active console is session {1}; refusing" -f $explorer.SessionId, $activeSession)
}

# --- 3. the desktop exists ------------------------------------------------------
Add-Type -AssemblyName System.Windows.Forms
$screen = [System.Windows.Forms.SystemInformation]::VirtualScreen
if ($screen.Width -le 0 -or $screen.Height -le 0) {
    throw 'no interactive session: the virtual screen is empty, so no window can open here'
}
Write-Output ("desktop {0}" -f $screen.ToString())

[pscustomobject]@{
    explorer_session = $explorer.SessionId
    console_session  = $activeSession
    user             = $activeUser
    screen           = $screen.ToString()
}
