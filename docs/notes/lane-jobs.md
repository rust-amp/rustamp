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

## The intermittent two-process stall (2026-10-18 13:00) - open, and it needs a number

`dsp_roundtrip::reset_keeps_rings_and_continues` and
`real_decode::end_of_stream_comes_only_after_the_ring_is_drained` are
**two-process** tests: the test spawns the resident, and the resident delivers
PCM over a ring. Both stall intermittently with "the resident stopped delivering
without a close or an error" at the **30 s** escape, and both finish in a
fraction of a second when they do run.

The record so far, all on the same code path:

| run | dsp_roundtrip | real_decode |
|---|---|---|
| r49 | green | green |
| r50 | **stall** | **stall** |
| r51 (quiet host) | green | green |
| r53 (quiet host, no lane built) | **stall** | green |
| isolated, one binary, quiet host | **stall** | not yet run |

Two things are now settled and one is not:

- **Not the shipped code.** `rustamp-dechost` and `rustamp-dsphost` depend on
  `rustamp-core`, `rustamp-decoders`/`rustamp-ipc`, `rustamp-mediahost` and
  `rustamp-proto`; none of the recent changes touched any of those.
- **Not simply host build load.** It stalled in r53 with no lane building, and
  again in an isolated single-binary run.
- **Open: how often.** A suite that is green four times in five is not the same
  claim as a suite that is green, and the release may not blur the two. Five
  consecutive isolated runs are being measured to get a rate.

`-SkipBuild` is **unusable for this**: it reuses a staging a previous run left
incomplete, which fails either immediately or - worse - reaches the guest with
no matching binary. The `ran 0` gate catches the second case, and the first
fails loudly, but neither is a measurement. Filtered runs stage fresh.

## Measured: 5 of 5 green in isolation (2026-10-18 14:45)

Five consecutive isolated runs of `dsp_roundtrip` on a quiet host, full staging,
`-Reset` each time: **5 passed, 0 stalled**, every guest run finishing in
0.26-0.29 s. The fifth took 190 s of wall time instead of the usual 135-138 s,
which means **lanes were building while it ran** - and the guest test still
passed in 0.27 s. So host load slows the run without killing the delivery.

That weakens the load explanation and leaves a sharper question, which is now
with the Claude diagnosis lane: the stall has appeared **only inside suite
runs** (two of four) and once immediately after a broken `-SkipBuild` staging,
and never in five isolated runs. In a suite run eighty-two binaries execute one
after another, so the candidate is a **resident or thread left behind by a
previous binary** competing with the two-process test, which would explain why
it happens in the suite and not alone.

## Measured rate: 8 of 10 warm, 5 of 5 fresh (2026-10-18 15:10)

The runner now has `-Repeat`, and its first real use measured the stall instead
of describing it. Ten warm repetitions of `dsp_roundtrip` in one guest, no reset
between them:

```
RATE filter='dsp_roundtrip': 8/10 passed (80,0%); 2/10 failed; 2/10 stalled
stalls at repetitions 2 and 9 - not adjacent
passing runs 3.0 s, stalling runs 33.1 s
```

Combined with the five fresh-guest runs (each with its own `-Reset`, all green),
that is **13 of 15 runs green, about one stall in seven**, with no cargo process
running during the measured run.

**A correction to the first reading.** The stall appeared at repetitions 2 and 9,
and repetition 3 ran clean immediately after the stall at 2, so this is **not**
persistent leftover state from a previous run - the first conclusion was too
strong and is withdrawn here. What survives is the plain fact: an intermittent
race in the two-process resident delivery, roughly one run in seven, at the
thirty second escape, with no close and no error, and the same test completing
in a quarter of a second whenever it does run. It is not the shipped code (the
dependency graph) and not host build load (no builds ran). The mechanism is with
the Claude diagnosis lane.

The `-Repeat` feature paid for itself in one run: a quotable rate, a JSON record
with every run's wall time and `resetBetweenRuns: false` stated honestly, and a
non-zero exit despite eight green repetitions, so one failure cannot hide in a
majority.

