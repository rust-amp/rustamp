# W12 — Inventory of every ignored or environment-gated test

**Date:** 2026-10-17. **Corrected:** 2026-10-18 — count (19 -> 20, Group D
gains the P8 DSP acceptance test), runner state (the X4/P8 staging the card
called open is done) and Group A's lane split (the four winit tests need the
interactive lane; the plain pass reports them red). Correction log at the
bottom; the companion card is `docs/tasks/W19-ignored-readiness.md`.
**Scope:** static inventory of the tests a plain
`cargo test --workspace --all-features` pass does *not* execute: the 20
`#[ignore]`d tests and the 4 environment-variable-gated skip sites (3 real
tests + 1 latent golden infrastructure). Read-only survey of this worktree;
no code, test or runner was touched, and `cargo test` was **not** run
anywhere (task constraint).

**Why this card exists:** BETA-readiness-2 §2/§5 established that several
suites have never been executed anywhere. This card answers the narrower,
permanent question: which tests are *structurally* barred from a plain run,
why each one is barred, and what exactly has to be staged or configured to
run it. (Suites such as `resident.rs`, `flush_contract.rs`, `dsp_control.rs`,
`real_decode.rs` and the P5 headless loader tests are **not** in this card:
they are unexecuted-so-far but fully runnable in a plain guest pass — a
different problem, solved by scheduling one `vm-run-tests.ps1` run, not by
staging fixtures.)

**How the guest runs tests** (`docs/notes/vm-run-tests.ps1` +
`docs/notes/vm-broker/guest/run-tests.ps1`): the host compiles and stages,
the guest executes every staged test binary; an `--ignored` filter prefix
makes each binary also run its `#[ignore]`d tests, and `--ignored:<name>`
additionally filters by binary name. The guest script sets
`RUSTAMP_TESTDATA=C:\rustamp\testmedia`, `RUSTAMP_TESTSKINS=C:\rustamp\testskins`
and `RUSTAMP_TESTPLUGINS=C:\rustamp\testplugins` when those trees were staged
from the owner's host directories, and `RUSTAMP_DECODER_FIXTURE` /
`RUSTAMP_DSP_FIXTURE` / `RUSTAMP_TESTPLUGIN_DSP` when the fixture binaries
and the DSP DLL were staged. The host runner builds and stages the i686
`rustamp-plughost32.exe` **binary** *and* the i686 plughost **test binaries**
beside the test binaries. A general `--ignored` pass leaves the two Group E
binaries out plainly (`LEFT OUT`, exit 0) unless the filter names them; the
four winit window binaries are **not** left out and fail in session 0 — see
Group A. (Corrected 2026-10-18: an earlier revision said the test binaries
were not staged and the three env vars were unset; the X4/P8 lanes closed
both.)

The shared VM gate is `require_test_vm()`
(`crates/rustamp-app/src/window.rs:137`, `crates/rustamp-plughost/src/host_window.rs:93`):
it refuses unless `COMPUTERNAME` equals the pinned test-VM name
(`DESKTOP-UL3J9H6`) or the staged-tree marker `C:\rustamp\code\Rustamp`
exists. It has deliberately no environment override.

---

## Group A — VM GUI: opens or spawns real windows (8 tests)

**What it needs:** two lanes, not one (corrected 2026-10-18; the VM record
`HANDOFF.md:37` says the four winit tests panic in the broker's session-0
PowerShell Direct session, so the interactive GUI-session lane is their
*primary* lane, not a fallback):

* **Interactive lane** (`vm-run-gui-session.ps1` -> guest
  `run-gui-session.ps1`, a one-shot scheduled task in the auto-logon
  session): the four winit window tests
  `the_equalizer_window_opens_at_its_docked_size_and_position`,
  `the_main_menu_popup_opens_at_the_measured_client_size`,
  `the_playlist_window_opens_docked_below_the_main_window` and
  `the_windowed_player_opens_a_window_inside_the_test_vm`. They pass there
  (`HANDOFF.md:70`). The plain `--ignored` pass does **not** leave these
  four binaries out, so it reports them `FAIL` — a known lane mismatch, not
  a defect; see the W19 card for the named runner gap.
