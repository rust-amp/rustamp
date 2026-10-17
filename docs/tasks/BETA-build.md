# BETA build — the first packaged artefact

**Built** 2026-10-17 from `main` at `f1b87b8`, with
`cargo xtask package --release --out D:\code\rustamp-beta\2026-10-17`.

The source tree at that commit is green in the headless test VM: **55 test runs,
0 failures**, plus four window tests in an interactive session and the two
real-device audio tests (`--ignored:loopback`, `--ignored:real_playback`).

## What the folder contains

| Path | What | Licence |
|---|---|---|
| `player/rustamp.exe` | the player, x86_64, release | GPL-3.0-or-later + `PLUGIN-EXCEPTION.md` |
| `player/rustamp-plughost32.exe` | the 32-bit host, bundled beside the player (RFC 0006 §8, the first `supervise.rs` candidate) | MIT OR Apache-2.0 (unchanged by the bundling) |
| `player/LICENSE`, `player/PLUGIN-EXCEPTION.md` | the GPL set | |
| `host/rustamp-plughost32.exe` | the host on its own, i686 | MIT OR Apache-2.0, no GPL binary |
| `host/LICENSE-MIT`, `host/LICENSE-APACHE` | the permissive set | |
| `manifest.json`, `player/manifest.json`, `host/manifest.json` | what went where and under which licence | |
| `README.md` | what works, what does not, how to run it | |

## Why it is shaped this way

`rustamp-plughost32.exe` is 32-bit and must sit in the same directory as the
64-bit player, so it links the C runtime statically and needs nothing installed.
The player directory keeps exactly the GPL set and the host directory exactly the
permissive set; bundling a copy of the host beside the player changes no licence
file on either side.

## Verification done here

- The packaging command completed and produced both artefacts plus the manifests.
- The layout was checked by hand against the manifests: the two binaries, the
  GPL set in `player/`, the permissive set in `host/`, the bundled host copy.
- `.github/scripts/check-permissive-licenses.sh` passes on the same commit, so
  every Cargo dependency in the shipped closure is reconciled against
  `Cargo.lock` and the permissive crates stay GPL-free.

## Not verified yet

**The packaged binary has not been run.** A release build opens a window, and no
window may open on the host, so a smoke run has to happen inside the test VM —
which needs a small job that launches the packaged player and proves it starts,
shows its window and exits. That is the next step for this artefact.

## Addendum 2026-10-17 — what has moved since the build (artefact bytes unchanged)

The artefact above is frozen at source commit `f1b87b8`; `git log
f1b87b8..HEAD` counts 72 commits since, none of which rebuilt it. For the full
artefact-vs-tree inventory see `docs/tasks/BETA-what-is-in-it.md`. Four lines
of this card need re-pointing, not rewriting:

- *"The packaged binary has not been run"* (§"Not verified yet"): the blocking
  job now exists — `docs/notes/vm-run-smoke.ps1` + guest `run-smoke.ps1`
  (`docs/tasks/BETA-smoke.md`). It has run five times and found four
  runner/staging defects (all fixed), but no run has yet recorded the full
  green proof, so the packaged window still counts as never proven open.
- *"the device-loss hook still being an RFC"*: RFC 0012 is now implemented
  across `rustamp-core` / `rustamp-output` / `rustamp-audio` with a headless
  suite (`docs/tasks/H5-output-error-hook.md`; `docs/tasks/BETA-readiness-2.md`
  §2e). In the tree, not in these bytes.
- *"no real plugin loading"*: the loader is now a verified-byte snapshot +
  pre-execution preflight with an `approved_image` gate
  (`docs/tasks/P5-dll-preflight.md`; `docs/tasks/BETA-readiness-2.md` §2c) —
  still VM-acceptance-pending and with no production approval store. In the
  tree, not in these bytes.
- *"the stub platform layer"*: single instance + CLI forwarding (F2) predates
  the build and IS in these bytes; the tray model + global-hotkey session (F3)
  post-dates it and its two `window.rs` seams are still open, so six chords
  register and none is delivered (`docs/tasks/F3-tray-hotkeys.md`;
  `docs/tasks/BETA-readiness-2.md` §2f).

Left as written: the layout table, the runtime paragraphs, and the suite
counts — all true of the tree *at `f1b87b8`*. The ring-reuse line stays too:
`docs/tasks/A8-ring-reuse.md` still records itself not fixed, while the
neighbouring stop/flush behaviour has been re-specified by A12 (in these
bytes) and A13 (fail-closed barrier, tree only).

## Known defects this build ships with

Named honestly in the README beside the binaries: the ring-reuse defect after
Stop/Play (`docs/tasks/A8-ring-reuse.md`, a retry in progress), the missing
visualiser (`docs/tasks/S08-vis.md`), no real plugin loading
(`docs/tasks/P3-dll-acceptance.md`), the stub platform layer, and the device-loss
hook still being an RFC (`docs/tasks/H4-output-error-rfc.md`). Every one of them
is a documented open item, not a surprise.
