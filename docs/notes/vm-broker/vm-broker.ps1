# The elevated worker behind the RustampVmBroker scheduled task.
#
# The orchestrator's shell has no administrator token and must not start host
# processes (nothing graphical may run on the host). So the orchestrator only
# WRITES A REQUEST FILE; this task picks it up on its 30 s trigger, runs it with
# highest privileges in session 0 (S4U, so no window is possible) and writes a
# response file.
#
# SAFETY: this is deliberately an ALLOW-LIST, not a command runner. It executes
# exactly the operations in the switch below, with fixed paths, and nothing
# else. A generic "run this command as administrator" broker would be a
# privilege-escalation hole; do not turn this into one.
#
#   request.json : { "op": "<one of the ops>", "args": { ... } }
#   response.json: { "ok": true|false, "op": ..., "result": ..., "error": ... }

$ErrorActionPreference = 'Stop'
# No progress bars, no prompts: both would be something visible.
$ProgressPreference = 'SilentlyContinue'
$ConfirmPreference = 'None'

# Instrumentation, deliberately the FIRST thing: if this file does not grow, the
# task never started the broker at all (a scheduling/S4U problem), which is a
# completely different fault from the script exiting early.
try {
    Add-Content -Path 'D:\code\rustamp-vm\broker\started.log' -Value ('{0} start pid={1} user={2} cwd={3}' -f (Get-Date -Format o), $PID, $env:USERNAME, (Get-Location))
} catch { }

$VmName        = 'rustamp-test'
$Root          = 'D:\code\rustamp-vm\broker'
$RequestPath   = Join-Path $Root 'request.json'
$ResponsePath  = Join-Path $Root 'response.json'
$LogPath       = Join-Path $Root 'broker.log'
$CredPath      = Join-Path $Root 'guest.cred'
$Staging       = 'D:\code\rustamp-vm\staging'      # host-side staging root
$GuestScripts  = Join-Path $PSScriptRoot 'guest'    # allow-listed guest scripts
$GuestRoot     = 'C:\rustamp'                       # guest-side destination root
$RepoRoot      = 'D:\code\Rustamp'
$GuestRepo     = 'C:\rustamp\code\Rustamp'

function Write-Log([string]$message) {
    try { Add-Content -Path $LogPath -Value ('{0} {1}' -f (Get-Date -Format o), $message) } catch { }
}

function Get-GuestCredential {
    if (-not (Test-Path $CredPath)) {
        throw "no guest credential at $CredPath - run store-guest-credential.ps1 once (as this user)."
    }
    Import-Clixml -Path $CredPath
}

function Wait-PowerShellDirect([int]$TimeoutSeconds = 600) {
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    $cred = Get-GuestCredential
    while ((Get-Date) -lt $deadline) {
        try {
            Invoke-Command -VMName $VmName -Credential $cred -ScriptBlock { $true } -ErrorAction Stop | Out-Null
            return $true
        } catch { Start-Sleep -Seconds 8 }
    }
    return $false
}

function Stop-GuestCleanly {
    $cred = Get-GuestCredential
    if ((Get-VM -Name $VmName).State -eq 'Running') {
        Invoke-Command -VMName $VmName -Credential $cred -ScriptBlock { Stop-Computer -Force } -ErrorAction SilentlyContinue | Out-Null
    }
    $deadline = (Get-Date).AddMinutes(4)
    while ((Get-VM -Name $VmName).State -ne 'Off' -and (Get-Date) -lt $deadline) { Start-Sleep -Seconds 3 }
    if ((Get-VM -Name $VmName).State -ne 'Off') { throw 'the guest did not shut down cleanly.' }
}

function Eject-Dvd {
    # D: must be free INSIDE the guest: the guest script does `subst D: C:\rustamp`
    # so that the compile-time host paths (D:\code\Rustamp\...) resolve there, and
    # the install ISO would otherwise hold D: as a DVD drive.
    $dvd = Get-VMDvdDrive -VMName $VmName -ErrorAction SilentlyContinue
    if ($dvd) {
        Write-Log 'ejecting the install DVD so the guest gets D: for subst'
        Remove-VMDvdDrive -VMName $VmName -ControllerNumber $dvd.ControllerNumber -ControllerLocation $dvd.ControllerLocation
    }
}

