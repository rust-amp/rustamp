# Lane ready queue

**Owner rule (2026-10-17):** when a lane finishes, its slot is filled again
immediately — verify and merge the finished one, then start the next task from
this list before anything else. Target: **four lanes running**, each in its own
worktree, no two lanes in the same crate, only one lane driving the VM.

Keep this list current: add what the latest audits found, remove what is done.
Each entry names the task, the crate it owns, the source (spec or card) and the
model class it wants.

## Ready now

| # | Task | Owns | Source | Wants |
|---|---|---|---|---|
| 1 | **BETA-07** — the playlist's visibility and check marks do not control its OS window | `rustamp-app`, `rustamp-ui` | `docs/tasks/BETA-readiness.md` | balanced |
| 2 | **BETA-09** — loading a playlist file is a dead end; the skip diagnostics exist only at the library API | `rustamp-app` | `docs/tasks/BETA-readiness.md`, `docs/tasks/E3-playlist-skips.md` | fast |
| 3 | **BETA-08** — the shuffle and repeat lights advertise behaviour that is not applied (S15) | `rustamp-audio`, `rustamp-playlist` | `docs/tasks/BETA-readiness.md`, `specs/S15-shuffle-repeat.md` if it exists | balanced |
| 4 | **The wake, cross-process** — prove the RFC 0007 wake across the 32/64-bit boundary, the way `ring-cross` does | `rustamp-ipc`, `xtask` | `docs/tasks/G2-ipc-blocking.md`, `docs/rfc/0007-blocking-consumer.md` | balanced |
| 5 | **Golden cases, second set** — double-size main window, the EQ with a raised band, a playing main window with a time and a title | `rustamp-ui` tests | `tests/golden/README.md` | fast |
| 6 | **The EQ presets** — the popup exists and does nothing; find what a spec settles and implement only that | `rustamp-ui`, `rustamp-app` | `specs/S07-eq-dsp.md` §4.2 | balanced |
| 7 | **`rustamp-mediahost`, the PCM transfer** — RFC 0006's remainder: the ring wake and real decoders behind the stubs | `rustamp-mediahost` | `docs/rfc/0006-out-of-process-hosting.md` | deep |
| 8 | **ReplayGain** — the M2 item, with the scanner | `rustamp-audio`, `rustamp-meta` | `docs/PLAN.md` M2 | deep |
| 9 | **The S10 probes** — run the five probes the card designed (dirty room, owner-approved) | research harness | `docs/tasks/S10-probes.md` | orchestrator |

## Needs a decision first (do not start)

- **BETA-05 / RFC 0012** — the device-error hook. The RFC is a draft; implementing
  it changes a `rustamp-core` contract, so it waits for the owner's acceptance.
- **The plugin DLL acceptance** (`docs/tasks/P3-dll-acceptance.md`) — needs the
  owner's approval, an approved reference installation and a proven guest.
- **S08** (`docs/tasks/S08-vis.md`) — the probe and the jobs are designed; the
  runs are the orchestrator's, and the spec is filled from their artefacts.
- **`xtask package` installer, signing, notices** — release-time work.
- **The visualiser, MilkDrop, the media library, Linux/macOS** — later
  milestones.

## How a lane is started

1. `worktree_create` with a name that says what it is (`waveNN-<topic>`).
2. A prompt with: the worktree path as the **first line**, the boundary rule, the
   files to read, the deliverable, the hard constraints (no `cargo test`, no
   downloads, no git writes, no window, no audio), and the owned paths.
3. Report back with metadata only; the orchestrator verifies **both** trees,
   runs fmt/clippy, then the VM suite, then merges.
