# Roadmap to Winamp 2.95 Parity

**Goal:** "looks and feels like Winamp 2.95, modern backend"  
**Date:** 2026-10-20  
**Current State:** M0 complete, M1 nearly done, BETA-10 done; 18 of the 20 specs in
`specs/README.md` are audited

> **Status update (2026-10-20, spec wave 275-308):** the section below is the
> original gap analysis. **Eight specs were audited on 2026-10-20** and are on
> `main` (`specs/README.md` is the authoritative list, and each spec file
> carries the audit date in its header): `S08` (analyser: ten bands, five
> shaded, the fall-off and the peak tables), `S11` (the embed/`gen.bmp` sheet
> inventory; the sprite rectangles are explicitly out of scope), `S12`
> (`winamp.ini` keys, the per-plug-in section convention, the `eq_data` parse),
> `S13` (command line, single instance, shell verbs), `S14` (title formatting -
> and its finding that the language belongs to the **input plug-ins**, not to
> the player), `S18` (the output plug-in owns the fades, the buffer and the
> prebuffer; **no crossfade is established for this build**), `S19` (the
> HTTP/1.0 streaming request and the status chain) and `S20` (the media library
> is a **plug-in**, with its own views, columns and a path-template language).
>
> **Implemented from those specs:**
>
> * the `winamp.ini` import - the CLI switch `--import-winamp-ini`, a
>   Preferences page and the honest "read in our own way" split
>   (`crates/rustamp-app/src/winamp_ini.rs`, `crates/rustamp-app/src/cli.rs`);
> * the S13 conformance including `/ADD`
>   (`crates/rustamp-app/src/cli.rs`,
>   `crates/rustamp-platform/src/shell_registration.rs`);
> * the analyser on the real S08 numbers (`crates/rustamp-vis/src/analyser.rs`,
>   used by `crates/rustamp-ui/src/visualization.rs`);
> * the S14 format engine - **implemented but unwired by default**
>   (`crates/rustamp-ui/src/title_format.rs`);
> * the HTTP/1.0 stream decoder - **implemented but unwired**
>   (`crates/rustamp-decoders/src/http_source.rs`,
>   `crates/rustamp-decoders/src/http_dec.rs`);
> * the S20 path-template work is **in flight** and is not on `main` yet.
>
> **Still missing:** `S16` (the Preferences pages) is **not started** - its
> labels have to come from the reference's own resources, so the measurement
> needs that path rather than a reading of our own page tree; `S10` is a
> revised draft pending ratification.
>
> **The review is closed.** `docs/tasks/W14-wave-review.md` is closed except
> `C1-2`/`C1-3`, which are **false by design** - the caller does the wiring.
>
> **Claims this document had wrong, and what replaces them:**
>
> | Wrong before | Correct now | Source |
> |---|---|---|
> | "S20 is missing a reference" | S20 is **audited**; the reference's library is a **plug-in**, not a player feature. The column layout, the default view set and the sort semantics stay **open** (S20 §8, questions 1-3); CD playback is out of scope | `specs/S20-media-library.md` |
> | "Crossfade between tracks" is a parity item | It is **not**: S18 §7 item 9 records that the reference's output plug-in carries no crossfade setting, that this is an absence claim about **one plug-in's string table**, and that the repo holds no measurement either way. The item is open, not required | `specs/S18-output-behaviour.md` |
> | "Title formatting" is a global player feature | The reference has **no global title-format setting**; the language belongs to the **input plug-ins**, and the player does not interpret a plug-in's keys. A global player setting is **our own choice** | `specs/S14-title-formatting.md` §1 |
> | "S08, S11-S14, S16, S18-S20" are missing specs | Only **S16** is not started; **S10** is a draft pending ratification | `specs/README.md` |
> | Native file picker "blocked by a Win32 binding decision"; drag-and-drop "not implemented at all"; add-directory "needs S13" | All three are implemented: the native open dialog, the native folder dialog and the Explorer drop | `crates/rustamp-platform/src/file_dialog.rs`, `crates/rustamp-app/src/window.rs` |

