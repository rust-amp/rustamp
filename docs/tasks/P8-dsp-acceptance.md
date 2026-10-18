# P8 — Real-DLL acceptance for the classic DSP plugin bridge

## 1. Status and scope

- **Status**: Staged and written, compile-checked only. **No runtime pass is
  claimed.** The test was not run, no library was loaded, no VM was started on
  this lane; the orchestrator runs it (section 6).
- **Boundary**: owned paths only (`crates/rustamp-plughost/**`,
  `docs/notes/vm-run-tests.ps1`, `docs/notes/vm-broker/guest/run-tests.ps1`,
  this card). No other crate, no smoke runner, no broker script, no dependency,
  no git write. Nothing under `D:\code\Rustamp`, `D:\code\rustamp-research` or
  `D:\code\rustamp-refbin` was touched. No SDK, Winamp source or disassembly was
  read. The fixture's bytes were read once as a plain file to list its printable
  strings (name, description, import DLL names); it was not loaded or executed.
- **Closes**: the "What stays unproven without a DLL" list of P6 §5, as far as a
  headless i686 run can (section 5 says what it still cannot).

## 2. The fixture: origin and licence (for the release paperwork)

| Field | Value |
|---|---|
| File | `dsp_optilab_core.dll` |
| Host path | `D:\code\rustamp-testplugins\dsp\dsp_optilab_core.dll` |
| Size | 289 792 bytes (283 KB) |
| SHA-256 | `78A140C9114B60B361DC7E9A0203DD6D6F23D567E1F309396CD78BAEA7797874` |
| Product | OptiLab Core 1.4.0 (description strings: `OptiLab Core 1.4.0`, `OptiLab Core DSP/Effect`) |
| Origin | github.com/dgl1984/optilab, release v1.4.0 (as stated by the owner who supplied it) |
| Licence | Apache-2.0 (as stated by the owner; the upstream licence text was not re-read on this lane) |
| Kind | 32-bit classic Winamp DSP plugin; exports `winampDSPGetHeader2`; imports ADVAPI32, COMCTL32, USER32, KERNEL32 |
| Use here | Test-only fixture, never shipped; mapped only inside the Hyper-V test VM through the P4/P5 verified-bytes loader |

Origin and licence are recorded as supplied by the owner; confirm them against
the upstream release before they go into `THIRD_PARTY.md` / the release notes.
No `THIRD_PARTY.md` change was made (a test fixture, not a dependency).

## 3. Staging (read-only copy plus environment variable)

The runner already copies `D:\code\rustamp-testplugins` wholesale into the guest
at `C:\rustamp\testplugins` (X8), so the DLL arrives at
`C:\rustamp\testplugins\dsp\dsp_optilab_core.dll` with no new copy mechanism.
Added on top, so a skipped test can never look like a pass:

- **Guest** (`docs/notes/vm-broker/guest/run-tests.ps1`): when that file exists,
  exports `RUSTAMP_TESTPLUGIN_DSP=<file>`, hashes it (SHA-256) and returns
  `testplugindsp` and `testplugindspsha256` in the result object (`$null` when
  absent: guest ground truth).
- **Host** (`docs/notes/vm-run-tests.ps1`): hashes the host file before staging
  and, after the run, prints exactly one `dsp plugin fixture:` line on **every**
  run, whatever the filter:
  - `staged and verified (guest RUSTAMP_TESTPLUGIN_DSP=..., SHA-256 ...)` — the
    only good state: guest copy hashes to the pinned value;
  - `NOT STAGED (...)`, `WRONG FILE on the host (...)`, `staged on the host, but
    the guest reported no RUSTAMP_TESTPLUGIN_DSP value`, or `guest copy has
    SHA-256 X, not the pinned Y` — each states the test proves nothing.
- **`--ignored` summary**: the new Group D inventory entry
  `real_dsp_plugin_loads_initializes_and_changes_audio_through_the_bridge` prints
  `[PASS]`/`[FAIL]`, or `[MISSING FIXTURE]` (counted separately) when the DSP
  fixture is not staged and verified, never a quiet `[NOT RUN]`.