* **Headless lane** (the plain `--ignored` pass): the three parameter-pinning
  tests (`the_new_dialogs_pin_their_os_window_parameters`,
  `opening_the_playlist_window_needs_an_os_window`,
  `opening_either_dialog_needs_an_os_window`) open no window, and
  `the_menu_popup_opens_at_the_measured_size` builds a winit window on a
  harness worker thread (`build_event_loop(true)`); no run record proves it
  in either lane, so the plain pass is its assigned lane and a winit panic
  there would move it to the interactive lane.

No fixtures beyond the staged tree (the `sine.flac` fixture is committed and
staged with `crates`). **Runner work: none** — but the plain pass stays red
on the four winit binaries until `run-tests.ps1` gains the same leave-out it
already has for Group E.

| Binary (target) | Test | File:line (`#[ignore]` / fn) | Why ignored |
|---|---|---|---|
| `rustamp-app` (lib tests) | `the_menu_popup_opens_at_the_measured_size` | `crates/rustamp-app/src/window.rs:2082` / `:2084` | Opens the real menu popup window (winit, off-main-thread loop); VM gate at `:2086`; lane unverified — see above |
| `rustamp-app --test headless` | `the_windowed_player_opens_a_window_inside_the_test_vm` | `crates/rustamp-app/tests/headless.rs:260` / `:261` | Spawns the real player `--no-audio <fixture>`, reads back OS geometry, kills it; VM gate at `:263`; interactive lane (winit) |
| `rustamp-app --test eq_window` | `the_equalizer_window_opens_at_its_docked_size_and_position` | `crates/rustamp-app/tests/eq_window.rs:40` / `:41` | Opens the equalizer window; VM gate at `:43` |
| `rustamp-app --test main_menu` | `the_main_menu_popup_opens_at_the_measured_client_size` | `crates/rustamp-app/tests/main_menu.rs:161` / `:162` | Opens the main-menu popup; VM gate at `:172` |
| `rustamp-app --test playlist_window` | `the_playlist_window_opens_docked_below_the_main_window` | `crates/rustamp-app/tests/playlist_window.rs:58` / `:59` | Opens main + playlist windows, reads back docking geometry; VM gate at `:61`; interactive lane (winit) |
| `rustamp-app` (lib tests) | `the_new_dialogs_pin_their_os_window_parameters` | `crates/rustamp-app/src/app.rs:3741` / `:3743` | Opens no window itself, but pins the parameters the presenter builds the four dialog windows from; VM gate at `:3747` |
| `rustamp-app` (lib tests) | `opening_the_playlist_window_needs_an_os_window` | `crates/rustamp-app/src/playlist_window.rs:660` / `:662` | Conservative ignore: opens no window, pins playlist-window size/origin constants only; no VM gate inside |
| `rustamp-ui` (lib tests) | `opening_either_dialog_needs_an_os_window` | `crates/rustamp-ui/src/playlist.rs:936` / `:938` | Conservative ignore: opens no window, pins the two spec'd dialog titles only |

## Group B — VM real-process: spawns the plugin host / creates the host window (3 tests)

**What it needs:** one guest run with `--ignored` **plus** a resolvable
`rustamp-plughost32.exe`. The existing runner already builds the i686 host
and stages it next to the test binaries (`vm-run-tests.ps1:102-129`), which is
exactly where `find_plugin_host()` looks. **Runner work: none** (the loud
"BLOCKED"/skip paths fire only if the host is missing).

