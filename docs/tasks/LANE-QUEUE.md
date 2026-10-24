# Lane ready queue

**Owner rule (2026-10-17):** when a lane finishes, its slot is filled again
immediately — verify and merge the finished one, then start the next task from
this list before anything else. Target: **four lanes running**, each in its own
worktree, no two lanes in the same crate, only one lane driving the VM.

Keep this list current: add what the latest audits found, remove what is done.
Each entry names the task, the crate it owns, the source (spec or card) and the
model class it wants.

## In flight (2026-10-19, wave 127-131)

Every lane owns its own worktree; the worktree path is the first line of the
prompt (owner rule, 2026-10-19 — see `AGENTS.md`).

| Worktree | Task | Owns | Model |
|---|---|---|---|
| `wave127-playlist-remove` | Playlist remove-all-dead-files + clear playlist (40222, 40214) | `rustamp-app`, `rustamp-proto` | free-deepseek-v4.1 |
| `wave128-playlist-sort` | Playlist sort menu (40209-40212) | `rustamp-app`, `rustamp-ui` | free-big-pickle |
| `wave129-jumpfile` | Jump to File dialog verification/wiring (40194) | `rustamp-app`, `rustamp-ui` | minimax-m3.1-flash-free |
| `wave130-docking` | Window docking/snapping (S05) | `rustamp-ui` | beam-free |
| `wave131-eqpresets-file` | EQ presets load-from-.eqf file (S07 §4.2) | `rustamp-app`, `rustamp-platform` | claude-balanced |

## Next wave (ready, disjoint crates)

Pick from these so no two lanes share a crate; refill each slot as soon as one
finishes (owner rule: keep the lanes full until every open point is done).

| # | Task | Owns | Source |
|---|---|---|---|
| A | **BETA-08** — shuffle/repeat lights vs. applied behaviour | `rustamp-audio`, `rustamp-playlist` | S15, `docs/tasks/BETA-readiness.md` |
| B | **Playlist format detection + M3U8/XSPF import** | `rustamp-playlist` | S06, `docs/tasks/E3-playlist-skips.md` |
| C | **ReplayGain** — scanner + playback | `rustamp-meta`, `rustamp-audio` | `docs/PLAN.md` M2 |
| D | **RFC 0012** — the device-error hook (accepted) | `rustamp-core`, `rustamp-output` | `docs/rfc/0012-output-error-hook.md` |
| E | **Plugin DLL acceptance** (owner-approved) | `rustamp-plughost`, `rustamp-plugin-abi` | `docs/tasks/P3-dll-acceptance.md` |
| F | **RFC 0006** — `rustamp-mediahost` PCM transfer | `rustamp-mediahost` | `docs/rfc/0006-out-of-process-hosting.md` |
| G | **S04 File-menu actions** — open file(s)/directory/location | `rustamp-app` | S04, S03 |
| H | **S07 EQ auto-load per track** | `rustamp-app`, `rustamp-dsp` | S07 |

## Ready now

| # | Task | Owns | Source | Wants |
|---|---|---|---|---|
| 1 | **BETA-07** — the playlist's visibility and check marks do not control its OS window | `rustamp-app`, `rustamp-ui` | `docs/tasks/BETA-readiness.md` | balanced |
| 2 | **BETA-09** — loading a playlist file is a dead end; the skip diagnostics exist only at the library API | `rustamp-app` | `docs/tasks/BETA-readiness.md`, `docs/tasks/E3-playlist-skips.md` | fast |
| 3 | **BETA-08** — the shuffle and repeat lights advertise behaviour that is not applied (S15) | `rustamp-audio`, `rustamp-playlist` | `docs/tasks/BETA-readiness.md`, `specs/S15-shuffle-repeat.md` if it exists | balanced |
| 4 | **The wake, cross-process** — prove the RFC 0007 wake across the 32/64-bit boundary, the way `ring-cross` does | `rustamp-ipc`, `xtask` | `docs/tasks/G2-ipc-blocking.md`, `docs/rfc/0007-blocking-consumer.md` | balanced |
| 6 | **The EQ presets** — the popup exists and does nothing; find what a spec settles and implement only that | `rustamp-ui`, `rustamp-app` | `specs/S07-eq-dsp.md` §4.2 | balanced |
| 7 | **`rustamp-mediahost`, the PCM transfer** — RFC 0006's remainder: the ring wake and real decoders behind the stubs | `rustamp-mediahost` | `docs/rfc/0006-out-of-process-hosting.md` | deep |
| 8 | **ReplayGain** — the M2 item, with the scanner | `rustamp-audio`, `rustamp-meta` | `docs/PLAN.md` M2 | deep |
| 9 | **The S10 probes** — run the five probes the card designed (dirty room, owner-approved) | research harness | `docs/tasks/S10-probes.md` | orchestrator |

