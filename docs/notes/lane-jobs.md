# Outstanding agy (Google) jobs

`agy` jobs **do not notify**. This file is the memory: every job id the
orchestrator starts goes here, and the Paseo heartbeat `ba05a239`
(`agy-job-check`, every 20 minutes, target `agent:0377dec`) polls the live ids,
verifies and merges what is done, and refills the slot.

A job that has stopped writing to its worktree is dead even if its state still
says running: check the worktree's newest write time, not just the state. Four
Google lanes once sat finished or stalled for three and a half hours while the
slots looked busy.

## Live

| job id | lane | started | task |
|---|---|---|---|
| (Mistral, subagent - notifies) | `wave73-resident-audio3` | 2026-10-18 03:20 | the three stubborn reds: two resident waits a widening did not fix, plus a regression in real_decode |

**agy is paused** since 2026-10-18 02:00: four jobs in a row failed with their
worktrees in place and with quota available, so the channel stopped carrying and
the work moved to Muse and Mistral. Start an agy job again only as a probe, and
only when a slot would otherwise sit empty.
## Done, merged

| lane | outcome |
|---|---|
| `wave54-output-error-hook` | RFC 0012 device-error hook |
| `wave54-wake-cross` | the blocking wake across the 32/64-bit boundary (G4) |
| `wave54-playlist-load` | the load-playlist path (N7) |
| `wave54-goldens-both-skins` | both golden namespaces, conflict resolved (`93a16f9`) |
| `wave55-xtask-package` | both residents shipped in the package (X2) |
| `wave61-s08-design` | the S08 measurement design |
| `wave65-app-red` | the app reds + easy-move wired (`24b33e4`) |
| `wave66-dsp-red` | the DSP crate's last red (`9bf50cc`) |
| `wave66-approval-store` | the loader's approval store (P7, `e8134cf`) |
| `wave66-ignored-staging` | staging for the ignored tests (X4, `2ebaaf0`) |
| `wave67-config-class` | the configuration writer drift as a class (N9, `99b2cb3`) |
| `wave65-wake-honest` | the wake-timeout claim made honest by measurement (G5, `0f12340`) |
| `wave67-ui-red` | the UI crate's two reds (J5, `6e22b56`) |
| `wave67-wave-review` | the adversarial review of tonight's wave (W14, `84366d5`) |

## Failed, relaunched elsewhere

| lane | died on | now on |
|---|---|---|
| `wave65-app-red` | Google job failed | Muse, merged |
| `wave65-wake-honest` | Google job failed | Mistral, running |

## How the check is run

1. `agy_status` on every id in **Live**.
2. The lane worktree's newest write time - a lane whose worktree has not changed
   in a while is stalled, whatever the job state says.
3. Done: verify (both trees, fmt/clippy, the suite), merge, move the row, and
   **refill the slot** from `docs/tasks/LANE-QUEUE.md` / `docs/tasks/HARD-TASKS.md`.
4. When a batch is merged, run the VM suite and read `[FAIL]` / `[HNG]`.

## File search

`es.exe` only: `C:\Program Files\Everything 1.5a\es.exe` (owner's standing
instruction, in AGENTS.md).

## Lesson the heartbeat caught (2026-10-18 00:00)

Two jobs were launched **before their worktrees existed**, so both failed at
once and their worktrees were never created - the same mistake the file already
warned about, made again. The heartbeat found it inside twenty minutes, which is
the whole point of it. Both were relaunched with the worktrees in place. **Create
the worktree first, then launch.**
## r38 (2026-10-18 00:30) - the first run with no hang

`ran 79, failed 9, hung 0`. The deadlock rebuild holds: `dsp_roundtrip` runs
(7 passed, 2 failed) instead of blocking the VM, and the runner reports every
binary. The two Rust-theme goldens were blessed and committed (`9600c95`), which
closed the two golden failures. What is left, in jobs: audio 7, dsp 4,
dsp_roundtrip 2 + real_decode 1, app 2 + platform 1, and the UI two (merged).
## r39 (2026-10-18 01:20) - eleven red tests left, none hung

`ran 79, failed 7, hung 0`, down from nine failed binaries. Closed since r38: the
two Rust-theme goldens (blessed, `9600c95`), the `resident` binary, five of the
audio seven and three of the DSP four. Closed since r39 by hand: the two
`dsp_roundtrip` tests and the one `real_decode` (`c9b4266`). What is left, in
jobs: audio 2, dsp 1, app 2 + platform 1, UI 2.

## Gate discipline (learned twice tonight)

Capture the tool's exit code into a variable **immediately** and gate on that
variable - never on `$LASTEXITCODE` after another pipeline step has run, because
that is how a red clippy was read as green and merged.