> **Status update (2026-10-19, continuous run - superseded by the note
> above):** the section below is the original gap analysis. Many of its items
> have since been implemented and are
> listed in `docs/HANDOFF-2026-10-19-dauerbetrieb.md` (48 merged lanes): native
> file/directory dialogs, drag-and-drop, save/load playlist with format
> detection, playlist keyboard + sort + remove-dead + clear + drag-reorder,
> jump-to-file, docking/snapping, window shaping, skin cursors, `pledit.txt`
> font, `viscolor.txt` palette, EQ presets/auto-load, ReplayGain (scanner +
> engine + config), WASAPI exclusive, device-change recovery, fades, all five
> plugin kinds, plugin crash isolation, external control (WM_WA_IPC), the media
> library (index, scan, view, renderer, window, covers, tag writes, roots in
> preferences), and the cross-process wake proof. What still blocks parity is
> listed under "Open points" in that session log: missing specs (S08, S11-S14,
> S16, S18-S20), RFC 0020 (dated draft), and the VM verification of everything
> window-shaped.

---

## How to read this document

| Marker | Meaning |
|---|---|
| ✅ | implemented and **wired** - the player reaches it |
| 🟡 | implemented but **unwired** - the code exists and is tested, the player does not reach it yet |
| ⚠️ | implemented as a **documented own choice** - honest, but not parity |
| ❌ | not implemented |
| 📄 | a spec that would have to be written or audited first |

Every status below names the file that carries it, so a claim can be checked
without trusting this document.

---

## Status Overview

### ✅ What Works (M0 + M1 + Partial M2)

**Core Infrastructure:**
- ✅ Audio engine: decode → DSP → output (gapless, seek, volume/balance)
- ✅ Decoders: MP3, FLAC, Vorbis, WAV, AAC/ALAC (symphonia) **and Opus**
  (`opus-rs`, sample-exact against ffmpeg) - `crates/rustamp-decoders`
- ✅ Output: WASAPI shared **and exclusive** mode (a shared-mode downgrade is
  reported), cpal fallback, null output - RFC 0019
- ✅ DSP: 10-band EQ + preamp (S07's settled parts; the band centres stay an
  own choice), decode chain - `crates/rustamp-dsp/src/eq.rs`
- ✅ Playlist model: entries, shuffle, repeat (engine-owned, S15)
- ✅ Configuration: read/write TOML, honours settings; every setting the
  player does not apply survives a save - `crates/rustamp-app/src/config_preserve.rs`
- ✅ Visualization: the shared classic analyser in `crates/rustamp-vis`, used
  by the UI

**UI - Main Window:**
- ✅ Skin loading: .wsz/zip/folder, BMP all depths, fallbacks
- ✅ Main window: layout, rendering, hit-testing
- ✅ Transport: play/pause/stop/prev/next, position slider
- ✅ Volume + Balance sliders
- ✅ Marquee scrolling, time display (elapsed/remaining)
- ✅ Window buttons: minimize/shade/close
- ✅ Clutterbar: A (always on top), I (file info), D (double-size)
- ✅ Clutterbar O/V: **implemented** (opens Options/Visualization menus)
- ✅ Double-size, windowshade
- ✅ **Title bar window-move fix** (2026-10-19)
- 🟡 **Title-format engine (S14)** - written, **off by default**
  (`crates/rustamp-ui/src/title_format.rs`)

**UI - Other Windows:**
- ✅ Equalizer window: sliders, ON/AUTO, graph, presets popup
- ✅ Playlist window: scrolling, selection, visibility toggle
- ✅ Media library window - `crates/rustamp-app/src/library_window.rs`

**UI - Dialogs:**
- ✅ **Native** Open-file(s) dialog (the common dialog, multiple selection) and
  the **native** folder dialog (`IFileOpenDialog` + `FOS_PICKFOLDERS`) -
  `crates/rustamp-platform/src/file_dialog.rs`
- ✅ Jump to Time dialog
- ✅ Load Playlist dialog (wired, Enter loads)
- ✅ Preferences dialog: 16 pages in a native Windows dialog, including the
  `winamp.ini` import page - `crates/rustamp-platform/src/prefs_dialog.rs`