## Released by the owner (2026-10-17) — ready to start

- **RFC 0012, the device-error hook** (`docs/tasks/H4-output-error-rfc.md`,
  `docs/rfc/0012-output-error-hook.md`) - **accepted by the owner, 2026-10-18.**
  The RFC's status line now reads Accepted; the contract change is released and
  may be implemented. Owns `rustamp-core`, `rustamp-output` and the engine's poll
  step in `rustamp-audio`, so it waits only for the audio crate to be free. Read
  the RFC for what it settles - do not implement more than it accepts.
- **The plugin DLL acceptance** (`docs/tasks/P3-dll-acceptance.md`) - **approved
  by the owner, 2026-10-18.** The card's own "not permission to run anything"
  line was true when it was written and is now superseded: the owner has released
  the acceptance work. Owns `rustamp-plughost`, `rustamp-plugin-abi` and the
  VM-only acceptance path. The loader and the Group D tests already exist, so read
  the card for what is still open rather than rebuilding them.
- **The S08 measurement runs** (`docs/tasks/S08-vis.md`). The probe and the
  three jobs are designed; the orchestrator runs them in the VM and the lane
  fills the spec from the artefacts.

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

## Source rotation (owner rule, 2026-10-19)

**Current sources: OpenAI + the free models only.** No new Claude lanes (owner
rule, 2026-10-19). Google is exhausted until 2026-10-24. Spread the lanes over
what is left instead of putting a whole wave on one provider; a source that
refuses means the lane moves to another one, never that it waits.

| Source | How | Notes |
|---|---|---|
| **Free** | `subagent` with `free-*`, or `general` + `model` | `free-deepseek-v4.1`, `free-big-pickle` work; `opencode/minimax-m3.1-flash-free` and `opencode/beam-free` have no visible limit — prefer them |
| **OpenAI** | `subagent` + `model` | use while its window has quota |
| **Claude** | — | **no new lanes** (owner rule, 2026-10-19) |
| **Google** | — | exhausted until 2026-10-24 |

## Polling (learned the hard way, 2026-10-17)

**`agy` jobs do not notify.** A `subagent` lane reports back by itself, but an
`agy_run` lane finishes silently, so the orchestrator must **poll** the job ids it
holds (`agy_status`) after every batch — otherwise finished lanes sit unnoticed
and the slots look empty while the work is done. The rule: after merging anything,
poll every outstanding agy job, verify and merge what is done, and refill **before**
starting new work. Six lanes were merged in one batch that way; before that, five
finished lanes had gone unnoticed for a while.

## Lane keeping (owner rule, 2026-10-17)

**Refill automatically, on every completion.** The owner's instruction: keep free
and Muse lanes running, and do the refilling by itself. The standing rule, applied
whenever a lane reports or an agy job is found finished:

1. **Verify, then merge** the finished lane (both trees, fmt/clippy, the suite).
2. **Refill its slot immediately** from `docs/tasks/LANE-QUEUE.md` and
   `docs/tasks/HARD-TASKS.md` - before starting anything else.
