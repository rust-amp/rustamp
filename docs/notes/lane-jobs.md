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
| `1792273239098557400-ba97ed3413f7df05a3c24f3f` | `wave67-ui-red` | 2026-10-17 23:40 | the UI crate's two red tests |
| `1792273239098557400-9f1338378cccccc6f6ea9b7d` | `wave67-config-class` | 2026-10-17 23:40 | the writer/loader drift as a class (N8 found the second instance) |

Running elsewhere (they notify, no ids needed here): `wave65-wake-honest` on
Mistral (the wake-timeout claim).

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