## r54 (2026-10-18 15:40) - the suite is green, with nothing left red

```
ran 83, failed 0, hung 0        exit=0
```

**The first fully green suite.** Eighty-three binaries, no failure, no hang, and
the new `deaf_wake` regression test passes (2 passed, 0.82 s), which is the
deterministic pin for the stall fix: it removes the wake signal entirely and
still demands bit-exact delivery from the header counters alone.

What is green now, and what each one cost:

| green | closed by |
|---|---|
| both golden binaries | re-blessed in the guest and promoted (`9467e47`) |
| `shuffle_repeat` | the corrected cycle test (`7841807`) |
| `dsp_roundtrip`, `real_decode` | the sliced PCM waits (`b67a5bc`) |
| `deaf_wake` | new, and it cannot pass by luck |
| the focus model, the visualiser clear, the licence texts, the import check | `b17aa81`, `f7b09ea`, `ff644db` |

## r53 (2026-10-18 13:35) - one red, and it was the stall

`ran 82, failed 1, hung 0`: the goldens were green and `shuffle_repeat` was
green, and the single red was `dsp_roundtrip` at the thirty second escape. That
run is what the rate measurement and the diagnosis grew out of.

## The release wave: six lanes merged, and the host work is proven (2026-10-18 21:20)

Merged since the owner released everything: `e585505` (the command-line half of
playlist loading, pinned, after proving the readiness line was stale),
`f6856ce` (shuffle and repeat are applied, refuting the readiness line, with the
one remaining lie named), `964d2d1` (the wake proved across the process boundary
on the ring that actually ships), `f52eb4b` (the device-error hook is largely
already implemented, with the three gaps named), `9b7ef7c` (the plugin DLL
acceptance taken forward, after writing down what was already done), `427c3de`
(projectM vendored with recorded provenance and the wrapper written against it).

**The two most valuable findings of the wave were refutations.** The wake harness
that already existed proves the wake across an edge in every one of its five
phases, and that is forced by how a consumer and a producer block - so it cannot
reach the failure class that caused the thirty-second stall and would have kept
passing while the product stalled. And three separate readiness lines turned out
to describe behaviour that had been fixed hours or days after the snapshot they
were written from, which is why every lane in this wave was told to verify its
gap before fixing it.

**projectM is in the tree, and the clean-room question was checked before it
went in.** The vendored release contains no Winamp material at all - no front
end, no SDK header, nothing whose name mentions it - and the library is loaded
dynamically rather than linked, so the permissive crate stays permissive, the
library stays a separate component with its own terms, and the relinking the
licence requires is possible. The provenance records the release and asset
identifiers, the size and the digest, and says plainly that the digest agreement
checks the bytes against the release metadata while no detached signature was
verified and nothing was built or run. The exact narrow packaging patch is in the
card, because the packaging tool was held by another lane.

**The release licence audit must be re-run**: this changes its inventory.

## The owner released everything, and the three open questions are closed (2026-10-18 20:45)

"Alles freigegeben." Recorded here because an approval that is not written down
is the thing that produced today's two wrong queue entries:

- **RFC 0012, the device-error hook: accepted.** The RFC's status line now reads
  Accepted rather than Proposed, and the contract change may be implemented. It
  had been listed in the queue as approved while its own text said Proposed; the
  approval is now real and dated instead of inferred.
- **P3, the plugin DLL acceptance: approved.** Its card's "not permission to run
  anything" line was correct when written and is now superseded by the owner.
- **projectM: implementation and bundling approved**, including the five
  decisions the review lane left open - the front end and build architecture, the
  package layout, the native dependencies and assets, the route by which the
  library source is delivered, and the authorisation to implement. The plan the
  review wrote stands: the library belongs in the player package under its own
  plugin directory, never in the permissive host package or its closure, with a
  visualisation wrapper written here because upstream has not maintained its
  Winamp front end since 2012, and with the licence notices, the third-party
  inventory and the full LGPL text added.

The release tag `0.1.0` was already released earlier today and stands.

## The packaging path is proven end to end on the host (2026-10-18 20:25)

