# BETA — what is in the packaged beta (artefact vs tree)

**Date:** 2026-10-18. **Scope:** docs-only inventory of the packaged beta as it
stands, reconciled at HEAD (`1912b98`). No `.rs` file, test, runner script,
`docs/HANDOFF.md` or review card was edited; nothing was executed.

**The one distinction this card keeps:** the artefact bytes are frozen at the
build source commit, while the tree has moved on. The package in
`D:\code\rustamp-beta\2026-10-17` was built from `main` at `f1b87b8`
(`docs/tasks/BETA-build.md:3-4`); the tree has moved forward substantially
since, incorporating major functionality (visualiser, shuffle/repeat, tray/hotkeys,
focus commands, PE import inspection, full licence texts, schema 3 packaging).
So below, **"in the bytes"** means `f1b87b8` plus the packaging commit `e00d922`,
and **"in the tree"** means HEAD (`1912b98`). Anything in §3 is newer than the
bytes a user downloads.

## 1. What the package contains, and how it starts

Contents (in the tree package definition, `xtask/src/package.rs` via X2, X10, ff644db):

| Path | What |
|---|---|
| `player/rustamp.exe` | the player, x86_64, release, static CRT |
| `player/rustamp-dechost-symphonia.exe` | symphonia decoder resident helper, x86_64, release, static CRT |
| `player/rustamp-dsphost-stages.exe` | gain/passthrough DSP resident helper, x86_64, release, static CRT |
| `player/rustamp-plughost32.exe` | bundled copy of the 32-bit host beside the player (RFC 0006 §8) |
| `player/LICENSE`, `player/PLUGIN-EXCEPTION.md` | the GPL set |
| `player/THIRD_PARTY.md` | GPL-3.0 §6 source offer, static CRT terms, Webamp notice, full MPL-2.0 and Apache-2.0 texts, 302-crate inventory |
| `player/QUICKSTART.md` | user quickstart guide |
| `host/rustamp-plughost32.exe` | the host on its own, i686, permissive licences only, no GPL binary |
| `host/LICENSE-MIT`, `host/LICENSE-APACHE` | the permissive set |
| `host/THIRD_PARTY.md` | strictly permissive terms, static CRT terms, MIT/Apache texts, 35-package closure |
| `manifest.json`, `player/manifest.json`, `host/manifest.json` | layout, licences, schema version 3, version/commit stamped, `artifact_imports_verified: true` (post-pe-imports) |
| `README.md`, `QUICKSTART.md`, `THIRD_PARTY.md` | top-level documentation, quickstart, and root license inventory |
| `<release_id>.zip`, `<release_id>.zip.sha256` | deterministic portable ZIP archive and SHA-256 checksum file |

Runtime shape (`docs/tasks/X1-package-runtime.md:13-50`): both binaries select
the MSVC C runtime statically (`-C target-feature=+crt-static` in the package
command factory), so no VC++ redistributable or app-local CRT DLLs are needed;
Windows system DLLs still are, and the 32-bit host needs WOW64 on x64 Windows.
`cargo xtask package --check` audits the layout and rejects stale CRT DLLs or
wrong-side licence files (`X1-package-runtime.md:92-101`).

How it starts (packaging QUICKSTART, `X1-package-runtime.md:77-88`, reconciled
with `crates/rustamp-app/src/cli.rs:53-63,109-139` and `config.rs:10-12,43-98`
at that revision): from PowerShell with one quoted local media path; `--skin`
takes an optional `.wsz`/`.zip`/folder (no external skin is bundled);
`--no-audio` opens the window silently with no audio-device requirement; the
`--null --seconds 5` diagnostic covers the no-window/no-sound case. A second
launch does not open a second player: it forwards its command line (file +
volume) through the named pipe to the running instance and exits
(`docs/tasks/F2-single-instance.md:48-64,72-94`; in the bytes — F2 predates the
build). Flags fixed at first-instance start-up (`--no-audio`, `--skin`,
`--seconds`) are refused with a message (`F2-single-instance.md:83-86`).

## 2. What a user can actually do with the bytes

Conservative set — only what pre-dates `f1b87b8` or is recorded in the
artefact's own README (`docs/tasks/BETA-build.md:46-53` names the open items,
not a feature list, so the positive claims below rest on the cards cited):

- **Play a local file to its end**, including through the production WASAPI
  path: the VM-only audio tests (`--ignored:loopback`, `--ignored:real_playback`)
  pass on the pre-build tree (HANDOFF takeover note; `docs/tasks/BETA-build.md:6-8`
  records the headless suite green at 55 runs / 0 failures plus the four window
  tests and the two real-device tests at the build commit).
