# Task card W18 — Release documentation reconciliation (tree vs shipped vs release plan)

**Date:** 2026-10-18. **Status:** Completed. Documentation-only task.
No Rust source files were modified, no Rust gates were executed (`cargo build`,
`cargo clippy`, `cargo fmt`, `cargo test`), and no git commit or git write was made.

---

## 1. Context and Objective

The Rustamp project (a clean-room Winamp 2.95 reimplementation in Rust) has
progressed across several recent implementation waves, landing significant
functional improvements across the audio engine, UI presenter, platform integration,
licensing, and packaging verification tooling.

However, key release documents still contained stale statements describing features
as skeletons, placebos, or unverified (e.g., spectrum analyzer contracted to zero,
shuffle/repeat as unconnected toggles, global hotkeys and tray icon as inert/model-only,
and `artifact_imports_verified: false` due to missing PE import tooling).

This task card documents the reconciliation of the release documents against the
actual codebase at HEAD (`1912b98`), ensuring every statement is truthful while
strictly preserving the honest boundaries between what is built/functional and what
remains unproven.

---

## 2. Owned Paths and Working Boundary

Per instructions, work was restricted strictly to the following documentation files:
- `docs/tasks/RELEASE-notes-template.md`
- `docs/tasks/BETA-what-is-in-it.md`
- `docs/tasks/BETA-readiness-2.md`
- `docs/tasks/RELEASE-plan.md`
- `docs/tasks/W18-release-docs.md` (this card)

No Rust source file (`.rs`), manifest, or project configuration was modified.
No other card, runner script, or workspace directory was touched.

---

## 3. What Landed and Was Reconciled

The following concrete implementations from recent commits were incorporated into
the release documentation:

### 3a. Built-in Visualiser (`feea30a`, `f7b09ea`, `W17-vis-clear`)
- **Implemented:** Wired into the interface pump in `crates/rustamp-app` directly
  from decoder source samples without allocation. Oscilloscope and 19-band spectrum
  analyzer are active during playback.
- **Immediate Clear on Stop:** Fixed in `f7b09ea`: the moment transport transitions
  to `TransportState::Stopped`, the visualizer resets immediately to static baseline
  in all modes (`Spectrum`, `Scope`, `ScopeDots`), zeroing bars, peaks, targets,
  and centering the scope, leaving no decaying tail.
- **Honest Boundary:** Visualizer response curves and decay ballistics are internal
  design choices, not measured against reference Winamp 2.95. Winamp visualization
  plugins (`vis_*.dll`) are not hosted.

### 3b. Shuffle and Repeat (`ade10e6`, `6a606ef`, `702719c`)
- **Implemented:** Full engine-owned shuffle and repeat architecture. The audio
  engine owns the random seed, generates the shuffle permutation, and coordinates
  the traversal plan with the decode worker across thread boundaries.
- **Cycle Rules:** Replanning at cycle boundaries enforces anti-repeat rules (the
  first track of a new cycle differs from the last track of the previous cycle).
  Pressing `Prev` on the first track of a repeated cycle restores the previous
  cycle plan.
- **Cleanup:** Orphaned, unconnected duplicate state machine in the playlist crate
  was deleted in favor of single engine authority.

### 3c. System Tray and Global Hotkeys (`d2c8e5d`, `8f946a9`, `bc828a7`)
- **Seams Closed:** Both follow-up seams in `window.rs` were completed:
  1. `take_pending_hotkeys()` is drained in the window pump and routed to `app.submit(command)`.
  2. `TrayIcon` attaches to the main `HWND`, handles right-click native context menu,
     left-click and double-click window restore/minimize toggles, and re-registers the
     icon upon receiving the shell `TaskbarCreated` message (Explorer restarts). Missing
     icon degrades gracefully to a warning.

### 3d. Window Focus Commands (`b17aa81`, `1912b98`, `W18-window-focus`)
- **Implemented:** Keyboard window navigation commands are operational:
  - `Ctrl+Tab` (`FocusNextWindow`) cycles across open docked windows, skipping closed ones.
  - `Alt+W` (`ShowMainWindow`) shows, un-minimizes, and focuses the main window (including
    unhiding after being minimized to the tray).
- **Honest Boundary:** Focus cycle `[Main, Equalizer, Playlist]` is implemented with a
  pure order model. The focus sequence is documented as an internal design choice
  rather than an empirical measurement of Winamp 2.95.

### 3e. Release Packaging and Complete Licence Inventory (`fa1dc77`, `ff644db`, `X10`)
- **Portable Release Archive:** Generates deterministic `<release_id>.zip` and `.sha256`
  checksum files without installer, background service, or registry modification.
- **Full Licence Texts:** Player notices (`player/THIRD_PARTY.md`) ship full 373-line
  canonical SPDX text of MPL-2.0 (for Symphonia) and Apache-2.0 text, alongside GPL-3.0
  §6 written source offer, MSVC static CRT terms, and Webamp attribution. Host notices
  (`host/THIRD_PARTY.md`) maintain strict permissive isolation (35-package closure).

