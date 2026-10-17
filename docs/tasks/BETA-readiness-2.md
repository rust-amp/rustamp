# BETA — Second readiness review (what a user would hit today)

**Date:** 2026-10-17. **Reviewed revision:** `fc3c8c7` (HEAD of this worktree).
**Scope:** static, clean-room re-review; findings only, no implementation changes.
**First review:** `docs/tasks/BETA-readiness.md` at `f9012dc` (twelve findings, BETA-01–BETA-12).
**Working boundary:** this worktree only. Nothing under `D:\code\Rustamp`,
`D:\code\rustamp-research` or `D:\code\rustamp-refbin` was read or written.
`cargo test` was **not** run and no player, window, audio device or VM job was
executed here (task constraint); `git log`/`grep`/reads only.

## Decision

**Still not ready for an ordinary first-user beta — but the failure set is
narrower and different.** Of the twelve first-review findings, **seven are
fixed or substantially fixed** (01, 02, 03, 04, 05, 12, and most of 06),
**two changed shape** (07 fixed-by-another-lane, 09 half-fixed), and **three
are still open in essentially original form** (08 shuffle/repeat, 10
vis/plugin/prefs skeletons, 11 platform beyond instance+tray-model+hotkey-model).
The new ground since the first review is real but largely **compile-only**:
the two residents, the engine-side adapter, the loader, the flush contract and
the mediahost concurrency fixes all landed with tests that were **written but
explicitly not executed** (lane constraint), and **the packaged beta has still
never been run anywhere**.

---

## 1. The twelve old findings, re-checked against today's code

