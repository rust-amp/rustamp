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
| `guest` (`run-smoke.ps1`) | In the interactive session (explorer-owned, never session 0): the player starts with `--no-audio <fixture>` and stays up; the exact `Rustamp` main window (player pid, visible) appears within 60 s at client **275×116** (S01 rest size, pinned on the packaged build); a live `rustamp-plughost32.exe` child of the player exists (the supervisor resolves the bundled copy first and spawns eagerly, and the host sends its startup `Hello` / per-`Ping` `Pong` only while its UI pump runs — so a live child proves the pair reached handshake service); `xtask capture` screenshots a `Rustamp*` window; the player prints `output null (windowed; silent, no audio device)` (silent path, not a real device); the player is killed and its exit recorded. |
| `verify` (host) | Session is non-zero; proof facts complete; window size re-asserted; PNG exists, is ≥512 bytes and carries the PNG signature; every file recomputed against the guest `hashes.json` (the `vm-run-capture.ps1` §7 integrity leg); `--list` log, stdout/stderr logs and transcript land in `staging\out\smoke\` for the eye. |

## How it fails (every case exits non-zero, naming the step)

`STEP package / build-tool / stage / guest / verify FAILED: <detail>`.
Named guest sub-steps: `start` (immediate exit, with code — 0xC0000135
names a missing runtime), `window` (no exact main window in 60 s, or wrong
client size, or the player died waiting), `resident-handshake` (no live
bundled-host child in ~10 s), `capture` (tool exit / no PNG / bad
signature), `silent-path` (no silent marker on stdout), `stop` (Kill did
not take), `session` (landed in session 0 / no desktop), plus the outer
refusals (no interactive session, no done sentinel, session mismatch).
A lingering host child after the kill is recorded (`hostReaped: false`) but
warns rather than fails.

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