A dry run from `6922e0a` into a clearly named practice directory, so that it could
not be mistaken for the release artefact:

- **`cargo xtask package --release --out ...` exited 0.** The player is x86_64
  with its two residents bundled beside it, the host is i686, the plugin host is
  bundled into both, the manifests are schema 3, and a zip with a checksum beside
  it was written. The release ID is `rustamp-0.1.0+2026-10-18.6922e0a` with
  product version `0.1.0`, so step 7's version consistency holds.
- **`cargo xtask package --check --out ...` exited 0**: layout, manifests, notices,
  the PE import tables of all five executables and the zip format and checksum all
  verified, with no binary executed.
- **Every shipped executable imports only Windows system DLLs** - no CRT DLL, no
  VC redist payload - and the host closure is GPL-free, which is the check that
  matters most for a release.
- What is **not** proven is the first launch on a clean OS, which is step 13, and
  that is exactly the gap the readiness audit named.

**A documentation defect found by doing it:** the release plan gives step 10 as
`cargo xtask pe-imports` with no arguments, and the tool refuses an empty file
list, so the documented command fails. The real packaged check is the integrated
one in `package --check`. The plan is corrected.

## The queue had a stale line, and a lane proved it instead of building a duplicate (2026-10-18 20:05)

Two lanes finished and both are merged (`aa107aa` mediahost, `586a780` the golden
card).

**The golden lane found no work to do, and that was the right answer.** The ready
queue asked for a second set of golden cases - a double-size main window, the
equaliser with a raised band, and a playing main window with a time and a title -
and all three already exist in both committed skin namespaces, with images
tracked in `main`. The line was stale: written before the work landed and never
removed. The proof is ancestry rather than assertion, the commit that introduced
the line being an ancestor of the commit that landed the work and not the other
way round, and the double-size images were checked offline as well - every
two-by-two block matching the single-size pixel over more than 120 000 samples
with no deviation. Adding a duplicate would have forced a blessing of a state
that was already pinned, which is a fabricated expectation rather than a test.
The line is struck from the queue. The one genuinely unpinned neighbour, a
double-size **playing** window, is named in the card with an honest note that its
added value is small because another case already doubles a playing state.

**The mediahost lane took the honest remainder of RFC 0006** and said so rather
than declaring the RFC done. Reading it against the code showed most of what the
RFC asks for already real; what was actually wrong was that decoder statistics
could be lost when a stream ended or changed format, and that a DSP return ring
closed early was reported as an ordinary end of stream instead of a persistent
error - a caller could have believed a truncated stream was complete. Both are
fixed, each with a regression test that fails if the fix is removed, and the
stale comment that explained the ring wake as edge-triggered is corrected. What
is still open is written down, including that a finished write side needs an
unambiguous positive completion signal, because a closed ring alone cannot
distinguish success from failure.

**And a queue contradiction is corrected rather than left to trap a lane:** P3,
the plugin DLL acceptance, is listed both as released by the owner and as needing
a decision, while its own card says plainly that it is not permission to run
anything. The entry now says what it is - a question for the owner.

## Two earlier verdicts overturned, and the consequence named (2026-10-18 19:20)

`f8d3e87`, gates green. Two reds survived the previous round, and chasing them
overturned both verdicts rather than patching the tests.

**The flush contract is the authority, and W20 was wrong.** The core contract
(`crates/rustamp-core/src/audio.rs:361-369`) is explicit: a backend that cannot
flush portably **must refuse** with `Error::Unsupported` rather than return `Ok`
and let stale audio play - "a silent `Ok` is the one answer that is worse than a
failure" - and **the engine treats a refusal as terminal**: it stops the run and
does not resume into an output it cannot clean, so **the caller has to recreate
the engine and the output**. W20 had made `Unsupported` mean "capability gap, so
continue", which is a contradiction of that contract, and the contract test
caught it exactly as it should: skipping the flush lets the listener hear the
stale backlog first. The engine and the contract test are back to the documented
behaviour, and nothing the test claims was weakened.

