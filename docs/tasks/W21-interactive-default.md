# W21 — Interactive session is the suite default

**Date:** 2026-10-18. **Status:** implemented; parser/static review only,
guest execution deliberately pending with the orchestrator. No green runtime
result is claimed.

**Boundary:** write only your own files in
`C:\Users\micro\.local\share\opencode\worktree\26de0e\wave97-interactive`,
and nothing under `D:\code\Rustamp`, `D:\code\rustamp-research` or
`D:\code\rustamp-refbin`. Owned/changed paths: the two host runners,
`docs/notes/vm-broker/guest/{run-tests,run-gui-session,prepare-test-staging}.ps1`,
`docs/tasks/RELEASE-plan.md` and this card. No Rust/contracts changed.

## Default and invocation

The owner rule in `AGENTS.md` is now the default execution path, not an opt-in:
the guest's session 0 has no desktop, so a failure opening a window, dialog,
menu or plugin host there is not product evidence.

From the integrated repo root, unelevated (commands for the orchestrator,
**not executed by this lane**):

```powershell
# Normal full suite; ignored semantics are unchanged.
powershell -NoProfile -ExecutionPolicy Bypass -File docs\notes\vm-run-tests.ps1 -Reset
# General ignored pass, including all five actual winit tests.
powershell -NoProfile -ExecutionPolicy Bypass -File docs\notes\vm-run-tests.ps1 -Filter '--ignored'
# Group E still requires separate fresh-checkpoint runs and baseline audio fixtures.
powershell -NoProfile -ExecutionPolicy Bypass -File docs\notes\vm-run-tests.ps1 -Reset -Filter '--ignored:real_playback'
powershell -NoProfile -ExecutionPolicy Bypass -File docs\notes\vm-run-tests.ps1 -Reset -Filter '--ignored:loopback'
```

Host compilation/staging stays unelevated. The elevated broker only touches
the VM. It invokes `run-gui-session.ps1`, which demand-starts a Limited,
Interactive-principal task as the auto-logon user. The task executes the
**same full `run-tests.ps1` payload**, now staged under `vm-test-runner/` and
included in the staging receipt and input hashes. The receipt is version 2:
old version-1 staging is deliberately refused by `-SkipBuild`; rebuild once.
Deploy the new/updated guest scripts to the broker's existing `guest/`
directory when integrating (the broker allows files there by plain name);
there is no broker allow-list expansion or arbitrary command parameter.

There is no four-case list to omit the fifth menu-popup test or select the
wrong same-stem binary. Normal/ignored/bless filters and repeat summaries keep
the main runner's meaning. `vm-run-gui-session.ps1` is now a thin compatibility
entry point: no filter means the general ignored pass, a filter means an
ignored **binary-name** filter, and its exit code is propagated. It no longer
has a separate, weaker staging implementation or test-name case selector.
W19-G1/G2 and W19's two-runner/session-0 procedure are superseded here;
W19's fixture, display and audio prerequisites/limits still apply.

The session id must match the discovered interactive session and be above
zero; the task checks a non-empty virtual screen and staged tree. A per-run
nonce binds the sentinel, returned proof and copied proof. Proof and complete
per-binary output logs return to the default host directory
`D:\code\rustamp-vm\staging\out\gui-session\gui-session\` (copyout nests
the source directory). Repetitions retain the printed/rate-summary results;
the copied proof/log directory holds the latest repetition.

## Gates retained (not made weaker)

- Successful complete build required before staging; hashed completion receipt
  and current-input verification still gate `-SkipBuild`; zero host selection
  still refuses before any guest operation.
- The shared runner still owns fixture environment/status, per-binary reports,
  failure diagnostics, ignored inventory, bless copyback, repeat accounting,
  and real process exit codes. Failures return to the host for its report,
  rather than becoming an empty/green object. Exit is explicitly 0 on success,
  1 on a failed/incomplete run; infrastructure exceptions stay nonzero.
- The per-binary bound stays **300 s**, with bounded kill/output-drain waits.
  A lost output/summary now also fails even if the process exits zero. The
  independent hang/stall and zero-executed-binary gates remain; an additional
  aggregate zero-executed-test-case gate rejects vacuous binary starts.
- Task bound is `60 + 360 * max(staged test binaries, 1)` seconds; the host
  broker bound is `180 + 360 * max(build test artifacts, 1)` seconds, retaining
  the worst-case per-binary margin plus task-launch/return allowance. No old
  fixed 20-minute GUI deadline truncates a full suite with multiple hangs.
  The task has no future trigger that could unexpectedly run it twice. An old
  Running/Queued task is refused; cleanup stops/awaits the task (10 s bound)
  before unregistering, retaining the definition and failing if it will not stop.
- Missing desktop, result, sentinel or matching proof is an infrastructure
  failure, never a session-0 fallback. Group E is still visibly LEFT OUT of the
  general ignored pass, not relabelled as passed. Nothing installs a driver,
  changes a fixture gate, or claims all ignored tests ran from an inventory line.

## Deliberate desktop-free exception

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File docs\notes\vm-run-tests.ps1 -DesktopFreeResidentFixtures -Filter '--ignored:pcm_process'
powershell -NoProfile -ExecutionPolicy Bypass -File docs\notes\vm-run-tests.ps1 -DesktopFreeResidentFixtures -Filter '--ignored:dsp_process'
```