- The test itself resolves the file from `RUSTAMP_TESTPLUGIN_DSP`, then
  `RUSTAMP_TESTPLUGINS\dsp\`, then `C:\rustamp\testplugins\dsp\`, then the host
  path, and **panics** (REFUSING LOUDLY) if none exists. It also re-pins size and
  SHA-256 itself before mapping anything.

## 4. The acceptance test

`crates/rustamp-plughost/tests/dsp_acceptance.rs`:
`real_dsp_plugin_loads_initializes_and_changes_audio_through_the_bridge`
(`#[ignore]`, VM-gated by `require_test_vm()` and an i686 check, like
`dll_acceptance.rs`). Steps and what each assertion proves:

1. Size and SHA-256 of the staged file equal the pinned constants.
2. **Failure mode, approval**: one flipped byte in the approved image →
   `load_plugin` returns `PluginLoadError::NotApproved` before any mapping (no
   DllMain can run).
3. **Failure mode, preflight/store**: `ApprovalStore::approve_bytes` accepts the
   fixture as a structurally valid i386 DSP export, then
   `load_plugin_with_store` loads it (verified-bytes snapshot, P4/P5/P7 path).
4. **Export resolves, header and module live**: `kind() == Dsp` (so
   `winampDSPGetHeader2` was resolved and the getter called), description read
   from plugin memory is non-empty and printed.
5. **Plugin configured**: `is_initialized()` (init returned 0, otherwise
   `InitFailed` would have been returned); `dsp_info()` shows `init`/`process`/
   `quit` callbacks present, `parent_window == window.hwnd_raw()`,
   `dll_instance == hinstance()` (host fields filled before `init`), and
   `host_active` (the host `dsp_is_active` callback answers 1 for exactly this
   module).
6. **Callback driven, audio changed**: 16 blocks of 1152 stereo 16-bit frames at
   44.1 kHz (440 Hz left, 660 Hz right, deterministic) go through
   `LoadedPlugin::dsp_process_i16`, which calls the host `dsp_process` callback,
   the very path an input plugin's decode loop uses (buffer grown to 2x, in
   place). Per block: returned frame count within half..double of input, and the
   buffer length equals the returned count. Then the output is compared to the
   unprocessed copy: differing sample count, max absolute difference, RMS in/out
   and frame counts are printed as one `DSP_MEASUREMENT` line, and the test
   asserts `differing > 0 || frames_out != frames_in`.
7. **Edge**: an empty buffer returns 0 frames and stays empty (passthrough, plugin
   not called).
8. **Quit**: `quit()` runs, `is_initialized()` false, `dsp_info().host_active`
   false, and `dsp_process_i16` returns `None` afterwards.

Supporting code added (`crates/rustamp-plughost/src/loader.rs`): public
`LoadedPlugin::dsp_info()` + `DspModuleInfo` and
`LoadedPlugin::dsp_process_i16()`, re-exported `DspModuleInfo` in `lib.rs`.
They are thin wrappers over the existing `host_dsp_is_active` /
`host_dsp_process` callbacks and the module struct; the bridge's behaviour is
unchanged. The existing synthetic P6 tests were not touched.

## 5. What the plugin makes impossible or unproven (read before trusting a pass)

- **Config dialog not driven.** A DSP's `config` callback opens a dialog; this
  run is windowless apart from the host window and no one is there to dismiss
  it. `has_config` is not even asserted. "Configured" therefore means
  *initialized in its default state*, not "settings applied". Whether the plugin
  persists settings (registry/ini) was not examined.
- **The effect in the default state is not known in advance.** The lane could not
  load the DLL, so whether OptiLab Core in its default state alters audio (as
  opposed to being inert until the user enables a module or preset in its
  dialog) is unknown. The test is written to demand a measurable change. If it
  fails with the message beginning `FINDING: the plugin ran but returned the
  audio bit-identical`, the correct reading is: export, init, host fields,
  activation and the `process` drive are proven (everything before assertion 6
  passed), the audible effect is not, and it cannot be proven headlessly without
  a way to configure it. Do not weaken the assertion; record the finding and
  decide whether a config-driven follow-up is wanted.