| Binary (target) | Test | File:line (`#[ignore]` / fn) | Why ignored |
|---|---|---|---|
| `rustamp-app` (lib tests) | `spawns_the_real_host_without_a_window` | `crates/rustamp-app/src/supervise.rs:1264` / `:1265` | Spawns a real process; VM gate at `:1266`; skips-with-note if no host binary resolves |
| `rustamp-app --test plughost_e2e` | `app_published_state_reaches_the_host_window` | `crates/rustamp-app/tests/plughost_e2e.rs:209` / `:210` | Supervisor spawns the real host, a known snapshot crosses the real control channel, the host's real window answers; fails loudly ("E2E BLOCKED, not passed") if no i686 host is resolvable |
| `rustamp-plughost` (lib tests) | `creates_the_real_host_window_vm_only` | `crates/rustamp-plughost/src/control.rs:424` / `:426` | Creates a real (hidden) Win32 `HostWindow`; VM gate at `:427` |

## Group C — VM two-process resident fixtures (4 tests)

**What it needs:** one guest run with `--ignored` **plus two staged fixture
binaries and two guest env vars** — both handled by the runners since the X4
lane (corrected 2026-10-18: an earlier revision of this card said the
runner did not set them). The host runner builds
`cargo build --offline -p rustamp-mediahost --example decoder_fixture
--example dsp_fixture` and stages both (`vm-run-tests.ps1:62-68, 88-100`);
the guest script sets `RUSTAMP_DECODER_FIXTURE` / `RUSTAMP_DSP_FIXTURE` to
their absolute paths (`run-tests.ps1:89-102`). The `expect(...)` in each
`fixture()` helper names these variables. **Runner work: none.**

| Binary (target) | Test | File:line (`#[ignore]` / fn) | Why ignored |
|---|---|---|---|
| `rustamp-mediahost --test pcm_process` | `named_pcm_two_process_transfer_and_seek` | `crates/rustamp-mediahost/tests/pcm_process.rs:38` / `:39` | Two real processes over real named rings; VM-only; requires `RUSTAMP_DECODER_FIXTURE` |
| `rustamp-mediahost --test pcm_process` | `host_exits_mid_stream_and_infinite_read_survives` | `crates/rustamp-mediahost/tests/pcm_process.rs:74` / `:75` | Abrupt resident-process exit mid-stream must wake the infinite read, never fake EOS; same fixture requirement |
| `rustamp-mediahost --test dsp_process` | `named_dsp_two_process_gain_round_trip_then_sticky_eos` | `crates/rustamp-mediahost/tests/dsp_process.rs:79` / `:80` | Two real processes; DSP ×2.0 round trip, drain, sticky EOS; requires `RUSTAMP_DSP_FIXTURE` |
| `rustamp-mediahost --test dsp_process` | `host_exits_mid_stream_and_infinite_read_survives` | `crates/rustamp-mediahost/tests/dsp_process.rs:127` / `:128` | Abrupt DSP-helper exit mid-stream; same fixture requirement |

## Group D — VM + i686 test binary + real approved plugin DLL (3 tests)

**What it needs:** the Hyper-V VM, the `rustamp-plughost` **test** binaries
built for `i686-pc-windows-msvc` (they panic loudly on x86_64 by design) and
the owner-approved `in_openmpt.dll` findable via `RUSTAMP_TESTPLUGINS`, the
guest-staged `C:\rustamp\testplugins\...` path or the orchestrator path
(`dll_acceptance.rs:42-83`, `loader.rs:1188-1215`), plus — for the P8 DSP
acceptance test — `dsp\dsp_optilab_core.dll` via `RUSTAMP_TESTPLUGIN_DSP`.
**Runner work: none** since the X4/P8 lanes (corrected 2026-10-18: an
earlier revision of this card said only the plughost *binary* was built for
i686): the host runner builds
`cargo test -p rustamp-plughost --all-targets --all-features --no-run
--target i686-pc-windows-msvc` with static CRT and stages the resulting
test binaries (`vm-run-tests.ps1:102-147`); the guest exports
`RUSTAMP_TESTPLUGINS` (`run-tests.ps1:78`) and `RUSTAMP_TESTPLUGIN_DSP`
with a SHA-256 the host re-verifies after the copy (`run-tests.ps1:79-87`).

