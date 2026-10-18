# W19 — Readiness of the ignored tests for one VM session

**Date:** 2026-10-18. **Scope:** make the ignored pass runnable in as few VM
sessions as possible with no surprise failure caused by a missing
prerequisite. Documentation only: this card plus the W12 corrections. No
Rust source, no runner, no VM, no broker, no test, no download, no commit.

**Companion:** `docs/tasks/W12-ignored-inventory.md`, corrected today (see
§1). Every host path below was read on 2026-10-18; every piece of guest
state is *recorded* state (`HANDOFF.md`) unless a named verification step
closes it from the host side.

---

## 1. What was corrected in the inventory (W12, done in the same pass)

1. **Count:** 19 -> **20** `#[ignore]`d tests. Group D gains
   `real_dsp_plugin_loads_initializes_and_changes_audio_through_the_bridge`
   (`crates/rustamp-plughost/tests/dsp_acceptance.rs:107`, the P8 fixture),
   added after W12 was written; the runner's `--ignored` summary already
   lists it. No test was removed; every name in the old tables still exists.
2. **Three runner-gap claims are closed.** The guest script *does* set
   `RUSTAMP_TESTPLUGINS` (`run-tests.ps1:78`), `RUSTAMP_DECODER_FIXTURE` /
   `RUSTAMP_DSP_FIXTURE` (`run-tests.ps1:89-102`) and `RUSTAMP_TESTPLUGIN_DSP`
   (`run-tests.ps1:79-87`); the host runner *does* build and stage the i686
   plughost **test** binaries (`vm-run-tests.ps1:102-147`) and the two
   mediahost fixture examples (`vm-run-tests.ps1:62-68, 88-100`). Groups C
   and D now need **no** runner work.
3. **Group A's lane split was mis-stated.** The four winit window tests
   panic in the broker's session-0 PowerShell Direct session
   (`HANDOFF.md:37`); the interactive GUI-session lane is their *primary*
   lane, not a fallback, and the plain pass (which does not leave them out)
   reports them `FAIL`. Details in §3, gap W19-G1.
4. **Line drift** in every table row (ignore/fn pairs and VM-gate lines);
   correction log at the bottom of W12.

---

## 2. Prerequisite table

One row per ignored group. "Staged?" is answered for the **host** staging
trees (`D:\code\rustamp-vm\staging\...`) as read today; guest-side state is
recorded, with the verification named.

