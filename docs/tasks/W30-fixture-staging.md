# W30 — Required acceptance fixture staging

## Scope and inputs

Own `docs/notes/vm-run-tests.ps1`, `docs/notes/vm-broker/guest/**`, and this card.
Use the clean-room acceptance tests, their fixture manifests, and the existing
media/skin/plugin staging workflow. No crate changes, dependencies, downloads,
reference execution, git writes, or guest suite execution.

## Contract

Build the seven plughost cdylib examples explicitly for i686 with the host's
static-CRT flags, and copy them beside both sets of staged test executables.
Missing builds/files must stop staging, not produce warnings or optional tests.
Keep the existing receipt/inventory mechanism and invalidate old receipts.

The base291 ignored comparator needs eleven real `.presented.png` /
`.reference.png` pairs plus `session-proof.json`. Copy an operator-supplied
capture directory through the same staging/broker path as other external
fixtures and export `RUSTAMP_BASE291_VM_CAPTURES` in the guest. Never fabricate
captures, reference pixels, or session proof. Refuse before guest operations if
a selected ignored acceptance test has missing required external fixtures.

## Acceptance and evidence

Record the exact fixture-to-test mapping, host gates (exit codes captured
immediately), Windows PowerShell 5.1 parser results, and next guest-run checks
below. Host checks do not establish that a GUI/plugin acceptance test passes.

## Findings and changes

The old runner relied on `cargo test --all-targets --no-run` to produce seven
loadable example cdylibs. That is not an explicit example DLL build. Each
missing DLL merely printed a warning, and staging could still get a valid
receipt. All seven now have explicit offline `cargo build --example ...
--target i686-pc-windows-msvc` invocations inside the same static-CRT
`RUSTFLAGS` scope as `rustamp-plughost32.exe`. Exit codes are captured on the
next statement. An unsuccessful build, missing/empty artifact, invalid PE or
non-i686 machine field throws. `finally` restores flags/error preference.

The existing layout is retained: DLLs sit in both `target/debug/deps` and
`target/i686-pc-windows-msvc/debug/deps`, where the acceptance finders look
beside their current executable. Copies are mandatory and SHA-256 compared.
Resident example executables and the real decoder resident also change from
optional copies/warnings to required, hash-checked copies. Empty i686 test
artifact selection now throws. Receipt version 3 invalidates version 2 stages;
the host runner script joins the existing hashed input inventory. `-SkipBuild`
still verifies the complete receipt and now explicitly checks required DLLs
and their architecture too. Rebuild once without `-SkipBuild` after merging.

The guest already exports `RUSTAMP_DSP_FIXTURE` for the mediahost resident EXE.
`dsp_fixture_acceptance` also reads that name, accepting any existing file
before searching for its own DLL. The child process environment now overrides
it with `dsp_rustamp_fixture.dll` **only for that acceptance binary**, preserving
the executable for `dsp_process` and other resident consumers.

### Refusing tests and exact requirements

All DLL names below are i686. Test functions reside in the named acceptance
binary under `crates/rustamp-plughost/tests/`.

| Binary | Refusing test | Required artifact(s) |
| --- | --- | --- |
| `gen_acceptance` | `a_real_general_purpose_plugin_is_loaded_answers_from_its_own_code_and_quits_cleanly` | `gen_rustamp_fixture.dll` |
| `control_load_acceptance` | `the_real_host_process_loads_a_general_purpose_dll_and_quits_it_cleanly` | `gen_rustamp_fixture.dll`, `rustamp-plughost32.exe` |
| `out_acceptance` | `a_real_output_plugin_is_loaded_gets_its_host_fields_and_quits_cleanly` | `out_rustamp_fixture.dll` |
| `in_acceptance` | `a_real_input_plugin_is_loaded_filled_plays_and_quits_cleanly` | `in_rustamp_fixture.dll` |
| `handoff_acceptance` | `a_real_input_plugin_hands_its_audio_to_the_selected_output_plugin` | `in_handoff_rustamp_fixture.dll`, `out_rustamp_fixture.dll` |
| `vis_acceptance` | `a_real_visualization_plugin_is_loaded_listed_started_drawn_and_quit_cleanly` | `vis_rustamp_fixture.dll` |
| `dsp_fixture_acceptance` | `a_real_dsp_plugin_is_loaded_listed_started_processed_and_quit_cleanly` | `dsp_rustamp_fixture.dll`, per-child environment override above |
| `fault_acceptance` | `a_faulting_plugin_ends_the_host_with_the_fault_code_and_a_report` | `crash_rustamp_fixture.dll`, `rustamp-plughost32.exe` |
| UI `base291_acceptance` | `base291_vm_presented_pixels_match_reference_captures` | `RUSTAMP_BASE291_VM_CAPTURES`, `session-proof.json`, 11 real presented/reference pairs |

Existing real-plugin requirements are preserved, not newly supplied or
downloaded: `dll_acceptance::loads_real_32bit_in_openmpt_dll_and_proves_host_interaction`,
`control_load_acceptance::the_real_host_process_loads_an_approved_dll_through_the_control_channel`,
and the plughost lib test `approved_real_dll_maps_headlessly_from_verified_snapshot`
need the approved `in_openmpt.dll` tree. `dsp_acceptance::real_dsp_plugin_loads_initializes_and_changes_audio_through_the_bridge`
needs `dsp/dsp_optilab_core.dll` with the existing pinned SHA-256. The runner
checks these before guest operations when their ignored binary is selected;
the guest checks presence/hash again before launching any test. The base291
nonignored suite similarly requires staged `testskins/base-2.91.wsz` and the
guest explicitly exports its staged path.