**Platform:**
- ✅ DPI awareness (per-monitor V2)
- ✅ Single instance + CLI forwarding
- ✅ Tray icon + context menu
- ✅ Global hotkeys (6 default Ctrl+Alt chords)
- ✅ Window focus navigation (Ctrl+Tab, Alt+W)
- ✅ Shell registration: the S13 verbs (`open`, `Play`, `Enqueue`,
  `ListBookmark`, `install`) for media files, playlist files, skin archives and
  directories - `crates/rustamp-platform/src/shell_registration.rs`

**Plugins/Hosting:**
- ✅ Plugin ABI types (S09)
- ✅ Plugin loader with approval store (snapshot + preflight)
- ✅ Plugin host binary (i686)
- ✅ Decoder resident (rustamp-dechost)
- ✅ DSP resident (rustamp-dsphost-stages)
- ✅ EQ presets: 9 standard presets
- ✅ Plugin UI selection: 5 pages, approval-gated, persistence
- ✅ **All five classic plug-in kinds load** (in_, out_, vis_, dsp_, gen_)

**Media Library (M5):**
- ✅ SQLite index, cancellable scan with progress and batch commits
- ✅ Multi-root, unicode freetext search, sort and pagination
- ✅ View model, renderer, window, cover column with a bounded artwork cache
- ✅ Tag editor with atomic writes and per-format capability reporting
- ✅ Roots, last-scan times and the damaged-database recovery path in the
  Preferences

**External control:**
- ✅ `WM_WA_IPC` / `WM_COMMAND` surface and the "Winamp v1.x" window -
  `crates/rustamp-app/src/external_control.rs`

**Packaging:**
- ✅ Self-contained runtime (static CRT)
- ✅ Package tool with manifests + license inventory
- ✅ Portable release ZIP with a SHA-256 checksum file
  (`xtask/src/package.rs`)
- ✅ QUICKSTART generation

**Skins:**
- ✅ Built-in default: Rust theme (33 files, 19 sheets)
- ✅ Look-alike skin selectable

---

## What's Missing for Winamp 2.95 Parity

The markers say what is still open: ✅ done, 🟡 done but not wired, ⚠️ our own
choice, ❌ not done.

### Critical (M2 - "It feels like Winamp")

#### 1. **Menus**
- ✅ Main menu: 19 rows, 4 separators, 6 submenus (S04) - the structure, the
  labels and the check states are the audited ones
  (`crates/rustamp-ui/src/menu.rs`)
  - ✅ File > Open File(s) - the native open dialog
  - ✅ File > Open Directory - the native folder dialog; the scan is
    **recursive as an own choice** (`crates/rustamp-app/src/directory.rs`)
  - 🟡 File > Open Location (URL) - the dialog and the playlist entry exist,
    but **playback is refused**: the decoder registry has no HTTP factory
    (see §13)
  - ✅ Play > Jump to File
  - ✅ Options > EQ presets - the native popup with Load (native `.eqf`
    picker) / Save / Delete
  - ✅ Options > Skin Browser
  - ✅ Options > Preferences - works
  - ✅ Visualization submenu - exists, 4 rows
- ✅ Playlist MISC menu (S04 §7) - `crates/rustamp-ui/src/menu.rs`
- ❌ The other playlist popup trees (File/Sort/List/Selection) and the
  right-click tree - **open in S04 §9**, so nothing may be invented for them

#### 2. **File Operations**
- ✅ Native OS file picker (the common dialog) and native folder picker
- ✅ Drag-and-drop files from Explorer → playlist
  (`WindowEvent::DroppedFile` in `crates/rustamp-app/src/window.rs`)
- ✅ Add Directory (recursive, an own choice)
- ✅ Remove selected entries from playlist
- ✅ Clear playlist
- ✅ Save/load playlist, M3U / M3U8 / PLS / XSPF, with format detection -
  `crates/rustamp-playlist/src/format.rs`

