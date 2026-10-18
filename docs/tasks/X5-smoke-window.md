# X5 — smoke window step: report what it saw, match the documented title

**Date:** 2026-10-18. **Status:** scripts + docs changed, VM not run.
**Worktree / write boundary:** `wave79-smoke-window` worktree only.
Owned paths: `docs/notes/vm-run-smoke.ps1`,
`docs/notes/vm-broker/guest/run-smoke.ps1`, `docs/tasks/BETA-smoke.md`,
this card. No crate, test runner or broker script touched; no download, no
new dependency, no git write, no window, no audio.

## 1. The failure that motivated this

The sixth packaged-player smoke run got further than any before — interactive
session 1, pinned virtual screen `{X=0,Y=0,Width=1824,Height=768}`, player
pid 3464 alive after 2 s — then failed here:

```text
STEP FAILED: window    no visible main window (exact title `Rustamp`, owned by the player) appeared within 60 s
```

That message names only the title it wanted. It cannot distinguish "the
window was there under a different title" (e.g. the documented error title
below) from "no window was there at all". The distinction costs a whole VM
run — the same lesson this lane has now learned five times over
(`docs/tasks/BETA-smoke.md:69-94`): a check that does not report what it saw
is a check that has to be run again.

## 2. The documented title rule (from code, not memory)

- `crates/rustamp-app/src/app.rs:350-362`: `App::window_title()` returns
  `Rustamp`, or `Rustamp - <error>` while an error is showing (`last_error`
  is `Some`). Own choice, stated plainly at the definition site: no spec
  gives the player an error surface; the native title bar stays legible
  whatever the skin.
- `crates/rustamp-app/src/window.rs:704-713`: `Windowed::sync_title`
  applies exactly `App::window_title()` to the main window when it changed.
- Other OS windows are never `Rustamp`-titled: playlist
  `Rustamp Playlist` (`window.rs:1113-1116`), equalizer
  `Rustamp Equalizer` (`window.rs:397-399`), menu `Rustamp Menu`
  (`window.rs:908-916`).

The old smoke check (exact `== 'Rustamp'`) was therefore **stricter than the
documented rule**: a run whose player opened its main window but was showing
an error (e.g. the `sine.flac` fixture failing to decode — precisely what the
smoke run is there to catch) would have failed the window step while the
window was in fact open. The fix matches the rule **without** weakening the
check into a substring match: accept `Rustamp` exactly **or** the documented
`Rustamp - ` prefix form. `Rustamp Playlist`, `Rustamp Equalizer`,
`Rustamp Menu` and any other `Rustamp*` title still do not satisfy the wait,
and ownership (player pid) + visibility + the separate `275x116` client-size
assertion are unchanged.

## 3. What changed

`docs/notes/vm-broker/guest/run-smoke.ps1` (inner payload only):

- `SmokeWin` gains `GetClassNameW`, plus `ClassName`/`Title` helpers
  (512-char title buffer, up from the inline 256).
- The window wait accepts the documented rule (`-eq 'Rustamp'` or
  `.StartsWith('Rustamp - ')`), records the actual title in `$mainTitle`,
  and the success proof now carries that title instead of a hardcoded
  `'Rustamp'`.
- On timeout, before failing, the step enumerates **every** top-level window
  owned by the player pid or its live children and writes one line per
  window (hwnd, pid, visibility, class, title, client size, rect; capped at
  20 lines) to `guest-transcript.txt`, and names the visible owned windows
  (or their absence) in the failure detail:
  - some visible owned windows: `...; visible owned windows were:
    pid=... title="..." class="..."; ...`
  - none: `...; no visible top-level window owned by player pid <pid> or
    its children was enumerated (<n> owned total)` — stated plainly instead
    of re-naming the wanted title.
- Header comment updated to the documented rule.

`docs/notes/vm-run-smoke.ps1` (host runner):

- The guest invocation is wrapped so a guest failure **still** best-effort
  copies `C:\rustamp\out\smoke` back (transcript with the window dump
  included) before rethrowing `STEP guest FAILED: ...`. A copyout failure
  warns but never masks the original guest error. Success path unchanged.

`docs/tasks/BETA-smoke.md`: `guest` row and failure paragraph updated to the
documented rule + dump; run history extended with this sixth-run finding.

## 4. What did NOT change (smoke properties kept)

- Package staged byte-for-byte, never rebuilt (player + bundled host +
  tool + fixture + host CRT beside the tool only).
- Player runs with `--no-audio` (null output) and a real window; client
  size still asserted `275x116` (S01 rest size); bundled-host child still
  required live (~10 s); capture still via `xtask capture` with PNG
  signature + size gates; silent-path stdout marker still required; player
  still killed with exit recorded; every file still hash-recomputed against
  the guest `hashes.json`.

## 5. Verification and boundary receipt

- PowerShell parser on both edited scripts, VM/broker/player untouched:
  `docs/notes/vm-run-smoke.ps1` — **0 errors**;
  `docs/notes/vm-broker/guest/run-smoke.ps1` — **0 errors**.
- No VM, broker, player, host, capture tool, test binary or reference
  binary was run; no window opened; no download, install, commit or git
  write; no new dependency.
- Provenance: clean-tree sources only (the two scripts, `BETA-smoke.md`,
  `crates/rustamp-app/src/app.rs:350-362`,
  `crates/rustamp-app/src/window.rs:704-713`). No Winamp source, SDK
  header, disassembly or dirty-room material read or touched.