### Capture staging and remaining operator input

`-Base291Captures <directory>` names the real capture source; its default is
`D:/code/rustamp-testcaptures/base291`. The directory must contain nonempty:

- `session-proof.json` from the capture session (not the next comparator run);
- `<case>.presented.png` and `<case>.reference.png` for `main-active`,
  `main-inactive`, `main-shade`, `main-double`, `eq-active`, `eq-inactive`,
  `eq-shade`, `eq-double`, `playlist-active`, `playlist-inactive`,
  `playlist-shade`.

Follow the existing external media/skin/plugin mechanism: read-only wholesale
host copy into the sibling staging directory `base291-captures`, then broker
`copyin` to `C:/rustamp/base291-captures`. Host source/stage inventories must
match. The guest validates all 23 files, sets the exact environment variable,
and returns `base291captures`; the host demands that confirmation. This does
not attest capture provenance beyond copying supplied evidence unchanged;
actual session/state/skin/DPI approval remains the operator's responsibility.

Only an ignored selection containing `base291_acceptance` needs captures.
Normal suites and unrelated targeted ignored runs do not gain that dependency.
No matching capture bundle was found under the existing `D:/code/rustamp-test*`
fixture roots. **The ninth test remains blocked until the orchestrator supplies
real approved captures.** A full `--ignored` run now fails before VM operations
with the first missing capture filename instead of running a suite full of
refusals. No crate change can legitimately manufacture those observations.

Eight own-plugin test cases in eight binaries should now reach actual plugin
loading instead of missing-fixture refusal. With a complete capture bundle,
the ninth test can execute its actual comparison; it may legitimately find
pixel differences. No acceptance assertions or ignore flags were changed.

## Next guest run (orchestrator only)

1. Run once **without `-SkipBuild`** to replace the old receipt. Look for seven
   `building required i686 cdylib ...` and seven `staged required i686 ...dll
   beside both sets of binaries` messages. Any missing artifact must stop here.
2. Targeted `--ignored:gen_acceptance`, `--ignored:out_acceptance`,
   `--ignored:in_acceptance`, `--ignored:handoff_acceptance`,
   `--ignored:vis_acceptance`, `--ignored:dsp_fixture_acceptance`,
   `--ignored:fault_acceptance`, and `--ignored:control_load_acceptance` runs
   should no longer report missing own-fixture DLLs. The control binary also
   needs the existing approved openmpt tree. DSP must load the DLL, not the
   mediahost EXE; `--ignored:dsp_process` must still spawn its resident EXE.
3. Provide the real capture directory with `-Base291Captures`, then use
   `-Filter '--ignored:base291_acceptance'` (or full `--ignored`). Expect
   `base291 captures: staged and guest-confirmed`, the exact guest environment
   path, and an actual pixel comparison rather than `VM capture directory
   required`. Missing proof or either side of any pair must stop before tests.
4. Use the existing interactive-session proof and per-binary logs; failures
   after load/comparison are product/evidence findings, not staging success.

## Verification receipts

- Windows PowerShell 5.1 helper/production-loop self-test:
  `powershell -NoProfile -File docs/notes/vm-broker/guest/test-fixture-staging.ps1`:
  **37 checks, exit 0**. Uses isolated synthetic PE/sentinel files only; tests
  explicit seven builds, both seven-DLL copies, build failure, missing artifact,
  empty/missing/wrong-architecture files, capture completeness/inventories,
  filter scoping, source/staging overlap guards and per-child DSP environment.
  Never invokes Cargo, broker,
  OS DLL loading, or a guest/GUI suite.
- Actual offline build of all seven examples for `i686-pc-windows-msvc` with
  `RUSTFLAGS=-C target-feature=+crt-static`: **exit 0**, no DLL loaded.
- The production staging loop was also executed against those seven actual
  built DLLs into an isolated directory in this worktree: **7 i686 PE checks,
  14 SHA-256-verified copies, exit 0**. The scratch stage was removed afterwards.
- Windows PowerShell **5.1.26100.9711** `Parser.ParseFile` reported **zero
  errors for all three changed/added PowerShell scripts**, exit 0.
- First host gates: fmt **0**, clippy **0**, test **101** during compilation:
  disk full (`no space on device`, LNK1318), not a failing test. Only this lane's
  generated `target/debug` was cleaned (one loaded proc-macro DLL remained
  locked; no other worktree was cleaned). Gates were repeated with
  `CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0`,
  `CARGO_INCREMENTAL=0`, `CARGO_BUILD_JOBS=2` to reduce disk pressure.
- Final host gate exits, each saved from `$LASTEXITCODE` immediately after its
  command: `cargo fmt --all -- --check` **0**;
  `cargo clippy --workspace --all-targets --all-features -- -D warnings` **0**;
  `cargo test --workspace --all-features` **0**. The test log contains 141
  summaries: **2,422 passed, 0 failed, 45 ignored** (including doc tests).
- Final self-test rerun: **37 passed, exit 0**. `git diff --check`: **0**.
  Temporary logs/scratch files were removed; no generated fixture is committed.
  No guest suite was run. No crate source was modified and no dependency added.

## Provenance

Only repository code/docs were read. No Winamp source, SDK headers,
disassembly, or research contents were read; no third-party code was copied.