Only these exact two filter values are accepted with the switch, before a
build/guest request. The guest independently allowlists the corresponding
binary stems (not a general substring selection for session 0):

| Binary | Tests | Why no desktop is involved |
|---|---|---|
| `pcm_process` | `named_pcm_two_process_transfer_and_seek`; `host_exits_mid_stream_and_infinite_read_survives` | Two-process named-ring/seek/EOS/disconnect checks against the console-free `decoder_fixture`, no player, UI or plugin DLL |
| `dsp_process` | `named_dsp_two_process_gain_round_trip_then_sticky_eos`; `host_exits_mid_stream_and_infinite_read_survives` | Two-process gain/ring/EOS/disconnect checks against the console-free `dsp_fixture`, no player, UI or plugin DLL |

These are W19 Group C, also explicitly documented as headless/VM-only in
`crates/rustamp-mediahost/tests/{pcm_process,dsp_process}.rs`. The explicit
exception isolates transport/resident liveness without requiring explorer or
a virtual screen, so no interactive task/proof/copyout is used for those
diagnostic commands. They still run **only in the VM**, with the same build,
staging, fixture, timeout, failure, empty-run and repeat gates. The default
suite does **not** omit them; it runs them interactively like everything else.
If these binaries gain tests with desktop dependencies, re-audit this narrow
exception rather than broadening it.

W19 A-2's three model-only tests (`the_new_dialogs_pin_their_os_window_parameters`,
`opening_the_playlist_window_needs_an_os_window`,
`opening_either_dialog_needs_an_os_window`) also open no window. They remain in
the default interactive suite: they share the app library binary with an
actual winit menu test, so a binary-wide session-0 exception would be unsafe.
No desktop-free exception is granted to Group B/D merely because a helper or
DLL is described as headless. Nor is one granted to the unverified menu test
or the audio endpoint tests.

## Stray-process file lock: kill before guest staging, not reboot

Before **every guest tree copyin**, including `-SkipBuild`, the host invokes
the new allow-listed `prepare-test-staging.ps1`. It enumerates only
`rustamp-plughost32.exe` in the guest, checks the executable is under
`C:\rustamp\code\Rustamp\` or its `D:\code\Rustamp\` alias (including
extended-path spelling), kills it and waits up to **10 s per process** for
exit/file release. Uninspectable paths and kill/wait failures throw. A
reference installation outside those roots is not touched. The subsequent
copy is still fail-fast and still names an inaccessible file; no retry or
suppressed access-denied error turns a pre-test staging failure green.

**Wall-time cost:** one extra broker guest request per invocation, plus the
kill/wait work (bounded above per helper; the whole request has a 120 s bound).
The broker client polls every 3 s, so even a no-op adds a polling round trip.
No reboot/auto-logon/checkpoint wait is added and live guest state is retained.
The host prints the actual cleanup wall seconds. No measured duration is
claimed here because no guest request was allowed. Explicit `-Reset` and the
audio fresh-checkpoint requirement are unchanged.

## Validation and boundary receipt

**Executed only:** file reads, diff/static review, PowerShell parser calls and
read-only git inspection in this worktree. Windows PowerShell
**5.1.26100.9711**, `System.Management.Automation.Language.Parser.ParseFile`
(and `ParseInput` for the embedded payload), reports:

| File | Parse result |
|---|---|
| `docs/notes/vm-run-tests.ps1` | 0 errors |
| `docs/notes/vm-run-gui-session.ps1` | 0 errors |
| `docs/notes/vm-broker/guest/run-tests.ps1` | 0 errors |
| `docs/notes/vm-broker/guest/run-gui-session.ps1` | 0 errors |
| Embedded `inner.ps1` in that launcher | 0 errors |
| `docs/notes/vm-broker/guest/prepare-test-staging.ps1` | 0 errors |

`git diff --check` also completed without errors. No runner was invoked to
obtain these syntax/whitespace receipts.

**Deliberately not executed:** any runner/payload (including cleanup), test or
Rust gate (`cargo fmt`, `cargo clippy`, `cargo test`, or even a no-run build),
VM, broker, scheduled task, window, audio device, reference/plugin binary,
download or dependency operation. No commit or other git write. The
orchestrator owns the machine and broker request file; runtime validation of
task launch, cleanup permissions, copied proof and all success/failure/hang/
stall/empty-run paths remains its job after integration. Parser success is
not runtime evidence.

All explicit writes are confined to the seven owned paths above in this
worktree. No forbidden-tree contents, Winamp source, SDK or disassembly were
read. Final `git status --short` is included in the lane report; it shows
only these owned paths, not changes to any Rust or unrelated file. This
worktree status does not by itself audit another checkout; the orchestrator
can independently check the canonical tree without granting this lane access.