- **Transport + seek**: play/pause/stop/next/previous, frame-accurate seek, gapless
  advance; stop-then-play clears the endpoint tail on flush-capable backends
  (`docs/tasks/A12-output-flush.md:51-78` — pre-build; the cpal refusal half of
  the story is post-build A13 and is NOT in the bytes, see §3).
- **Equaliser**: the 10-band EQ + preamp run at index 0 of the DSP chain with a
  limiter behind them; the fresh-config default is engaged and flat (unity, hence
  inaudible), synced from the engine at start-up
  (`docs/tasks/BETA-readiness-2.md:§2h`, tree at `fc3c8c7` — the engaged-default
  behaviour itself predates the build via `cdead5f`, which M18 places before the
  r32 hang, i.e. pre-build; the exact doc lines are cited at the newer revision).
- **Skins**: the built-in default is the drawn Rust theme (19 sheets, 33 files,
  zero substitutions), the look-alike stays selectable (`--skin lookalike` /
  `builtin:lookalike`); cursor fallback, look-alike dimension re-pinning and the
  generator README rewrite are post-build J4 work and are NOT in the bytes
  (`docs/tasks/J4-skin-fallbacks.md`; `docs/tasks/W11-docs-sweep.md:C1-C9`).
- **Configuration**: a real file, honoured-only content, loud abort on malformed
  input, saved on clean exit (`docs/tasks/N6-config.md` via
  `docs/tasks/BETA-readiness-2.md:§1` BETA-12 row — N6 predates the build; in the
  bytes `easy_move` had no drag path, but in the tree it is now wired by N8
  (`24b33e4`) at `app.rs:558-560` and `window.rs:1283-1293,1460-1468`).
- **Open-file path**: the Eject key releases an open-media dialog that stages a
  typed, dropped or Backspace-corrected path and confirms with Enter
  (`docs/tasks/BETA-readiness-2.md:§1` BETA-03 row, tree at `fc3c8c7`; the dialog
  code cited there post-dates the artefact in part — §3's S06i3 work
  (`35cefe1`) is post-build — so for the bytes this is the weakest claim in the
  list and should be re-checked against `f1b87b8` before being promised).

## 3. The ~30 post-build changes: reachable in the tree vs only modelled

None of this section is in the downloaded bytes. "Reachable" means the code path
is complete and only execution proof is missing; "modelled" means a seam is
still open and no user action can cross it today.

**Reachable in the tree (complete, proof pending):**

- **Device-error hook (H5, `e3f73be`).** RFC 0012 Option A implemented:
  `OutputDevice::stream_error` (`crates/rustamp-core/src/audio.rs:388-399`),
  WASAPI failure-streak and cpal error-callback latches, `Engine::poll` checks it
  every pump into `handle_output_error` (fail to Stopped, freeze timeline,
  publish `Event::Error`). Headless suite written
  (`docs/tasks/H5-output-error-hook.md:§2`); real disconnection timing stays
  real-device work (`H5-output-error-hook.md:§3`).
- **Verified-byte loader (P5, `8a91563`).** Snapshot-then-`LoadLibraryExW`-only-the-
  snapshot, passive structural preflight (machine `0x014C`, PE32-only, exact
  getter) before any execution, `approved_image` allowlist, `ImageChanged` /
  `NotApproved` refusals (`docs/tasks/P5-dll-preflight.md:19-86`). Still missing:
  a production approval store; the VM-only `in_openmpt` acceptance rerun and the
  i686 headless mapping test (`P5-dll-preflight.md:113-144`).
- **Honest flush contract (A13).** cpal `flush` refuses with `Unsupported`
  instead of false `Ok`; every required flush goes through `Engine::flush_output`
  with a fail-closed restart barrier; failed seeks no longer invalidate anything
  (`docs/tasks/A13-flush-contract.md:37-100`). The old A12 universal no-replay
  claim is superseded, not repeated (`A13-flush-contract.md:57-61`).
- **ReplayGain, end to end in the tree (E4 + A15 + A16).** Lossless tag carriage
  and pure-function analysis (`docs/tasks/E4-replaygain.md`); applied on the
  decode thread *before* the EQ so the limiter still sees hot corrections and the
  converter's final authority still holds (`docs/tasks/A15-replaygain.md:40-80`);
  default mode `Disabled` (no loudness change until asked), ±24 dB preamp, peaks
  deliberately unused (`A15-replaygain.md:95-108`); parser unified so
  `rustamp-meta` owns the format (`docs/tasks/A16-replaygain-unify.md`). No UI or
  config surface wires it up: `audio.replaygain` is a warned-and-unapplied key
  (`crates/rustamp-app/src/config.rs:127`).
