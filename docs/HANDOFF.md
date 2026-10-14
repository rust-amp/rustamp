# Handoff: Rustamp project takeover

Status as of **2026-10-14**, end of the first session (Claude Code orchestrator). This document is everything a new agent needs to continue the work. Read it completely before you do anything.

---

## 1. What this project is

**Rustamp** is a new media player that looks, feels and behaves like **Winamp 2.95** on Windows, built in **Rust** on modern technology.

- **Windows first.** Linux and macOS come later (milestone M6). Classic plugins will run there under Wine.
- **The player is 64-bit only** (`rustamp.exe`, x86_64). Only the separate **plugin host `rustamp-plughost32.exe` is 32-bit** (i686), so it can load classic 32-bit Winamp 2.x plugins: `in_`, `out_`, `vis_`, `dsp_` and `gen_`. MilkDrop 2, AVS and Geiss have priority.
- **License: GPL-3.0-or-later.**
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
| `D:\code\rustamp-refbin` | Downloaded reference binaries plus `MANIFEST.md` with SHA-256 hashes: Winamp 2.95 (stock, md5/sha1 match archive.org), 2.95 full noaod, Winamp 5.66 full, Geiss 4.29, Geiss 2, MilkDrop 2.25c source zip, libopenmpt 0.8.9 (with in_openmpt.dll). Never run anything; nothing installed | dirty (binaries) |
| `C:\Tools\ghidra_12.1.4_PUBLIC` | Ghidra (for dirty agents) | |

Tools available: Rust 1.99 stable with the targets `x86_64-pc-windows-msvc` and `i686-pc-windows-msvc`, git, gh, ffmpeg, 7-Zip, Python 3.14, VS 2022 (dumpbin, not on PATH).

---

## 4. What exists (milestone M0 complete, M1 started)

Workspace: 21 members (20 crates in `crates/` plus `xtask`), edition 2024, resolver 3. **55 tests green**; fmt and clippy (`-D warnings`) are clean, and the i686 build of the plugin host works.

| Crate | Status |
|---|---|
| `rustamp-core` | **Contracts frozen** (see [RFC 0001](rfc/0001-core-contracts.md)): `Command`, `Event`, `PlayerState`, `EqState`, `WindowId`, `Decoder`/`DecoderFactory`/`DecoderRegistry`, a pull-based `OutputDevice` (`open` → `start(RenderFn)`), `DspStage`, `VisFrame` (2×576 waveform and spectrum), the concrete `Playlist` with stable `EntryId`s, `TrackMetadata`, `Config` (serde/TOML), the bus (`command_channel`, `EventBus`), and the `mock` feature (MockDecoder, NullOutput, MockDsp). Units: ms as `u64`, volume `u8` 0–255, balance `i8`, EQ in dB as `f32`. **Contract changes only through a new RFC in `docs/rfc/`** |
| `rustamp-ipc` | Protocol v0 (`Message` enum, postcard), `Transport` trait, `MemoryTransport`, `StreamTransport`, u32-LE length framing (16 MiB cap). Shared-memory rings for PCM and vis data are still missing |
| `rustamp-plugin-abi` | **Done, per S09**: `#[repr(C)]` types `InModule`/`InModuleExt`, `OutModule`, `VisHeader`/`VisModule`, `DspHeader`/`DspHeader5x`/`DspModule`, `GeneralPlugin`, constants and export names. 116 layout asserts (x86). Open points are in `crates/rustamp-plugin-abi/OPEN_QUESTIONS.md` |
| `rustamp-decoders` | **Done**: symphonia 0.5.5 for MP3, FLAC, Vorbis, WAV, AIFF, CAF, AAC and ALAC (MP4), and MKA. Gapless, sample-accurate seeking (4096-frame pre-roll), skips corrupt packets, metadata, `default_registry()`, 25 tests. **No Opus** (needs libopus via FFI). Contract proposals are in `crates/rustamp-decoders/CONTRACT_NOTES.md` |
| `rustamp-testkit` | PNG load and save, `compare`/`Diff`, `write_diff_image`, the `Snapshot` trait, `assert_golden` (bless via `RUSTAMP_BLESS=1`). The `lookalike` skin namespace is in the repo; `classic-base` comes from `$RUSTAMP_TESTDATA` and is skipped when that variable is unset |
| `xtask` | `cargo xtask golden [--bless] [--skin]`, `cargo xtask capture --list / --class / --title / --out / --client-only` (generic Win32 window capture), `cargo xtask plughost [--release]`; `package` is still a stub |
| everything else | Stubs (a `lib.rs` with doc comments only): audio, dsp, output, meta, playlist, skin, gfx, ui, vis, milkdrop, platform, prefs, library; app and plughost are `main` stubs |

