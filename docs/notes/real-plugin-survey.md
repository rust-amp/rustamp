# Real classic plug-ins through the production host (2026-10-24)

What was done: eleven plug-ins of the Winamp 5.66 installer (extracted with
7-Zip, **never executed on the host**; `D:\code\rustamp-testplugins\w566`, a
`PROVENANCE.txt` and `SHA256SUMS.txt` beside them, not in this repository) were
loaded one by one into the real i686 host in the Hyper-V VM by
`crates/rustamp-plughost/tests/real_plugin_survey.rs`
(`vm-run-gui-session.ps1 -Filter real_plugin_survey`). Each plug-in gets its
own host process and its own approval store entry. The table is also written to
`C:\rustamp\out\gui-session\plugin-survey.txt` in the guest and copied back.

## Run 1: the plug-ins alone

| plug-in | result |
|---|---|
| `dsp_sps.dll` | refused: `Dsp plugin returned unsupported version 0x21 (expected 0x20)` |
| `gen_hotkeys`, `gen_jumpex`, `in_flac`, `in_vorbis`, `in_wave`, `out_disk`, `out_wave`, `vis_milk2`, `vis_nsfs` | refused: `LoadLibraryExW failed with error code 126` (a dependency is missing in the guest) |
| `vis_avs.dll` | **the host died** while loading it (the process boundary did its job: the player is unaffected) |

Cause of 126: these plug-ins import the VC++ 2008 runtime (`msvcr90.dll`),
`libFLAC`, `libsndfile`, `nxlite` and, for MilkDrop, `d3dx9_*` (DirectX runtime;
not in the installer and not in the guest).

## Run 2: with the runtime DLLs staged beside them

`msvcr90.dll` plus its manifest (also in a `Microsoft.VC90.CRT` folder),
`libFLAC.dll`, `libsndfile.dll`, `nxlite.dll`.

| plug-in | result |
|---|---|
| `dsp_sps.dll` | refused, `0x21` (unchanged) |
| `vis_avs.dll` | host died (unchanged) |
| `gen_hotkeys`, `gen_jumpex`, `in_flac`, `in_vorbis`, `in_wave`, `out_disk`, `out_wave`, `vis_milk2`, `vis_nsfs` | **no reply within 10 s** (the host is silent; it does not answer the load) |

Loaded: **0 of 11**. The fixtures that pass in the Group D tests (our own
`gen_`, `in_`, `out_`, `vis_`, `dsp_` fixtures, `in_openmpt.dll`, the Optilab
DSP) are unaffected.

## What this means

1. **A silent host is a defect.** A plug-in that waits for something (a Winamp 5
   service, a window message the host does not answer, a dialog) blocks the
   control channel for the whole load. The player can time the request out, but
   the host should answer with a refusal instead of nothing. Next step:
   find out where the host thread waits (a trace around `LoadLibraryExW`,
   `DllMain` and `init`) for one small plug-in such as `out_wave.dll`.
2. **Winamp 5 DSP plug-ins carry header version `0x21`.** The ABI crate has the
   5.x header type; the host accepts only `0x20`. Accepting `0x21` is a small,
   specified change (S09) once the module layout is checked.
3. **`vis_avs.dll` kills the host.** Process isolation works as designed, but a
   crash dump or a structured exception filter in the host would say where.
4. **MilkDrop needs the DirectX 9 runtime in the guest.** `vis_milk2.dll`
   imports `d3dx9_24..43`; the guest has none. Installing the DirectX
   end-user runtime into the base checkpoint is a provisioning step.
5. The Winamp 5.x plug-ins here also assume a Winamp 5 host (services, the
   `api_*` interfaces). Only plug-ins written against the classic 2.x ABI are
   expected to load on this host; treat "does not load" for the Wasabi-based
   ones as expected, not as a bug.
