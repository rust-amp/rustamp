# Handoff: Rustamp project takeover

Status as of **2026-10-15**, after the second session (OpenCode orchestrator): lanes A (audio engine), C (skin/gfx), D (main window and its behaviour), E (playlist I/O), G (shared-memory rings) and L (licensing split) are integrated on `main`; S01, S02, S03 and S17 are audited. This document is everything a new agent needs to continue the work. Read it completely before you do anything.

---

## 1. What this project is

**Rustamp** is a new media player that looks, feels and behaves like **Winamp 2.95** on Windows, built in **Rust** on modern technology.

- **Windows first.** Linux and macOS come later (milestone M6). Classic plugins will run there under Wine.
- **The player is 64-bit only** (`rustamp.exe`, x86_64). Only the separate **plugin host `rustamp-plughost32.exe` is 32-bit** (i686), so it can load classic 32-bit Winamp 2.x plugins: `in_`, `out_`, `vis_`, `dsp_` and `gen_`. MilkDrop 2, AVS and Geiss have priority.
- **License: split** (RFC 0004). The player and most crates are GPL-3.0-or-later with a §7 plugin exception; `rustamp-proto`, `rustamp-plugin-abi`, `rustamp-ipc` and `rustamp-plughost` are MIT OR Apache-2.0. See `THIRD_PARTY.md` and `README.md`.
- **Skins.**
  - The installer bundles a newly drawn **look-alike default skin**: classic layout and colors, but no copied pixels.
  - The **original Winamp base skin** (from webamp `base-2.91`) is offered only as a **separate optional download**.
  - **Tests run with both skins.**
- The full architecture and milestone plan is in **[docs/PLAN.md](PLAN.md)**, the binding plan approved by the user. Read it.

### About the user
- They are the project owner and communicate in **German**. Reply in German; code, comments and docs are in English.
- **Run at most two sub-agents in parallel** on this machine. Pick models by task complexity: stronger models for architecture and complex code, cheaper ones for simple research and boilerplate.
- **Ask before every download** and before installing or running Winamp or plugin binaries.
- Token and usage quotas are tight. When the user says stop, finish the running tasks and stop.
- Commit only when asked. The repo-local git identity is `Rustamp <rustamp@project>`. Personal names or e-mail addresses must never appear in the repo or in commit messages.

---

## 2. Clean-room process (legally critical; read this before you touch anything)

The binding rules are in **[CLEANROOM.md](../CLEANROOM.md)**. In short:

