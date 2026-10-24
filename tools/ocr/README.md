# OCR helper (`tools/ocr`)

`Ocr.exe` reads printed text out of `vm-capture` screenshots through the
built-in Windows OCR API (`Windows.Media.Ocr`, WinRT). It takes a PNG path
and prints the recognised text plus per-word bounding boxes to stdout:

```powershell
D:\code\tools\ocr\Ocr.exe <capture>.png
```

## Why it exists

The capture harness compares pixels only; lanes still need a reproducible way
to turn pixels into quoted labels. This helper is that way: it is present on
every Windows machine, works offline, needs no install, and its `WORD[…]`
boxes are what let lanes place a label inside a row band
(see `docs/notes/ocr-for-captures.md`, the canonical workflow).

## Source and binary

The C# source (`Ocr.cs`) is ours: written clean-room for this project, no
third-party code. Per the owner's decision the **source and the build script
live here in the repo**, while the **built binary lives outside it** in
`D:\code\tools\ocr\`. Keep binaries out of the repository; rebuild from here.

## Build

```powershell
powershell -ExecutionPolicy Bypass -File tools\ocr\Build-Ocr.ps1
```

The script compiles with the inbox .NET Framework `csc.exe` against the
Windows SDK `Windows.winmd` facade and writes `Ocr.exe` to
`D:\code\tools\ocr\` (created if missing; override with `-OutputDir`).
It fails loudly if the SDK facade it needs is missing.
