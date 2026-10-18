# BETA smoke — first run of the packaged player (VM-only)

**Date:** 2026-10-17. **Status:** runner written, not executed. The packaged
player (`D:\code\rustamp-beta\2026-10-17`, built from `main` at `f1b87b8`)
has still never been run; this lane adds the job that proves it starts,
shows its window and exits, per `docs/tasks/BETA-build.md:39-43`.

**Worktree / write boundary:** `wave58-vm-smoke` worktree only. New files:
`docs/notes/vm-run-smoke.ps1` (host runner), `docs/tasks/BETA-smoke.md`
(this card), plus one declared adjunct below:
`docs/notes/vm-broker/guest/run-smoke.ps1` (new allow-listed guest script).
No access to the canonical, research or reference-binary trees; no
modification to the existing runners, the broker worker, any crate, `xtask`,
or `tests/golden/**`. No third-party source copied.

## Why a guest script was unavoidable (owner review required)

The runner needs the packaged player to open a real window **in the guest's
interactive session** and a PNG of that window brought back. The broker's
`guest` op executes only allow-listed `.ps1` files from the repo's
`docs/notes/vm-broker/guest/` directory, and every existing guest script
runs fixed payloads (test binaries, the reference-capture harness,
diagnostics) — none can start an arbitrary staged EXE or screenshot it.
Reaching the interactive session at all needs the Scheduled-Task-as-
Interactive-user shape, which only exists as guest-side code. This is the
exact precedent of the GUI-session lane (`docs/notes/vm-gui-session.md:4`):
"one new allow-listed guest script, which the broker picks up from the repo
path. No broker re-registration is needed." `vm-broker.ps1`,
`invoke-broker.ps1` and every existing runner are untouched; the allow-list
is filename-based, so no broker change or re-registration is needed. Like
every broker-adjacent change, the new guest script deserves the owner's
review before it runs in the guest — it is flagged as such here, and the
host runner fails loudly (`STEP guest`) if the script is not yet integrated
and allow-listed.

## Invocation (orchestrator, host, unelevated; nothing graphical here)

```powershell
powershell -ExecutionPolicy Bypass -File docs\notes\vm-run-smoke.ps1 [-PackageDir D:\code\rustamp-beta\2026-10-17] [-Reset]
```

Defaults: newest folder under `D:\code\rustamp-beta`; `base-provisioned`
only with `-Reset` (otherwise plain start, like the existing runners).

## What each step proves