| # | Test/group | Prerequisite | Where it lives on the host | Staged? |
|---|---|---|---|---|
| A-1 | `the_equalizer_window_opens_at_its_docked_size_and_position`, `the_main_menu_popup_opens_at_the_measured_client_size`, `the_playlist_window_opens_docked_below_the_main_window`, `the_windowed_player_opens_a_window_inside_the_test_vm` (4 tests) | A desktop: the guest's auto-logon interactive session plus the persistent virtual display (DWM needs a presentable surface); the staged tree; `rustamp.exe` for the spawn test | Display driver zip `D:\code\rustamp-vm\driver-src\VirtualDisplayDriver-x64.zip`, expanded at `D:\code\rustamp-vm\staging\drivers\vdd` | **Ready (recorded).** `HANDOFF.md:44/70` record the session + display provisioned and all four tests passing in the interactive session (2026-10-17). Verify: `assert-interactive-session.ps1` + `assert-attached-display.ps1` (Phase 0). |
| A-2 | `the_new_dialogs_pin_their_os_window_parameters`, `opening_the_playlist_window_needs_an_os_window`, `opening_either_dialog_needs_an_os_window` (3 tests) | The staged tree only; they open no window | — | **Ready.** Compiled and staged with `crates/`. |
| A-3 | `the_menu_popup_opens_at_the_measured_size` (1 test) | The staged tree; it builds a winit window on a harness worker thread (`build_event_loop(true)`) — its lane is unverified (gap W19-G2) | — | **Ready** (staged with `crates/`); the open question is its lane, not its bytes. |
| B | `spawns_the_real_host_without_a_window`, `app_published_state_reaches_the_host_window`, `creates_the_real_host_window_vm_only` (3 tests) | The staged tree plus a resolvable i686 `rustamp-plughost32.exe` beside the test binaries | Built by the host runner from `D:\code\Rustamp` (`vm-run-tests.ps1:102-129`) | **Ready.** The runner builds and stages it on every run; the spawn tests report a loud `BLOCKED` if it is missing, never a vacuous pass. |
| C | `named_pcm_two_process_transfer_and_seek`, `host_exits_mid_stream_and_infinite_read_survives` (pcm), `named_dsp_two_process_gain_round_trip_then_sticky_eos`, `host_exits_mid_stream_and_infinite_read_survives` (dsp) (4 tests) | The staged tree plus `decoder_fixture.exe` + `dsp_fixture.exe` and the guest vars `RUSTAMP_DECODER_FIXTURE` / `RUSTAMP_DSP_FIXTURE` | Built by the host runner from `D:\code\Rustamp` (`vm-run-tests.ps1:62-68, 88-100`) | **Ready.** The runner builds and stages both fixtures; the guest sets both vars (`run-tests.ps1:89-102`). The `expect(...)` in each `fixture()` helper refuses loudly when a var is unset. |
| D | `loads_real_32bit_in_openmpt_dll_and_proves_host_interaction`, `approved_real_dll_maps_headlessly_from_verified_snapshot`, `real_dsp_plugin_loads_initializes_and_changes_audio_through_the_bridge` (3 tests) | The staged tree plus the i686 plughost **test** binaries, `in_openmpt.dll` (with its sibling `openmpt-mpg123.dll`) for the first two, and `dsp\dsp_optilab_core.dll` for the third | `D:\code\rustamp-testplugins` -> staged at `D:\code\rustamp-vm\staging\code\testplugins` | **Ready, verified today.** The staging tree holds `libopenmpt\Winamp\in_openmpt.dll` (1 967 104 bytes) plus the sibling, and `dsp\dsp_optilab_core.dll` whose SHA-256 equals the pinned `78A140C9114B60B361DC7E9A0203DD6D6F23D567E1F309396CD78BAEA7797874` (staged == owner copy). The guest exports `RUSTAMP_TESTPLUGINS` and `RUSTAMP_TESTPLUGIN_DSP` and re-hashes the DLL after the copy. |
| E-1 | `the_real_device_plays_a_reference_file_to_its_end` (1 test) | A real audio endpoint in the guest (WASAPI shared default) plus the committed `sine.flac` | Fixture staged with `crates/` (`crates/rustamp-decoders/tests/fixtures/sine.flac`) | **Ready (recorded).** `HANDOFF.md:69` records the test passing in the guest. The endpoint is the VM's virtual audio path, not hardware. Verify: `check-audio.ps1`. |
| E-2 | `the_cable_carries_the_fixture_tone` (1 test) | The VB-Audio Virtual Cable ("CABLE Input" / "CABLE Output") plus `sine.flac` | Driver pack `D:\code\rustamp-vm\driver-src\VBCABLE_Driver_Pack43.zip`, expanded at `D:\code\rustamp-vm\staging\drivers\vbcable` | **Ready (recorded).** `HANDOFF.md:69` records the cable installed and the test passing. Verify: `check-audio.ps1`. Reinstall path: `install-vbcable.ps1` + `fix-vbcable-once.ps1` (§4). The audio lane needs `-Reset` (§5, Phase 3). |
| F | `corpus_skins_load_without_panicking`, `corpus_skins_render_a_frame_without_panicking`, `compare_sheet_dimensions_against_fixture_when_env_set`, `classic-base` golden (latent) | A skin corpus (`RUSTAMP_SKIN_CORPUS`), `RUSTAMP_TESTSKINS` with `base-2.91.wsz`, `RUSTAMP_TESTDATA` | `D:\code\rustamp-testskins` -> `D:\code\rustamp-vm\staging\code\testskins`; `D:\code\rustamp-testmedia` -> `D:\code\rustamp-vm\staging\code\testmedia` | **Mixed.** testskins and testmedia are staged and the guest sets both vars (`run-tests.ps1:69-74`), so the `rust_theme` dimension comparison can run. The skin **corpus** is staged by no runner: both corpus tests skip by design. `classic-base` has no committed case: nothing to run. |