**The host-window test was not a test bug, and W21's verdict is overturned.**
W21 called it a test bug and added a five-second polling loop; the same test
still failed afterwards, and its runtime jumped from a fifth of a second to
fifteen. The fault was in `supervise.rs`, in the product, and that is where it is
fixed.

**The consequence, named rather than left to be discovered:** with the refusal
terminal again, `crates/rustamp-app/tests/loopback.rs:544` - which asserts the
engine's last error is none after the fixture tone - will fail again on the
virtual cable, because that test is the caller and the caller is the one the
contract obliges to recreate the engine and the output. That is not a test
blemish to be papered over: it is the caller-side duty, and whether the
application does it on a device that refuses the flush is an open question that
this note hands to a lane.

## The flush defect and the three interactive reds (2026-10-18 18:40)

Both merged (`88d17c0`), both with green gates.

**The flush defect is more than a test problem.** The chain: end of stream,
`finish_run`, `flush_output`, and `CpalOutput::flush` returning `Unsupported`
because the backend cannot flush an endpoint buffer. Under the A13 policy
`flush_output` treated **every** error, including that one, as fatal: it paused
the output, closed the device and latched a failure. And `stop()` and
`seek_frames()` call `flush_output` too, so **any stop or seek on such an
endpoint destroyed the output for good**. The sibling playback test passed only
because it never looked at the engine's last error. That is a product defect, not
a strict test, and it is why the cable test mattered.

**All three interactive reds were test bugs**, each decided on its own evidence
and none written off as session 0: the about-dialog test held the version string
hardcoded where the dialog builds it from the crate version, so it now asserts
the same way and cannot go stale again; and the two plugin-host tests asserted
things the headless or older path could not provide. No claim was weakened and no
sleep was introduced.

**One gap named rather than left:** the reset restores a baseline taken before
the virtual cable was installed, so a reset run has no cable afterwards and the
audio group cannot run in the same pass. The clean fix is to put the cable into
the baseline with the broker's checkpoint operation, so every future run has it.

## The interactive run: 8 GUI tests freed, and three reds named (2026-10-18 18:10)

The owner rule earned itself in its first use. With a desktop:

- **All eight Group A GUI tests pass** - the window, the menu popup, the
  equaliser window, the playlist window, the dialogs and the two tests that only
  this lane can evidence. In session 0 they were red or not run.
- **Group C, the four resident fixtures: all pass.**
- **Two ignored tests fail in the interactive session too**, so they are **not**
  the session-0 artefact the others turned out to be, and the readiness audit had
  not listed them as expected-red either: the 32-bit DLL acceptance test that
  proves host interaction (`dll_acceptance.rs:175`) and the host-window
  end-to-end test (`plughost_e2e.rs:237`). Had the rule not been set, "session 0"
  would have been accepted for both and two real findings would have gone
  unnamed.
- **A third red appeared only in the interactive suite run**:
  `about_dialog_body_reflects_workspace_version` (`app.rs:4071`) asserts the
  about dialog says `RUSTAMP 0.1.0-beta.1` while it now says `RUSTAMP 0.1.0`.
  That is the version change colliding with a hardcoded string, and it is the
  orchestrator's own doing: the version lane was told not to touch `crates/**`.
  The honest fix is to assert against the crate version so it cannot go stale
  again.

All three are with a lane. A full guest log is kept at
`C:\Users\micro\AppData\Local\Temp\opencode\vm-three-reds-full.log`.

## Phase 1 in the interactive session: 4 of 4 green (2026-10-18 17:40)

```
gui session 1 as rustamp on DESKTOP-UL3J9H6; screen {X=0,Y=0,Width=1824,Height=768}
ran 4, failed 0
  the_equalizer_window_opens_at_its_docked_size_and_position    ok  0.05s
  the_main_menu_popup_opens_at_the_measured_client_size         ok  0.07s
  the_playlist_window_opens_docked_below_the_main_window        ok  4.68s
  the_windowed_player_opens_a_window_inside_the_test_vm         ok  3.02s
```

**The two predicted winit reds are decided: session-0 artefacts, not defects.**
`headless` and `playlist_window` pass with a desktop, so the rule the owner set is
what decided it, and the two tests that only this lane can evidence are now
green.