- **True-peak safety net (A14).** The resampler can rebuild peaks up to ~+3 dBFS
  above the limiter's sample-peak ceiling by physics, not by bug; the converter
  now holds a final ±1.0 (0 dBFS) ceiling on the resampled path, equal-rate audio
  byte-untouched (`docs/tasks/A14-dsp-correctness.md:44-115`). A true-peak
  limiter is named future work, not claimed (`A14-dsp-correctness.md:103-115`).
- **Second decoder factory (B4, `1da2e18`).** A real `std`-only RIFF/WAVE factory
  (`wave_dec.rs`) beside symphonia, so RFC 0013's tie-break is a real code path
  with real ties, content-beats-extension, and genuine refusals
  (`docs/tasks/B4-second-decoder.md:36-88`).
- **Residents + engine adapters (M12/M13/M15/M19).** A real symphonia decoder
  resident (`rustamp-dechost-symphonia`), a real gain/passthrough DSP resident
  (`rustamp-dsphost-stages`), and engine-side adapters presenting each as the
  core `Decoder` / `DspStage` the engine already drives — zero changes to
  `decode.rs`/`engine.rs` decode paths, opt-in only, wire `PROTOCOL_VERSION`
  untouched (`M12:§1-3`, `M13:§1-3`, `M15:§1`, `M19:§1-3`). Mid-run resident
  failure latches into a `DspFailureWatch` and fails the track loudly, never the
  player (`M19:§3`).
- **Playlist model interactions (S06i4, `0207c6a`).** Range select, ctrl-toggle,
  block drag / `Alt+Up`/`Down`, delete/crop with the frozen core-`Playlist` rule
  for the current entry; pure functions plus the command-seam spec the window
  lane must call (`docs/tasks/S06i4-playlist-interactions.md:§1-3`).
- **Load-playlist Enter (N7 `9e8c5d0`, verified N8 `24b33e4`).** Enter confirms through
  `confirm_load_playlist` (`crates/rustamp-app/src/dialogs.rs:188-191,224-239`),
  loads via `App::load_playlist_file` (`app.rs:1431-1491`, queue replaced, track 0
  played, report recorded, skips noted), and closes the dialog; empty confirm keeps
  the dialog open (`app.rs:2974`).
- **Easy-move window dragging (N8, `24b33e4`).** Main-window background press drags the
  window via `Windowed::try_easy_drag` (`crates/rustamp-app/src/window.rs:1283-1293,1460-1468`)
  and `App::easy_drag_main` (`crates/rustamp-app/src/app.rs:558-560`); controls keep
  their presses untouched. Pinned by `easy_move_decides_which_presses_drag_the_window`.
- **Shipped host residents (X2, `3213cdb`).** `xtask/src/package.rs:32-38` now
  packages `rustamp-dechost-symphonia.exe` and `rustamp-dsphost-stages.exe` beside
  the player and 32-bit plugin host.
- **Production plugin approval store (P7, `e8134cf`).** Verified-hash allowlist
  implemented in `crates/rustamp-plughost/src/approval.rs` and enforced by
  `loader.rs:676-678` before loading.
- **Built-in Visualiser (feea30a, f7b09ea, W17-vis-clear).** Implemented and wired to
  the running player in the interface pump after engine polling and before refresh,
  ingesting decoder source samples. When playback stops (`TransportState::Stopped`),
  both oscilloscope and spectrum modes reset immediately to static baseline (zeroing
  bars, peaks, targets, centering scope, setting `VisInput::Missing`), with no decaying
  spectrum tail retained. Golden updates/re-blessing (`vis_stopped_tail.png`) and
  live VM desktop execution proof pending. Ballistics and response curves are an
  internal design choice rather than a measurement matched to Winamp; Winamp
  visualisation plugins are not hosted.
- **Shuffle & Repeat, engine-owned (ade10e6, 6a606ef, 702719c).** Traversal order is
  owned by the audio engine, which generates the single shuffle permutation and
  coordinates the plan with the decode worker across thread boundaries; cycle replanning
  honours anti-repeat rules; `Prev` on the first track restores the previous cycle plan;
  orphaned duplicate state machine in the playlist crate deleted in favor of engine authority.