**Docs:**
- `specs/S09-plugin-abi.md`: audited, statically validated; runtime confirmation against 2.95 is still pending. The other specs (S01–S20) are not started; see `specs/README.md`.
- `docs/notes/W-wacup-changelog.md`: WACUP digest. A sample of only 9 out of ~300 entries.

---

## 5. Open points

1. **RFC 0002 (core contracts).** Bundle these three changes:
   - Add `Decoder::skipped_packets()`, defaulting to 0.
   - Make `ReadSeek` require `Send + Sync`. The decoder currently wraps its reader in a `Mutex`.
   - Add a signal for a format change mid-stream (an error variant or event).
2. **Unicode paths over IPC.** A `PathBuf` that is not valid UTF-8 cannot be encoded yet (the fix would be WTF-8 or u16).
3. **`WindowId::Gen(u32)`** still has no numbering scheme.
4. **Value ranges.** The EQ (±12 dB), volume and balance ranges are provisional until S02 and S07 exist.
5. **S09 open items**, to be checked at runtime against the real 2.95:
   - The vis-plugin thread model.
   - The sign and scaling of the 576-sample data.
   - The order of DSP and EQ.
   - Which callbacks may be null.
   - Version acceptance.
   - The full WM_COMMAND ID table.
6. **MilkDrop 2 needs a 5.x host.** It requires several 5.x IPC calls and the HWND entry point, so the plugin host must emulate a partial 5.x IPC surface (relevant for S10).
7. **AVS 2.81d** has not been found as a separate installer; AVS ships with Winamp 5.66. **MilkDrop 1.04** returns a 404. **in_vgm, in_sid, a DSP plugin and a scrobbler gen plugin** have not been found yet.
8. CI (`.github/workflows/ci.yml`) has not run yet: there is no GitHub remote.

---

## 6. Next steps (prioritized, at most two agents in parallel)

Each task is a task card (template: `.github/ISSUE_TEMPLATE/task-card.md`): goal, lane, owned paths, allowed and forbidden sources, specs, contract, acceptance tests. Two parallel agents must have **disjoint owned paths**, and the root `Cargo.toml` belongs to one agent at most.

**Wave 1 (recommended to start next):**
1. **Lane A, audio engine** (`rustamp-audio`, `rustamp-output`; also does RFC 0002 in core):
   - Decode thread, then an `rtrb` ring, then the output callback, with no allocations or locks in the real-time path.
   - Transport commands from `Command`, events onto the `EventBus`.
   - Gapless track transitions; position from `played_frames`.
   - WASAPI shared output (`wasapi` crate), with `cpal` as fallback; a `NullOutput` test.
   - Volume and balance; a vis tap that fills `VisFrame` with the raw form for now (the S08 derivation comes later).
   - Goal: a CLI example that plays a file.
2. **Lane C, skin parser** (`rustamp-skin`, `rustamp-gfx`):
   - Load .wsz, .zip or folders.
   - Read BMPs at every bit depth, case-insensitive file names, `pledit.txt`, `viscolor.txt`, `region.txt`, cursors.
   - Port the **sprite tables from webamp** (MIT, `skinSprites`/`skinParser`) with attribution in `THIRD_PARTY.md`.
   - Fall back to the default skin when files are missing.
   - Compositor with blit, integer scaling and bitmap fonts (`text.bmp`, `numbers`/`nums_ex`).
   - Test against a corpus of real skins (the webamp skin museum; ask the user before downloading).

**Wave 2:**
3. **Lane E, playlist** (`rustamp-playlist`): M3U, M3U8, PLS, B4S and XSPF I/O, sort and randomize. `rustamp-meta`: tags via `lofty` and title formatting (wait for S14).
4. **Dirty room, separate agent:** install Winamp 2.95 into a **sandbox folder** (**ask the user first**), drive it with `cargo xtask capture`, and capture reference screenshots of every window and state with the original skin. Then write **S01** (geometry), **S02** (main window behavior), **S03** (shortcuts) and **S07** (EQ: band frequencies, filter design, slider mapping; render sweeps through `out_disk`). Every spec then goes through the Spec Auditor before it is exported.

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
cargo build -p rustamp-plughost --target i686-pc-windows-msvc
cargo xtask golden
```

Agents verify their own results, but always re-run these checks yourself. Do not trust an agent's report blindly.
