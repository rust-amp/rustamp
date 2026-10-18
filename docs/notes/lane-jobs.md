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
| `9f38e282` / `89c6d365` | Paseo-Antigravity `paseo-wave-review`, `paseo-release-checklist` | 2026-10-18 11:20 | wave review (`W16`) and the executed release checklist - **both merged** (`f634b51`, `35faaf5`) |
| `paseo-wave90-audio` | Paseo-Antigravity | 2026-10-18 11:45 | W16-F02 (the orphaned `PlaybackOrder`), W16-F04 (`Prev` at a cycle start), W16-F07 (non-finite PCM sanitised before the tap) |
| `paseo-wave91-prev` | Paseo-Antigravity | 2026-10-18 12:40 | the one new code red r51 named: `shuffle_repeat_prev_across_cycle_boundary`, the F04 fix's own test |
| `wave91-focus` | Claude subagent | 2026-10-18 12:40 | land the window focus model rescued from `wave26-windows`: `FocusNextWindow` (Ctrl+Tab) is `not implemented yet` in the tree, and `ShowMainWindow` only sets a flag |
| `wave91-bless` | Claude subagent | 2026-10-18 12:45 | a `--bless` path through the broker, so the two `vis_stopped_tail` goldens can be re-blessed in the guest and copied back |

**Outstanding after the merges:** the two stopped-tail goldens
(`tests/golden/lookalike/vis_stopped_tail.png` and
`tests/golden/rust-theme/vis_stopped_tail.png`) were rendered under the old
decaying stop and must be re-blessed **in the VM** before the golden test can be
green again. Expect them red in the next suite run, and do not read that as a
regression. The mechanism: `cargo xtask golden --bless` is not a renderer, it
runs the `golden_*` test functions, so it is `cargo test` and belongs in the
guest. Bless them there, copy the two PNGs back through the broker, and commit
them.