#### 3. **Playlist Editor**
- ✅ Scrolling, selection, visibility
- ✅ Drag-reorder entries (with the insert line)
- ✅ Jump to File search
- ✅ Sort (the S03 §2 rows `40209`-`40212`, with S06's semantics) - the
  **popup** that carries it in the reference is one of the open S04 §9 trees
- ✅ Misc menu (S04 §7)
- 🟡 Title formatting (S14) - the engine exists and is **off by default**; the
  row text is where it hangs, which is **our own choice** (S14 §1 offers it as
  a reading)
- ✅ Playlist file format detection (M3U vs PLS vs XSPF)

#### 4. **Docking & Snapping** (S05)
- ✅ Window docking (main + EQ + playlist), snapping, group moves,
  persistence - `crates/rustamp-ui/src/docking.rs`

#### 5. **Shortcuts**
- ✅ Main window shortcuts: the S03 §1 table (62 rows)
- ✅ Playlist shortcuts: the S03 §2 table as data (83 bound, 5 explicitly
  unimplemented)
- ❌ Equalizer shortcuts: S03 §3 **has no table yet**
  (`crates/rustamp-ui/src/shortcuts.rs`)
- ⚠️ Global shortcuts: the six default Ctrl+Alt chords are an **own choice**;
  the S03 §4 table (29 rows) is not registered beyond them

#### 6. **Keyboard Navigation**
- ✅ Up/Down, Home/End, Page Up/Down, Enter to play, Delete to remove, Ctrl+A
- ✅ Shift-range and Ctrl-toggle selection gestures (own choices; S06 settles
  only the single click and SEL)

---

### Important (M2/M3 - Polish)

#### 7. **Classic Visualizations**
- ✅ The analyser follows S08: the area and the band count (ten bands, five in
  the shaded area), the level scale in sixteenths of a pixel, the fall-off step
  and the peak factors - `crates/rustamp-vis/src/analyser.rs`
- ⚠️ The band **frequency edges**, the update rate and the direction of the
  peak marker's travel are own choices (S08's open questions)
- ✅ Vis plugin hosting (`vis_` kinds load, the render loop is paced, the
  configuration reaches the host)
- ❌ MilkDrop 2 / AVS / Geiss against the **real** plug-in DLLs - the host is
  implemented, the acceptance runs are VM-only and still outstanding

#### 8. **Plugin System** (M3)
- ✅ Plugin loader + approval; i686 host binary
- ✅ All five kinds load: `in_` (host driver + acceptance fixture), `out_`,
  `vis_`, `dsp_`, `gen_`
- ✅ Plugin crash isolation and a bounded restart policy (RFC 0006 §8)
- ✅ Plugin configuration routing for every kind
- ❌ The **embed window** (the frame that hosts a general-purpose plug-in): the
  sheet inventory is audited (S11) but the sprite rectangles are explicitly
  **out of scope** there, and the player has no embed window
- ❌ `out_disk` equivalent (a plug-in that writes to a file)

#### 9. **Equalizer**
- ✅ 10-band EQ + preamp, sliders, ON/AUTO
- ✅ 9 standard presets popup - **clickable**, with Load / Save / Delete
- ✅ Custom presets: save / load / delete
- ✅ `.eqf` file format (S07 §4.2)
- ✅ Auto-load per track (`eq-autoload.toml`, own-choice rules)
- ⚠️ Frequency response: the band centres are an own choice (the base skin's
  printed labels), not a measured filter response

#### 10. **Media Library**
- ✅ Library database, view, renderer, window, query/filter, tag editor,
  album art - as our own modern design
- ✅ The reference's library is a **plug-in** with its own four views, seven
  fields and a path template (S20); our first-class library is therefore an
  **own design**, not a deviation
- ❌ The column layout, the default view set and the sort semantics against the
  reference - **open** (S20 §8, questions 1-3)
- ❌ CD playback / ripping - out of scope for S20, and not a parity item

#### 11. **External Control** (M2)
- ✅ `WM_WA_IPC`, `WM_COMMAND` and the "Winamp v1.x" window
- ✅ File associations and the shell verbs (S13 §3)
- ❌ Media keys (SMTC), taskbar thumb buttons, jump lists
- ❌ Third-party tools (AutoHotkey, scrobblers, Rainmeter) exercised against
  the real tools

---

### Nice-to-Have (M4+ - Modern features)

