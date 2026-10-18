# Rustamp 0.1.0

**Release ID:** `rustamp-0.1.0+2026-10-18.f38ad02`
**Product version:** `0.1.0`
**Commit:** `f38ad02`
**Artefact:** `D:\code\rustamp-beta\v0.1.0-f38ad02` and its zip beside it, with a
SHA-256 file for the zip.
**Signed:** no (unsigned by owner decision). **Auto-update:** none.

This is the first tagged release. It is a portable zip: unpack it and run
`player\rustamp.exe`. There is no installer and nothing is written outside the
unpacked folder except the usual per-user data.

## What is in the bytes

- The player, 64-bit, with its two resident helper processes
  (`rustamp-dechost-symphonia.exe`, `rustamp-dsphost-stages.exe`) beside it.
- The 32-bit plugin host `rustamp-plughost32.exe`, bundled into the player and
  also shipped on its own as the `host` package.
- `README.md`, `QUICKSTART.md`, the licence texts, the plugin exception and the
  third-party inventory.
- Every shipped executable imports only Windows system DLLs: no CRT DLL, no VC
  redist payload. All four are built against the static MSVC CRT.
- The host package's dependency closure is GPL-free.

## What was verified, and how

| Check | Result |
|---|---|
| Format and lints on every target and feature | clean |
| Licence audit, permissive closures and notices | clean |
| Full test suite, interactive session of the test VM | **84 passed, 0 failed, 0 hung** |
| Ignored inventory (Groups A-E, 20 entries) | **18 ran, 0 failed**; the 2 audio tests run separately and both pass |
| Real audio endpoint: reference playback | pass |
| Real audio endpoint: the fixture tone through the virtual cable | pass |
| Package build and re-audit of the produced layout | clean |
| Offline PE import inspection of every shipped executable | only system DLLs |
| Packaged smoke in the VM | pass: real window at client 275x116, bundled host child, silent-path marker, all files byte-identical afterwards |

## Steps not performed, and what that means

- **Clean-machine run (checklist step 13): NOT PERFORMED.** The machine has two
  checkpoints, `base-provisioned` and `wave5-eq-playback-green`, and neither is a
  clean operating system. The claim that the package "starts on a clean machine"
  is exactly the claim that step exists to test, so it is **not** claimed here.
  What is proven is that it starts and runs on a provisioned Windows guest with
  no developer runtime *payload* needed, because the CRT is static and only
  system DLLs are imported - but that is a static argument plus a provisioned
  machine, not a clean-OS run.
- **Signing (step 14): skipped by owner decision.** The binaries are unsigned, so
  a downloader sees no publisher identity and Windows may show a reputation
  prompt.
- **Final hash manifest over signed bytes (step 15): not applicable**, because
  there are no signed bytes.

## What this release is not

- It is not plugin compatibility. The bundled host serves a real control channel
  and the acceptance tests load a real 32-bit input and a real DSP plugin in the
  VM, but the loader is not a sandbox and approved plugin bytes still run their
  own `DllMain`. There is no user-facing plugin picker or configuration wiring.
- It is not a visualisation release. The live visualiser renders no spectrum, and
  the projectM bundling that was approved on this day is not part of this package.
- Virtual audio endpoints are evidence that audio reaches and returns from a
  device. They are not evidence of audio quality or of behaviour on real
  hardware.