- **Tray + global hotkeys (F3, d2c8e5d, 8f946a9, bc828a7).** Both `window.rs` seams are
  closed. Hotkeys are drained in the window pump through standard command mappings;
  tray icon attaches to the shell, responds to left-click, double-click, and native
  context menu, and re-registers on `TaskbarCreated` after Explorer restarts; missing
  icon degrades gracefully to a warning.
- **Window focus commands (b17aa81, 1912b98, W18-window-focus).** `Ctrl+Tab`
  (`FocusNextWindow`) cycles across open docked windows, skipping closed ones; `Alt+W`
  (`ShowMainWindow`) shows, un-minimizes, and focuses the main window (including
  un-hiding after tray minimize). Pure order model `[Main, Equalizer, Playlist]`.
  Honest note: the focus order is an own choice rather than a reference measurement.
- **Offline PE import inspection (ff644db, W17-xtask-findings).** `xtask/src/pe_imports.rs`
  parses import and delay-load directories, refusing any non-allowlisted module or dynamic
  CRT DLL. Checked on packaging; sets `artifact_imports_verified: true` in manifests.
  Offline static check; proves nothing about clean machine execution.
- **Full licence texts in notices (ff644db, W17-xtask-findings).** Player notices
  (`player/THIRD_PARTY.md`) embed full text of MPL-2.0 (373 lines, SPDX canonical) and
  Apache-2.0.

**Only modelled (a seam is still open — no user action crosses it):**

- **Classic plugin user wiring.** While the approval store exists (`approval.rs`),
  user-facing plugin selection and configuration UI wiring is not present, and no
  sandbox claim is made (`P5-dll-preflight.md:88-112`; `docs/tasks/BETA-readiness-2.md:§4`).
- **Skeletons and placebos:** EQ presets popup (exists, does nothing); preferences
  pages (`rustamp-prefs`, draws selected name only); window menu submenus (deferred);
  window menu Exit (closes only popup, asserted by test); Clutter O/V buttons (emit
  no command).
- **ReplayGain UI surface:** Engine setters exist, config key warned-and-unapplied.

## 4. What is honestly NOT in the beta (either bytes or tree)

Clean-machine execution proof (static PE imports are verified to reference only
Windows system DLLs, allowing `artifact_imports_verified: true`, but first launch
on a pristine machine without developer runtimes has not been executed); physical
audio hardware verification (device-loss recovery, hardware latency, and driver
compatibility require physical hardware); running real Winamp plugins in the shipped
player, or any UI wiring for plugin configuration; visualiser fidelity matched
to a reference Winamp implementation (ballistics and response curves are internal
design choices), and Winamp visualization plugins are not hosted; placebo controls
(Clutter O/V, menu submenus, menu Exit-as-quit, EQ presets, prefs/plugin pages);
ReplayGain settings UI surface.

## 5. Executed proof vs compile-only (as of HEAD `be08b17`)

**Executed in the VM at least once (green):** the headless suite at the build
commit (55 runs / 0 failures, `docs/tasks/BETA-build.md:6-8`); the two VM-only
audio tests (`--ignored:loopback`, `--ignored:real_playback`) and the four
window tests in the interactive session (takeover record); the plugin-host
end-to-end (`plughost_e2e`) and host-window tests (takeover record); the red run
behind `cdead5f` that returned (67 runs / 5 binaries / 7 tests,
`docs/tasks/M18-hang-hunt.md:26-28`). **Runner executions:** the smoke runner
has run five times and found four runner/staging defects, all fixed
(`docs/tasks/BETA-smoke.md:69-92` plus the `17c3c12`/`6873089` amendments) —
but no run has yet recorded the full green proof (window at 275×116, live
bundled-host child, silent marker, clean kill), and the packaged window has
therefore still never been proven open.

**Written but never executed anywhere (compile-only — fmt/clippy/build only):**
the M12 `real_decode.rs` suite, the M13 `dsp_roundtrip`/`dsp_control` suites,
the M15 `resident.rs` adapter + engine suites, the M19 DSP-adapter suites, the
A13 `flush_contract.rs` + cpal-refusal suites, the M17 condition-wait updates,
the P5 parser/loader suites including the i686 headless mapping and the
`in_openmpt` acceptance rerun, the F2/F3 headless suites, the B4 `selection.rs`
suites (desk-checked only, `B4-second-decoder.md:145-152`), the E4 tag suites,
the A14 (17 tests) and A15 (9 + 7 tests) suites, the J4 cursor/lookalike suites,
the S06i4 model suites, and every M18 stall bound (test-only fixes,
`M18-hang-hunt.md:301-313`). The full suite at HEAD has additionally never
returned green: r32 and r34 hung in the guest without returning
(`M18-hang-hunt.md:23-34`), and the X3 runner fix that names the hanging binary
carries only parse/AST/local-child evidence (`docs/tasks/X3-runner-hang.md:59-64`).