#### 12. **Advanced Audio** (M4)
- ✅ WASAPI exclusive mode (a shared-mode downgrade is reported)
- ✅ ReplayGain (scanner + engine + configuration)
- ✅ Output fades (stop/pause): the equal-power fade stage; the fade-out
  length is configurable and reaches the engine
- ✅ Device change recovery
- ❌ `out_disk` equivalent
- ❌ **Crossfade between tracks - not a parity item.** S18 §7 item 9: the
  reference's output plug-in carries no crossfade setting, and that is an
  absence claim about **one plug-in's string table**, not about the reference;
  the repo holds no measurement either way. A crossfade would be **our own
  feature** (the DSP crate's wet/dry crossfade and equal-power fade are
  unrelated to it)

#### 13. **Streaming** (M4)
- 🟡 HTTP/1.0 streaming: the source and the decoder are written from S19
  (the request shape, the status chain, `Content-Length` as the truncation
  oracle) - **one branch is missing**: `default_registry()`
  (`crates/rustamp-decoders/src/lib.rs`) registers only the symphonia and the
  wave factory, so a `MediaSource::Url` is still refused by name
- ❌ ICY metadata (station and track names) - not implemented
- ✅ M3U8 / XSPF playlist formats (a playlist feature, not streaming)

#### 14. **Native MilkDrop** (M4)
- ❌ Butterchurn port to Rust - `crates/rustamp-milkdrop` is an M0 skeleton
- ✅ A clean-room projectM wrapper exists in `crates/rustamp-vis` (a different
  thing from a MilkDrop port)
- ❌ Preset browser, EEL2 expression engine, warp/comp shaders

#### 15. **Additional Codecs** (M4)
- ✅ Opus (`opus-rs`, BSD-3)
- ✅ libopenmpt through the `in_openmpt` plug-in
- ❌ game-music-emu (NSF/SPC/VGM), sidplayfp (SID), WavPack, APE (Monkey's
  Audio), MPC (Musepack)

#### 16. **Configuration** (M2)
- ✅ Read/write TOML
- ✅ `winamp.ini` import - the CLI switch, a Preferences page, and the report
  that names every key it **read in our own way** rather than the reference's
  (`crates/rustamp-app/src/winamp_ini.rs`)
- ❌ `plugin.ini` emulation: S12 §8 gives the per-plug-in section convention
  and §6 the `eq_data` parse, but we neither read nor write a plug-in's file

#### 17. **Installer** (Release)
- ✅ A portable ZIP with a SHA-256 checksum file (`xtask package`) - the
  **only** distribution form today
- ❌ NSIS or MSI installer
- ❌ Code signing - **decided: the 0.1.0 release ships unsigned**
- ✅ Auto-update - **decided: NO for 0.1.0** (nothing to build)

#### 18. **Cross-Platform** (M6 - Future)
- ❌ Linux backend, macOS backend, plugin host under Wine

---

## Specs

`specs/README.md` is the authoritative status list; the table below repeats it
and adds what each one changed for this roadmap. **§N** in the last column
points at the numbered gap-analysis section below; a reference such as
"the §2 table" points at a section of the spec itself.