| Binary (target) | Test | File:line (`#[ignore]` / fn) | Why ignored |
|---|---|---|---|
| `rustamp-plughost --test dll_acceptance` (i686) | `loads_real_32bit_in_openmpt_dll_and_proves_host_interaction` | `crates/rustamp-plughost/tests/dll_acceptance.rs:86` / `:87` | Loads the real 32-bit `in_openmpt.dll` through the P5 snapshot loader, runs `winampGetInModule2`/`init`, creates a real Win32 host window, answers `IPC_GETVERSION` with `0x2905`; VM gate at `:90`, i686 panic at `:93`, missing-DLL loud refusal at `:75` |
| `rustamp-plughost` (lib tests, i686) | `approved_real_dll_maps_headlessly_from_verified_snapshot` | `crates/rustamp-plughost/src/loader.rs:1182` / `:1183` | P5 mapping test: approved bytes still equal the snapshot, mapped path differs, no window/getter/init; VM gate at `:1184`, i686 panic at `:1186`, requires `RUSTAMP_TESTPLUGINS` at `:1189`; the OS necessarily runs TLS/`DllMain` |
| `rustamp-plughost --test dsp_acceptance` (i686) | `real_dsp_plugin_loads_initializes_and_changes_audio_through_the_bridge` | `crates/rustamp-plughost/tests/dsp_acceptance.rs:107` / `:108` | P8 (added after this card was written; the runner's summary already lists it): loads the real 32-bit `dsp_optilab_core.dll` (pinned SHA-256 `78A140C9…7874`) through the verified-bytes loader, proves `NotApproved` on one flipped byte, resolves `winampDSPGetHeader2`, drives `process` over 16 blocks and demands a measurable change; VM gate at `:110`, i686 panic at `:111`, requires `RUSTAMP_TESTPLUGIN_DSP` |

## Group E — VM + a real audio endpoint (2 tests)

**What it needs:** one guest run with `--ignored:<name>` **plus an audio
endpoint in the VM** — a hardware/VM-configuration question, not a staging
one. Both tests fail loudly (never skip, never pass vacuously) when no
endpoint exists. The `sine.flac` fixture is committed and staged
automatically. In a *general* `--ignored` pass both binaries are left out
plainly; they run only when the filter names them. **Recorded state**
(corrected 2026-10-18): `HANDOFF.md:69` records both tests passing in the
guest — `real_playback` on the production WASAPI path, `loopback` across the
installed VB-Cable — with the operational detail that the audio tests need
`-Reset` (a fresh checkpoint): without it the cable still carries the
previous run's audio and the loopback pre-roll check refuses ("the cable
carried a 440 Hz tone before playback started"). Verify the endpoint/cable
with `check-audio.ps1` before the run; the reinstall path is
`install-vbcable.ps1` + `fix-vbcable-once.ps1` (see the W19 card).

| Binary (target) | Test | File:line (`#[ignore]` / fn) | Why ignored |
|---|---|---|---|
| `rustamp-app --test real_playback` | `the_real_device_plays_a_reference_file_to_its_end` | `crates/rustamp-app/tests/real_playback.rs:77` / `:78` | The only test on the production device path (WASAPI shared mode, cpal fallback): plays audible audio to the end on the real default endpoint; VM gate at `:80`; needs Hyper-V audio redirection or a virtual device |
| `rustamp-app --test loopback` | `the_cable_carries_the_fixture_tone` | `crates/rustamp-app/tests/loopback.rs:450` / `:451` | Plays into "CABLE Input" and records "CABLE Output": needs the VB-Audio Virtual Cable installed in the VM (guest install scripts `install-vbcable.ps1` / `fix-vbcable-once.ps1` exist); Goertzel analysis must find the 440 Hz tone; VM gate at `:453` |

## Group F — environment-gated silent skips (3 tests + 1 latent infrastructure)

These are **not** `#[ignore]`d: they run in every plain pass but return
immediately without proving anything when their fixture directory is absent.
The guest runner stages `RUSTAMP_TESTDATA`/`RUSTAMP_TESTSKINS` only if the
owner's host directories exist; neither skin-corpus variable is staged by
any runner today.

| Binary (target) | Test | File:line | Gate | What it needs |
|---|---|---|---|---|
| `rustamp-skin --test corpus` | `corpus_skins_load_without_panicking` | `crates/rustamp-skin/tests/corpus.rs:40` (skip `:42-48`) | `RUSTAMP_SKIN_CORPUS`, defaulting to `D:\code\rustamp-free-sources\skins-corpus` | A real classic-skin corpus (`.wsz`/`.zip`); **no runner stages it** — the test always skips in the guest as the runner stands |
| `rustamp-ui --test render` | `corpus_skins_render_a_frame_without_panicking` | `crates/rustamp-ui/tests/render.rs:477` (skip `:479-485`) | same `RUSTAMP_SKIN_CORPUS` | same corpus; same always-skip status |
| `rustamp-skin --test rust_theme` | `compare_sheet_dimensions_against_fixture_when_env_set` | `crates/rustamp-skin/tests/rust_theme.rs:211` (skip `:212-224`) | `RUSTAMP_TESTSKINS` | The owner's `D:\code\rustamp-testskins` staged into the guest (runner already supports this); needs `base-2.91.wsz` inside; dimensions only, never pixels (legal rule at `:229`) |
| (latent) `rustamp-testkit` golden helper | `classic-base` golden comparisons | `crates/rustamp-testkit/src/golden.rs:264-269` | `RUSTAMP_TESTDATA` | **No committed test case uses the `classic-base` namespace today** (only the testkit self-test at `golden.rs:436` asserts the skip itself); the second-set lane that would add them is still queued (BETA-readiness-2 §2i) — nothing to run yet |

Footnote: several guest-only suites (`pcm_named.rs`, `dsp_named.rs`,
`pcm_process.rs`, `dsp_process.rs`, `real_decode.rs`, `dsp_roundtrip.rs`)
carry `#![cfg(windows)]` and compile to zero tests on a non-Windows target.
The project is Windows-only and both host and guest are Windows, so this is
not a gate in practice — noted only so a future non-Windows build is not
mistaken for a passing suite.

---

## What a beta must execute before shipping, and what can wait

**Must execute (first guest `--ignored` pass, in this order):**

1. **Group A (all 8).** These pin the geometry and liveness of every window
   the user will see, including the one test that spawns the actual player
   binary (`headless.rs:260`). BETA-readiness-2 §3 item 4 already names
   "the windowed four"; the four parameter-pinning tests ride along for
   free in the same pass. **Two lanes, not one** (corrected 2026-10-18):
   the four winit tests go through `vm-run-gui-session.ps1` in the
   guest's interactive session; the plain `-Filter '--ignored'` pass
   covers the four non-winit ones and reports the winit binaries red
   (see Group A's header and the W19 card).
2. **Group B (all 3).** The packaged player spawns the plugin host eagerly
   (BETA-smoke step table) and the supervisor/host handshake is a shipped
   runtime dependency; today its only evidence is compile-only
   (BETA-readiness-2 §2f-era "written but unexecuted" class). The i686 host
   the runner already stages is exactly what they resolve.
3. **Group D (all three).** The P5 loader is the only security-relevant
   unproven-by-execution path in the tree: "approved bytes map, snapshot
   path differs, `DllMain` ran, IPC answered `0x2905`" has zero execution
   evidence, and §4 of BETA-readiness-2 forbids shipping unapproved-DLL
   execution claims. The i686-test-binary staging the card once called a
   runner gap is done (X4): the runner builds and stages the i686 plughost
   test binaries and the plugin DLLs are staged and hash-verified — the
   remaining prerequisite is the guest run itself (see the W19 card).
4. **Group E1 (`real_playback`).** It is the only test on the production
   output path; every other suite drives the null output. A beta that has
   never pushed samples through WASAPI to a real endpoint has an untested
   last mile (BETA-05 residual). The blocker is a VM configuration
   decision — enable audio redirection / a virtual endpoint — and the test
   is designed to fail loudly until that is done, which is the right
   forcing function.

**Can wait, per item:**

- **Group C (two-process resident/DSP fixtures).** The packaged beta does
  not ship the resident helpers (no `xtask` packaging, engine registration
  is opt-in — M12 §5, M13 §5, BETA-readiness-2 §2b), so a beta user cannot
  reach the code these tests pin. They become mandatory before the resident
  path is packaged or enabled, not before this beta ships. They are cheap,
  though: the runner already builds and stages the two fixture binaries and
  the guest already sets both env vars (corrected 2026-10-18), so they
  schedule into the same guest pass with no further work.
- **Group E2 (`loopback`).** It proves sample *content* on the wire —
  deeper than a beta gate. `real_playback` already proves the device path
  opens, advances and drains; the VB-Audio cable plus Goertzel analysis is
  verification depth for the audio-quality milestone, not a ship blocker.
- **Group F (all).** The corpus tests broaden robustness against skins the
  product does not ship, and no runner stages the corpus anyway; the
  `RUSTAMP_TESTSKINS` dimension comparison runs whenever the owner's skins
  dir exists (already wired in the guest). The `classic-base` golden path
  has no committed case, so there is literally nothing to execute. None of
  these can mask a beta-blocking defect.

**Summary count:** 20 `#[ignore]`d tests (A 8, B 3, C 4, D 3, E 2) and 4
env-gated skip sites (F 3 tests + 1 latent). Of the ignored 20: **13 are
beta-required** (A 8, B 3, the two Group D loader tests), **2 more are
strongly recommended** (E 1 and the P8 Group D DSP acceptance test), and
**5 can wait** (C 4, E 2). The 4 env-gated sites are deferrable
indefinitely for this beta.

## Correction log (2026-10-18)

This card was corrected on 2026-10-18 by the W19 lane
(`docs/tasks/W19-ignored-readiness.md`), read-only, no code touched:

1. **Count:** 19 -> 20 `#[ignore]`d tests. Group D gains
   `real_dsp_plugin_loads_initializes_and_changes_audio_through_the_bridge`
   (`crates/rustamp-plughost/tests/dsp_acceptance.rs:107`), added by the P8
   lane after this card was written; the runner's `--ignored` summary
   already listed it. No test was removed; every name in the 2026-10-17
   tables still exists.
2. **Runner state (three claims closed):** the guest script *does* set
   `RUSTAMP_TESTPLUGINS`, `RUSTAMP_DECODER_FIXTURE` and `RUSTAMP_DSP_FIXTURE`
   (`run-tests.ps1:78, 89-102`), and the host runner *does* build and stage
   the i686 plughost **test** binaries (`vm-run-tests.ps1:102-147`) —
   Groups C and D now need **no** runner work.
3. **Group A's lane split:** the four winit tests panic in the broker's
   session-0 PowerShell Direct session (`HANDOFF.md:37`); the interactive
   GUI-session lane is their *primary* lane, not a fallback, and the plain
   pass (which does not leave them out) reports them `FAIL`. W12's
   "presumed sufficient" was a presumption, not a record.
4. **Line drift:** `app.rs:3257->3741`, `window.rs:1742->2082`,
   `headless.rs:254->260`, `playlist_window.rs:55->58`,
   `control.rs:419->424`, `loader.rs:773->1182`,
   `rustamp-ui/playlist.rs:894->936`, `pcm_process.rs:23/54->38/74`,
   `dsp_process.rs:35/72->79/127` (ignore/fn line pairs; the VM-gate lines
   moved with them). Test names unchanged.

## Boundary and verification receipt

- Read and written: only this worktree; the files touched are this card and
  `docs/tasks/W19-ignored-readiness.md`.
- `cargo test` was **not run**; no test binary, example, player, VM job,
  window or audio device was touched; no download, no commit, no git write,
  no crate/runner edits.
- No Winamp source, SDK, disassembly or forbidden-tree contents were read.
- `git status --short` (worktree, taken after writing — pasted in the W19
  report) shows exactly these two cards and nothing else.