3. **Targets:** two Muse lanes (the owner's floor until 2026-10-18 midday), at
   least one free-tier lane, **one Claude lane through the Claude provider while
   its quota lasts, and it takes the rather difficult work - never agy's Claude
   bucket**, one Codex lane while its quota lasts, and agy filled to its cap of
   four. Ten lanes is a healthy wave.
4. **Match the source to the work:** ordering, threads, clocks, buffers, second
   processes, numeric subtlety and anything whose proof needs the VM go to Codex;
   well-specified, local, checkable-by-reading work goes to a free-tier lane;
   everything in between goes to Muse.
5. **Check the table, not the story:** `powershell -ExecutionPolicy Bypass -File
   docs\notes\lanes.ps1` prints every worktree, how many files it has changed and
   what it is based on, plus the canonical tree's cleanliness. A lane that reports
   work its worktree does not contain is a dead lane; a lane that writes into the
   canonical tree is a broken lane - both have happened.
6. **Worktree first.** Create the worktree before launching the lane; two lanes
   were launched into paths that did not exist yet.
7. **Never wait on a source that failed.** Relaunch on another source
   (`docs/tasks/HARD-TASKS.md` section 1 keeps the failure log).

## How a lane is started

1. `worktree_create` with a name that says what it is (`waveNN-<topic>`).
2. A prompt with: the worktree path as the **first line**, the boundary rule, the
   files to read, the deliverable, the hard constraints (no `cargo test`, no
   downloads, no git writes, no window, no audio), and the owned paths.
3. Report back with metadata only; the orchestrator verifies **both** trees,
   runs fmt/clippy, then the VM suite, then merges.

## Ready from the reviews (2026-10-17)

Two adversarial reviews landed. They are the source of the next wave, not prose
to admire.

**M14 — host framework, the two residents** (`docs/tasks/M14-host-framework-review.md`).
19 claims are pinned by a test that can fail, 7 hold in one direction or one
lane only, 5 are asserted in prose alone. Ready work, in the order it rates them:

1. **RFC 0014 is drafted and merged** (`docs/rfc/0014-fallible-dsp-stage.md`,
   `9708063`): a DSP stage must be able to fail loudly *with a message* instead
   of panicking. The owner decides four things before code follows - stop or
   bypass on a stage failure, whether the error earns its own kind, the resident
   restart policy, and confirmation that no out-of-tree implementer exists.
2. **A reset does not wake a worker blocked in an empty-ring read**
   (`mediahost/src/dsp.rs:176`, `control.rs:191`); latent today because the host
   only resets between runs.
3. **A truncated file can still present as a clean short end of stream**
   (`dechost/tests/real_decode.rs:326` accepts either terminal).
4. **The prose-only gaps**: the URL refusal, an abrupt-player-loss mirror for
   the decoder, a bounded-read timeout mirror for the DSP.
5. **The test rings ignore the ring direction production refuses**
   (`mediahost/tests/pcm_transfer.rs:121`).

**W10 - the six-lane batch** (the review was running when this was written).

**VM-only, and therefore not a lane's work:** both two-process spawn-and-kill
tests, real resident runs, the engine-side adapters, latency compensation, and
any contention behaviour.

## Google lanes go through Paseo (owner rule, 2026-10-18)

Antigravity lanes are started with `paseo run --provider antigravity` (modes
Default, Auto Edit, YOLO) instead of the agy bridge in OpenCode, because Paseo
can be watched: `paseo ls` lists the agents, `paseo logs <id>` shows the
timeline, `paseo attach <id>` follows a live run, and `paseo permit` answers a
permission request. The agy bridge does none of that and, worse, reports nothing
when a job finishes - which is how four finished lanes once sat unnoticed for
three and a half hours.

What Paseo can and cannot drive, as its own provider list reports it:

| provider | status | use |
|---|---|---|
| antigravity | available | the Google lanes, from now on |
| claude | available | Claude lanes (a comparison with the OpenCode route is in flight) |
| opencode | available | possible, unused so far |
| pi | available | a further source |
| codex | **unavailable** | Codex stays on the OpenCode subagent route |
| muse | **unavailable** | Muse stays on the OpenCode subagent route |

Jobs already running through the agy bridge are left to finish rather than
duplicated; every new Google task starts through Paseo.