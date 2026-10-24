# X4 — Staging the Ignored and Gated Tests for Beta

**Date:** 2026-10-17. **Status:** runner and guest staging gaps closed; ready for one guest `--ignored` run.
**Scope:** implementation of the staging and environment configuration identified in
`docs/tasks/W12-ignored-inventory.md`.

---

## 1. Summary of closed gaps and runner work

W12 identified 19 `#[ignore]`d tests and 4 environment-gated sites. This task closes the
runner and staging gaps for the 17 tests a beta needs and can execute headlessly:

| Group | Tests | What was missing | How closed |
|---|---|---|---|
| **Group A** (VM GUI) | 8 tests | None (staged with repo) | Executed by `run-tests.ps1` when called with `--ignored`. Real Win32 windows created headlessly. |
| **Group B** (VM real process) | 3 tests | Resolvable `rustamp-plughost32.exe` | Staged beside test binaries in `target\debug\deps` and `target\debug`. Supervisor resolves eagerly. |
| **Group C** (Two-process resident fixtures) | 4 tests | `decoder_fixture.exe` and `dsp_fixture.exe` binaries not built/staged; `RUSTAMP_DECODER_FIXTURE` and `RUSTAMP_DSP_FIXTURE` unset | Host runner builds `cargo build --offline -p rustamp-mediahost --example decoder_fixture --example dsp_fixture` and stages both to `target\debug\examples` and `target\debug`; guest runner sets `RUSTAMP_DECODER_FIXTURE` and `RUSTAMP_DSP_FIXTURE` to absolute paths. |
| **Group D** (i686 plugin tests + approved DLL) | 2 tests | `rustamp-plughost` test binaries not compiled for i686; `D:\code\rustamp-testplugins` not staged; `RUSTAMP_TESTPLUGINS` unset | Host runner builds i686 test binaries (`cargo test -p rustamp-plughost --all-targets --all-features --no-run --target i686-pc-windows-msvc`) with static CRT (`-C target-feature=+crt-static`) and stages them into `target\debug\deps`; excludes x86_64 plughost test binaries; stages `D:\code\rustamp-testplugins` into guest `C:\rustamp\testplugins`; guest runner sets `RUSTAMP_TESTPLUGINS=C:\rustamp\testplugins`. |
| **Group E** (Real audio endpoint) | 2 tests | Physical/virtual audio endpoint in VM | **Left out plainly**: `real_playback` requires a real WASAPI default endpoint (Hyper-V audio redirection decision); `loopback` requires VB-Audio Virtual Cable installed. In an `--ignored` suite pass, both are marked `LEFT OUT` with diagnostic rather than faking an endpoint or failing. Direct invocation (`--ignored:real_playback`, `--ignored:loopback`) remains enabled. |
| **Group F** (Environment-gated skips) | 3 tests + 1 latent | Skin corpus / testskins / golden test cases | `D:\code\rustamp-testskins` staged when present on host (`RUSTAMP_TESTSKINS`). Free skin corpus is not staged (tests continue to skip safely). Golden `classic-base` has no committed cases yet. |

---

## 2. Changes to runners

### Host runner (`docs/notes/vm-run-tests.ps1`)
1. **Filter x86_64 plughost tests:** In `cargo test --workspace --no-run`, skip collecting `rustamp-plughost` test binaries for the x86_64 target (RFC 0006 §8: plughost is an i686 crate; 32-bit DLL mapping panics by design on x86_64).
2. **Build mediahost fixture examples:** Run `cargo build --offline -p rustamp-mediahost --example decoder_fixture --example dsp_fixture`.
3. **Stage mediahost fixture examples:** Copy `decoder_fixture.exe` and `dsp_fixture.exe` to `$Stage\target\debug\examples` and `$Stage\target\debug`. Copy CRT DLLs to `target\debug\examples`.
4. **Build and stage i686 plughost test binaries:** Run `cargo test -p rustamp-plughost --all-targets --all-features --no-run --target i686-pc-windows-msvc` with static CRT (`-C target-feature=+crt-static`). Copy resulting test executables (`rustamp_plughost-*.exe` and `dll_acceptance-*.exe`) into `$Stage\target\debug\deps` and `$Stage\target\i686-pc-windows-msvc\debug\deps`.
5. **Stage test plugins directory:** Copy `D:\code\rustamp-testplugins` to staging and copyin to `C:\rustamp\testplugins` over broker, matching the contract of `testmedia` and `testskins`.
6. **Update guest timeout calculation:** Include both x86_64 and i686 test binaries in `$allBinariesCount`.
7. **Ignored tests execution summary:** When `-Filter` contains `--ignored`, parse passed and failed test names across all result objects and print a structured summary of all 19 inventory tests:
   - `[PASS]` / `[FAIL]` for executed ignored tests.
   - `[BLOCKED]` for Group E tests naming the required host/VM decision.
   - Aggregate counts (ran, blocked, total).