**Google now starts through Paseo, not the agy bridge** (owner rule, 2026-10-18).
Paseo agents are watchable (`ls`, `logs`, `attach`, `permit`) and their worktrees
live under `C:\Users\micro\.paseo\worktrees\3ikyavhh\`. `agy` stays paused as
described below. Codex and Muse stay on OpenCode subagents, which do notify.

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
| `paseo-tray-findings` | the four small tray findings the verification left behind (`bc828a7`) |
| `paseo-wmapp` | answered, not merged: winit 0.30.13 uses no `WM_APP` messages, so `WM_APP + 1` is collision-free - no change needed |
| `wave89-merge-reds` | the mediahost helper now skips liveness frames; the xtask layout test can no longer be flipped red by a file lock (`c6a0765`) |
| `wave89-shuffle-reds` | **the engine now owns the shuffle plan** (the worker built a second, divergent one), the stopped navigation, and three waits that did not wait (`6a606ef`) |
| `paseo-wave-review` | the adversarial wave review, W16 (`f634b51`) |
| `paseo-release-checklist` | the eighteen release steps classified and the host-side ones executed with exit codes (`35faaf5`) |
| `paseo-wave90-vis` | the visualiser clears the moment playback stops, in every mode, and the test no longer dodges the claim (`f7b09ea`) - **two stopped-tail goldens must be re-blessed in the VM** |
| `wave90-xtask` (Claude) | the full MPL-2.0 and Apache-2.0 texts shipped in the notices, the false placebo sentence in the generated README corrected, and a hand-written PE import inspector that refuses rather than passes (`ff644db`) - **closes release step 10** |
| `paseo-wave90-audio` | deleted the orphaned `PlaybackOrder` with the layering evidence, restored the previous cycle plan on `Prev`, and kept the NaN sanitizer because RFC 0014 mandates it (`702719c`) |
| `wave91-bless` (Claude) | a `--bless` path through the broker: the guest learns the token beside `--ignored`, the host gains a bless-output step and a gate on missing images (`2b54468`) |
| `wave91-focus` (Claude) | **landed the rescued focus model**: `FocusNextWindow` cycles the open windows and `ShowMainWindow` now restores and focuses, hand-rebased with a Minimize/Show ordering fix and the S03 correction (`b17aa81`) |
| `paseo-wave91-prev` | the one code red r51 named: **the test was wrong, not the fix** - it compared queue order with cycle order, and the wrap check is now stronger (`7841807`) |

### Where the W16 review stands

| Finding | Outcome |
|---|---|
| W16-F01 engine/worker shuffle divergence | **closed by `6a606ef`**, independently confirmed by the review and re-audited by the audio lane |
| W16-F02 orphaned `PlaybackOrder` | **closed** (`702719c`) - deleted, with the layering argument recorded |
| W16-F03 shuffle state at the queue end | **closed by `6a606ef`**, re-audited |
| W16-F04 `Prev` at a cycle start | **closed** (`702719c`) |
| W16-F05 mid-run toggle handover | **closed by `6a606ef`**, re-audited |
| W16-F06 spectrum ghost after stop | **closed** (`f7b09ea`) - two goldens still need re-blessing in the VM |
| W16-F07 non-finite PCM sanitised before the tap | **refuted, not fixed**: RFC 0014 section 4.5 mandates the sanitizer and a pinned test asserts it, so the decoder is right. What remains is one **contract amendment in `rustamp-ui`**, written out in `docs/tasks/W17-audio-findings.md` section 3: `VisError::NonFinitePcm` is an offline validation invariant, not a live pipeline error, and must not halt playback. Small, and still open. |
| W16-F08 licence texts | **closed** (`ff644db`) |
| W16-F09 stale placebo README | **closed** (`ff644db`) |

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

## r49 (2026-10-18 11:10) - the first run on the upgraded VM

`ran 82, failed 4, hung 0`, on 8 vCPU. **Every old red is gone**: the gapless
seam, the audio resident, `dsp_roundtrip`, `real_decode`, the UI and the app no
longer appear. The four failing binaries were all from the newest merges, and
their five tests are named in the cards: the two S15 shuffle tests, the gapless
stop test, the mediahost decoder-open helper and the xtask layout check. Two
lanes took them (`c6a0765`, `6a606ef`), and the review independently confirmed
the same engine/worker divergence the shuffle lane had found.

## r50 (2026-10-18 11:35) - the confirmation run

Started against `6a606ef` with all five fixes merged. Read `[FAIL]` / `[HNG]`
before drawing any conclusion from it.

**Result:** `ran 82, failed 2, hung 0`. The four r49 reds are closed, and the
two remaining are **not** code defects. `dsp_roundtrip`
(`reset_keeps_rings_and_continues`, `dsp_roundtrip.rs:180`) and `real_decode`
(`end_of_stream_comes_only_after_the_ring_is_drained`, `real_decode.rs:155`)
both hit the **30 s stall escape** with "the resident stopped delivering" -
while the very same binaries ran in **0.26 s** and **0.85 s** in r49.

Two hypotheses, both settled:

- **A regression from the five fixes is refuted by the dependency graph.**
  `rustamp-dechost` and `rustamp-dsphost` depend on `rustamp-core`,
  `rustamp-decoders`/`rustamp-ipc`, `rustamp-mediahost` and `rustamp-proto` -
  **not on `rustamp-audio`**, which is the only crate `6a606ef` touched, and the
  `decoder.rs` change in `c6a0765` is a test helper. A logic defect also fails
  fast; it does not hang for exactly the stall bound.
- **The cause is host load.** r50 ran while three lanes were doing
  `cargo clippy --workspace --all-targets --all-features` and
  `cargo build --workspace` on the same machine as the 8-vCPU guest. A binary
  that normally finishes in a quarter of a second took thirty. The 30 s class is
  therefore **load-induced**, not a property of the guest alone: the upgrade
  reduced it, but the orchestrator can recreate it by building while the suite
  runs.

**Rule for every suite run from now on:** the VM suite owns the host. Do not
start lanes that build, and do not build, while a suite run is in flight. A
targeted re-run of those two binaries on a quiet host decides it.

## Stray file in the canonical tree (2026-10-18 11:49)

An empty `package.rs`, zero bytes, appeared in the **canonical tree root**
`D:\code\Rustamp` at 11:49:25 - inside the window in which the xtask lane
started. It was quarantined to
`C:\Users\micro\AppData\Local\Temp\opencode\stray-package.rs` rather than
deleted, so the evidence survives while the tree stays clean, and the lane's own
worktree is untouched and correct (`xtask/src/package.rs`, a new
`xtask/licenses/MPL-2.0.txt`, a new `xtask/src/pe_imports.rs`).

It is the boundary-violation class the notes already warn about, and the likely
mechanism is worth naming: **an OpenCode subagent inherits the session working
directory**, so a shell redirect or a bare file creation inside a lane whose
prompt says "write only in your worktree" can still land in `D:\code\Rustamp`.
The lane's edits are unaffected because they used absolute paths. The rule that
follows: a lane that runs commands must `Set-Location` into its worktree first,
and the orchestrator must check `git status --short` in the canonical tree
before every merge - which is how this one was caught.

## r51 (2026-10-18 12:21) - the run on a quiet host, and the load question settled

`ran 82, failed 3, hung 0`, with **no lane building** while it ran. The two
binaries that stalled at the thirty second bound in r50, `dsp_roundtrip` and
`real_decode`, are **green here**, in the same commit's code. That settles it:
the stall class was the host load three lanes put on the guest, not a property
of the code or of the guest alone. The rule stands - the suite owns the host.

The three reds, and only these:

- `golden_lookalike_visualization` and `golden_rust_theme_visualization`:
  **expected**. Both fail on `lookalike/vis_stopped_tail` and
  `rust-theme/vis_stopped_tail` ("39 of 31900 px differ"), which is exactly the
  re-bless the visualiser lane predicted. Not a regression.
- `shuffle_repeat_prev_across_cycle_boundary` (`shuffle_repeat.rs:370`,
  `left: 2, right: 0`): **a real red**, and it is the F04 fix's own new test
  failing against the fix. In a lane.

## The `wave26-windows` rescue (2026-10-18 12:45)

Cleaning up 181 accumulated lane worktrees freed 47 GB, and one of the two that
could not be removed turned out to hold **real work that never landed**:

- `FocusNextWindow` (Ctrl+Tab, command id 40063) is `not implemented yet` in
  `crates/rustamp-app/src/app.rs` today.
- `ShowMainWindow` (Alt+W, id 40258) only sets `main_visible`, so it never
  restores or focuses the OS window.
- The rescued patch supplies both: a focus cycle over the docked stack, a
  pending focus request and a pending restore request, 226 added lines plus a
  new `focus.rs`, with the original author's honest note that the order is an
  own choice and what a run would have to do to measure the reference's.

The owner's decision was to land it, so `wave91-focus` owns it. The other
worktree, `wave32-s07-audit`, held a duplicate of
`docs/tasks/S07R1-eq-audit.md`, which the tree already has, and was removed.
Both files are preserved under
`C:\Users\micro\AppData\Local\Temp\opencode\worktree-rescue\`.

## Two runner defects the targeted re-run exposed (2026-10-18 12:00)


The targeted re-run of `dsp_roundtrip` and `real_decode` (`-Filter ... -SkipBuild`)
did not measure what it was meant to measure, and it failed in two ways worth
keeping:

- **`-SkipBuild` reuses a stale, partial staging.** The first run died in
  `Copy-Item` on `target\debug\deps\audit-*.exe`, a file the staged tree still
  listed but no longer had. It failed loudly (exit 1), which is correct, but the
  lesson is that `-SkipBuild` is only safe against a staging a full run produced
  in one piece. A filtered run should stage fresh.
- **A run that executes nothing reported a pass.** The second run printed
  `ran 0, failed 0, hung 0` and exited **0**, because the exit gate only looked
  at `failed` and `hung`. A filter that matches nothing - or a staging that lost
  the binaries - therefore read exactly like a green run. This is the dangerous
  one: it can turn "I proved nothing" into "it passed". The runner now fails
  loudly when `ran` is zero, naming the filter and how many binaries were staged.