Also staged today, for completeness: `D:\code\rustamp-vm\staging\drivers`
holds the full driver sources (`vbcable\`, `usbmmidd\`, `vdd\`), and a
duplicate testmedia tree sits directly at `D:\code\rustamp-vm\staging\testmedia`
(same content; the runner's per-run staging target is the one under
`staging\code\`, recreated from `D:\code\rustamp-testmedia` on every run).

---

## 3. The gaps (named exactly)

- **W19-G1 — the four winit binaries are not left out of the plain
  `--ignored` pass.** `run-tests.ps1` leaves out only Group E
  (`run-tests.ps1:240-267`); `eq_window`, `main_menu`, `playlist_window`,
  `headless` (and the `rustamp-app` lib binary, gap G2) run in session 0 and
  the four window tests panic there (`HANDOFF.md:37`). The plain pass is
  **red** on those binaries until the runner gains the same leave-out it
  already has for Group E. Staging cannot fix this; the GUI lane
  (`vm-run-gui-session.ps1`) is the only evidence for the four.
- **W19-G2 — the menu-popup test's lane is unverified.**
  `the_menu_popup_opens_at_the_measured_size` builds a winit window
  (`build_event_loop(true)`, `window.rs:2082`) but is in neither the GUI
  lane's case list (`run-gui-session.ps1:100-105`) nor any run record. If it
  panics in session 0, the `rustamp-app` lib binary is red and the test
  stays unproven; the fix is to add it to the GUI lane's four cases (a
  runner change, out of scope here).
- **W19-G3 — guest baseline state is a guest question.** Whether
  `base-provisioned` still holds the interactive session, the virtual
  display and the VB-Cable cannot be answered from the host. The recorded
  answers (`HANDOFF.md:44/69/70`, 2026-10-16/17) say yes. Phase 0 closes
  it with three read-only assertions; §4 names the remediation.
- **W19-G4 — no host-side VB-Cable installer.** The display has
  `vm-provision-virtual-display-host.ps1`; the cable's reinstall is manual
  broker calls: `copyin` the drivers, guest `install-vbcable.ps1`, reboot,
  guest `fix-vbcable-once.ps1`. `install-vbcable.ps1` throws if
  `C:\rustamp\drivers\vbcable\vbMmeCable64_win7.inf` or
  `deviceinstaller64.exe` is missing from the guest — the drivers must be
  copied in first.
- **W19-G5 — two virtual-display script paths exist.** The signed MttVDD
  path (`install-vdd.ps1`) is the one recorded working (`HANDOFF.md:44`).
  The unsigned usbmmidd path (`provision-virtual-display.ps1` +
  `install-vdisplay.ps1` + host `vm-provision-virtual-display-host.ps1`) is
  recorded as never presenting a monitor on the Windows 11 guest —
  "documented and left installed but unused". Use the MttVDD path.
- **W19-G6 — `-Reset` is destructive and required for the audio lane.**
  The loopback pre-roll check refuses if the cable still carries the
  previous run's audio (`HANDOFF.md:69`). The audio runs therefore carry
  `-Reset`; if the baseline does not contain the cable/display, a reset
  wipes them. Phase 0 must confirm the baseline first; if a remediation
  reinstalled anything into the live state, a later `-Reset` undoes it, so
  the checkpoint question (broker can create, not remove) belongs to the
  owner.
- **W19-G7 — Group F corpus is unstaged by design.** No gap for the
  ignored pass, but the ignored pass does not exercise the skin corpus at
  all, and the `classic-base` golden lane has no committed case.

---

## 4. What the guest scripts already do (one line each)

- `run-tests.ps1` — sets `RUSTAMP_TESTDATA` / `RUSTAMP_TESTSKINS` /
  `RUSTAMP_TESTPLUGINS` / `RUSTAMP_TESTPLUGIN_DSP` (+ hash) /
  `RUSTAMP_DECODER_FIXTURE` / `RUSTAMP_DSP_FIXTURE` when staged, leaves the
  Group E binaries out plainly in a general `--ignored` pass, and bounds
  each binary at 300 s.
- `install-vbcable.ps1` — installs VB-Cable headlessly (trust the catalog
  signer via certutil, `pnputil /add-driver`, device-node create with the
  staged `deviceinstaller64.exe` — never the vendor GUI, which would hang in
  session 0); expects the drivers under `C:\rustamp\drivers`; ends with
  "reboot the guest".
- `fix-vbcable-once.ps1` — removes all VB-CABLE device nodes and installs
  exactly one, so a doubled cable cannot confuse the endpoint lookup;
  same staged-driver expectation; ends with "reboot, then check
  AudioEndpoint".
- `install-vdd.ps1` — installs the **signed** MttVDD virtual display
  driver (vendor-cert trust + `pnputil` + devcon-style `Root\MttVDD`
  create) and reports the display; the path recorded working in the guest.
- `provision-virtual-display.ps1` (guest) +
  `vm-provision-virtual-display-host.ps1` (host) — the **unsigned**
  usbmmidd IDD path: test-signing phase + install phase + monitor attach +
  session/display proofs + checkpoint re-take; recorded as never presenting
  a monitor on the Windows 11 guest (gap W19-G5).
- `install-vdisplay.ps1` — creates the usbmmidd IDD device and enables the
  indirect display (`deviceinstaller64 enableidd 1`); part of the unused
  usbmmidd path.
- `set-vdd-mode.ps1` — pins the MttVDD monitor to 1024x768 (or off) for
  capture runs; self-verifying.
- `assert-interactive-session.ps1` — proves `explorer.exe` runs in a
  session above zero with a non-empty virtual screen (the desktop claim).
- `assert-attached-display.ps1` — proves a persistent attached display
  (`ATTACHED_TO_DESKTOP` + a primary screen + DWM composition enabled).
- `check-audio.ps1` — read-only: lists audio endpoints, whether the
  VB-CABLE driver is in the store, and whether setup processes are alive.
- `vm-run-gui-session.ps1` (host) — compiles, stages, runs the four winit
  tests in the interactive session via a one-shot scheduled task, copies
  the session proof and per-test logs back to
  `D:\code\rustamp-vm\staging\out\gui-session`.

---

## 5. The command sequence (one session)

All commands run from the repo root. Phase 0 first: it is what turns "the
record says so" into "the guest says so", and it leaves the guest in a
known-fresh state.

```powershell
# --- Phase 0: baseline verification (one broker round) ----------------------
powershell -ExecutionPolicy Bypass -File docs\notes\vm-broker\invoke-broker.ps1 -Op restore -Name base-provisioned -TimeoutSeconds 900
powershell -ExecutionPolicy Bypass -File docs\notes\vm-broker\invoke-broker.ps1 -Op guest -Script assert-interactive-session.ps1 -TimeoutSeconds 600
powershell -ExecutionPolicy Bypass -File docs\notes\vm-broker\invoke-broker.ps1 -Op guest -Script assert-attached-display.ps1 -TimeoutSeconds 600
powershell -ExecutionPolicy Bypass -File docs\notes\vm-broker\invoke-broker.ps1 -Op guest -Script check-audio.ps1 -TimeoutSeconds 600
```

All three assertions must be green. Remediation if one is not (gap
W19-G3/G4/G5): `copyin` `D:\code\rustamp-vm\staging\drivers` ->
`C:\rustamp\drivers` if the drivers are missing from the guest, then
`install-vdd.ps1` (display) or `install-vbcable.ps1` (cable), a broker
stop/start reboot, then the assertion again; `fix-vbcable-once.ps1` after
a cable install. If a remediation reinstalled anything, remember gap
W19-G6: a later `-Reset` undoes it.

```powershell
# --- Phase 1: GUI lane — the four winit window tests -----------------------
powershell -ExecutionPolicy Bypass -File docs\notes\vm-run-gui-session.ps1
# proof + per-test logs: D:\code\rustamp-vm\staging\out\gui-session
# 0 failures expected; the proof names the session, user and screen

# --- Phase 2: headless ignored pass -----------------------------------------
powershell -ExecutionPolicy Bypass -File docs\notes\vm-run-tests.ps1 -Filter '--ignored'
# the -Filter form the runner accepts: '--ignored' (general) or
# '--ignored:<binary-name>' (per binary); pass it as ONE positional arg
# (vm-run-tests.ps1:268-270 — an array would bind only its first element)
# runs: Groups B, C, D + the four non-winit Group A tests (14 tests)
# expected red: eq_window / main_menu / playlist_window / headless binaries
# (session-0 winit panic — gap W19-G1, not a defect); Group E left out
# plainly; the W12 inventory summary prints the 20-entry status

# --- Phase 3: audio lane (Group E) — -Reset is mandatory --------------------
powershell -ExecutionPolicy Bypass -File docs\notes\vm-run-tests.ps1 -Reset -Filter '--ignored:real_playback'
powershell -ExecutionPolicy Bypass -File docs\notes\vm-run-tests.ps1 -Reset -Filter '--ignored:loopback'
```

**Where the VB-Cable install sits, and why:** in Phase 0, before any test
run. (a) `loopback` fails loudly without the cable — its `MISSING CABLE`
panic is by design — so the cable is a hard prerequisite, not a nice-to-have.
(b) The install needs a reboot before the endpoints enumerate, and a reboot
would disturb a running pass. (c) Phase 3 runs with `-Reset`, which
restores the baseline — so the cable must be *in the baseline* (or
reinstalled after each reset) before Phase 3; Phase 0 is what establishes
which of those is true.

---

## 6. What forces the split

The four winit window tests panic in session 0 — the broker's PowerShell
Direct session has no desktop (`HANDOFF.md:37`; `run-gui-session.ps1:7-11`)
— so they need the interactive-session lane, which is a **different
runner** (a scheduled task as the auto-logon user, `vm-run-gui-session.ps1`).
That is one forced split: GUI lane and headless pass are two commands by
design. The audio tests add two more filtered commands because they need
`-Reset` (the cable otherwise carries the previous run's tone into the
pre-roll check, `HANDOFF.md:69`). Nothing forces a **second session**:
Phases 0-3 are sequential commands in one sitting, with no reboot unless
Phase 0 finds something missing.

---

## 7. The honest limits — what the ignored pass will still not prove

1. **The four winit tests stay red in the plain pass** until `run-tests.ps1`
   gains the leave-out they need (gap W19-G1); only the GUI lane evidences
   them. A green-looking `--ignored` run that includes them is impossible
   today.
2. **The menu-popup test may stay unproven** if it panics in session 0
   (gap W19-G2).
3. **Group E proves the device path opens, advances and drains, and the
   tone crosses the cable** — not audio quality, not hardware, not
   device-loss recovery (the BETA-05 residual stands; the endpoint is
   virtual).
4. **Group D proves mapping plus a few ABI calls on two DLLs** — not a
   sandbox (approved bytes still run `DllMain`), not the config dialog,
   "changed" not "correct" (P8 §5), and no user-facing plugin
   selection/config wiring.
5. **Group F still skips** (no corpus staged, no `classic-base` cases) —
   the ignored pass does not touch the skin corpus at all.
6. **Group C proves the wire protocol, not the packaged beta** — the
   artifact itself has never been launched (BETA-readiness-2 §2k;
   RELEASE-plan steps 8-13 remain).
7. **Green today is not green at the release commit** — the ignored pass
   must be re-run at the release commit (RELEASE-plan steps 4-5).
8. **An inventory line is not a pass** — read each binary's own result
   (`RELEASE-plan` step 5's "It does not prove" column).
9. **Nothing proves non-Windows** — the guest-only suites carry
   `#![cfg(windows)]` (W12 footnote).

---

## 8. Boundary and verification receipt

- Read and written: only this worktree; the files touched are this card and
  `docs/tasks/W12-ignored-inventory.md`.
- **No Rust gate was run, deliberately:** the task is documentation and the
  orchestrator needs the host free for the suite run. No `cargo` command,
  no test binary, example, player, VM job, window or audio device was
  touched; no download, no new dependency, no commit, no git write.
- Host reads only: the four staging trees, `D:\code\rustamp-vm\driver-src`,
  and the repo's own `docs/` and `crates/` (test names, ignore attributes,
  gate lines) inside this worktree. Nothing under `D:\code\Rustamp`,
  `D:\code\rustamp-research` or `D:\code\rustamp-refbin` was written; no
  Winamp source, SDK, disassembly or forbidden-tree contents were read; no
  plugin or reference binary was loaded or executed.
- `git status --short` (this worktree, taken after writing — pasted in the
  lane report) shows exactly these two cards and nothing else.