| ID | Title | Status | What it changed here |
|---|---|---|---|
| S01 | Window geometry, sprite rects and pixel layout deltas | audited | - |
| S02 | Main window behaviors | audited | - |
| S03 | Keyboard and mouse shortcut table, per window | audited | §5: the §2 table is bound, the §3 table does not exist yet |
| S04 | Menu trees | audited | §1: the main menu and the MISC menu are settled, the other playlist trees stay open (S04 §9) |
| S05 | Docking and snapping rules | audited | §4: docking is implemented |
| S06 | Playlist editor | audited | §3: sort and drag-reorder are implemented; the selection gestures are own choices |
| S07 | Equalizer | audited | §9: presets, `.eqf` and auto-load are implemented; the band centres stay own choices |
| S08 | Classic visualization | audited (2026-10-20) | §7: the analyser follows the audited numbers |
| S09 | Plugin ABI | audited – static; runtime confirmation pending | §8 |
| S10 | `WM_WA_IPC` / `WM_COMMAND` catalog | **revised draft, pending ratification** | the external-control surface is implemented; the catalog is not ratified yet |
| S11 | Embed-window API and `gen.bmp` layout | audited (2026-10-20) | §8: the sheet inventory is known, the sprite rectangles are out of scope |
| S12 | `winamp.ini` keys and plugin.ini conventions | audited (2026-10-20) | §16: the import is implemented; `plugin.ini` is not |
| S13 | Command-line switches, single instance, drag-and-drop rules | audited (2026-10-20) | §11: the CLI and the shell verbs conform, `/ADD` included |
| S14 | Title formatting | audited (2026-10-20) | §3: the engine is implemented and unwired; a global player setting is our own choice (S14 §1) |
| S15 | Shuffle algorithm | audited | the order model is the engine's own |
| S16 | Preferences pages and options | **not started** | the page set, its order and its labels are our own choice; see below |
| S17 | Skin-loading quirks | audited | - |
| S18 | Output behaviors (fades, buffer, prebuffer) | audited (2026-10-20) | §12: the fades belong to the output plug-in; **no crossfade is established** |
| S19 | Streaming and ICY metadata display | audited (2026-10-20) | §13: the request shape and the status chain are implemented; the decoder is unwired |
| S20 | Media Library plug-in (2.9x) | audited (2026-10-20) | §10: the reference's library is a plug-in; the columns, views and sort semantics stay open |

**Still missing (dirty room):**

| Spec | Blocks | Note |
|---|---|---|
| **S16** | the Preferences page set, their order and their labels | **not started**. The labels have to arrive in the reference's **own words** (spec request §6, question 2), so the measurement reads the reference's resources rather than our own page tree. Until then the page tree and the wording are our own choice - `crates/rustamp-platform/src/prefs_dialog.rs` says exactly that |
| **S10** | the full `WM_WA_IPC` / `WM_COMMAND` catalog for plug-in compatibility | a revised draft with the lane's corrections applied; pending the Spec Auditor's ratification |

The request for both is filed in
[`docs/spec-requests/2026-10-20-missing-specs.md`](../spec-requests/2026-10-20-missing-specs.md);
it lists the nine that were still open on 2026-10-20 - **eight** of them
(S08, S11, S12, S13, S14, S18, S19, S20) were delivered and audited that same
day. Only S16 was not.

---

## Implementation Priority (Next Steps)

### **Phase 1: Make it Usable (High Priority)**

1. **Wire the HTTP decoder** (one branch)
   - Add an `HttpDecoderFactory` and register it, so a `MediaSource::Url` is
     opened instead of refused
   - Then: `File > Open location` plays, and the status line can show the
     buffering state
2. **The S16 measurement** (dirty room)
   - The Preferences page tree, the controls and their labels in the
     reference's own words, read from the reference's resources rather than
     from our own page tree
3. **Turn the S14 engine on** - a switch in the Preferences (our own choice,
   since the reference has no global setting)
4. **The playlist popup trees** - File/Sort/List/Selection and the right-click
   tree, once S04 §9 is settled by a measurement
5. **Test the packaged release in the VM** (the critical verification gap)

### **Phase 2: Make it Feel Right (Medium Priority)**

6. **S03 §3** - the equalizer shortcut table (it does not exist yet)
7. **S03 §4** - the global shortcut table beyond the six defaults
8. **The EQ band centres** - if a measurement ever establishes them, the
   current own choice is replaced by it
9. **The library's columns, views and sort semantics** - the three open
   S20 questions
10. **The embed window** - once S11's rectangles are measured

### **Phase 3: Plugin Compatibility (M3)**

11. **Plugin Hosting**
    - Run the `vis_`, `in_`, `out_` and `dsp_` acceptance fixtures against the
      **real** plug-in DLLs (VM only)
    - `out_disk` equivalent
    - The embed (general-purpose) window
12. **External Control Window**
    - The S10 catalog, once it is ratified
    - Test with real external tools

### **Phase 4: Modern Features (M4)**