- **"Changed" is not "correct".** The test pins that the output differs from the
  input, with the measurement printed. It does not pin what the effect should be
  (no reference output exists, and the plugin's internal DSP is not specified).
- **Length-range violations are invisible by design.** The bridge falls back to
  the input count when a plugin reports a count outside half..double (P6 §4), so
  a lying plugin looks like an unchanged length. The test can only assert the
  returned count is in range; it cannot tell a genuine in-range answer from the
  fallback. Likewise crash and wedge are unhandled limitations (P6 §4): a crash
  takes the 32-bit test process down (visible as a failed binary in the run), a
  wedge shows as a hang caught by the runner's per-binary timeout, not by the
  test.
- **`init` refusal is not exercisable on this DLL** (it returns 0); that path
  stays covered only by the synthetic P6 test. The approval-mismatch refusal is
  the real-DLL failure mode proven instead.
- **Not through a real input plugin.** The drive uses the host callback directly
  with a synthetic signal; chaining it inside `in_openmpt.dll`'s decode loop is
  not attempted. Only 16-bit stereo 44.1 kHz was tried.
- **DLL is retained after quit** (existing policy); free-after-quit safety is
  still unproven.

## 6. Command and what a pass proves

Orchestrator, one VM pass for just this test binary (the guest filter after
`--ignored:` selects the binary by name; the test name is the exact one above):

```
powershell -ExecutionPolicy Bypass -File docs\notes\vm-run-tests.ps1 -Filter '--ignored:dsp_acceptance'
```

(`-Filter '--ignored'` runs every ignored test and prints the same fixture lines
plus the W12 summary.) Read, in this order:

1. the `dsp plugin fixture:` line must say `staged and verified` — anything else
   means the run proves nothing;
2. `[ok  ] dsp_acceptance-<hash>.exe` and, with `--ignored`, `[PASS] Group D ...
   real_dsp_plugin_loads_initializes_and_changes_audio_through_the_bridge`;
3. the printed `DSP description:`, `DSP module info:` and `DSP_MEASUREMENT` lines
   in the diagnostics (run the binary with `--nocapture` if they are needed and
   the runner swallows passing output).

A **pass** proves: a real third-party 32-bit classic DSP DLL, identified by
pinned SHA-256, maps through the verified-bytes loader (a tampered image is
refused first), its `winampDSPGetHeader2` resolves and returns a valid 0x20
header and module, `init` returns 0 with the host fields filled, the host
`dsp_is_active`/`dsp_process` callbacks drive its `process` on the calling
thread with an in-range frame count, the audio comes back measurably different
from the input, and `quit` deactivates it. It does **not** prove audio quality,
the config dialog, input-plugin chaining, crash/wedge handling or the effect
being correct (section 5).

## 7. Files and line counts

`splitlines` counts after the change:

| File | Lines | Change |
|---|---|---|
| `crates/rustamp-plughost/tests/dsp_acceptance.rs` | 274 | new acceptance test |
| `crates/rustamp-plughost/src/loader.rs` | 1607 | `dsp_info`, `dsp_process_i16`, `DspModuleInfo` (was 1517) |
| `crates/rustamp-plughost/src/lib.rs` | 33 | re-export `DspModuleInfo` (was 31) |
| `docs/notes/vm-run-tests.ps1` | 413 | DSP fixture hash, report line, inventory entry (was 375) |
| `docs/notes/vm-broker/guest/run-tests.ps1` | 382 | `RUSTAMP_TESTPLUGIN_DSP`, hash, result fields (was 370) |
| `docs/tasks/P8-dsp-acceptance.md` | this card | new |

## 8. Build results (run in this worktree)

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | exit 0 |
| `cargo clippy --workspace --all-targets --all-features --offline -- -D warnings` | exit 0 (two findings fixed on the way: `is_multiple_of`, `RangeInclusive::contains`) |
| `cargo build --workspace --offline` | exit 0 |
| PowerShell parser on both runner scripts | 0 errors each |

These are default-target (x86_64 host) compile checks; the test body is only
meaningful as an i686 binary in the VM.

## 9. Verification checklist

- [x] No test was run (`cargo test` never invoked)
- [x] No library was loaded on this host (the fixture was read as bytes only)
- [x] No VM or broker was started
- [x] No window was opened, no audio played
- [x] No download, no new dependency, no `cargo add`
- [x] No commit, no git write (`git status --short` is the receipt)
- [x] Only owned paths changed