| Step | What it proves |
|---|---|
| `package` audit | `player\rustamp.exe`, `player\rustamp-plughost32.exe`, `host\rustamp-plughost32.exe` exist and are non-empty; SHA-256 recorded. The bundled copy beside the player is the first `supervise.rs` candidate (RFC 0006 §8). Missing/empty binaries fail here, before any VM work. |
| `build-tool` | Only `cargo build -p xtask` (the existing, reviewed window-capture tool) is compiled. The player and host are **never rebuilt**: they are staged byte-for-byte, so the proof is of the shipped bytes, not a fresh build. |
| `stage` | Verbatim package copies + capture tool (+ host CRT DLLs beside the tool only — the packaged binaries are static-CRT per X1) + the repo's committed `sine.flac` fixture (known-good media, so a decode failure names the package, not BETA-04-style bad input). |
| `guest` (`run-smoke.ps1`) | In the interactive session (explorer-owned, never session 0): the player starts with `--no-audio <fixture>` and stays up; the main window (documented rule `Rustamp`, or `Rustamp - <error>` while an error is showing per `crates/rustamp-app/src/app.rs:350-362`, owned by the player pid, visible) appears within 60 s at client **275×116** (S01 rest size, pinned on the packaged build) — a timeout now dumps every owned top-level window it saw (title, class, rect, visibility); a live `rustamp-plughost32.exe` child of the player exists (the supervisor resolves the bundled copy first and spawns eagerly, and the host sends its startup `Hello` / per-`Ping` `Pong` only while its UI pump runs — so a live child proves the pair reached handshake service); `xtask capture` screenshots a `Rustamp*` window; the player prints `output null (windowed; silent, no audio device)` (silent path, not a real device); the player is killed and its exit recorded. |
| `verify` (host) | Session is non-zero; proof facts complete; window size re-asserted; PNG exists, is ≥512 bytes and carries the PNG signature; every file recomputed against the guest `hashes.json` (the `vm-run-capture.ps1` §7 integrity leg); `--list` log, stdout/stderr logs and transcript land in `staging\out\smoke\` for the eye. |

## How it fails (every case exits non-zero, naming the step)

`STEP package / build-tool / stage / copyin / copyout / guest / verify
FAILED: <detail>`.

The copy-in fails two distinguishable ways: the broker's path-policy refusal
(its own `copyin ... must live under ...` message — this runner's path
expression is wrong) or an in-guest failure (a RemoteException after the
paths were accepted — the known case is an access-denied naming a staged
file, i.e. the guest still holds that file open, a leftover process from a
previous run; remedy: a fresh guest via `-Reset`). See
`docs/tasks/X6-smoke-copyin.md`.
Named guest sub-steps: `start` (immediate exit, with code — 0xC0000135
names a missing runtime), `window` (no main window in 60 s under the
documented rule — the failure names every visible owned top-level window
seen, or says none was enumerated — or wrong client size, or the player
died waiting), `resident-handshake` (no live
bundled-host child in ~10 s), `capture` (tool exit / no PNG / bad
signature), `silent-path` (no silent marker on stdout), `stop` (Kill did
not take), `session` (landed in session 0 / no desktop), plus the outer
refusals (no interactive session, no done sentinel, proof with no session
field, session mismatch).
A lingering host child after the kill is recorded (`hostReaped: false`) but
warns rather than fails.

## What the first real runs found (why the runner exists)

Six defects in seven runs, each found by running, not by reading: (1) the
`package` audit refused an incomplete build (a missing load-bearing binary —
the check working as designed, no script change); (2) the guest script called
`Invoke-CimMethod -GetOwner`, a parameter that does not exist — fixed to
`-MethodName GetOwner`; (3) the outer session check refused with `ran in
session , ...`: failure proofs carried no `session` field, so any inner step
failure surfaced as an empty session and masked the real cause. Fixed by
stamping the seen session into every failure proof (inner `Finish-Failure`
and the `catch` leg) and refusing a session-less proof with its own
diagnostic that names the failed step. The interactive mechanism itself is
unchanged and is the GUI-session lane's shape, not a parallel one: discover
the explorer-owned session (fallback: the Active `query session` console
row), register a one-shot Scheduled Task as that user with `LogonType
Interactive` / `RunLevel Limited` (console desktop, no UAC question),
`Start-ScheduledTask` now, wait on the `done.json` sentinel with a bound,
unregister, then refuse session 0 / a mismatch loudly.; (4) the fifth run
failed at the copy-in with `access to the path "rustamp-plughost32.exe" was
denied` — and the first diagnosis of it was wrong. That refusal was not the
broker's path policy (the policy refusal is the broker's own `copyin source
must live under ...` message, and it did not fire): broker.log's
`op=copyin args=` line records the source the broker actually received,
`D:\code\rustamp-vm\staging\smoke` — absolute, under the staging root the
broker owns. The error was raised INSIDE the guest (a RemoteException): the
broker's copy-in clears `C:\rustamp\smoke` and copies the staged tree in,
and it could not delete/overwrite
`C:\rustamp\smoke\player\rustamp-plughost32.exe` because a process in the
guest still held it open — the previous run's player (its payload failed
before the kill step and `Finish-Failure` exits without killing it, so it
lived on to its `--seconds 120` backstop) and/or its bundled host child,
which can outlive the player (the recorded `hostReaped: false` finding).
The "fix" that landed for it (commit 6873089) changed no path — only a
comment; the `$Stage` line predates it — and the error came back unchanged
on the next run, which is what disproved the staging-root diagnosis. (5) The
sixth run reached the interactive session (session 1,
pinned virtual display) with the player alive after 2 s, then failed the
window step with a message that named only the wanted title — it could not
say whether a player window existed under a different title or no window
existed at all. Fixed without weakening the check (card
`docs/tasks/X5-smoke-window.md`): the wait now accepts exactly the
documented title rule, and a timeout dumps every owned top-level window it
saw; the host runner best-effort-copies the guest outputs back even when
the guest step throws. (6) The seventh run failed at the same copy-in with
the same in-guest access-denied on `rustamp-plughost32.exe`, again with the
absolute staging path in broker.log's args line, ~20 minutes after the
sixth run's player had started. The repetition — after the same copy-in had
already succeeded three times from clean-guest states (10-17 20:33 and
20:34, 10-18 07:41; ~3-4 s each, against ~1 s for both failures, which is
the broker's first in-guest file operation) — pinned the cause to the
guest's state, not to the runner's paths: a leftover host child holds the
staged binary open and the broker's copy-in cannot replace it. Fixed in the
runner (card `docs/tasks/X6-smoke-copyin.md`): the copy-in source is
resolved to an absolute path and verified under the staging root before the
call, the copy-out destination is verified under the results root, and a
copy-in failure now reports the path-policy refusal and the in-guest case
apart, naming the real cause and the remedy (a fresh guest: `-Reset`). The
deeper fix — the guest payload's failure paths exiting without killing the
player — belongs to the guest script, which this lane does not touch.

## What it still does not prove

A real audio endpoint (null output only), a real plugin (no DLL is
loaded — the host answers its control channel with no plugin behind it),
a long run (the player lives ~half a minute, then is killed — no clean-exit
claim), pipe-byte handshake content (liveness is proven, the framed bytes
are the supervisor's private channel), pixel-exact art (the PNG will show
the BETA-02 magenta placeholder), decode beyond the one committed fixture,
or which `Rustamp*` window the PNG shows when main and playlist both match
(the proof records the selected hwnd/title/size and the full `--list`).

## Verification and boundary receipt

- PowerShell syntax checks only (the VM is in use by the orchestrator's
  suite run; nothing graphical may run on this host):
  `[System.Management.Automation.Language.Parser]::ParseFile` on both new
  scripts — no errors. `Get-Help` on the host runner — help renders.
- No VM, broker, player, host, capture tool, test binary or reference
  binary was run; no window opened (no session here can open one); no
  download, install, commit or git write; no new dependency.
- Provenance: clean-tree sources only — `BETA-build.md` (artefact),
  `vm-run-tests/gui-session/capture.ps1` + runbook (division of labour,
  single-object result contract, copyout + hash leg, Interactive-task
  shape), `vm-gui-session.md` (guest-script precedent), `xtask/capture.rs`
  (capture tool), `rustamp-app` `cli/lib/window/supervise` (flags, stdout
  marker, GUI rails, candidate order, eager spawn). No Winamp source, SDK,
  disassembly or dirty-room material read or touched.
- `git status --short` (worktree): exactly the three new files above; no
  tracked modification.