13. **ICY metadata** for the stream display
14. **Native MilkDrop** - the butterchurn port, the preset browser
15. **The remaining codecs** (game-music-emu, sidplayfp, WavPack, APE, MPC)

---

## Testing & Verification Gaps

**Never Tested:**
- ❌ Packaged release run in the VM (the package exists; the smoke run and the
  clean-machine launch are **blocked** - `docs/tasks/RELEASE-checklist-run.md`)
- ❌ The ignored (window) test suite - blocked on the VM's interactive session
- ❌ The plugin acceptance tests against real plug-in DLLs (VM only)
- ❌ Clean machine test (no dev runtimes) - no bare OS baseline

**Partially Tested:**
- ⚠️ Clutter O/V popups - code works, never manually tested in the VM GUI
- ⚠️ Plugin UI pages - code works, never manually tested in the VM GUI
- ⚠️ The folder picker, the prefs pages, window shaping, skin cursors, the
  exclusive output and the device-loss test - all still need the VM
- ⚠️ Load Playlist Enter - wired, never VM-verified

**Closed:**
- ✅ The W14 wave review (`docs/tasks/W14-wave-review.md`): every finding that
  described a **defect** is fixed (`F1`-`F7`, `C4-2`, `C4-3`, `C5-1`, `C5-2`,
  `C6-1`, `C6-2`, `F6`). What remains is `C1-2`/`C1-3`, which are **false by
  design** - the invariant lives in the caller, not in the framework - and
  `C6-3`, a dangling card reference.

---

## Decisions Needed

1. **URL support:** which schemes? `http` only (the decoder refuses `https`
   by name - there is no TLS in this tree), or `https` too?
2. **Plugin Approval:** auto-approve signed plugins? Manual only?
3. **Installer:** NSIS or MSI? (the portable ZIP exists; code signing is
   decided **no** for 0.1.0)
4. **Git Remote:** where to host? GitHub/GitLab/self-hosted?
5. **Crossfade:** do we want it as **our own** feature? It is not a parity
   item (S18 §7 item 9), so it is a product decision, not a conformance one.

**Decided and closed:**
- File picker: the native Windows dialogs (no egui dialog)
- Drag-and-drop: an Explorer drop **appends** to the playlist; a dropped
  playlist file is resolved into its entries (own choice). A dropped
  **folder** is added as a plain path - the recursive scan belongs to
  "Open directory" (`crates/rustamp-app/src/directory.rs`), not to the drop
- Add directory: recursive (an own choice)

---

## Estimated Effort (Rough)

| Phase | Items | Effort |
|-------|-------|--------|
| Phase 1: Usable | 5 items | ~2-3 weeks |
| Phase 2: Feel Right | 5 items | ~2-4 weeks |
| Phase 3: Plugins | 2 items | ~3-5 weeks |
| Phase 4: Modern | 3 items | ~4-6 weeks |
| **Total** | **15 major items** | **~11-18 weeks** |

This is the **original estimate from 2026-10-19** and it is **stale**: five of
the fifteen Phase 1/2 items are already done (the native dialogs, the
drag-and-drop, the playlist operations, the menu actions, the playlist
keyboard). Re-estimate before using it for planning.

This assumes:
- No major architecture changes
- Parallel work on independent items
- Testing in the VM as we go

---

## Next Immediate Actions

1. ✅ **Fix Title Bar Window-Move** - DONE (2026-10-19)
2. ✅ **Native file picker** - DONE (`rustamp-platform::file_dialog`)
3. ✅ **Drag-and-Drop** - DONE
4. ✅ **Playlist Remove/Clear** - DONE
5. ⏭️ **Wire the HTTP decoder** (the one missing branch) and turn the S14
   engine on behind a switch
6. ⏭️ **Test the packaged release in the VM** (the critical verification gap)

---

## Owner Decisions Needed Now

**A. URL support:**
- `http` only for 0.1.0, or `https` as well?

**B. Crossfade:**
- Do we implement it as our own feature? (It is not a parity item.)

**C. Priority:**
- Should we close the verification gap (the VM runs) or keep filling the
  feature list?

**D. Release Timeline:**
- Target date for 0.1.0?
- Should we ship what is there, or wait for the S16 measurement?