**Structurally barred from a plain run** (not the same as unexecuted — these
need staging/fixtures by design): the 19 `#[ignore]`d tests and 4
environment-gated skip sites inventoried in `docs/tasks/W12-ignored-inventory.md`
(Groups A–F), of which Groups A (8 window), B (3 real-process) and D (2 real-DLL)
are beta-required before shipping, E1 (real playback) is strongly recommended,
and C/E2/F are deferrable (`W12-ignored-inventory.md:147-200`).

## 6. What the smoke runner does and does not prove

Does (`docs/tasks/BETA-smoke.md:45-68`): audits the staged bytes (existence,
non-emptiness, SHA-256), stages byte-for-byte without rebuilding, launches the
packaged player with `--no-audio` in the interactive session, asserts the exact
`Rustamp` main window at client 275×116, asserts a live `rustamp-plughost32.exe`
child (eager spawn + Hello/Pong handshake service), screenshots via
`xtask capture`, asserts the silent-path stdout marker, kills the player, and
re-verifies every file against guest hashes — failing non-zero naming the step.
Its own defect history (empty-session masking, owner-lookup parameter, staging
path policy) is part of the proof it works as a runner.

Does not (`docs/tasks/BETA-smoke.md:94-103`): a real audio endpoint (null output
only), a real plugin (control channel only, no DLL), a long or clean-exit run
(~half a minute, then killed), pipe-byte handshake content, decode beyond the
one committed `sine.flac` fixture, or which `Rustamp*` window the PNG shows when
several match. Two stalenesses inside that card are noted here, not fixed here
(outside owned paths): its status line still says "runner written, not
executed" while its own § now records five runs, and its PNG paragraph still
says the screenshot "will show the BETA-02 magenta placeholder" although the
shipped default has been the Rust theme since J2 (`docs/tasks/W11-docs-sweep.md:C2`).

## 7. What this card could not determine

- Whether any readiness-2 tree behaviour newer than `f1b87b8` (notably the
  BETA-03/BETA-06 dialog and control fixes cited at `fc3c8c7`) is also in the
  artefact bytes — the artefact was not rebuilt and was not diffed file-by-file
  here. §2 flags the weakest claim; the rest should be read as "in the product
  line", not "in these bytes", until someone checks them against `f1b87b8`.
- Whether plugin user-facing wiring holds (`docs/tasks/BETA-readiness-2.md:§5`).
  (Note: the `~7.8 ms` G3 figure was marked by G5 as an unevidenced single observation
  with its tick mechanism withdrawn in `docs/tasks/G3-wake-timeout.md:90-118`, and
  F11's double-print was confirmed from code at `lib.rs:87` and `main.rs:93`).
- Actual pixels of the Rust-theme default (no golden namespace),
  clean-machine launch/reputation prompts, device-loss timing, sound quality —
  all need the VM or hardware (`docs/tasks/BETA-readiness-2.md:§5`).
- Current suite totals at HEAD: quoted counts are per-commit snapshots
  (55 runs at `f1b87b8`; 67 runs in the red run behind `cdead5f`), not today's
  number. Nothing was run here, so no new total is claimed.

## 8. Boundary and verification receipt

- Read: `AGENTS.md`, `CONTRIBUTING.md`, `docs/HANDOFF.md` (read only),
  `docs/tasks/BETA-build.md`, `BETA-readiness-2.md`, `BETA-smoke.md`,
  `W12-ignored-inventory.md`, `X1-package-runtime.md`, `X1-xtask-package.md`,
  `F2/F3-single-instance/tray-hotkeys.md`, `H5-output-error-hook.md`,
  `P5-dll-preflight.md`, `A12/A13/A14/A15/A16`, `E4-replaygain.md`,
  `B4-second-decoder.md`, `M12/M13/M15/M18/M19`, `J4-skin-fallbacks.md`,
  `S06i4-playlist-interactions.md`, `W11-docs-sweep.md`, `X3-runner-hang.md`,
  plus `git log`/`git show --stat`/read-only `grep` for the seam checks above.
- Wrote: only this card. Edited: only `docs/tasks/BETA-build.md` (dated
  addendum, see that card). No `.rs` file, test, golden image, `docs/HANDOFF.md`,
  review card or runner script touched. No cargo, test, window, audio, download,
  git write or commit.