| Source | Rule |
|---|---|
| Winamp 5.x source (github.com/alexfreud/winamp), Winamp SDK headers, disassembly of the binaries | **Dirty room only.** Never copy from them, never read them as an implementer |
| Winamp 2.95 binary and plugin DLLs | Dirty room only: reverse engineering for interoperability, run and observe |
| foobar2000 | Behavior and architecture analysis only |
| WACUP changelog (https://getwacup.com/changelog/) | Inspiration only, no code, assets or verbatim text |
| DeaDBeeF, webamp (MIT), butterchurn (MIT), WDL/EEL2 (zlib), libopenmpt (BSD) | Copying allowed, with attribution in [THIRD_PARTY.md](../THIRD_PARTY.md) and the license headers kept |
| MilkDrop 2.25c source (the `vis_milk2/` files with the Nullsoft BSD-3 header) | Copying allowed (BSD-3). **Exception:** the Winamp SDK headers bundled in the zip (`Winamp/`, `Wasabi/`, `Agave/`, `vis.h`, `api.h`) are dirty |

**Workflow:**
1. A dirty-room agent reads the source or analyzes the binary and writes a **spec** in its own words: prose and tables, no code, no transliterated pseudo-code. The spec lands in `D:\code\rustamp-research\drafts\`.
2. A **Spec Auditor** agent (also dirty) checks it for copied wording or code. Only after a PASS is the spec exported to `specs/S-xx-*.md` in this repo, with a provenance header.
3. **Clean-room agents** implement only from `specs/` and the free sources.

**What this means for you as the orchestrator:**
- **You must stay clean yourself.** Never open `D:\code\rustamp-research`, the Winamp source, SDK headers or disassembly, and do not paste their content into prompts. Dirty-room work always goes to **separate agents or sessions**, which report back to you **metadata only** (file names, status, open questions).
- Every implementation prompt states explicitly: forbidden sources, allowed sources, which specs to use, which paths the agent owns, and the done criteria.
- Behavior of the real **2.95 binary takes precedence** over the 5.x source.

---

## 3. Locations

| Path | Contents | Clean? |
|---|---|---|
| `D:\code\Rustamp` | This repo (clean, GPL). Branch `main` | clean |
| `D:\code\rustamp-research` | Private dirty-room repo: `drafts/` (S09 draft), `notes/S10-seed.md` (first WM_WA_IPC/WM_COMMAND list), `inventory/binaries.md`, `audits/S09-audit.md`, `tools/` (Python PE and NSIS parsers), `extracted/` (unpacked installers, Winamp 5.x source; git-ignored). Not committed | **dirty, never read it yourself** |
| `D:\code\rustamp-refbin` | Downloaded reference binaries plus `MANIFEST.md` with SHA-256 hashes: Winamp 2.95 (stock, md5/sha1 match archive.org), 2.95 full noaod, Winamp 5.66 full, Geiss 4.29, Geiss 2, MilkDrop 2.25c source zip, libopenmpt 0.8.9 (with in_openmpt.dll). Winamp 2.95 was installed by the dirty room into a sandbox for reference captures; the clean room never runs these files | dirty (binaries) |
| `C:\Tools\ghidra_12.1.4_PUBLIC` | Ghidra (for dirty agents) | |

Tools available: Rust 1.99 stable with the targets `x86_64-pc-windows-msvc` and `i686-pc-windows-msvc`, git, gh, ffmpeg, 7-Zip, Python 3.14, VS 2022 (dumpbin, not on PATH).

---

## 4. What exists (M0 complete, M1 well started)

Workspace: 22 members (21 crates in `crates/` plus `xtask`), edition 2024, resolver 3. **405 tests green** (`cargo test --workspace --all-features`); fmt and clippy (`-D warnings`) are clean, the i686 tests of `rustamp-ipc` and `rustamp-plugin-abi` pass, `cargo xtask ring-cross` passes all four combinations, and the i686 build of the plugin host works. The workspace declares `rust-version = "1.99"` (see `CONTRIBUTING.md`).

**Licensing is split** (RFC 0004, see the License section of `README.md`): `rustamp-proto`, `rustamp-plugin-abi`, `rustamp-ipc` and `rustamp-plughost` are **MIT OR Apache-2.0**; every other crate is **GPL-3.0-or-later** and carries the GPL-3.0 §7 additional permission from `PLUGIN-EXCEPTION.md`. The dependency closure of the four permissive crates contains no GPL-licensed crate — that invariant must hold whenever a dependency is added there.

| Crate | Status |
|---|---|
| `rustamp-core` | **Contracts frozen** (see [RFC 0001](rfc/0001-core-contracts.md)): `Command`, `Event`, `PlayerState`, `EqState`, `WindowId`, `Decoder`/`DecoderFactory`/`DecoderRegistry`, a pull-based `OutputDevice` (`open` → `start(RenderFn)`), `DspStage`, `VisFrame` (2×576 waveform and spectrum), the concrete `Playlist` with stable `EntryId`s, `TrackMetadata`, `Config` (serde/TOML), the bus (`command_channel`, `EventBus`), and the `mock` feature (MockDecoder, NullOutput, MockDsp). Units: ms as `u64`, volume `u8` 0–255, balance `i8`, EQ in dB as `f32`. **Contract changes only through a new RFC in `docs/rfc/`.** [RFC 0002](rfc/0002-audio-contracts.md) (accepted) added `Decoder::skipped_packets()`, `ReadSeek: Send + Sync` and `Error::FormatChanged` + `ErrorKind::FormatChanged`. `Playlist::apply_order`/`insert_many` are linear now (internal) |
| `rustamp-proto` | **MIT OR Apache-2.0.** The types that cross the process boundary, extracted from `rustamp-core` per RFC 0004: the visualization types and constants, the wire subset of `Command`/`Event`, `PlayerState` and its parts, and the playlist/window identifiers the protocol uses. No internal dependencies and no GPL dependency; `rustamp-core` re-exports everything at the old paths, so no caller changed |
| `rustamp-ipc` | **MIT OR Apache-2.0.** Protocol **v1** (`Message`, postcard; `RingOffer`/`RingReady`/`RingRefused`), `Transport` trait, `MemoryTransport`, `StreamTransport`, u32-LE length framing (16 MiB cap), and **shared-memory SPSC rings** for PCM and vis data per [RFC 0003](rfc/0003-shm-rings.md): byte-identical layout on x86/x86_64, lock-free, allocation-free, generation-guarded. Depends on `rustamp-proto`, not on `rustamp-core`. RFC 0003's open questions Q1–Q5 are follow-ups |
| `rustamp-plugin-abi` | **Done, per S09**: `#[repr(C)]` types `InModule`/`InModuleExt`, `OutModule`, `VisHeader`/`VisModule`, `DspHeader`/`DspHeader5x`/`DspModule`, `GeneralPlugin`, constants and export names. 116 layout asserts (x86). Open points are in `crates/rustamp-plugin-abi/OPEN_QUESTIONS.md` |
| `rustamp-decoders` | **Done**: symphonia 0.5.5 for MP3, FLAC, Vorbis, WAV, AIFF, CAF, AAC and ALAC (MP4), and MKA. Gapless, sample-accurate seeking (4096-frame pre-roll), skips corrupt packets, metadata, `default_registry()`, 25 tests. **No Opus** (needs libopus via FFI). Contract proposals are in `crates/rustamp-decoders/CONTRACT_NOTES.md` |
| `rustamp-audio` | **Engine (M1)**: decode thread → `rtrb` ring → output callback with no allocation or lock on the render path, transport from `Command`, events on the bus, gapless, frame-accurate seek, volume/balance gain, format conversion, raw vis tap. CLI: `cargo run -p rustamp-audio --example play -- <file>` (`--null`, `--output`, `--volume`). Resampler is linear interpolation; the EQ/DSP chain and `rubato` come with S07/M2 |
| `rustamp-output` | `NullOutput` with a driveable clock (test double), WASAPI **shared** mode, cpal fallback, `default_output()`. Exclusive mode, device-change recovery and `out_disk` come later |
| `rustamp-skin` | **Loader**: `.wsz`/`.zip`/folder, BMP 1/4/8/16/24/32 bpp incl. `BI_RLE8`, `pledit.txt`, `viscolor.txt`, `region.txt`, cursors, per-file fallback with a substitution report, sprite table. Sprite rects and default colours are ported from webamp (MIT) and marked provisional until S01/S17. `CONTRACT_NOTES.md` in the crate |
| `rustamp-gfx` | RGBA canvas with clipped blits, integer 2× scaling, `text.bmp` and `numbers`/`nums_ex` bitmap fonts |
| `rustamp-playlist` | **I/O**: M3U/M3U8/PLS/XSPF read and write, content sniffing, relative-path resolution, Windows-1252 fallback, seeded `randomize`/`shuffle` helpers. B4S unsupported (no public format description available); the shuffle sequence (S15) and title formatting (S14) are open. `CONTRACT_NOTES.md` in the crate |
| `rustamp-meta` | **Tags (M1)**: reads and writes tags through `lofty` 0.25 onto `TrackMetadata` with a fixed precedence (primary tag, then ID3v2, Vorbis comments, MP4 ilst, APE, ID3v1, RIFF info, AIFF text), the track/disc pair forms, best-attempt parsing so malformed tag bytes never panic, artwork as a raw-bytes side channel, and a batch scan returning one result per path. Properties are deliberately not scanned, so `duration_ms`/`bitrate_kbps` stay `None` for the decoder to fill. Winamp title formatting is deferred to S14. `CONTRACT_NOTES.md` in the crate |
| `rustamp-ui` | **Main window (M1)**: `MainWindowLayout` with every S01 rectangle in one place, a plain `MainWindowState`, a renderer that paints a frame through `rustamp-gfx`, `hit_test` over the interactive widgets, and a `winit` + `softbuffer` presenter with integer 2× double-size. On top of that, a **headless `MainWindow` behaviour machine** from the audited S02: it consumes `Command`/`Event`s, takes time as a parameter, and emits `Command`s for the transport buttons, the sliders (clamped drags), the five clutterbar actions, the toggles and the time-display click, with pressed sprites, momentary lit-while-held, the eight exact hint strings and blank readouts when stopped. Example: `cargo run -p rustamp-ui --example main_window -- <skin>`. **Main-window keyboard (S03 §1, task D3):** `shortcuts.rs` holds the 62-row main table and the 29-row global table as data, a `Key`/`Modifiers` model free of `winit`, `lookup`, and `MainWindow::key_down` returning `Commands`, `Handled` or `Unhandled(action)`; the `winit` key mapping lives in the presenter. Menus (S04), docking (S05) and the other windows are not implemented |
| `rustamp-testkit` | PNG load and save, `compare`/`Diff`, `write_diff_image`, the `Snapshot` trait, `assert_golden` (bless via `RUSTAMP_BLESS=1`). The `lookalike` skin namespace is in the repo; `classic-base` comes from `$RUSTAMP_TESTDATA` and is skipped when that variable is unset |
| `xtask` | `cargo xtask golden [--bless] [--skin]`, `cargo xtask capture --list / --class / --title / --out / --client-only` (generic Win32 window capture), `cargo xtask ring-cross` (32-bit vs 64-bit shared-memory ring check), `cargo xtask plughost [--release]`; `package` is still a stub |
| everything else | Stubs (a `lib.rs` with doc comments only): dsp, vis, milkdrop, platform, prefs, library; app and plughost are `main` stubs |

**Docs:**
- `specs/S01-window-geometry.md`, `S02-main-window-behaviour.md`, `S03-shortcuts.md`, `S09-plugin-abi.md`, `S17-skin-loading-quirks.md`: audited. S01 round 2 (equalizer interior, playlist upper half) is drafted but not exported. The rest (S04–S08, S10–S16, S18–S20) are not started; see `specs/README.md`.
- `docs/notes/L-licensing-plugins.md`: licensing research (proprietary and GPL-incompatible plugins, options, consequences per source, recommendation). Not legal advice.
- `docs/notes/SDK-licensing.md`: reading of the Winamp plugin SDK's own terms. Result: nothing in the SDK restricts a third-party host — no licence agreement, no registration, no restriction to Winamp, no ban on third-party hosts, no field-of-use limit, and commercial use is expressly allowed. The one header that carries a grant (`wa_ipc.h`) is zlib-style; the five interface headers carry **no grant at all**, so they must not be copied or redistributed — which the clean-room rules already forbid, so implementing the ABI from spec facts is also the licensing-correct route. The 2024 source release put the SDK into a repository under the restrictive Winamp Collaborative License. Not legal advice.
- `docs/notes/W-wacup-changelog.md`: WACUP digest. A sample of only 9 out of ~300 entries.
- `docs/rfc/0002-audio-contracts.md` and `docs/rfc/0003-shm-rings.md`: accepted.
- `docs/tasks/`: the task cards of waves 1 and 2 (`A1`, `C1`, `E1`, `G1`), each with goal, owned paths, allowed and forbidden sources, spec dependencies, contract and acceptance tests.

---

## 5. Open points

1. **RFC 0002 is done and accepted.** Follow-up for the decoders lane: drop the now-redundant `Mutex` in the decoder's reader wrapper (the `Sync` bound makes it unnecessary) and fix the wording in its `CONTRACT_NOTES.md`.
2. **Contract questions raised by the lanes.**
   - `rustamp-playlist` (`CONTRACT_NOTES.md`): `PlaylistEntry` has no field for `#EXTART`/`#EXTALBUM`/XSPF `<creator>` — decide together with S14 whether to add fields or an `extra` map; confirm that non-`http` schemes become `MediaLocation::Url`.
   - `rustamp-ipc` (RFC 0003, Q1–Q5): a blocking consumer (event or semaphore), a private DACL or `Global\`-scoped name, folding `RingError` into `IpcError`, a late-joiner consumer that starts at the newest frame, and a compact header.
   - **Verification gap closed:** `cargo xtask ring-cross` runs a 32-bit producer against a 64-bit consumer and the reverse, for both ring kinds. Remaining RFC 0003 open questions: Q1–Q5.
   - `rustamp-meta` (`CONTRACT_NOTES.md`): add `disc_total: Option<u32>` to `TrackMetadata` (today it goes into `extra["DISCTOTAL"]`), and decide a contract for artwork — a side-channel type rather than bytes inside `TrackMetadata`, which would be heavy to clone.
   - `rustamp-audio`: if the engine must expose playlist `EntryId`s (`TrackChanged.id` is `None` today), that is a public-surface change and needs an RFC.
3. **Unicode paths over IPC.** A `PathBuf` that is not valid UTF-8 cannot be encoded yet (the fix would be WTF-8 or u16).
4. **`WindowId::Gen(u32)`** still has no numbering scheme.
5. **Value ranges.** The EQ (±12 dB), volume and balance ranges are provisional until S02 and S07 exist. The engine's provisional volume/balance laws live in `rustamp-audio/src/gain.rs`, the vis derivation is deliberately left at "raw last 576 samples, spectrum zero" until S08.
6. **S09 open items**, to be checked at runtime against the real 2.95:
   - The vis-plugin thread model.
   - The sign and scaling of the 576-sample data.
   - The order of DSP and EQ.
   - Which callbacks may be null.
   - Version acceptance.
   - The full WM_COMMAND ID table.
7. **Spec requests collected from the lanes:** S06 (exact Sort/Randomize menu entries, sort stability, remembering the loaded format), S14 (is `PlaylistEntry::title` the formatted or the raw title; which fields feed `%1`–`%7`), S15 (when is the shuffle sequence built and invalidated; does the current entry stay current; where does the seed come from), S08 (waveform/spectrum derivation, publish cadence, `timestamp_us`), S02/S07 (volume slider law, balance law), S18 (fades/prebuffering), and a public description of **B4S**.
8. **Open rendering questions the UI lane exposed** (all S01/S02 gaps, not bugs): the `nums_ex.bmp` cell layout is still unmeasured (S01 open question 6, S17 §3) so `nums_ex` skins draw at the `numbers.bmp` positions; the `text.bmp` opacity rule (colour key vs. opaque blit) is ambiguous in S01; the shade internals and the marquee/kbps/kHz field widths are webamp-provisional; and the rendering rules for the leading "no digit" cell, the decoder-active condition and the volume/balance frame mapping are provisional guesses that S02 must settle.
9. **S01/S02 contradictions to reconcile** (found while auditing S02/S03; the next dirty-room run must fix them, not guess):
   - S01 says the small window-button cells in the title-bar sheet are unused, but S02's pressed state **does** draw them. Resolved in the UI lane: the normal state blits the title-bar body, the pressed state blits the small cells.
   - S01 labels the equalizer slider thumb position as the 0 dB setting; it is actually the top of the travel.
   - S01 has three provisional playlist rows that do not match the render (the title-bar strip and the bottom-bar button rectangles).
   - A title-format example in the S02 draft used characters the bitmap font cannot render.
   - **Where the four window buttons sit** is in neither S01 nor S02 — the UI lane took the positions from the free webamp CSS and marks them provisional. S01/S02 must settle them.
10. **DPI and window sizing (lane F / S05).** The example presenter is DPI-unaware, so on the sandbox machine (system DPI 144) its client area is 183×77 and the 275×116 layout is blitted into a smaller area; double-size resizes the softbuffer surface but not the OS window. Neither is a layout bug, but both must be fixed when the platform lane makes the process per-monitor DPI aware and owns window sizing.
11. **Process note (kept deliberately):** the first dirty-room capture session claimed it used no synthetic input, which was false — real desktop clicks had driven several states. The audit corrected the provenance and re-established every click-derived claim. Capture manifests must now record the input method, the DPI awareness and the coordinate space; the sandbox machine runs at system DPI 144, so a DPI-unaware process sees the main window as 183×77 instead of 275×116.
12. **Still open from S01 round 2** (drafted, not exported): the equalizer slider-track decomposition and body colouring, the dragged thumb sprite, the On/Auto held sprites, two graph line strips, the playlist tile pitch and boundaries, the playlist scrollbar arrows and travel, the playlist bottom-bar hit areas and pressed state, end-of-track behaviour, whether the clutterbar can be hidden, whether skins can change the hint strings, how the four shortcut tables combine, modifier-click gestures, and playlist drag-reorder.
13. **MilkDrop 2 needs a 5.x host.** It requires several 5.x IPC calls and the HWND entry point, so the plugin host must emulate a partial 5.x IPC surface (relevant for S10).
14. **AVS 2.81d** has not been found as a separate installer; AVS ships with Winamp 5.66. **MilkDrop 1.04** returns a 404. **in_vgm, in_sid, a DSP plugin and a scrobbler gen plugin** have not been found yet.
15. CI (`.github/workflows/ci.yml`) has not run yet: there is no GitHub remote. An optional CI check that the four permissive crates stay GPL-free does not exist yet.
16. **sidplayfp is not linked.** `rustamp-decoders` only wraps symphonia today. The containment design exists as **RFC 0006 (draft, `docs/rfc/0006-out-of-process-hosting.md`)**: copyleft decoders (sidplayfp first) and GPL/unstable DSP run in a GPL helper (`rustamp-dechost`/`rustamp-dsphost`), PCM over RFC 0003 rings, control over new v2 `Message`s. Implementation is blocked on RFC 0003 Q1 (lossless delivery — drops are unacceptable for decode/DSP paths) and on HANDOFF-3 for non-UTF-8 paths; no PCM crosses processes under a drop policy before that.
17. **Merged worktrees.** Nine detached worktrees under `C:\Users\micro\.local\share\opencode\worktree\26de0e\` (`wave1-audio` to `wave7-ui-behaviour`) are fully merged into `main` and clean; each carries its own `target/` (several GB). Ask the user before removing them.
18. **Shortcut findings (task D3).**
    - About 30 S03 actions have no `Command` and come back as `Unhandled(action)` with their command id: open files/location/directory, jump to file/time, ten tracks back/forward, start/end of list, stop after current, skin browser, refresh skin, minimize, new instance, easy move, file info, bookmarks, the visualization plug-in actions, minibrowser, video, about, playlist search, close focused window, next window, show main window, and the open-main-menu key. They need an **RFC 0005** (new `Command` variants); `LoadSkin` needs a path and `OpenPrefs` cannot carry a page.
    - The volume key step is a placeholder (`VOLUME_STEP = 10` of 255): neither S02 nor S03 states it, so the dirty room must measure it.
    - The combination of the window table and the global table is provisional (S03 section 6.1): the window table is checked first, so `Ctrl+L` means "open location".
    - Key repeat delivers the action again (an implementation choice, not an S03 fact).
    - The live `winit` key path (`map_key`, AltGr, letter versus keypad) is untested because no window was opened; only the table, the dispatch and the code mapping are covered by tests.
    - The playlist (S03 section 2) and equalizer (S03 section 3) tables are not filled because those windows do not exist.

---

## 6. Next steps (prioritized, at most two agents in parallel)

Each task is a task card (template: `.github/ISSUE_TEMPLATE/task-card.md`): goal, lane, owned paths, allowed and forbidden sources, specs, contract, acceptance tests. Two parallel agents must have **disjoint owned paths**, and the root `Cargo.toml` belongs to one agent at most.

**Status of the dirty room:** S01, S02, S03, S09 and S17 are audited and exported. **S01 round 2 was audited on 2026-10-15 with verdict FAIL — do not export** (`D:\code\rustamp-research\audits\S01-round2-audit.md`): one section failed (sliders/thumbs: thumb-identity conflict with audited S01, column-origin delta, saved-value provenance), the rest passed with corrections. Resolved by the draft: the EQ 0 dB substance and the playlist rows as refutation (both blocked in form/wording); already settled by S02 with no regression: window-button cells, title-format example. **Not settled: the four window-button positions.** Missing captures: full track tiling + body colouring, dragged thumb, On/Auto held sprites, graph line strips, scrollbar arrows/travel, bottom-bar hit areas + sprite use, tile-pitch method. Next run: offline measurement from existing crops first, then a guarded-input session only for what crops cannot isolate. S07 and the other specs are not started. The S03 main-window shortcuts (card `docs/tasks/D3-shortcuts.md`) are integrated. Candidate next lanes: RFC 0005 for the missing `Command`s, platform/DPI (lane F), the S01 round 2 follow-up captures, a dirty-room measurement of the volume-key step.

**Wave 1 — DONE and integrated on `main`:**
1. **Lane A, audio engine** (`rustamp-audio`, `rustamp-output`, RFC 0002 in core): decode thread, `rtrb` ring, output callback, transport, gapless, WASAPI shared, cpal fallback, `NullOutput`, volume/balance, raw vis tap, CLI example. **Integrated as `c73d33d`; RFC 0002 accepted.**
2. **Lane C, skin parser** (`rustamp-skin`, `rustamp-gfx`): `.wsz`/`.zip`/folder loading, BMP at every bit depth, `pledit.txt`/`viscolor.txt`/`region.txt`, cursors, per-file fallback, webamp sprite tables with attribution, compositor with blits, integer scaling and bitmap fonts. **Integrated as `cee2782`; the sprite tables stay provisional until S01/S17.**

**Wave 2:**
3. **Lane E, playlist** (`rustamp-playlist`): **I/O done** — M3U, M3U8, PLS and XSPF, sort and randomize. **Integrated as `3178813`.** Still open: B4S (needs a public format description), the shuffle sequence (S15) and title formatting. `rustamp-meta`: tags via `lofty` onto `TrackMetadata`, integrated as `6dc3ebf`; title formatting waits for S14.
4. **Lane G, shared-memory rings** (`rustamp-ipc`): PCM and vis rings, RFC 0003. **Integrated as `881b776`; RFC 0003 accepted.**
5. **Dirty room:** S01, S17, S02 and S03 are audited and exported. **S01 round 2 is drafted** (equalizer interior, playlist upper half) and needs its own audit; it also surfaced the S01 contradictions listed in open point 9, which the same run must reconcile.
6. **Licensing** (`docs/notes/L-licensing-plugins.md`, RFC 0004): **integrated as `4bb917d`.** The plugin-facing crates are MIT OR Apache-2.0 and `rustamp-proto` holds the wire types; the player stays GPL-3.0-or-later with a §7 plugin exception; the per-crate exception reference and the RFC's errata are in the same change. The Winamp SDK's own terms are read in `docs/notes/SDK-licensing.md` — nothing in them restricts a third-party host, and the five plugin interface headers carry no grant, so they must not be copied or redistributed. Remaining: the plugin-author-facing document, the `cargo xtask package` split of the two binaries, and an optional CI check that the permissive crates stay GPL-free.

**After that (M1/M2):**
- Lane D, UI windows with `winit` + `softbuffer`. Order: main window, EQ, playlist, then docking/snapping, windowshade and double-size. This needs S01–S06.
- Lane J, the look-alike default skin (art).
- Lane F, platform: single instance, tray, global hotkeys, SMTC, and a `Winamp v1.x`-compatible window for external tools.
- **M3: plugin host.** S10 and S11 come first, then load vis plugins in plughost32 (fake `Winamp v1.x` window, embed frames using `gen.bmp`), then in/dsp/gen/out plugins.
- **M4: native MilkDrop.** Port butterchurn and/or the BSD-licensed MilkDrop 2.25c `vis_milk2` code to `wgpu`. Shader path: HLSL → SPIR-V (shaderc) → WGSL (naga).

---

## 7. Verification (run after every agent result)

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo test -p rustamp-plugin-abi --target i686-pc-windows-msvc
cargo test -p rustamp-ipc --target i686-pc-windows-msvc
cargo build -p rustamp-plughost --target i686-pc-windows-msvc
cargo xtask ring-cross
cargo xtask golden
```

Agents verify their own results, but always re-run these checks yourself. Do not trust an agent's report blindly.
