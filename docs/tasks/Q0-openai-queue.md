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
3. **`rustamp-dsp` remainder: the limiter (and a ReplayGain hook).**
   Goal: the crate's own doc promises a resampler and a limiter; the
   resampler lives in `rustamp-audio/src/convert.rs`. Prefer a maintained
   crate over a self-implementation (`CONTRIBUTING.md` §Dependencies), and
   implement what is left as a `DspStage` from `specs/S07-eq-dsp.md` (and the
   S18 card), with the same "measured vs own choice" discipline the EQ stage
   documents. Contract-free (no `rustamp-core` change).
4. **A second engine-lifecycle pass, now that F2 is fixed.**
   Goal: re-run the invariant audit of `docs/tasks/A3-engine-lifecycle-review.md`
   against the post-F2 code and close what is still open (the three
   "could not verify" items: a concurrent finish_run/stop race, an mpsc
   disconnect, and hardware `played_frames` monotonicity).

## Done since this card was written (do not queue again)

- **Engine lifecycle review** — done by `wave33-engine-lifecycle` (`adbc81f`),
  F1 fixed, F2 handed to `wave35-ring-drain`.
- **DSP layering RFC** — done as **RFC 0011** (`7b69f92`): the measured
  dB↔0..63 conversion moved to `rustamp-proto`; `rustamp-dsp` no longer depends
  on `rustamp-plughost` at all.
- **The EQ slider dead zone** (S07R1 finding 3) — done as `b39fb7f`.
- **The EQ UI desync** (S07R1 finding 2) — `wave35-eq-ui-sync`, in flight.

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