Still undecided, and named rather than assumed: the other two reds of the
`--ignored` pass, the host-window end-to-end test and the 32-bit DLL acceptance
test, were not among these four. The new default path decides them the same way
- green there means a session-0 artefact, red there means a real finding.

One environment note from this run: the previous attempt died before any test
because a stray `rustamp-plughost32.exe` held its own file and the staging was
refused with access denied on that path. A guest restart cleared it, and the
staging then succeeded.

## Owner rule: tests run in the interactive session (2026-10-18 17:20)

The guest's session 0 has no desktop. Everything that opens a window, a dialog,
a menu or a plugin host therefore fails there for a reason that has nothing to
do with the product: the four winit binaries, the host-window end-to-end test,
and the 32-bit DLL acceptance test that proves host interaction.

**From now on the suite runs through the interactive-session path.** The
practical consequence for reading a result: a window-shaped failure seen in
session 0 is **not** a product defect until it has been re-run in the interactive
session. Two of the four reds in the `--ignored` pass were predicted for exactly
this reason, and the other two are being decided the same way.

Two related things learned while doing it:

- A stray `rustamp-plughost32.exe` left alive by a previous run **locks its own
  file**, so the next staging fails with access denied on that path before a
  single test runs. The failure is loud and names the file, which is the right
  behaviour, but a restart of the guest clears it and should be part of the
  routine before a run that stages over a previous one.
- The GUI-session path stages far more (436 MB against 84 MB), so it is the
  slower of the two and should not be used for runs that cannot need a desktop.

## The virtual cable, and a real defect it exposed (2026-10-18 16:45)

The guest now has exactly one VB-Audio Virtual Cable: the first install left two
device nodes, `fix-vbcable-once.ps1` removed both and installed one, and after a
stop/start cycle the guest reports exactly one `CABLE Output` and one
`Lautsprecher (VB-Audio Virtual Cable)`, both OK, driver `oem3.inf` version
1.0.3.5 signed by Vincent Burel. The install uses the INF route on purpose: the
vendor setup opens a GUI and waits for clicks that never come in session 0.

The two audio tests then ran against a real endpoint for the first time:

- **`real_playback`: PASS** - the player opens the virtual cable and writes to it.
- **`loopback`: FAIL**, and it is a **product defect**, not an environment one:
  `output flush failed: unsupported operation: cpal endpoint-buffer flush; close
  and recreate the output stream instead; output closed`. The tone is never
  played, because the player treats an unsupported buffer flush as fatal, closes
  the output and kills playback. On any device whose backend lacks that flush the
  player cannot play at all. The error message itself names the remedy.

A lane owns it. This is exactly the class of defect the notes predicted only a
real device could find.

## The ring root fix (2026-10-18 16:05)

`c96935a`. All **six** edge guards in `crates/rustamp-ipc/src/shm/ring.rs` are
gone - the diagnosis listed four PCM sites, but the two visualiser sites carried
the same unsound edge. Every advance now signals; the only surviving guards are
facts about **this** side (a wake-less ring does not look at an event, and a call
that moved no frame signals nothing), which is exactly what made the old ones
unsound. Cost: one extra signal per advance on a wake ring.

**RFC 0007 section 6.2 is withdrawn, not corrected** - the proof's load-bearing
premise is false, and correcting it would have left the unsound rule standing as
the recommendation while the code did something else. The withdrawn text stays
in the document verbatim and dated, with the counterexample, and the two sound
alternatives are recorded as open item O15. Nothing on the wire, in the header
or in the API moves.

The acceptance test that "could not fail" now fails on a zero read, and three
deterministic tests pin the rule through the plain, blocking and visualiser
paths by taking a third handle on the ring latches and requiring a signal on an
advance that does not cross the empty or full boundary. All six of those
assertions fail under the old code.

One small follow-up is named and not done: `crates/rustamp-mediahost/src/pcm.rs`
lines 26-31 still describe the edge as live code. It needs one sentence, not a
behaviour change, and the slice it justifies is still justified without it.

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