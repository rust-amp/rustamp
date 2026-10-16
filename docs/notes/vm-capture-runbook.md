# Reference captures in the headless VM (replaces the sandbox)

Why the captures moved, what replaces the sandbox's guarantees, and the exact
path from "orchestrator runs one command" to "captures back in the research
tree". Process only — nothing here is derived from Winamp source, the SDK or
the binary. The VM is dirty-room territory, exactly like the sandbox was; the
clean-room rules in [CLEANROOM.md](../../CLEANROOM.md) are unchanged.

Owner decisions behind this move (2026-10-16, see `docs/HANDOFF.md` §1):

- The **Windows Sandbox is forbidden** and is never launched again.
- The **headless Hyper-V test VM (`rustamp-test`)** is the only host for
  anything graphical, including every reference capture.
- The **harness is unchanged** — only the host moves. The VM path needs a way
  to get the captures **back** to the host, which the broker did not have.

The sandbox runbook (`VM-capture-host.md`) stays as the record of what was
measured there. This file is the replacement procedure.

## 1. What replaces "freshly booted"

The sandbox gave a pristine, identical OS on every boot — the strongest
provenance stateable. The VM replaces it with a **checkpoint restore** to a
named checkpoint, by default **`base-provisioned`**:

- `base-provisioned` is a shut-down, consistent disk state created once by the
  scripted path (`vm-setup-and-verify-test-host.ps1`: provision, reboot,
  clean shutdown, checkpoint). Its creation is scripted, so the state is
  reached deterministically.
- Every capture run starts with `broker -Op restore -Name <baseline>`, which
  discards everything written since the checkpoint and boots the guest from
  it. Capture-state baselines (`base-flat-main`, `base-eq-focused`, … — the
  catalogue from `VM-capture-host.md` §4) become **child checkpoints** created
  once from scripted setup, restored the same way. The rule is unchanged:
  restore a baseline, perform exactly **one** action, capture, restore again.
- A run **proves** it started from that state with three recorded facts, all
  written into `run-provenance.json` next to the captures:
  1. the checkpoint **name and creation time** (`broker -Op list`, before the
     restore),
  2. the **restore timestamp** (host clock, when the broker reported the
     restore done),
  3. the guest's **`LastBootUpTime`** (queried inside the guest by the
     capture script — a restore-then-boot must show a boot time *after* the
     restore timestamp).

Where the sandbox's provenance **cannot** be reproduced exactly, plainly
stated: a sandbox boot is pristine *by construction* (there is no earlier
state at all), while a checkpoint is only as clean as the moment it was
taken — any drift baked into `base-provisioned` (a hand-clicked dialog, a
background update that ran before the checkpoint) repeats in every run
instead of being impossible. Mitigations, all recorded: the checkpoint is
created by the scripted provision path, never by hand; its creation metadata
is kept; and the smoke test (§5) re-verifies determinism before every
campaign. What is *stronger* than the sandbox: mid-run states can be frozen
as named checkpoints, so each fact gets an atomic, repeatable provenance
instead of "the state drifted between runs".

## 2. Division of labour (unchanged from the test path)

