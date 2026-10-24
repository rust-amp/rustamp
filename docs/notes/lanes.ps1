# lanes.ps1 - the lane table, on one screen.
#
#   powershell -ExecutionPolicy Bypass -File docs\notes\lanes.ps1
#
# Lists every lane worktree with the number of files it has changed and the
# commit it is based on, so the orchestrator can see at a glance which lanes are
# still working, which have stopped without a report, and which slots are free.
# It reads git metadata only: it starts nothing, touches nothing and writes
# nothing.

[CmdletBinding()]
param(
    [string]$Worktrees = (Join-Path $env:USERPROFILE '.local\share\opencode\worktree\26de0e'),
    [string]$RepoRoot = 'D:\code\Rustamp'
)

$ErrorActionPreference = 'Stop'

if (-not (Test-Path $Worktrees)) {
    Write-Host "no worktree root at $Worktrees"
    exit 0
}

Write-Host ("canonical tree: {0}" -f (git -C $RepoRoot log --oneline -1))
$dirty = @(git -C $RepoRoot status --short)
if ($dirty.Count -gt 0) {
    Write-Host "  WARNING: the canonical tree has uncommitted changes:"
    $dirty | ForEach-Object { Write-Host ("    " + $_) }
} else {
    Write-Host '  canonical tree is clean'
}
Write-Host ''

$rows = @()
foreach ($dir in Get-ChildItem -Path $Worktrees -Directory | Sort-Object Name) {
    # Not every directory under the worktree root is still a worktree: one was
    # left behind without a repository. Report it as such instead of failing.
    if (-not (Test-Path (Join-Path $dir.FullName '.git'))) {
        $rows += [pscustomobject]@{ Lane = $dir.Name; Changed = 'n/a'; Base = '(not a worktree)' }
        continue
    }
    # Native commands write to stderr for ordinary conditions, and the script's
    # Stop preference would turn that into a fatal error.
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'SilentlyContinue'
    $changed = @(git -C $dir.FullName status --short 2>$null)
    $head = (git -C $dir.FullName log --oneline -1 2>$null)
    $ErrorActionPreference = $previous
    $rows += [pscustomobject]@{
        Lane    = $dir.Name
        Changed = $changed.Count
        Base    = $head
    }
}

$rows | Format-Table -AutoSize | Out-String -Width 200 | Write-Host

$active = @($rows | Where-Object { $_.Changed -gt 0 })
Write-Host ("{0} of {1} worktrees have work in progress" -f $active.Count, $rows.Count)
if ($active.Count -gt 0) {
    Write-Host '  in progress:'
    $active | ForEach-Object { Write-Host ("    {0} ({1} files)" -f $_.Lane, $_.Changed) }
}
