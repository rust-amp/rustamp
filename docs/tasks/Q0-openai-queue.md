# Q0 — OpenAI 5h-window queue (owner rule, 2026-10-16)

**Rule (owner):** the OpenAI provider is set up in opencode. Spend its tokens
**sensibly until Sunday 2026-10-18**: whenever a 5h window resets, the
orchestrator immediately starts the next sensible task on an OpenAI model.
**After the Sunday reset, use no OpenAI model again until the owner explicitly
approves it.** One lane per task, each in its own worktree, verified before
integration like every other lane.

"Sensible" here means a task a strong model does better than a free one:
review with `file:line` evidence, design-heavy implementation from a spec, or
audit-grade verification. Nothing that a free lane can do just as well.

## Ready to run (top first)

1. **M3h Group B — the plugin-host findings that need spec decisions.**
   Source: `docs/tasks/M3h-codex-findings.md` (Group B). Goal: decide each
   Group-B item against `specs/S10-ipc.md` and the audited S09, then implement
   the ones the spec settles. Owned paths: `crates/rustamp-plughost/src/**`,
   `specs/S10-ipc.md` (read). Worktree: `wave30-codex-m3review` is finished;
   open a fresh one.
2. **M3h Group C — robustness of the host chain.**
   Source: `docs/tasks/M3h-codex-findings.md` (Group C). Goal: close the
   robustness findings (shutdown, error paths, lifetime) with tests. Owned
   paths: `crates/rustamp-plughost/src/**`,
   `crates/rustamp-app/src/{host,supervise}.rs`.
3. **Engine lifecycle review (post-`d9fbdcd`).**
   Goal: review `rustamp-audio`'s run lifecycle for the *class* of bug just
   fixed — every place that re-bases `clock_origin`, `timeline_origin`,
   `track_start`, `sync.written` or `sync.active` on stop/pause/seek/finish,
   and prove the invariants (`buffered_frames()`, `position_frames()`,
   `timeline_frames()`) hold across every transition. Deliverable: findings
   with `file:line` plus tests for any hole. This is review-grade and exactly
   what a strong model adds.
4. **`rustamp-dsp` remainder: the limiter (and a ReplayGain hook).**
   Goal: the crate's own doc promises a resampler and a limiter; the
   resampler lives in `rustamp-audio/src/convert.rs`. Design and implement the
   limiter as a `DspStage` from `specs/S07-eq-dsp.md` (and the S18 card), with
   the same "measured vs own choice" discipline the EQ stage documents.
   Contract-free (no `rustamp-core` change).
5. **DSP layering RFC.**
   Goal: draft the RFC that moves the measured `eq_classic_from_db` conversion
   out of `rustamp-plughost` into `rustamp-core`/`rustamp-proto`, so
   `rustamp-dsp` can drop its dev-dependency on the plugin host. Small, but
   it touches a contract crate, so it needs the RFC first.

## Not ready (do not queue yet)

- **EQ reference shapes** (the real filter Q, centres, band law, overs): needs
  the reference binary and a capture/audio-level measurement — a dirty-room
  task, owner approval required, and a spec first.
- **Group A follow-ups**: the two soundness fixes are already merged
  (`588adee`); anything left is inside Group B/C above.
- **`xtask package`**: deferred by the owner to release time.

## After each run

- The lane's report is a claim. Verify the artefacts in **both** the worktree
  and the canonical tree, run `fmt`/`clippy`, then the VM suite, then merge.
- Record which task ran and what it found here (append, do not rewrite).
