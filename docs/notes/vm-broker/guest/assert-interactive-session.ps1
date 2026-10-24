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
# BEFORE a run. Three signals are inspected, but only two of them gate:
#
#   Signal 1 (DECISIVE - this script throws without it): explorer.exe is
#   running in a session above zero as a real user. Explorer owns the console
#   desktop, so a running explorer outside session 0 proves Winlogon performed
#   an interactive logon and a console desktop exists. No explorer, or an
#   explorer in session 0, means the guest has no desktop for a window.
#
#   Signal 2 (CORROBORATION ONLY - never required, never throws on its own):
#   `query session` showing an Active console row. A console nobody is
#   connected to (headless Hyper-V, no VMConnect/Enhanced-Session viewer) is
#   reported as Disconnected, not Active - the owner's 2026-10-16 run proved
#   exactly this: explorer ran in session 1 as the guest user while `query
#   session` showed no Active console row for ten minutes. Demanding Active
#   therefore refuses a healthy headless session. When the row IS present its
#   session/user are reported so a reader can cross-check them against signal 1;
#   a mismatch is reported as a warning, not a refusal, because the row tracks
#   viewer attachment while signal 1 tracks the logon.
#
#   Signal 3 (REQUIRED - this script throws without it): the virtual screen is
#   non-empty. A session without a desktop to open a window on (zero-sized
#   virtual screen) proves nothing for the window tests, so this still fails
#   loudly. Note this checks that A desktop exists, not that a MONITOR is
#   attached - the separate display proof (assert-attached-display.ps1) checks
#   the persistent attached display. The two claims stay separate so a run can
#   say which one it has: session without display, or session with display.
#
# It needs no staged tree and no desktop of its own: it inspects system-wide
# state, so running it from the broker's session-0 PowerShell Direct session
# is fine. It proves the session; it does not use it.

$ErrorActionPreference = 'Stop'

# --- Signal 1 (decisive): explorer owns the console desktop --------------------
$explorer = Get-Process -Name explorer -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $explorer) { throw 'no interactive session: explorer.exe is not running; the guest has no desktop to run a window on' }
if ($explorer.SessionId -eq 0) { throw 'no interactive session: explorer.exe sits in session 0, which has no desktop; refusing instead of reporting a pass' }
$owner = (Get-CimInstance Win32_Process -Filter ("ProcessId = " + $explorer.Id) | Invoke-CimMethod -MethodName GetOwner)
Write-Output ("explorer session {0} as {1}\{2}" -f $explorer.SessionId, $owner.Domain, $owner.User)

# --- Signal 2 (corroboration only): the `query session` console row ------------
# A headless console with no viewer attached is Disconnected, not Active, so an
# absent Active row is the NORMAL headless state and must not throw. Present
# rows are reported for the cross-check; a mismatch against signal 1 warns.
$consoleSession = $null
$consoleUser = $null
$consoleState = $null
foreach ($line in (query session 2>$null)) {
    # Rows look like ">console  rustamp  1  Active" or "console  rustamp  1  Disc".
    if ($line -match '^\s*>?\s*console\s+(\S+)\s+(\d+)\s+(\S+)') {
        $consoleUser = $Matches[1]
        $consoleSession = [int]$Matches[2]
        $consoleState = $Matches[3]
        break
    }
}
if ($null -eq $consoleSession) {
    Write-Output 'console row: none (`query session` shows no console row at all; normal when no viewer is attached - not a refusal)'
} elseif ($consoleState -ne 'Active') {
    Write-Output ("console row: session {0} as {1} is '{2}', not Active (normal headless state: nobody is viewing the console - not a refusal)" -f $consoleSession, $consoleUser, $consoleState)
} else {
    Write-Output ("console row: Active session {0} as {1} (corroborates signal 1)" -f $consoleSession, $consoleUser)
    if ($consoleSession -ne $explorer.SessionId) {
        Write-Warning ("explorer sits in session {0} but the Active console row is session {1}; the row tracks viewer attachment, explorer tracks the logon, so this warns instead of refusing" -f $explorer.SessionId, $consoleSession)
    }
}

# --- Signal 3 (required): the desktop exists -----------------------------------
Add-Type -AssemblyName System.Windows.Forms
$screen = [System.Windows.Forms.SystemInformation]::VirtualScreen
if ($screen.Width -le 0 -or $screen.Height -le 0) {
    throw 'no interactive session: the virtual screen is empty, so no window can open here'
}
Write-Output ("desktop {0}" -f $screen.ToString())

[pscustomobject]@{
    explorer_session = $explorer.SessionId
    explorer_user    = ("{0}\{1}" -f $owner.Domain, $owner.User)
    console_session  = $consoleSession
    console_user     = $consoleUser
    console_state    = $consoleState
    screen           = $screen.ToString()
}