| # | First-review claim | Status today | Evidence |
|---|---|---|---|
| BETA-01 | No self-contained runtime policy; cold-machine `0xC0000135` risk; mixed-bitness CRT collision | **FIXED** | `xtask/src/package.rs:36` pins `+crt-static` for both binaries; `RUNTIME_MANIFEST` (`package.rs:37`) declares static linkage, no redist/DLLs; `check_runtime_contract` enforces the factory command (`package.rs:697-735`); `package --check` audits existing layouts and rejects stale CRT DLLs (`package.rs:878-942`); QUICKSTART + runtime policy staged (`package.rs:186,235,641-672`). Residual: `artifact_imports_verified:false` in the manifest — PE imports still unverified, and no clean-machine run exists (see §4). |
| BETA-02 | First-run/default/fallback skin is flat magenta, font included | **FIXED, changed shape** | Built-in default is now the drawn Rust theme: `crates/rustamp-app/src/skin.rs:88-91` loads `BuiltinDefaultSkin` (Rust theme); J2 proved 19/19 sheets at exact sizes and 33/33 files (`docs/tasks/W10-batch-review.md` J2-1/2/7). No `PLACEHOLDER_RGBA` path is reachable for the built-ins (W10 J2-4 HOLDS). **Shape change:** the default is no longer the look-alike; `BuiltinLookalikeSkin` + the `lookalike` skin path stay reachable (W10 J2-5 HOLDS). Stale-doc residue: `crates/rustamp-app/src/skin.rs:6` still calls it the "look-alike default skin" (W10 F10 third bullet, unfixed — J4 owned only the skin crate). |
| BETA-03 | No ordinary GUI route from empty queue to a media file | **LARGELY FIXED** | Eject releases `OpenMedia{Files, play:true}` (`crates/rustamp-ui/src/behaviour.rs:438-441`); the dialog types full paths incl. `:`, `\`, `.` via OS text (`crates/rustamp-app/src/dialogs.rs:173-178`); Backspace deletes; Explorer drop stages a pick; Enter confirms and closes (`dialogs.rs:151-158`). Still open: no native OS picker, no paste path, Shift+click open-directory gap (`behaviour.rs:423-426` names it), directory-confirm adds one file, URLs refused by the decoder. |
| BETA-04 | Missing/unsupported media → phantom Playing; errors vanish at GUI boundary | **FIXED at app layer** | `surface_error` (`crates/rustamp-app/src/app.rs:1201-1233`): notes the failure, titles it with the entry name (`window_title`), stops a running transport unless EOS already reported, and re-syncs the UI transport so an optimistic Playing light goes out. `Event::Error` is drained (`app.rs:715`) instead of ignored; headless run breaks on `last_error` instead of hanging (`app.rs:1101-1104`) and returns it (`app.rs:1117-1119`). Residual: no new VM/user run proves the silence is gone; engine-worker open-failure loudness was not re-traced here beyond the existing `engine.rs:1023` error publish. |
| BETA-05 | Device failure unrecoverable/unreportable; H4 design-only | **SUBSTANTIALLY FIXED, reshaped** | RFC 0012 implemented (`e3f73be`): `OutputDevice::stream_error` (`crates/rustamp-core/src/audio.rs:388-399`, default `None`), WASAPI failure-streak latch (`wasapi_out.rs:423-425,837-846`), cpal error-callback latch (`cpal_out.rs:400-402,493-499`), `Engine::poll` checks it every pump (`engine.rs:891-897`) into `handle_output_error` (`engine.rs:1103`). A flush refusal now fail-closes (A13, §2). Residual: no real-hardware loss run; recovery means a **new** Engine/output lifecycle (no auto-rebuild) — honest but austere. |
| BETA-06 | Hit-testable but dead primary controls | **MOSTLY FIXED, residue named** | Fixed: Eject, Minimize, Shade, Close all release commands (`behaviour.rs:438-444`); ClutterA/D/I act (`behaviour.rs:449-457`); window-menu opens the popup (`behaviour.rs:461`). Still dead: **Clutter O/V emit nothing** (`behaviour.rs:429-430` documents it); **menu submenu parents open nothing** (`menu.rs:47-50,107-129`, `SubmenuDeferred` by design); **Exit closes only the popup, never the player** (`menu.rs:242-251`, asserted by test). |
| BETA-07 | Playlist visibility/check marks don't control its OS window | **FIXED (by the N5 lane, confirmed by reading)** | `sync_playlist_window` creates/drops the OS window to follow `App::playlist_open` (`window.rs:1089-1101`); PL button, menu row, skinned and OS close all flip the one flag (`window.rs:1091-1092,1187-1191`). The follow-up W10 review of the playlist lane confirms BETA-07 needed no change. Not re-verified: the *initial* `playlist_open` default value. |
| BETA-08 | Shuffle/Repeat lights advertise unimplemented playback | **STILL OPEN, unchanged** | Flags still app-owned only: toggle/publish (`app.rs:1255-1268`, `publish_modes`), engine still ignores them — decode advance is a plain index step to `EndOfStream` (`decode.rs:450-481`, no shuffle/repeat arm; no `SetShuffle` handling in `engine.rs:554-605` region per first review, structure unchanged). Still blocked on S15. Do not ship as working controls. |
| BETA-09 | Playlist-file loading is a dead end | **HALF-FIXED, changed shape** | UI half landed (`35cefe1`): the Load Playlist dialog takes a typed, staged or dropped path and formats the skip report (`playlist_window.rs:443-479` tests; `dialogs.rs:239` field). **But `Enter` on the Load Playlist dialog is still consumed and does nothing** (`dialogs.rs:130-133`, "RFC 0010 leaves its confirmation open"), and no `read_playlist` call site exists in the app (`playlist_window.rs` grep: only the dialog-open assertion at `:437`). Library report APIs exist (E3); no wired reader. A user can name a playlist file and nothing happens. |
| BETA-10 | Vis/plugin/prefs are skeletons | **STILL OPEN, narrowed** | Vis: spectrum still hard-zero, pinned by test (`vistap.rs:13,173` — "S08 owns this"); renderer draws no live visualiser. Plugin: the **loader is real** now (P5, §2) with VM-only acceptance, but user-facing plugin selection/config wiring was **not re-verified** in this review (no claim made either way). Prefs: still name-only pages (`rustamp-prefs`, dialog draws selected name); EQ presets popup exists, does nothing. All must stay excluded/disabled, not sold as features. |
| BETA-11 | Platform integration is DPI-only stub | **NARROWED, not closed** | Done: single instance + CLI forwarding (`408132f`; `main.rs:68-77,99-143`), tray **model** (7 entries → Commands, `tray.rs`), hotkey **model + registry + session** (defaults `Ctrl+Alt`, `hotkeys.rs`; session started `main.rs:73`). **Both `window.rs` follow-up seams are still open:** no `take_pending_hotkeys` drain exists anywhere in the app crate, and no `TrayIcon::attach` call exists in `window.rs` — hotkeys register globally but can never deliver a command; the tray icon never appears. Media keys/SMTC/taskbar/file-assoc/external-control window: still absent. |
| BETA-12 | Config read-only; settings not consumed/saved | **FIXED** | `save()` exists (`config.rs:346-356`), runs on clean exit (`window.rs:238-243`); malformed input aborts loudly (`config.rs:214`, `lib.rs:87`); unhonoured keys warned + omitted (`config.rs:114-186,254-341`); W10 F1 self-corruption fixed through serde (`c32cf7b`; regression test with `WindowId::Gen(3)` at `config.rs:443-539`). **Known residue (W10 F5, persists):** `easy_move` is listed honoured (`config.rs:141`) and persisted (`config.rs:287-288`) but **no drag path reads it** — `window.rs` has no `drag_window`; `Moved` arms only record positions; `easy_move` is read only for the menu check (`app.rs:520-521`, `menu.rs:68`). Same softer pattern for `eq.state.auto` (carried, no auto-preset behaviour — S07-open). |

### W10-batch-review reconciliation (39 claims, 10 false — which are still open in code)

- **F1 Gen-window self-corruption — CLOSED** (`c32cf7b`; writer uses the serde form the loader accepts; round-trip test pins `Gen(3)` at `config.rs:452-539`).
- **F2 loader TOCTOU — CLOSED by P5** (snapshot design: `loader.rs:9-12,369-435`; source path never reaches `LoadLibraryExW`; `approved_image` allowlist at `loader.rs:386-390`; `ImageChanged`/`NotApproved` refusals with tests at `loader.rs:709-759`). Residuals per the card itself: no production approval store; approved bytes still execute `DllMain` (trust decision, documented `loader.rs:33`).
- **F3 cpal flush gap — CLOSED honestly by A13**: cpal `flush` now refuses (`cpal_out.rs:381-382`, documented) instead of false `Ok`; every required flush goes through `Engine::flush_output` with fail-closed restart barrier (`docs/tasks/A13-flush-contract.md:37-56`). The old universal no-replay claim is superseded, not repeated.
- **F4 flush-on-failed-seek — CLOSED by A13** (`Worker::apply_seek` sends `Seeked` only on decoder success; refusal sends internal `SeekFailed`, no state reset — `A13:86-106`).
- **F5 `easy_move` advertised-but-unhonoured — STILL OPEN** (see BETA-12 row).
- **F6 lookalike dimension test loss — CLOSED by J4** (`tests/lookalike.rs` rewritten for `BuiltinLookalikeSkin` + routing guard `builtin_default_skin_is_still_the_rust_theme`).
- **F7 G3 wake measurement overclaim — STILL OPEN as a prose problem**: the `>= 1 ms` bound stands; the `~7.8 ms` figure has no in-tree artefact (`G3-wake-timeout.md:80` only). M17 replaced wall-clock waits with conditions where it mattered, but the card's mechanistic claim is unrepaired.
- **F8 hostile-DLL-reaches-DllMain — CLOSED as a scope correction**: preflight moved before mapping (`pe_preflight.rs`, `loader.rs:405-471` flow), structural refusals precede `LoadLibraryExW`; post-mapping version/getter checks are honestly labelled post-execution ABI refusals (`P5:80-86`). Not-a-sandbox remains stated.
- **F9 cursor fallback gap — CLOSED by J4** (`resolve_cursor_fallbacks`, `skin.rs:468`; 13 absent-on-both-sides names stay `None` by rule; 4 new tests).
- **F10 stale docs — 2 of 3 CLOSED by J4** (generator README rewritten; lookalike test retargeted). **Third bullet persists:** `app/src/skin.rs:6` still describes the built-in default as the look-alike skin.
- **F11 double-printed malformed-config error — NOT RE-VERIFIED, likely persists:** both print sites still exist (`app/src/lib.rs:87` prints, then `main.rs:93` prints the returned `Err`). Needs a 5-minute check, not a claim.
- **F12 buffered-flush test pins the call, not the consequence — MOOT/UNCHANGED:** the test still asserts the call; A13 added the consequence-pinning doubles (`flush_contract.rs`, unexecuted — §2).

---

## 2. New ground since the first review, and what is still open in it

### 2a. The two host residents (`rustamp-dechost`, `rustamp-dsphost`) — test-pinned vs prose-only

M14 (`docs/tasks/M14-host-framework-review.md`) counts **19 claims pinned by a
fail-able test, 7 partial, 5 prose-only**. Load-bearing prose-only edges:
decoder abrupt-player-loss mirror (DSP has it at `dsp_roundtrip.rs:340`,
decoder does not), DSP bounded-read `TimedOut` arm, URL refusal unit case,
serve-level generation-reuse guards (open-level pinned at `pcm_named.rs:43`).
M17 (`docs/tasks/M17-dsp.../M17-mediahost-findings.md`) closed three of these
with written-but-unexecuted tests: wall-clock waits → conditions
(`control.rs`, `pcm_transfer.rs`, new `dsp_control.rs` + `support/streams.rs`),
`DspReset`-while-idle via 20 ms wait slices (own scheduling choice, documented
as no latency guarantee), truncated-file contract narrowed to honest short-EOS
for this backend/fixture (`dechost/tests/real_decode.rs` now requires short
EOS + bit-identity + `Ok(())`, error leg dropped by design). Explicitly left
open by M17: **M14 F1** (needs the RFC lane), **F4 test-double direction**,
other queue/generation findings, all real-process exits, packaging of the new
binaries, and any Winamp reference behaviour.

### 2b. The engine-side adapter (M15, `a9b80d3`) — proven by test vs by prose

`crates/rustamp-audio/src/resident.rs` (996 lines: `HostDecoder`,
`ProcessDecoder`, `ResidentFactory`, `ResidentConfig`) + `tests/resident.rs`
(570 lines). Design is the honest one (adapter implementing core `Decoder`,
zero changes to `decode.rs`/`engine.rs`, opt-in via registration, wire and
`PROTOCOL_VERSION` untouched). **Every behaviour claim is pinned by a named
test** (bit-exact transfer + sticky EOS, seek-lands, refusal-loud,
death-loud, bounded-wait-then-retry, gapless resident→in-process source
change, format-change drain, past-the-end clamp). **None of those tests has
been executed anywhere:** the lane ran fmt/clippy/build only (constraint), and
no VM-suite record since the merge names them passing — the suite counts in
HANDOFF stop before this merge. Additionally still missing by the card's own
list: two-process real-binary runs (bit-identity vs in-process, mid-stream
kill, missing-helper degradation), engine-side **DSP/stage adapter**
(nothing — M14 F1 hole flows through), latency compensation, and `xtask`
packaging of the helpers. Status: **strong evidence on paper, zero execution
evidence.**

### 2c. The loader (P5, `8a91563`) — what it proves and what it does not

Verified-byte snapshot + passive structural preflight (machine `0x014C`,
PE32-only, bounded tables, exact getter, no forwarded/decorated/unsorted
names) all precede `LoadLibraryExW`; wrong-arch/mutated-source/disappeared-
source refusals have headless tests; the VM-only `in_openmpt` acceptance now
checks the mapped bytes equal the snapshot and the path differs. **Proven by
reading; executed nowhere** (compile-only lane; VM acceptance pending,
including the new headless mapping test's i686 build). What it is not: a
sandbox, a malware detector, an approval store (the project still needs one),
or protection against same-principal injection / kernel / namespace attacks —
all stated in the card. Production rule stays: nothing executes without an
independently approved `approved_image`.

### 2d. The flush contract (A13, `e2a8f3b` + `b40ef8d`)

Rule is now honest per backend: WASAPI schedules its reset, null drops its
queue, cpal refuses with `Unsupported`. Engine consequence: any required-flush
failure deactivates the ring, closes the output and latches a restart barrier
— no silent resume, no auto-rebuild (an austere but defined state). Failed
seeks no longer invalidate anything. **Evidence:** new `flush_contract.rs`
(8 tests incl. both endpoint kinds, refusal-blocks-restart, natural-end
keeps position) + cpal refusal unit test — **all written, none executed.**
Needs the VM like everything else; WASAPI reset/driver compliance explicitly
remains real-device work.

### 2e. Skin fallbacks (J4, `fc3c8c7` = HEAD)

Cursor fallback follows the sheet rule incl. the absent-on-both-sides
`None`; lookalike dimensions re-pinned with its five non-spec sizes called
out; generator README describes the Rust-theme generator and corrects the
"at build time" claim. Only in-tree cursor consumer impact: synthetic skins
gain 11 fallback cursors + substitutions (no assertion pinned counts).
Left: the two out-of-owned-path doc lines (J2 card claim + app skin docs —
the latter visibly stale at `app/src/skin.rs:6`).

### 2f. Tray and hotkeys (F3, `4682139`)

Model + registry + session are in (`platform/src/tray.rs` 315 lines,
`hotkeys.rs` 792 lines; headless mapping/bookkeeping tests written,
unexecuted). **The two `window.rs` seams are open:** hotkey firings queue
process-globally but nothing drains them (`take_pending_hotkeys` appears
nowhere in the app crate — only the F2 `take_pending` drain at
`window.rs:1487`), and `TrayIcon::attach` is never called. Net user effect
today: **six global chords registered, zero delivered; no tray icon.**
Either wire the ~8 follow-up lines in `window.rs` (VM checklist in the card)
or do not start the session.

### 2g. Configuration

Fixed (BETA-12 row): real file, loud malformed abort, honoured-only content,
round-trips incl. `Gen(n)`, saved on clean exit. The remaining lie is small
and known: `easy_move` (and softly `eq.auto`).

### 2h. Equalizer engaged-by-default

Three defaults coexist deliberately: wire `EqState::default().enabled=false`
(`proto/src/state.rs:72-78` — what a bare `PlayerState` says); fresh
**config** engaged + flat (`core/src/config.rs:155-167`, unity ⇒ inaudible,
spec-open per S07 §12 item 13); UI machine default engaged
(`ui/src/equalizer.rs:352-360`), synced from the engine at startup so app,
renders and goldens agree (wave-10 repair `cdead5f`). Honest as documented;
the On/`use_eq` link itself stays unmeasured.

### 2i. Goldens for both skins

`lookalike` namespace committed (`tests/golden/lookalike/*.png`,
`ui/tests/golden_lookalike.rs`); `classic-base` cases skip without
`RUSTAMP_TESTSKINS`. **The actual shipped default — the Rust theme — has no
golden namespace:** `ui/tests/` contains only `golden_lookalike.rs`, and the
second-set lane (`LANE-QUEUE.md:20`) was still queued at handoff. First-run
pixels of the default skin are therefore unpinned.

### 2j. Visualiser

Still unmeasured and still a skeleton: waveform tap raw, spectrum zero by
contract-test (`vistap.rs:173`); S08 probe + three jobs designed, runs are the
orchestrator's and have no recorded result; S08 spec unwritten. Nothing to
beta-test here beyond confirming it stays off/latent.

### 2k. The packaged beta that has never been run

`docs/tasks/BETA-build.md` (commit `e00d922`): built 2026-10-17 from
`f1b87b8` into `D:\code\rustamp-beta\2026-10-17` (player + self-contained
host + manifests + README). §"Not verified yet" is explicit — **no window,
no launch, no exit has ever happened with this artefact**, because a release
build opens a window and no window may open on the host. No later commit
records a packaged run (`git log --grep` finds only the record itself). The
needed VM job (stage packaged folder → launch → prove window → exit) is
designed nowhere and run nowhere.

---

## 3. The five things to fix first (beta order)

1. **Run the packaged beta in the VM and record it.** The artefact exists and
   has never executed: launch, prove the window, exit cleanly, note
   reputation/smart-screen behaviour. Every other item below is theory until
   the thing a user downloads has opened once. (BETA-build.md:41-44.)
2. **Close or remove the F3 seams.** Drain `take_pending_hotkeys()` into
   `submit` next to the F2 drain (`window.rs:1487` neighbourhood) and attach
   `TrayIcon` once the main `HWND` exists — or stop starting `HotkeySession`
   (`main.rs:73`). Shipping global chords that can never fire is a live
   side-effect with no function.
3. **Make Load-Playlist Enter do something or disable it.** The dialog accepts
   a path and consumes Enter into nothing (`dialogs.rs:130-133`). Wire the E3
   report APIs into a real load-with-skip-report, or close the dialog path
   until then — same placebo class as BETA-08.
4. **Execute the unexecuted suites in the VM.** `resident.rs` (M15),
   `flush_contract.rs` + cpal refusal (A13), `dsp_control.rs`/M17 transfer
   updates, P5 headless mapping (i686) + `in_openmpt` acceptance, plus the
   windowed four. The beta's load-bearing new semantics (fail-closed flush
   barrier, resident bit-identity, snapshot mapping) are compile-only today.
5. **Stop advertising what does not work.** Shuffle/Repeat (BETA-08),
   Clutter O/V, menu submenus + Exit, EQ presets, vis/prefs pages, plugin
   selection: hide, disable, or label as unmeasured. Fold in the small
   honesties: `easy_move` (omit or honour — `config.rs:141` claims it),
   the stale `app/src/skin.rs:6` line, and the probable F11 double print
   (`lib.rs:87` + `main.rs:93`).

---

## 4. What a beta must not ship with

- A package whose runtime contract is unproven on a clean machine
  (`artifact_imports_verified:false`; no packaged run — §2k).
- Global hotkeys that register but cannot deliver (§2f).
- A Load-Playlist dialog that takes a path and does nothing (§1 BETA-09).
- Placebo controls presented as working: Shuffle/Repeat, O/V menus, menu
  Exit/submenus, EQ presets, vis/plugin/prefs selection (§1 BETA-06/08/10).
- Any reintroduction of: phantom Playing over silence (keep
  `app.rs:1201-1233` + VM proof), cpal flush claimed as success (keep
  `cpal_out.rs:381-382` refusal + restart barrier), config self-corruption
  (keep `c32cf7b` round-trip test), magenta default (keep 33-file Rust theme
  complete), path-based plugin load (keep `approved_image` gate).
- Unapproved-DLL execution anywhere in a user path (no auto-approval; P5
  trust rule).
- Claims of VM/hardware proof that never ran (packaged launch, resident
  two-process runs, device-loss timing, the G3 `~7.8 ms` figure as fact).
- Truncated files presented as verified integrity: keep M17's honest
  short-EOS contract + its documented limit.

---

## 5. What this review could not determine

- Whether the shipped EXEs import only system DLLs, launch on a truly clean
  OS, or trigger reputation prompts (no artefact inspection or run here).
- Whether `resident.rs`, `flush_contract.rs`, M17's new tests, P5's loader
  tests, or the tray/hotkey headless tests pass — none executed anywhere
  per their cards, and no post-merge VM record names them.
- Whether the engine worker's open-failure path is loud in every branch
  (app-side handling verified; worker arms re-traced only to `engine.rs:1023`).
- Whether plugin user-facing wiring (select/config a DLL from the UI) exists
  (loader verified; app arms not re-read).
- Whether F11's double print still fires (both sites exist; flow unconfirmed).
- Actual pixels of the Rust-theme default (no golden namespace), sound
  quality, device-loss timing, resampler/DSP numeric behaviour, any real
  plugin compatibility, installer/signing/notices (explicitly release-time
  work per queue).
- Anything requiring a window, device, or the VM — by task constraint.

---

## 6. Boundary and verification receipt

- Read and written: only this worktree; only file created: this card.
- `cargo test`: **not run.** No test binary, example, player, package
  command, reference binary or graphical program was run. No download, no
  commit, no git write, no dependency change.
- `git status --short` (inside the worktree, taken after writing — pasted
  below) must show exactly `?? docs/tasks/BETA-readiness-2.md` and no
  tracked modification.