| Step | Where | Elevation | What |
|---|---|---|---|
| Compile/stage | host | **unelevated** | `vm-run-capture.ps1`: stage the harness directory, the one job file and the reference copy under `D:\code\rustamp-vm\staging\` — copied blindly, never inspected |
| VM control | **broker** | elevated, session 0 | restore the baseline checkpoint, start, `copyin`, run the allow-listed guest script, **`copyout`** (new, §3) |
| Capture | guest | console session | the allow-listed `run-capture.ps1`: prove the baseline, check the display gate, run **one** job through the harness entry point, hash every output |
| Verify | host | unelevated | recompute SHA-256 over the copied-out files and compare against the guest-written manifest — equal hashes mean byte-identical |

The broker **never builds** (compiling would execute repository build scripts
as administrator). The guest script is allow-listed by plain filename, same
as `run-tests.ps1`. The orchestrator runs one command:

```powershell
powershell -ExecutionPolicy Bypass -File docs\notes\vm-run-capture.ps1 -Job <name> [-Baseline base-flat-main]
```

and gets back: the captures plus manifests in
`D:\code\rustamp-vm\staging\out\<name>\`, verified hash-equal, with
`run-provenance.json`. Moving them into the research tree's run directory is
the orchestrator's deliberate step (the runner never writes into
`rustamp-research` itself).

## 3. The return path: broker `copyout`

The broker could copy **in**, not **out** — and the captures are the
evidence, so they must land on the host unchanged. Chosen solution: a new
allow-listed broker op **`copyout`**, mirroring `copyin`:

- `copyout -Source C:\rustamp\out\<job> -Dest <staging>\out\<job>` copies a
  guest tree to the host over the same PowerShell Direct session mechanism
  (`Copy-Item -FromSession`), binary-safe for PNGs.
- Same constraint model as `copyin`, reversed: the guest source must live
  under `C:\rustamp\out` (so a request cannot exfiltrate arbitrary guest
  paths), the host destination must live under `<staging>\out` (so a request
  cannot overwrite the staged code or anything else on the host).
- Rejected alternative: a guest-side copy to a host-readable share (SMB /
  Enhanced-Session drive redirection). It needs a network path or a session
  mode the capture work deliberately avoids (drive redirection reintroduces
  the disconnected-desktop problem class), new credentials or firewall
  exceptions, and a second integrity story. `copyout` needs none of that:
  no network, no new secret, and integrity is the hash check in §4.

Broker change (in `vm-broker/`): the `copyout` case in `vm-broker.ps1` plus
the `ValidateSet` entry in `invoke-broker.ps1`. The broker script runs from
the repo path, so the change takes effect once integrated — **registering or
re-registering the scheduled task is the owner's step**, and any edit to the
elevated worker deserves the owner's review before it runs. Reported as a
finding in §7.

## 4. Integrity: byte-identical, with manifests

1. In the guest, `run-capture.ps1` writes every capture and manifest under
   `C:\rustamp\out\<job>\`, then writes `hashes.json`: relative path →
   SHA-256 for every file except itself and `run-provenance.json`.
2. `copyout` copies that tree to `<staging>\out\<job>\`.
3. On the host, `vm-run-capture.ps1` recomputes SHA-256 over every received
   file and compares it entry by entry against `hashes.json`. Any mismatch
   (or a missing manifest) fails the run loudly — nothing is promoted into
   the research tree.
4. `run-provenance.json` (checkpoint name + creation time, restore
   timestamp, guest boot time, display facts, harness exit, job name) travels
   with the captures and is stored with them in the research tree.

## 5. The 1:1 requirement (pinned, gated, refused on mismatch)

The VM's display is pinned, not inherited: provisioned at **100 % scale**
(`LogPixels` 96, `Win8DpiScaling` 0), resolution **1024×768**, captured into
`base-provisioned` — unlike the sandbox, which followed the host's
laptop/dock/monitor state (96 dpi vs 144 dpi across runs).

Two layers enforce 1:1, and both refuse instead of comparing wrongly:

1. **Guest pre-gate** (`run-capture.ps1`, before the reference binary
   starts): asserts `LogPixels` = 96 and the virtual screen = 1024×768. On
   mismatch it exits non-zero with the measured values and runs nothing.
2. **Harness gate** (unchanged, inside the harness as before): asserts the
   physical client rect is exactly 275×116, the capture is 275×116, and a
   known sprite matches at 1:1 with zero differing pixels (twin pixel **plus
   SHA-256** identity). A smoke test that compares two captures against each
   other is not a gate — two equally-scaled captures compare equal while
   both are wrong — so the gate compares against the reference, and a
   failure aborts loudly with the measured values in the failure record.

Every manifest records the display facts per run; captures from runs with
different display facts are never compared.

## 6. Smoke test before the VM is trusted for captures

1. Restore the capture baseline, capture twice with no action in between —
   the two captures must be **byte-identical**.
2. Restore, perform one known action, capture, and compare against the
   recorded evidence for the matching named checkpoint.
3. Run one staged job end to end through `vm-run-capture.ps1`, including the
   `copyout` + hash verification, and confirm the provenance file's boot
   time is after the restore timestamp.
4. Only after all three pass is the guest accepted as the capture host, and
   only then does the dirty room move its campaigns there.

## 8. The ignored winit window tests run in the interactive session

The capture path above is not the only GUI work the VM hosts. The four
ignored winit window tests (`eq_window`, `main_menu`, `playlist_window`, the
windowed `headless` case) panic in session 0, where the broker's guest
sessions land - they need the auto-logon console desktop. That path is
documented separately in [`vm-gui-session.md`](vm-gui-session.md): the host
stages the same tree unelevated, the allow-listed guest script
`run-gui-session.ps1` registers a one-shot Interactive-session task (the
`register-broker-task.ps1` precedent, pointed at the console session), and
the run proves its session id, user and window client sizes back to the host
via the existing `copyout` op. A run that lands in session 0 fails loudly.

## 7. Findings (things this change needs from outside its owned paths)

- **Broker registration is the owner's step.** The `copyout` op is written
  and waiting in `vm-broker/`; it runs only once integrated into the
  canonical tree, and the elevated worker change should be reviewed before
  the task next fires.
- **Harness entry point in the VM is unconfirmed.** The guest script invokes
  the same bootstrap entry point the sandbox `LogonCommand` used, with the
  staged paths remapped — but the harness tree itself was never opened here
  (clean-room rule), so the exact entry filename, its flags, and whether it
  assumes sandbox specifics (e.g. logging off at the end to close the
  sandbox — harmless in the VM, but must be confirmed) need the
  orchestrator/dirty room to check. If the harness needs a VM mode (a
  `-NoLogoff` flag, a VM job directory), that change belongs to the harness
  in the research tree, not to this lane.
- **Resolution pinning needs a one-time scripted step.** The provision
  script pins the scale (100 %); the 1024×768 resolution must be set once in
  the guest by script and baked into `base-provisioned`, otherwise the
  checkpoint inherits a hand-set value. Until that step exists and the
  checkpoint is re-created from it, the pre-gate's resolution check is the
  only guard.
- **Running the reference copy in the guest needs the owner's
  confirmation.** The existing approval covers the host sandbox, not
  automatically the guest (same reservation as `VM-capture-host.md` §3).
- **Nothing here was executed.** No broker invocation, no guest script, no
  capture, no VM or sandbox launch, no test run, no commit — all scripts and
  this runbook are written, not run. The smoke test (§6) is the
  orchestrator's first execution.