function Stage-Tree {
    param([string[]]$TestExes)
    if (Test-Path $Staging) { Remove-Item -Recurse -Force $Staging }
    $stage = Join-Path $Staging 'code\Rustamp'
    $null = New-Item -ItemType Directory -Force -Path (Join-Path $stage 'target\debug\deps')
    foreach ($exe in $TestExes) { Copy-Item $exe (Join-Path $stage 'target\debug\deps') -Force }
    $debugDir = Join-Path $RepoRoot 'target\debug'
    foreach ($dll in Get-ChildItem -Path (Join-Path $debugDir 'deps') -Filter '*.dll' -ErrorAction SilentlyContinue) {
        Copy-Item $dll.FullName (Join-Path $stage 'target\debug\deps') -Force
    }
    if (Test-Path (Join-Path $debugDir 'rustamp.exe')) { Copy-Item (Join-Path $debugDir 'rustamp.exe') (Join-Path $stage 'target\debug') -Force }
    foreach ($crt in 'vcruntime140.dll', 'vcruntime140_1.dll', 'msvcp140.dll') {
        $p = Join-Path $env:SystemRoot "System32\$crt"
        if (Test-Path $p) { Copy-Item $p (Join-Path $stage 'target\debug') -Force }
    }
    Copy-Item (Join-Path $RepoRoot 'crates') (Join-Path $stage 'crates') -Recurse -Force
    $TestExes | ConvertTo-Json | Set-Content (Join-Path $stage 'target\debug\test-list.json') -Encoding UTF8
    return $stage
}

function Copy-TreeToGuest {
    param([string]$Stage)
    $cred = Get-GuestCredential
    Invoke-Command -VMName $VmName -Credential $cred -ScriptBlock {
        param($d) New-Item -ItemType Directory -Force -Path $d | Out-Null
    } -ArgumentList (Split-Path $GuestRepo -Parent)
    $session = New-PSSession -VMName $VmName -Credential $cred
    try {
        Copy-Item -Path (Join-Path $Stage '*') -Destination $GuestRepo -ToSession $session -Recurse -Force
    } finally {
        Remove-PSSession $session
    }
}

function Invoke-GuestScript {
    param([string]$ScriptName, [string[]]$ScriptArgs)
    if ([string]::IsNullOrWhiteSpace($ScriptName)) { throw 'guest needs args.script' }
    if ($ScriptName -match '[\\/]' -or $ScriptName -notmatch '^[A-Za-z0-9._-]+\.ps1$') {
        throw "guest script name '$ScriptName' is not allowed (plain name ending in .ps1, no path)."
    }
    $scriptPath = Join-Path $GuestScripts $ScriptName
    if (-not (Test-Path $scriptPath)) { throw "no such allow-listed guest script: $ScriptName" }
    if (-not (Wait-PowerShellDirect 120)) { throw 'PowerShell Direct is not reachable.' }
    $cred = Get-GuestCredential
    $guestArgs = @()
    if ($ScriptArgs) { $guestArgs = @($ScriptArgs) }
    Invoke-Command -VMName $VmName -Credential $cred -FilePath $scriptPath -ArgumentList $guestArgs
}

function Get-CargoPath {
    throw 'the broker never builds: compiling executes build scripts from the repository, which must never happen with elevated rights. Build on the host side, unelevated.'
}