### 3f. Executable PE Import Inspection (`ff644db`, `W17-xtask-findings`)
- **Static Verification Tooling:** `xtask/src/pe_imports.rs` parses PE import and
  delay-load directories of all packaged binaries against a strict Windows system DLL
  allowlist, rejecting dynamic C runtime DLLs (`msvcp*.dll`, `vcruntime*.dll`, `ucrtbase.dll`).
- **Manifest Stamping:** Packaging tool executes PE import verification and honestly
  stamps `artifact_imports_verified: true` into release manifests.
- **Honest Boundary:** Offline static PE import inspection proves system DLL allowlist
  compliance only; it does not prove execution on a bare clean OS without developer runtimes.

---

## 4. What Was Deliberately Preserved as Unproven

In accordance with strict reporting and documentation integrity guidelines, the
following claims were explicitly preserved as **unproven / not supported**:

1. **Clean Machine Launch:** While static PE import verification passes and
   `artifact_imports_verified: true` is set, actual execution on a pristine
   Windows machine lacking developer tools and runtimes has not yet been executed
   in a clean VM checkpoint (Step 13).
2. **Physical Audio Hardware:** Validated in hypervisor virtual machines with null
   or software sinks; physical sound card latency, hardware device-loss recovery,
   and diverse driver compatibility are not certified.
3. **Winamp Plugins:** The 32-bit helper process (`rustamp-plughost32.exe`) and
   byte-level approval store exist, but user-facing UI configuration is not yet
   implemented. Real Winamp plugins are not proven running in the shipped player.
4. **Visualiser Fidelity:** Built-in oscilloscope and spectrum analyzer are functional
   and clear on stop, but frequency response curves and decay ballistics are internal
   design choices, not measured against reference Winamp. External Winamp visualization
   plugins are not hosted.
5. **Window Focus Order:** Pure focus cycle `[Main, Equalizer, Playlist]` is an
   intentional internal design choice rather than a measurement.
6. **Placebo / Skeletons Remaining:** Clutter bar O/V buttons emit no commands; menu
   submenus are deferred skeletons; the window menu Exit item closes only the popup;
   EQ presets popup and preferences pages remain skeletons; ReplayGain user settings
   UI is absent (engine setters exist, config key warned-unapplied).

---

## 5. Summary of Edits Made Across Documents

| Document | Key Changes Applied |
|---|---|
| `docs/tasks/RELEASE-notes-template.md` | In §5, moved Visualiser, Shuffle/Repeat, Tray/Hotkeys, and Focus commands to "Proven & Functional"; added static PE import verification. In §5 Unproven, clarified clean machine launch remains unexecuted, and updated placebos list. In §6, noted full texts of MPL-2.0 and Apache-2.0 in player notices. |
| `docs/tasks/BETA-what-is-in-it.md` | Updated header date/HEAD to `1912b98`. In §1, updated table of package contents (residents, licence files, zip, schema 3, import verification flag). In §3, moved Visualiser, Shuffle/Repeat, Tray/Hotkeys, Focus commands, and PE import check into "Reachable in tree". In §4, removed stale claims (vis zero, hotkeys without seams, shuffle/repeat ignored) while preserving unproven clean machine run, real audio hardware, and plugin UI. |
| `docs/tasks/BETA-readiness-2.md` | In Decision and §1 table, updated BETA-01 (`artifact_imports_verified: true` via static check), BETA-08 (FIXED in engine), BETA-10 (PARTIALLY FIXED: vis functional and clears on stop), BETA-11 (LARGELY FIXED: tray/hotkeys seams closed, focus commands added). In §2, updated 2f (tray/hotkeys closed), 2j (vis wired and clears), added 2l (focus commands), 2m (PE import check), 2n (licence texts). In §3 and §4, removed shuffle/repeat and vis from placebos list. |
| `docs/tasks/RELEASE-plan.md` | In G3 and Checklist Step 10, updated PE import inspection to completed status via `xtask/src/pe_imports.rs`, noting clean VM run (Step 13) remains unproven. In §5 "Never claim" table, updated clean machine and placebo rows. In §6, marked R3 as completed. |
| `docs/tasks/W18-release-docs.md` | Created this comprehensive task card. |

---

## 6. Verification and Host Safety Receipt

- **Files modified:** Markdown documentation files only (the 4 specified paths plus this card).
- **Rust source files:** Zero `.rs` files were modified, created, or deleted.
- **Rust gates:** Deliberately ran **no** `cargo build`, `cargo clippy`, `cargo fmt`,
  or `cargo test`. The host was kept completely unburdened and cheap to allow the
  orchestrator to run the full test suite in the virtual machine without resource contention.
- **Git operations:** No `git commit`, `git tag`, `git push`, or other git writes were performed.
