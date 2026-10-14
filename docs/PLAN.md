# Rustamp: full implementation plan

## Context
We are building **Rustamp**, a new media player that looks, feels and behaves like **Winamp 2.95** on Windows. It is written in Rust on a modern audio and graphics stack, runs classic Winamp 2.x plugins (above all visualizations such as MilkDrop 2 and AVS), and will ship for Linux and macOS later, with Wine running the legacy plugins there. The repo `D:\code\Rustamp` is empty, so this is a greenfield project. Many LLM agents will do the work in parallel, so the plan is built around **contracts first, then parallel lanes**, with a strict **clean-room** process for anything taken from Winamp itself.

Decisions already made:
- **License: GPL-3.0.** This means we may take code from DeaDBeeF (its zlib core and its GPL/LGPL plugins), webamp (MIT), butterchurn (MIT), WDL/EEL2 (zlib) and libopenmpt (BSD).
- **Skins.**
  - The bundled default is a **look-alike skin**: newly drawn artwork that closely mimics the classic base skin's layout, colors and style, without copying any pixels.
  - The **original base skin** (taken from webamp's `base-2.91`) is offered as a **separate, optional download** with its own repo and package. It is never bundled in the installer.
  - **Tests run with both skins.** The original skin is used for pixel-exact comparison with real Winamp 2.95. The look-alike skin is used to check rendering and layout of what we actually ship.
- **Reference binaries.** Agents find Winamp 2.95 and the classic plugins in archival sources (Internet Archive, winampheritage mirrors) and **ask the user before each download**. Binaries are stored outside both repos.

---

## 1. Legal and clean-room process (applies to every lane)

| Source | Status | Who may read it |
|---|---|---|
| alexfreud/winamp (Winamp **5.x** source) | must not be copied | **Dirty room only** |
| Winamp 2.95 binary and classic plugin DLLs, plus disassembly (Ghidra headless) | interoperability reverse engineering | **Dirty room only** |
| Winamp SDK headers and docs | treat as dirty | Dirty room only; the ABI goes into specs as *facts* (sizes, offsets, constants, call order) |
| foobar2000 | analyze behavior and architecture, copy nothing | Dirty room writes design notes |
| WACUP changelog (https://getwacup.com/changelog/) | inspiration only: feature ideas, plugin-compatibility quirks, modernization ideas. No code, assets or verbatim text; WACUP is closed source | Everyone |
| DeaDBeeF, webamp, butterchurn, WDL/EEL2, libopenmpt, projectM (LGPL, reference only) | copy allowed, with attribution | Everyone |
| MilkDrop 2.25c source (geisswerks.com, `milkdrop_225c_src.zip`): files under `vis_milk2/` that carry the Nullsoft BSD-3 header | copy allowed, with BSD notice kept and attribution | Everyone. **Exception:** the bundled Winamp SDK headers in that zip (`Winamp/`, `Wasabi/`, `Agave/`, `vis_milk2/vis.h`, `api.h`) and any file without a BSD header are dirty. Check the `ns-eel2/` license separately before use |

How the clean room works:
- **Two repos.** `rustamp` (clean, public) and `rustamp-research` (dirty, private). The research repo holds notes, disassembly, screenshots and draft specs.
- **Dirty-room agents** (R-agents) write specs in prose plus tables: behaviors, timings, pixel coordinates, constants, state machines and ABI layouts. They write **no code and no pseudo-code transliterated from the source**, and they never commit to `rustamp`.
- **Spec Auditor agent.** It checks every spec for copied code, comments or distinctive identifiers. Names that interoperability requires, such as `In_Module`, `WM_WA_IPC` and the IPC numbers, are allowed. Only after the audit is a spec exported to `rustamp/specs/S-xx-*.md`, with a provenance header that records the sources, the agent and the date.
- **Clean-room agents** (implementers) get worktrees and prompts that forbid access to the Winamp source, the research repo and disassembly. They may freely read webamp, DeaDBeeF and the other open projects. Each PR carries a provenance checklist.
- **Precedence.** The alexfreud repo is Winamp **5.x**, so every behavioral spec must be **validated against the real 2.95 binary**. Where the two differ, 2.95 behavior wins.

---

## 2. Architecture

**64-bit Rust main process**, plus a **32-bit plugin host process** (classic plugins are x86 DLLs), connected by an IPC layer.

```
rustamp.exe (x64)                                     rustamp-plughost32.exe (i686)
 ├ app / command bus / state store                    ├ loads in_/out_/vis_/dsp_/gen_ DLLs
 ├ audio engine: decode → DSP → output (f32)          ├ fake "Winamp v1.x" main window (answers WM_WA_IPC,
 │   └ vis tap (576-sample waveform + spectrum)       │   forwarding to the main process over IPC)
 ├ skin engine + pixel compositor (softbuffer)        ├ fake out-module for in_ plugins → PCM over shm ring
 ├ windows: Main / EQ / Playlist / Gen(embed) / Vis   ├ embedWindow frames drawn with gen.bmp (shared skin crate)
 ├ native MilkDrop (butterchurn port, wgpu)           └ DSP chain round-trip over shm
 ├ "Winamp v1.x" compatible window (external control)
 └ platform layer (Win32 now, mac/Linux later)  ◄──── shm rings + control pipe/socket ────►
```

### Tech stack
- **Windowing:** `winit` 0.30, plus a raw-handle platform shim (`windows` crate) for `SetWindowRgn` (region.txt), skinned cursors, and layered or always-on-top windows.
- **Skin rendering:** CPU pixel compositor in RGBA buffers, presented with `softbuffer`, with integer scaling for double-size. This gives pixel-exact output.
- **Visualization:** `wgpu`. MilkDrop shaders go HLSL → SPIR-V through `shaderc` (glslang HLSL front end), then SPIR-V → WGSL through `naga`.
- **Menus and tray:** `muda` for native popup menus (Winamp used native Win32 menus), `tray-icon`, `global-hotkey`.
- **Dialogs and prefs:** `egui` with a Win9x/Classic theme, one implementation for every platform. It sits behind a trait so native dialogs can replace it later if needed.
- **Audio:**
  - Decoding: `symphonia` (MP3, FLAC, Vorbis, WAV, AAC/ALAC), `opus` via FFI, and DeaDBeeF-derived FFI decoders: libopenmpt (mod/xm/it/s3m), game-music-emu, sidplayfp, WavPack, APE, MPC.
  - Output: WASAPI (`wasapi` crate) in shared and exclusive mode, with `cpal` as the cross-platform fallback.
  - Processing: resampling with `rubato`, lock-free rings with `rtrb`, FFT with `rustfft`.
- **Metadata and library:** `lofty` (tags), `rusqlite` (library), `ureq`/`reqwest` (HTTP streams with ICY metadata).
- **Config:** TOML internally, with **winamp.ini import** and plugin-facing ini emulation (plugins read and write `winamp.ini` and `plugin.ini`).
- **Packaging:** NSIS installer (a nod to the original), via an `xtask`.

### Cargo workspace (`rustamp/crates/*`)
| Crate | Responsibility |
|---|---|
| `rustamp-core` | Domain types, `Command`/`Event` enums, state store, config, error types (**the contract crate, frozen early**) |
| `rustamp-audio` | Engine threads, transport, gapless, ReplayGain, crossfade/fades, vis tap |
| `rustamp-decoders` | `Decoder` trait plus symphonia and FFI decoders (DeaDBeeF ports) |
| `rustamp-dsp` | Winamp-style 10-band EQ and preamp (from spec S07), resampler, limiter |
| `rustamp-output` | WASAPI, cpal, null and disk writer (`out_disk` equivalent) |
| `rustamp-meta` | Tags, Winamp 2 title formatting (%1–%7 style), ID3v1/v2 |
| `rustamp-playlist` | Model, shuffle (S15), sort and randomize, M3U/M3U8/PLS/B4S/XSPF I/O |
| `rustamp-skin` | .wsz/.zip/folder loading, BMP (all bit depths), pledit.txt, viscolor.txt, region.txt, cursors, fallback to the default skin, sprite tables (ported from webamp `skinSprites`/`skinParser`, MIT) |
| `rustamp-gfx` | Compositor, blits, bitmap fonts (text.bmp, numbers/nums_ex), system-font text for the playlist (`cosmic-text`) |
| `rustamp-ui` | Window components and widgets (buttons, sliders, marquee, time display), hit testing, docking/snapping manager, windowshade, double-size, easy-move, menus |
| `rustamp-vis` | Built-in spectrum analyzer and oscilloscope (S08), vis window host, wgpu context |
| `rustamp-milkdrop` | Native MilkDrop 2 port of butterchurn (presets, EEL2 expression engine via a WDL port or our own interpreter/cranelift JIT, warp/comp shaders, blending) |
| `rustamp-plugin-abi` | `#[repr(C)]` Winamp plugin structs and constants (from spec S09/S10); builds for both x64 and i686 |
| `rustamp-ipc` | Transport trait (shm ring + named pipe on Windows; socket/stdio for Wine later) and protocol enums (bincode) |
| `rustamp-plughost` | **i686 binary**: DLL loading, fake Winamp window, fake out-module, embed frames |
| `rustamp-platform` | Win32: single instance plus command-line forwarding, SMTC/media keys, taskbar thumb buttons, jump lists, file associations, the `Winamp v1.x` external-control window; mac/Linux stubs |
| `rustamp-prefs` | egui preferences tree (S16), skin browser, plugin config launcher |
| `rustamp-library` | Media Library (2.9x gen_ml-like), later milestone |
| `rustamp-app` | Binary that wires everything together |
| `xtask` | Build, i686 cross-build, packaging, golden-test runner, skin packaging |

Separate repos: `rustamp-skin-default` (the look-alike default skin: newly drawn art, bundled in the installer), `rustamp-skin-classic-base` (the original base skin, optional separate download, used in tests), `rustamp-research` (dirty, private), `rustamp-testdata` (reference captures, audio fixtures).

---

## 3. Specs the dirty room must produce (they feed the clean lanes)
Ordered by priority. Each spec is validated against the 2.95 binary through automation (screenshots, timing, `out_disk` captures) and against the source where useful.

- **S01** Window geometry, sprite rects and pixel layout deltas that webamp gets wrong or misses
- **S02** Main window behaviors: clutterbar (O/A/I/D/V), title marquee (speed, separator, scroll-on-drag), time display (elapsed/remaining, blink on pause), kbps/kHz, mono/stereo, button press states
- **S03** Complete keyboard and mouse shortcut table, for each window
- **S04** Menu trees (main, right-click, playlist button menus, EQ presets), with labels, order and checked states
- **S05** Docking and snapping rules (distances, group moves, behavior on resize and double-size)
- **S06** Playlist editor: selection model, drag-reorder, scrolling, mini time display, sort/misc/list menus, Jump-to-file search semantics, resize steps (25×29)
- **S07** Equalizer: band centers (60, 170, 310, 600, 1k, 3k, 6k, 12k, 14k, 16k), filter design and Q, slider↔dB mapping, preamp, auto-load, `.eqf`/`winamp.q1` formats, EQ graph curve drawing
- **S08** Classic vis: spectrum computation, bar and peak falloff speeds, analyzer modes, oscilloscope styles, refresh rate, **exactly how the 576-point waveform and spectrum data for plugins are built**
- **S09** Plugin ABI: in_/out_/vis_/dsp_/gen_ structs, versions, threading, call order, lifetime, the SA/VSA callbacks
- **S10** `WM_WA_IPC` and `WM_COMMAND` catalog. Prioritize by scanning the imports and calls of the top-N real plugins and external tools (AutoHotkey scripts, scrobblers, Rainmeter)
- **S11** Embed-window API and `gen.bmp` layout and behaviors
- **S12** winamp.ini keys and semantics; plugin.ini conventions
- **S13** Command-line switches, single-instance and enqueue behavior, drag-and-drop rules
- **S14** Title formatting; **S15** shuffle algorithm; **S16** preferences pages and options
- **S17** Skin-loading quirks: missing files, palette/BMP quirks, nums_ex fallback, pledit.txt defaults, region.txt
- **S18** Output behaviors (out_ds-style fades, buffer, prebuffer); **S19** streaming/ICY display
- **S20** Media Library (2.9x) and CD playback, for later milestones
- **D-notes:** foobar2000 pipeline observations (component model, ReplayGain, gapless and seeking strategy), behavior only
- **W-notes:** WACUP changelog digest. It lists plugin-compatibility quirks that real plugins depend on, which feed S09/S10 and the plugin compatibility matrix, plus modernization ideas for M4. Summaries are written in our own words.

---

## 4. Parallel lanes and agent roles
Every work package is a task card containing: goal, allowed sources, the spec(s) it depends on, the crate or module it owns, the interface contract, acceptance tests and a provenance checklist. Each agent works in its own git worktree, with small PRs into trunk and CI gating.

| Lane | Agents | Owns | Depends on |
|---|---|---|---|
| **R** Research (dirty) | R-src (source reader), R-bin (runs and automates 2.95), R-dis (Ghidra), Spec Auditor | `rustamp-research`, `specs/` | binaries |
| **A** Audio engine | 2 | audio, output, dsp | core, S07, S18 |
| **B** Decoders/formats | 2–3 (one per format family) | decoders, meta | core |
| **C** Skin and gfx | 2 | skin, gfx | core, S01, S17 |
| **D** Windows/UI behavior | 3 (main / EQ / playlist) | ui | C, S02–S06 |
| **E** Playlist and library | 1–2 | playlist, library | core, S06, S14, S15 |
| **F** Platform | 1 | platform, app wiring | core, S10, S13 |
| **G** Plugin host | 2 | plugin-abi, ipc, plughost | S09–S11 |
| **H** Vis/MilkDrop | 2–3 | vis, milkdrop | audio vis tap, S08 |
| **I** Prefs/dialogs | 1 | prefs | ui, S16 |
| **J** Art and skin packaging | 1 (art) | look-alike default skin (new art, every sprite sheet), optional original-skin download package | S01 |
| **Q** QA harness | 1–2 | golden and differential test infrastructure | all |

Integration Lead agent: owns `rustamp-core` and `rustamp-app`, merges, and resolves contract changes through RFCs in `docs/rfc/`.

---

## 5. Milestones (waves)

**M0 Foundation (mostly sequential, small, done first)**
1. Repo, GPL-3.0, workspace skeleton, CI (fmt, clippy, test, `i686-pc-windows-msvc` build of plughost), CONTRIBUTING with clean-room rules and PR provenance template.
2. **Freeze the contracts:** `rustamp-core` (`Command`, `Event`, `PlayerState`, `PlaylistModel` traits, `Decoder`/`Output`/`Dsp` traits, `VisFrame`), plus `rustamp-ipc` message enum v0. Provide mocks for each so lanes can start against them.
3. Set up the research repo; fetch binaries (with user approval); R-agents start S01–S10.
4. Build the QA harness skeleton: headless render-to-PNG of any window state, and an automation driver for real Winamp 2.95 (Win32 messages plus screenshots).

**M1 "It plays" (lanes A, B, C, D-main, E in parallel)**
- Decode, output and transport (play, pause, stop, seek, volume, balance), gapless.
- Skin loading plus the main window pixel-exact with the classic-base skin, buttons, posbar, volume and balance, marquee, time display.
- Basic playlist model, file open, drag and drop.

**M2 "It feels like Winamp" (D, E, F, I, J, H-classic)**
- EQ window with the S07-accurate EQ, presets and graph.
- Playlist editor fully implemented.
- Docking, snapping, windowshade, double-size, easy-move, all shortcuts and menus.
- Skin browser, preferences, original default skin.
- Tray, global hotkeys, media keys and SMTC, single instance and CLI, `Winamp v1.x` external-control window.
- Classic spectrum and oscilloscope; winamp.ini import.

**M3 "Classic plugins" (G, H)**
- plughost32 running vis_ plugins (MilkDrop 2.x, AVS 2.81d, Geiss, vis_classic), with embed windows framed by gen.bmp.
- Then dsp_, in_ (in_openmpt, in_vgm, in_sid), gen_ (scrobblers, hotkeys) and out_.
- Crash isolation: restart the host on plugin crash.

**M4 "Modern underneath"**
- Native MilkDrop (butterchurn port) with a preset browser.
- WASAPI exclusive mode, ReplayGain plus scanner, crossfade, HTTP/Icecast/SHOUTcast streaming with ICY titles, HiDPI integer scaling (2×/3×) beyond double-size, M3U8/XSPF.

**M5 Media Library** (2.9x-like, SQLite, tag editor, CD playback and ripping optional)

**M6 Cross-platform**
- Linux and macOS backends for platform and windowing (non-rectangular windows through transparency masks), cpal output.
- plughost32 under Wine with the socket transport; native MilkDrop is already portable.

Parallelism inside each milestone: one agent per crate or window, all coding against the frozen `rustamp-core` traits and mocks. The R lane always works one milestone ahead of the clean lanes.

---

## 6. Verification
- **Unit and property tests** for each crate: playlist operations, shuffle, title format, skin parsing on a corpus of ~200 real .wsz skins from the webamp skin museum (must not panic, sprite fallbacks correct).
- **Golden image tests, run with both skins:**
  - *Original base skin:* headless rendering of every window, state and widget is compared pixel-for-pixel with captures R-bin takes from real Winamp 2.95. Captures live in `rustamp-testdata`, which is a reference, not shipped.
  - *Look-alike default skin:* Rustamp's own approved snapshots, to catch regressions. Real Winamp 2.95 is also captured while running the look-alike skin (it is a valid .wsz), so the same pixel-for-pixel comparison works for it as well.
- **Differential behavior tests:** both real Winamp and Rustamp expose the same `Winamp v1.x` IPC and `WM_COMMAND` surface, so **one script drives both** (key sequences, clicks, IPC queries) and compares the resulting state (position, playlist, EQ, window positions).
- **Audio tests:**
  - Decoders are bit-exact against reference decoders.
  - EQ frequency response and preamp curves are compared with sweeps rendered through real Winamp `out_disk`.
  - Gapless is checked with sample-accurate join tests.
  - A soak test verifies there are no allocations on the audio thread.
- **Plugin compatibility matrix:** CI on a Windows runner launches plughost32 with a curated plugin set and records load, render and IPC results.
- **Manual acceptance per milestone:** a side-by-side session with real Winamp 2.95, covering the S03 shortcut checklist, docking, skins and MilkDrop presets.
- Run locally with `cargo run -p rustamp-app`, `cargo xtask golden`, `cargo xtask plughost`.