$response = [ordered]@{ ok = $false; op = $null; nonce = $null; result = $null; error = $null }
try {
    if (-not (Test-Path $RequestPath)) { exit 0 }

    # The task runs every 30 s, so it must act only on a request it has not run
    # yet: the client writes request.json, we remember what we executed.
    $seenPath = Join-Path $Root 'last-request.txt'
    $raw = Get-Content -Path $RequestPath -Raw
    $hash = (Get-FileHash -Path $RequestPath -Algorithm SHA256).Hash
    if ((Test-Path $seenPath) -and ((Get-Content $seenPath -Raw).Trim() -eq $hash)) { exit 0 }

    $request = $raw | ConvertFrom-Json
    $response.op = $request.op
    # NOT `$args`: that is an automatic PowerShell variable.
    $arguments = $request.args
    # The nonce is echoed so a client can tell its own response from an older one.
    if ($arguments -and $arguments.nonce) { $response.nonce = $arguments.nonce }
    Write-Log ("op={0} args={1}" -f $request.op, ($arguments | ConvertTo-Json -Compress -ErrorAction SilentlyContinue))

    switch ($request.op) {
        'probe' {
            $vm = Get-VM -Name $VmName -ErrorAction SilentlyContinue
            $response.result = [ordered]@{
                exists = [bool]$vm
                state  = if ($vm) { [string]$vm.State } else { $null }
                cpu    = if ($vm) { $vm.ProcessorCount } else { $null }
                memory_gb = if ($vm) { [int]($vm.MemoryStartupBytes / 1GB) } else { $null }
                dvd    = [bool](Get-VMDvdDrive -VMName $VmName -ErrorAction SilentlyContinue)
                has_credential = Test-Path $CredPath
                snapshots = @(Get-VMSnapshot -VMName $VmName -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Name)
            }
        }
        'eject-dvd' { Eject-Dvd; $response.result = [ordered]@{ dvd = [bool](Get-VMDvdDrive -VMName $VmName -ErrorAction SilentlyContinue) } }
        'start' {
            Eject-Dvd
            if ((Get-VM -Name $VmName).State -ne 'Running') { Start-VM -Name $VmName }
            $response.result = [ordered]@{ started = $true; psdirect = (Wait-PowerShellDirect) }
        }
        'stop' { Stop-GuestCleanly; $response.result = [ordered]@{ state = [string](Get-VM -Name $VmName).State } }
        'checkpoint' {
            $name = [string]$arguments.name
            if ([string]::IsNullOrWhiteSpace($name)) { throw 'checkpoint needs args.name' }
            if (Get-VMSnapshot -VMName $VmName -Name $name -ErrorAction SilentlyContinue) { throw "checkpoint '$name' already exists" }
            Stop-GuestCleanly
            Checkpoint-VM -Name $VmName -SnapshotName $name
            Start-VM -Name $VmName
            $response.result = [ordered]@{ created = $name; snapshots = @(Get-VMSnapshot -VMName $VmName | Select-Object -ExpandProperty Name) }
        }
        'restore' {
            $name = [string]$arguments.name
            if (-not (Get-VMSnapshot -VMName $VmName -Name $name -ErrorAction SilentlyContinue)) { throw "no checkpoint named '$name'" }
            if ((Get-VM -Name $VmName).State -ne 'Off') { Stop-GuestCleanly }
            Restore-VMSnapshot -VMName $VmName -Name $name -Confirm:$false
            Eject-Dvd
            Start-VM -Name $VmName
            $response.result = [ordered]@{ restored = $name; psdirect = (Wait-PowerShellDirect) }
        }
        'list' {
            $response.result = @(Get-VMSnapshot -VMName $VmName -ErrorAction SilentlyContinue |
                Select-Object Name, SnapshotType, CreationTime, ParentSnapshotName)
        }
        'guest' {
            $guestArgs = @()
            if ($arguments.guestArgs) { $guestArgs = @($arguments.guestArgs) }
            $response.result = Invoke-GuestScript -ScriptName ([string]$arguments.script) -ScriptArgs $guestArgs
        }
        'copyin' {
            $source = [string]$arguments.source
            $dest   = [string]$arguments.dest
            if ([string]::IsNullOrWhiteSpace($source) -or [string]::IsNullOrWhiteSpace($dest)) { throw 'copyin needs args.source and args.dest' }
            $fullSource = [System.IO.Path]::GetFullPath($source)
            if (-not $fullSource.StartsWith([System.IO.Path]::GetFullPath($Staging), [System.StringComparison]::OrdinalIgnoreCase)) {
                throw "copyin source must live under $Staging (got $fullSource)"
            }
            $destFull = [System.IO.Path]::GetFullPath($dest)
            if (-not $destFull.StartsWith($GuestRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
                throw "copyin destination must live under $GuestRoot (got $destFull)"
            }
            $cred = Get-GuestCredential
            # Clear the destination and copy the CONTENTS: copying a directory
            # into an existing directory nests it (…\Rustamp\Rustamp\…), which is
            # exactly the bug that left a stale test list behind and made the
            # guest run a joined string as a command.
            Invoke-Command -VMName $VmName -Credential $cred -ScriptBlock {
                param($d)
                if (Test-Path $d) { Remove-Item -Recurse -Force $d }
                New-Item -ItemType Directory -Force -Path $d | Out-Null
            } -ArgumentList $destFull
            $session = New-PSSession -VMName $VmName -Credential $cred
            try {
                Copy-Item -Path (Join-Path $fullSource '*') -Destination $destFull -ToSession $session -Recurse -Force
            } finally { Remove-PSSession $session }
            $response.result = [ordered]@{ copied = $true; source = $fullSource; dest = $destFull }
        }
        'run-tests' {
            # DELIBERATELY REFUSED. The broker runs with elevated rights, and
            # compiling would execute build scripts from the repository as
            # administrator - arbitrary code with admin rights, i.e. a
            # privilege-escalation hole. Building and staging are not privileged
            # and belong to the unelevated host side (docs\notes\vm-run-tests.ps1).
            # The broker only touches the VM: eject-dvd, start, stop, checkpoint,
            # restore, list, copyin, guest.
            throw 'the broker never builds. Compile and stage on the host side, unelevated, then use eject-dvd/start/copyin/guest.'
        }
        default { throw "unknown op '$($request.op)'" }
    }
    $response.ok = $true
}
catch {
    $response.ok = $false
    $response.error = $_.Exception.Message
    Write-Log ("ERROR {0}" -f $_.Exception.ToString())
}

$response | ConvertTo-Json -Depth 10 | Set-Content -Path $ResponsePath -Encoding UTF8
try { $hash | Set-Content -Path (Join-Path $Root 'last-request.txt') -Encoding UTF8 } catch { }
Write-Log ("done ok={0}" -f $response.ok)
