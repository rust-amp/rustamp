# The VM capture host (Hyper-V)

Why we run reference captures in a virtual machine, what has to be configured,
and how a capture is taken so that its provenance is beyond doubt.

This document is **process**, not reference knowledge: it contains nothing
derived from Winamp source, the SDK or the binary. The VM itself is
dirty-room territory, exactly like the current sandbox — the clean-room rules in
[CLEANROOM.md](../../CLEANROOM.md) are unchanged.

## 0. Two routes, and when to use which

| | **Windows Sandbox** | **Hyper-V guest** |
|---|---|---|
| Setup | one `Enable-WindowsOptionalFeature` plus a restart; payload is already present | Windows ISO, install, auto-logon, ~10–20 min |
| Baseline | **pristine and identical on every boot** — the strongest provenance we can state | snapshot catalogue, restored in seconds |
| Mid-run states | none: a baseline means a fresh boot plus the setup actions | freeze and restore any state |
| Driven from the host | only through the `.wsb` `LogonCommand`; everything must be scripted inside | full PowerShell control (`Start-VM`, `Checkpoint-VM`, `Restore-VMSnapshot`) |
| Persistence | none — wiped on close | persistent, copy files in and out |
| Best for | the immediate capture work and "does it work at all" | M3 plugin work, long capture campaigns, reproducible golden images |

The prepared sandbox configuration is `vm-capture-host.wsb` next to this file.
Both routes need an **elevated** shell for their host-side steps; the
orchestrator shell is not elevated.

## 1. Why a VM (and why not "because it is faster")

Winamp 2.95 is a 1999 program; any VM is overpowered for it. Speed is not the
reason. Four things are:

1. **A genuinely interactive desktop.** Every capture failure on the host has
   the same shape: the agent processes are not on the interactive desktop
   (`GetForegroundWindow()` = 0, `SendInput` returns `ACCESS_DENIED`, screen
   capture fails), even while the session reports `Aktiv`. In a guest with
   **auto-logon at the console** the console session is interactive, and a
   process inside the guest can inject real input.
2. **Snapshots.** Captures stop being one-shot: restore a baseline, perform
   *one* action, capture, restore. Each fact gets an atomic, repeatable
   provenance ("snapshot `base-eq-unfocused`"), instead of "the state drifted
   between runs" or "the skin-load method is unrecorded" — both of which really
   happened on the host.
3. **A fixed display scale.** The host runs at 250 % with a stale registry
   `AppliedDPI` of 144, so a DPI-unaware observer reads 110×46 for a window
   that is 275×116. A guest pinned to 100 % makes `GetClientRect == 275×116`
   unambiguous.
4. **Isolation.** Input automation hits a disposable guest, never the owner's
   desktop. That is the direct answer to the incident where a capture session
   claimed it had used no synthetic input while real clicks had driven states.

It is also the right home for milestone M3, where real 32-bit plugin DLLs must
be loaded and allowed to crash: snapshot, load, observe, roll back.

## 2. Host facts (measured 2026-10-15)

| Item | Value |
|---|---|
| Machine | Lenovo 82JQ, Ryzen 7 5800H, 16 logical CPUs, 31.9 GB RAM |
| Hypervisor | **active** (`HypervisorPresent` = True, `vmms` running) |
| Hyper-V PowerShell module | 2.0.0.0 — `New-VM`, `Checkpoint-VM`, `Restore-VMSnapshot`, `Get-VM`, `Get-VMSnapshot`, `Start-VM` present |
| VMware Workstation | installed, but **not** the recommended path here: with Hyper-V active it only runs in the WHP compatibility mode |
| Elevation | the orchestrator shell is **not** elevated; VM creation and snapshot restore need an admin shell (or Hyper-V Administrators) |

## 3. Guest configuration checklist

Create the VM in an elevated shell.

- **Generation 2**, 4 GB RAM, 2 vCPUs, 60 GB dynamically expanding VHDX. Winamp
  needs none of that; the headroom is for Windows itself.
- **No network after setup.** The reference copy is copied in, so the guest can
  be offline; fewer moving parts.
- Install Windows, then set a local account with **auto-logon at the console**
  (the `AutoAdminLogon` registry values or `netplwiz`). This is the whole point
  of the exercise: the console session must be interactive without anyone
  logging in.
- **Disable lock screen, screen saver and sleep.** A locked session has no input
  desktop, which is exactly the failure we are escaping.
- **Display scale 100 %**, fixed resolution (1024×768 or 1280×1024).
- **Do not use RDP and do not enable Enhanced Session Mode** for the capture
  work: both reintroduce the disconnected-desktop problem. The basic console
  session (`vmconnect.exe`) is what the automation expects.
- Copy the already-extracted Winamp 2.95 sandbox
  (`D:\code\rustamp-sandbox\winamp295`) into the guest. **This runs the
  reference binary in a new location and needs the owner's confirmation** — the
  existing approval covers the host sandbox, not automatically a guest.
- Copy the capture harness and its manifest writer (dirty-room tooling; see
  §5).
- Optional but useful: a Defender exclusion for the capture directory, so a
  capture is never delayed by a scan.

## 4. Snapshot catalogue

Baselines are created once and then restored before every capture. Names are
lower-case and stable, because they end up in manifests.

