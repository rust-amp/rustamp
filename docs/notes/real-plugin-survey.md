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


## Run 3-5 (2026-10-25): what was wrong, and what is fixed

Owner decision 2026-10-25: **5.x plug-ins are accepted** (S09 U-7 closes as a
range): DSP headers 0x20/0x21/0x22, visualization 0x101/0x102, general and
output 0x10/0x11, input 0x100/0x101. A "wide" description is read by looking at
the first two bytes, because some 5.66 plug-ins carry the wide version but an
ANSI literal.

**The silent host (finding 1) is fixed, and its cause was not what it looked
like.** The nine plug-ins that import `msvcr90.dll` did not hang in the
plug-in's code. The C runtime raised its own modal dialog
(`Microsoft Visual C++ Runtime Library`, `R6034`: *an attempt to load the C
runtime library incorrectly*) on the host's UI thread, and nobody clicked it.
`tasklist /v` on the stuck host named the window. Two causes, two fixes:

1. **A watchdog** (`dialog_watchdog.rs`): after 3 s of a load, standard dialogs
   of the UI thread are recorded and dismissed, and the refusal quotes them. The
   host now always answers, whatever a plug-in raises.
2. **The runtime itself** (`side_by_side.rs`): the 5.x plug-ins have **no
   manifest of their own**; they inherit `Microsoft.VC90.CRT` from
   `winamp.exe`'s manifest. The host now builds an activation context that
   declares the `Microsoft.VC*.CRT` folders found beside the plug-in, one level
   above it or beside the host, and activates it **only while a plug-in loads**.
   (A loose `msvcr90.dll` next to the plug-ins is wrong and produces `R6034`;
   only the assembly folder works.)

Result of the last run, 14 plug-ins and their libraries in the guest:

| plug-in | result |
|---|---|
| `gen_hotkeys`, `gen_jumpex`, `out_wave` | **loaded** |
| `in_flac`, `in_vorbis`, `in_wave`, `out_disk`, `dsp_sps` | access violation in `init` (named by the host's fault guard) |
| `vis_avs`, `vis_milk2` | access violation in the header getter |
| `vis_nsfs` | the entry point returned no module |

The access violations are the next layer, not a loader problem: the 5.x
plug-ins ask the host window for Winamp 5's service manager
(`IPC_GET_API_SERVICE`) and dereference what they get back. Supporting them
means answering that request with the Wasabi services they use
(`api_application`, `api_memmgr`, `api_config`, ...), which is its own task.
`vis_milk2` additionally needs the DirectX 9 runtime in the guest.

The survey stays informational: it prints each plug-in's outcome, the host's
trace of calls into plug-in code (`RUSTAMP_HOST_TRACE`) and, for a refusal, the
dialogs the watchdog dismissed.


## Run 6 (2026-10-25): what the faults read

The fault report now names the faulting instruction and the address it used,
and `RUSTAMP_HOST_TRACE` records every non-system window message a plug-in sends
the host. Seven plug-ins fault; six read through a **null** pointer and
`dsp_sps` through the value **1** (the host's answer for "unsupported"). Before
faulting `out_disk` asked for ids 334 and 3025, `in_vorbis` for 3015 and
`dsp_sps` for 336; `in_flac`, `in_wave`, `vis_avs` and `vis_milk2` asked for
nothing. The 5.x service layer is **not built**: its layout comes from the
Winamp SDK headers, which the clean room does not read. The request to the dirty
room is `docs/spec-requests/2026-10-25-s21-winamp5-host-services.md`.


## Run 7 (2026-10-25): S21 implemented - 10 of 11 plug-ins load

`specs/S21-winamp5-host-services.md` (audited, see `spec-audit-s21.md`) is
implemented in `host_services.rs` and the loader: a service manager with the
language service, the settings paths and random generators for the 5.x start-up
requests (answered only to requests from the host's own thread; everything else
keeps the 2.95 answers of S10), the header getters of visualization and DSP
plug-ins take the main window, 5.x input modules get the service manager
pointer, and descriptions are read after `init`.

Two more things the survey found that the spec could not know:

* A plug-in's directory has to stay in the process's library search, and the
  default search is set to application directory, System32 and the added
  directories, because `in_wave` loads `libsndfile.dll` by a plain
  `LoadLibrary` inside `init`.
* The VC++ runtime context must stay active on the UI thread after the load,
  for the same reason: those later loads inherit it.

Result in the Hyper-V guest, all plug-ins of the Winamp 5.66 installer that were
extracted: **loaded** `dsp_sps`, `gen_hotkeys`, `gen_jumpex`, `in_flac`,
`in_vorbis`, `in_wave`, `out_disk`, `out_wave`, `vis_avs` and **`vis_milk2`
(MilkDrop 2.25c)**; refused `vis_nsfs`, which returns no module on a current
Windows for its own reasons (S21 section 7). "Loaded" means the host mapped the
plug-in, ran its getter and `init` and read its description; playing audio
through them, a visualization window and the configuration dialogs are not
exercised yet.
