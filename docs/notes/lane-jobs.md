# Outstanding agy (Google) jobs

`agy` jobs **do not notify**. This file is the memory: every job id the
orchestrator starts goes here, and every wake-up polls the ids in it with
`agy_status`, verifies and merges what is done, and refills the slot.

A job that has stopped writing to its worktree is dead even if its state still
says running: check the worktree's newest write time, not just the state. Four
Google lanes once sat finished or stalled for three and a half hours while the
slots looked busy.

## Live

| job id | lane | started | notes |
|---|---|---|---|
| `1792270956574926700-678c0b60495d80ea9cf8798f` | `wave65-app-red` | 2026-10-17 23:0x | app crate's two red tests + the easy-move wiring (W10 F5) |
| `1792270956574926700-7f0408ce1dd0ad3c463cf432` | `wave65-wake-honest` | 2026-10-17 23:0x | make the wake-timeout claim honest (W10 F7) |
| `1792271096501457800-35ca8739b7ccd2f3c537eb9b` | `wave66-dsp-red` | 2026-10-17 23:1x | the DSP crate's one red test |
| `1792271096501457800-0c6ffcd99f14ba26be0d6b14` | `wave66-approval-store` | 2026-10-17 23:1x | the approval store the loader lacks (P5) |
| `1792271096501457800-5274ea47c223c58922d46860` | `wave66-ignored-staging` | 2026-10-17 23:1x | stage what the ignored tests need (W12) |

## Done, merged (keep for the record)

| job id | lane | outcome |
|---|---|---|
| `1792257047258665800-c66f3f1669fa7539bf040dee` | `wave54-output-error-hook` | merged: RFC 0012 device-error hook |
| `1792258406725280900-524a6cf0c36690210f076282` | `wave55-xtask-package` | merged: both residents shipped in the package |
| (lane worktrees) | `wave54-wake-cross`, `wave54-playlist-load`, `wave54-goldens-both-skins`, `wave61-s08-design` | merged or in hand |

## How the check is run

1. `agy_status` on every id in **Live**.
2. For each, the worktree's newest write time - a lane whose worktree has not
   changed in a while is stalled, whatever the job state says.
3. Done: verify (both trees, fmt/clippy, the suite), merge, move the row to
   **Done**, and **refill the slot** from `LANE-QUEUE.md` / `HARD-TASKS.md`.

## The recurring check, and why it is not a Paseo heartbeat yet

The owner asked for a timer. Paseo is installed as a desktop app and its CLI
lives at `C:\Users\micro\AppData\Local\Programs\Paseo\resources\bin\paseo.cmd`,
but `paseo heartbeat create` refuses with `requireCallerAgentId`: it must be
called from a Paseo agent, and this session is an OpenCode session (no
`PASEO_*` variable, and no Paseo MCP tools are exposed here). So the heartbeat
has to be created from the Paseo UI, with this prompt:

> Check the outstanding Google (agy) jobs: read docs/notes/lane-jobs.md, poll
> every live job id with agy_status, and for each check the lane worktree's
> newest write time - a job can say running while the lane stopped hours ago.
> For anything done or stalled: verify in both trees (fmt, clippy, the suite),
> merge, move the row to Done, and refill the slot from docs/tasks/LANE-QUEUE.md.

Until then the check is done by hand on every wake-up, which is what this file
exists to make cheap.

## File search

`es.exe` only: `C:\Program Files\Everything 1.5a\es.exe` (owner's standing
instruction, recorded in AGENTS.md).