| Snapshot | State |
|---|---|
| `base-flat-main` | main window, no track loaded, nothing focused |
| `base-eq-unfocused` | EQ visible, main focused |
| `base-eq-focused` | EQ visible and focused |
| `base-pe-unfocused` | playlist visible, main focused |
| `base-pe-focused` | playlist visible and focused |
| `base-playlist-empty` / `-6` / `-30` / `-100` | playlist populated with that many entries, scrolled to top |
| `base-skin-<name>` | a named skin loaded |

**Capture rule:** restore a baseline, perform exactly **one** action, capture,
restore again. The manifest names the snapshot and the action. Never take two
facts from one drifted state.

## 5. What lives where

| Artifact | Location | Clean? |
|---|---|---|
| This runbook | `D:\code\Rustamp\docs\notes\` | clean (our own process) |
| Capture harness, manifests, captures, snapshots of evidence | `D:\code\rustamp-research\` | **dirty** |
| The existing generic capture tool | `cargo xtask capture` in the repo | clean, reusable (it is a plain Win32 window grab with no Winamp knowledge) |

## 6. Manifest schema (every capture)

- `snapshot` — the baseline restored before the action
- `action` — what was done, in one line
- `input_method` — `real-mouse` or `posted-message` (never implied, always
  stated)
- `dpi_awareness` — of the measuring process, and the guest's scale
- `coordinate_space` — window-relative client pixels or screen pixels
- `harness` — harness name and version
- `verdict` — the zero-difference comparison against the baseline

## 7. Smoke test before the VM is trusted

1. Restore `base-flat-main`, capture twice with no action in between.
2. The two captures must be **byte-identical** (that proves determinism).
3. Restore, perform one known action (e.g. focus the EQ), capture, and compare
   against the recorded `base-eq-focused` evidence.
4. Only after all three pass is the guest accepted as the capture host.

## 9. Tools inside the sandbox

The owner authorised installing whatever the capture work needs **inside the
sandbox** — PowerShell 7, helpers, anything. The sandbox has network access for
that (`<Networking>Enable</Networking>` in `vm-capture-host.wsb`), and the
reference copy stays a local read-only mapping, so nothing about the reference
material depends on the network.

Two consequences to design around:

- **The sandbox is wiped on close**, so an install is *per boot*. The bootstrap
  either scripts the install (and pays the download every run) or sticks to the
  built-in Windows PowerShell 5.1 — which is what the probes ran on, and it was
  sufficient.
- Anything the bootstrap installs must be reproducible from a script in the
  research repo, never a hand-made state, or the next boot silently differs and
  the capture provenance is worthless.

## 10. Windows Sandbox: measured 2026-10-15

The feature is enabled (`Enable-WindowsOptionalFeature -Online -FeatureName Containers-DisposableClientVM -All`, `RestartNeeded: False`) and a probe ran inside a guest. Measured inside the sandbox:

- **`DPI_FOR_SYSTEM` = 96 (100 %)**, screen 1011×644 — a guest pinned to 100 % removes the whole 250 %/144/110×46 confusion.
- `INPUT_DESKTOP` = AVAILABLE, `SendInput` returned 3 events with `err=0`, and the cursor moved to the exact target (220,160) — **real input is accepted inside the sandbox**, unlike on the host where `SendInput` is silently dropped.
- The first click did not reach our own window: the window had no foreground (`FOREGROUND_BEFORE` = 0) and the click was consumed by activation. The corrected probe (activate, then click) did not produce output — one more debug pass is needed before the sandbox route is trusted.
- **Two sandboxes cannot map the same host folder at the same time**; close one before starting the next, or the `LogonCommand` never sees the mapped files.
- **A launch may show a confirmation dialog on the interactive desktop, and only the owner can dismiss it.** The agent's own process is not on the interactive desktop, so its window enumeration returns nothing (measured: zero windows, zero dialogs, while the sandbox booted fine once the owner confirmed). Do not try to automate that confirmation; report it and wait for the owner. This is the actual reason early launches appeared to fail with `exit code 0` and no session — **not** the killed processes, which was my first, wrong conclusion.
- **Never kill the `WindowsSandbox*` processes.** `WindowsSandboxServer` and `WindowsSandboxRemoteSession` are the host-side infrastructure, not just the client; killing them has broken a running sandbox in practice and must not be repeated. Close a sandbox by logging off **inside** it (allowed, and what the bootstrap does), or by closing its window — never by killing processes, and never with a logoff, shutdown or restart against the host.

Verdict: promising for deterministic 100 % captures, but not yet verified end to end. The host now delivers real input through `SetCursorPos` + legacy `mouse_event`, so the sandbox is not on the critical path for S01 round 2.

## 10. Operational hazards found while capturing

- **Our own browser window steals the foreground.** A full-screen OpenCode/Chrome window occluded all three Winamp windows and re-took the foreground about a second after any raise. The capture harness's guard correctly refused to act; every future run must re-raise Winamp and re-verify the guard before each action, and never send input to the browser.
- **`SendInput` is silently dropped on the host** (returns `n=1`, `err=0`, cursor does not move) while `SetCursorPos` + legacy `mouse_event` work. Cause unexplained; record it in every manifest.
- **Locale trap:** an unquoted `-Down 6,3` under the host's `de-AT` culture is parsed as `63`. The harness now parses points from text.

## 11. Migration

Once the guest passes the smoke test, the dirty room moves there and the host
sandbox is retired — **after asking the owner**, since removing it is not
reversible without the original archive.
