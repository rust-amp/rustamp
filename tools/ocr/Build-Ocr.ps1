# Builds tools/ocr/Ocr.cs into Ocr.exe with the .NET Framework compiler.
#
# The binary lands OUTSIDE the repo in D:\code\tools\ocr\ (owner decision);
# only the source and this script are canonical in the repo. Run:
#
#   powershell -ExecutionPolicy Bypass -File tools\ocr\Build-Ocr.ps1
#
# or with an explicit output directory:
#
#   powershell -ExecutionPolicy Bypass -File tools\ocr\Build-Ocr.ps1 -OutputDir D:\code\tools\ocr
#
# Opens no window and needs no download: it uses csc.exe and the Windows SDK
# metadata that ship with the machine. Fails loudly if any of them is missing.

param(
  [string]$OutputDir = "D:\code\tools\ocr"
)

$ErrorActionPreference = "Stop"

$Csc = "C:\Windows\Microsoft.NET\Framework64\v4.0.30319\csc.exe"
$Winmd = "C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.19041.0\Windows.winmd"
$Facade = "C:\Program Files (x86)\Reference Assemblies\Microsoft\Framework\.NETFramework\v4.7.2\Facades\System.Runtime.dll"
$WinRtInterop = "C:\Windows\Microsoft.NET\Framework64\v4.0.30319\System.Runtime.WindowsRuntime.dll"

foreach ($dep in @($Csc, $Winmd, $Facade, $WinRtInterop)) {
  if (-not (Test-Path $dep)) {
    Write-Error "Missing required build input: $dep. Install nothing; ask the owner."
    exit 1
  }
}

$Source = Join-Path $PSScriptRoot "Ocr.cs"
if (-not (Test-Path $Source)) {
  Write-Error "Helper source not found: $Source."
  exit 1
}

New-Item -ItemType Directory -Force $OutputDir | Out-Null
$Out = Join-Path $OutputDir "Ocr.exe"

& $Csc /nologo /t:exe "/out:$Out" "/r:$Winmd" "/r:$Facade" "/r:$WinRtInterop" $Source
if ($LASTEXITCODE -ne 0) {
  Write-Error "csc.exe failed with exit code $LASTEXITCODE."
  exit 1
}

$size = (Get-Item $Out).Length
Write-Host "Built $Out ($size bytes)."
