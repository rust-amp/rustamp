# Create the headless test VM for Rustamp.  RUN ONCE, IN AN ELEVATED SHELL.
#
# Why this exists: no test and no GUI may run on the host (docs/HANDOFF.md, the
# GUI rule). The VM is started with Start-VM and is **never** opened with
# VMConnect, so nothing of it appears on the owner's desktop. Everything after
# the install is driven over PowerShell Direct (VMBus, no network, no window).
#
# Windows in the guest runs unactivated; that is fine for a test host.
#
#   powershell -ExecutionPolicy Bypass -File docs\notes\vm-create-test-host.ps1
#
# Afterwards: install Windows once (see docs/notes/VM-capture-host.md, "the test
# VM"), then run vm-provision-test-host.ps1 over PowerShell Direct.

$ErrorActionPreference = 'Stop'

$VmName = 'rustamp-test'
$Iso    = 'C:\Users\micro\Downloads\Microsoft Windows 11 AiO 26H2 Build 26300.9711 - Pentium\CCSA_X64FRE_DE-DE.iso'
$Root   = 'D:\code\rustamp-vm'
$Vhdx   = Join-Path $Root "$VmName.vhdx"
$Ram    = 6GB
$Cpus   = 4
$DiskGB = 80

if (-not (Test-Path $Iso)) { throw "ISO not found: $Iso" }
if (Get-VM -Name $VmName -ErrorAction SilentlyContinue) { throw "VM '$VmName' already exists." }

New-Item -ItemType Directory -Force -Path $Root | Out-Null

Write-Host "creating $DiskGB GB dynamic VHDX at $Vhdx"
New-VHD -Path $Vhdx -SizeBytes ($DiskGB * 1GB) -Dynamic | Out-Null

Write-Host "creating VM $VmName (generation 2, $Cpus vCPU, $([int]($Ram/1GB)) GB, no network)"
New-VM -Name $VmName -Generation 2 -MemoryStartupBytes $Ram -VHDPath $Vhdx | Out-Null
Set-VMProcessor -VMName $VmName -Count $Cpus
Set-VMMemory    -VMName $VmName -DynamicMemoryEnabled $false

# Windows 11 needs Secure Boot + a virtual TPM.
Set-VMFirmware -VMName $VmName -EnableSecureBoot On -SecureBootTemplate 'MicrosoftWindows'
Set-VMKeyProtector -VMName $VmName -NewLocalKeyProtector
Enable-VMTPM -VMName $VmName

# No automatic checkpoints: our own named ones are the only ones.
Set-VM -Name $VmName -AutomaticCheckpointsEnabled $false
Set-VM -Name $VmName -AutomaticStartAction Nothing
Set-VM -Name $VmName -AutomaticStopAction ShutDown

Add-VMDvdDrive -VMName $VmName -Path $Iso
Set-VMFirmware -VMName $VmName -FirstBootDevice (Get-VMDvdDrive -VMName $VmName)

Write-Host "starting $VmName (headless - do NOT open VMConnect unless you are installing Windows)"
Start-VM -Name $VmName
Start-Sleep -Seconds 3
Get-VM -Name $VmName | Format-List Name, State, Generation, ProcessorCount, MemoryStartup
Write-Host @'

Next:
  1. Install Windows in the guest. This is the ONE step that needs a window, and
     it is the owner's own deliberate action: open VMConnect, click through the
     setup (a local account is enough - no Microsoft account), and close
     VMConnect again. Do not enable Enhanced Session Mode.
  2. Then tell the orchestrator: it provisions the guest over PowerShell Direct
     (auto-logon, fixed resolution at 100% scaling, lock screen and sleep off)
     and from then on runs everything headless.
'@