### Guest runner (`docs/notes/vm-broker/guest/run-tests.ps1`)
1. **Set `RUSTAMP_TESTPLUGINS`:** Export `C:\rustamp\testplugins` when present.
2. **Set `RUSTAMP_DECODER_FIXTURE` and `RUSTAMP_DSP_FIXTURE`:** Locate staged `decoder_fixture.exe` and `dsp_fixture.exe` and export their absolute paths.
3. **Skip daemon helpers:** Prevent `Get-ChildItem` in `target\debug\deps` from attempting to execute `rustamp-plughost32.exe` or `rustamp.exe` as test suites.
4. **Group E plain leave-out:** In a general `--ignored` run, record `real_playback` and `loopback` as `LEFT OUT: requires VM audio endpoint decision` (exit 0) rather than failing the run.
5. **Capture passed tests:** Extract individual passing test names (`test <name> ... ok`) alongside failing test names into the binary result object.

---

## 3. What one guest run now executes

When the orchestrator runs:
```powershell
powershell -ExecutionPolicy Bypass -File docs\notes\vm-run-tests.ps1 -Filter '--ignored'
```

The guest executes **17 ignored tests** across 4 groups:
- **Group A (all 8):**
  1. `the_menu_popup_opens_at_the_measured_size` (`rustamp-app`)
  2. `the_windowed_player_opens_a_window_inside_the_test_vm` (`rustamp-app::headless`)
  3. `the_equalizer_window_opens_at_its_docked_size_and_position` (`rustamp-app::eq_window`)
  4. `the_main_menu_popup_opens_at_the_measured_client_size` (`rustamp-app::main_menu`)
  5. `the_playlist_window_opens_docked_below_the_main_window` (`rustamp-app::playlist_window`)
  6. `the_new_dialogs_pin_their_os_window_parameters` (`rustamp-app`)
  7. `opening_the_playlist_window_needs_an_os_window` (`rustamp-app::playlist_window`)
  8. `opening_either_dialog_needs_an_os_window` (`rustamp-ui::playlist`)
- **Group B (all 3):**
  9. `spawns_the_real_host_without_a_window` (`rustamp-app::supervise`)
  10. `app_published_state_reaches_the_host_window` (`rustamp-app::plughost_e2e`)
  11. `creates_the_real_host_window_vm_only` (`rustamp-plughost::control`)
- **Group C (all 4):**
  12. `named_pcm_two_process_transfer_and_seek` (`rustamp-mediahost::pcm_process`)
  13. `host_exits_mid_stream_and_infinite_read_survives` (`rustamp-mediahost::pcm_process`)
  14. `named_dsp_two_process_gain_round_trip_then_sticky_eos` (`rustamp-mediahost::dsp_process`)
  15. `host_exits_mid_stream_and_infinite_read_survives` (`rustamp-mediahost::dsp_process`)
- **Group D (all 2):**
  16. `loads_real_32bit_in_openmpt_dll_and_proves_host_interaction` (`rustamp-plughost::dll_acceptance`, i686)
  17. `approved_real_dll_maps_headlessly_from_verified_snapshot` (`rustamp-plughost::loader`, i686)

**Left out plainly (Group E, 2 tests):**
- `the_real_device_plays_a_reference_file_to_its_end` (`rustamp-app::real_playback`): needs Hyper-V audio endpoint redirection or virtual WASAPI device.
- `the_cable_carries_the_fixture_tone` (`rustamp-app::loopback`): needs VB-Audio Virtual Cable installation in VM.

---

## 4. Verification and boundary receipt

- **PowerShell syntax verification:**
  `[System.Management.Automation.Language.Parser]::ParseFile` on both `docs\notes\vm-run-tests.ps1` and `docs\notes\vm-broker\guest\run-tests.ps1`: 0 errors.
- **Cargo formatting:** `cargo fmt --all -- --check` passed cleanly.
- **Cargo clippy:** `cargo clippy --workspace --all-targets --all-features -- -D warnings` passed cleanly (exit 0).
- **Cargo build:** `cargo build --workspace --offline` and `cargo build -p rustamp-plughost --target i686-pc-windows-msvc --offline` passed cleanly.
- **No tests, no VM, no window, no audio, no git commit:** Confirmed.
