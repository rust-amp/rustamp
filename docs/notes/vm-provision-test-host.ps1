# Provision the headless test VM.  Runs INSIDE the guest.
#
# The orchestrator runs this over PowerShell Direct from an elevated host shell,
# so nothing appears on the owner's desktop:
#
#   $c = Get-Credential            # the guest's local account, e.g. rustamp
#   Invoke-Command -VMName rustamp-test -Credential $c -FilePath docs\notes\vm-provision-test-host.ps1
#
# It sets the guest up so that test runs are headless, reproducible and
# window-free, and it prints the facts the manifests need.

$ErrorActionPreference = 'Continue'

function Show($label, $value) { Write-Output ("{0,-28} {1}" -f $label, $value) }

# --- 1. auto-logon at the console -------------------------------------------
# The console session must be interactive without anyone logging in: a test that
# opens a window inside the guest needs a desktop, and the reference captures
# need one too. This never affects the host.
$user = $env:USERNAME
$domain = $env:COMPUTERNAME
$winlogon = 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon'
Set-ItemProperty -Path $winlogon -Name AutoAdminLogon   -Value '1'
Set-ItemProperty -Path $winlogon -Name DefaultUserName  -Value $user
Set-ItemProperty -Path $winlogon -Name DefaultDomainName -Value $domain
Show 'auto-logon user' $user

# --- 2. fixed display, 100 % scaling ----------------------------------------
# The whole point of this VM: the display is OURS. It must not follow the
# owner's laptop/dock/monitor state (that is what made the host captures
# non-comparable: 96 dpi on the laptop screen, 144 dpi on the 4K monitor).
Set-ItemProperty -Path 'HKCU:\Control Panel\Desktop' -Name LogPixels -Value 96 -Type DWord
Set-ItemProperty -Path 'HKCU:\Control Panel\Desktop' -Name Win8DpiScaling -Value 0 -Type DWord
New-Item -Path 'HKCU:\Control Panel\Desktop\PerMonitorSettings' -Force | Out-Null

Add-Type -AssemblyName System.Windows.Forms
Show 'screen' ([System.Windows.Forms.SystemInformation]::VirtualScreen)
Show 'dpi (should be 96)' (Get-ItemProperty 'HKCU:\Control Panel\Desktop' -Name LogPixels -ErrorAction SilentlyContinue).LogPixels

# --- 3. no lock screen, no screensaver, no sleep ----------------------------
# A locked or asleep guest is a guest whose tests fail for the wrong reason.
powercfg /change monitor-timeout-ac 0  | Out-Null
powercfg /change standby-timeout-ac 0  | Out-Null
powercfg /change disk-timeout-ac 0     | Out-Null
Set-ItemProperty -Path 'HKCU:\Control Panel\Desktop' -Name ScreenSaveActive -Value '0' -ErrorAction SilentlyContinue
Show 'power' 'monitor/standby/disk timeouts off, screensaver off'

# --- 4. facts for the manifests ---------------------------------------------
Show 'user' $env:USERNAME
Show 'computer' $env:COMPUTERNAME
Show 'os' (Get-CimInstance Win32_OperatingSystem).Caption
Show 'build' (Get-CimInstance Win32_OperatingSystem).BuildNumber
Show 'powershell' $PSVersionTable.PSVersion.ToString()
$cargo = Get-Command cargo -ErrorAction SilentlyContinue
if ($cargo) { Show 'cargo on PATH' $cargo.Source } else { Show 'cargo on PATH' 'none (expected: the host compiles)' }
Write-Output 'provisioning done - reboot the guest once so auto-logon and the scale take effect'
