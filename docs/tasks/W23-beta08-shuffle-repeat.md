# W23 / BETA-08 — Shuffle and repeat readiness revalidation

Date: 2026-10-18. Reviewed HEAD: `6946dd84`.
Worktree: `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave106-beta08`.
Status: **historical diagnosis partly refuted; NOT closed**. Static evidence,
not a new test or VM result. No production implementation changed.

## Boundary, sources and stop condition

Write only this worktree, and nothing under `D:\code\Rustamp`. Owned paths are
`crates/rustamp-audio/**`, `crates/rustamp-playlist/**` and this card. Application,
UI, IPC, media/plugin hosts and xtask are not owned and were not changed.
Read AGENTS, HANDOFF, CLEANROOM, PLAN, CONTRIBUTING and the complete
`docs/tasks/BETA-readiness.md` before examining the implementation.

The requested spec **does exist**, with exactly the requested name:
`specs/S15-shuffle-repeat.md` (audited 2026-10-18, introduced by `ade10e6`).
Only exported specs, Rustamp source and local Git history were used. No Winamp
source, SDK, disassembly, research tree or reference binary was read or run.
No third-party code, dependency, download, install, commit or Git write.

This finding was recorded before any implementation change. The instruction
to stop rather than decide S15's open cases applies: section 7 still leaves
manual Next at the final item and the elapsed-time-dependent Prev case open.
The application changes below are also outside ownership. This card must not
be integrated as a claim that BETA-08 is fixed.

## 1. What the original readiness line gets wrong today

The review assessed `f9012dc6` on 2026-10-17. Its assertion that the engine
ignores mode commands and the worker only increments/decrements is no longer
true. Do not implement another unused order planner to repair that old claim.

| Current evidence (paths relative to the worktree) | Finding / commit evidence |
| --- | --- |
| `crates/rustamp-app/src/app.rs:872-889` | Every command reaches `Engine::apply`, **before** app ownership is checked. The stale mode ownership match in `routing.rs:49-52` does not prevent forwarding. Forwarding is from `efd1d884`. |
| `crates/rustamp-audio/src/engine.rs:904-907` | All four toggle/set commands actually call the mode setters, since `ade10e64`. |
| `crates/rustamp-audio/src/engine.rs:557-612`, `620-630` | Snapshot modes are engine fields; setters forward to the worker and publish `ModesChanged`, since `ade10e64`. |
| `crates/rustamp-audio/src/decode.rs:607-632` | Natural end and Next choose sequential/shuffled successors; repeat wraps sequentially or replans a shuffled cycle. All four combinations have an actual implementation, since `ade10e64`. |
| `crates/rustamp-audio/src/decode.rs:634-697` | Prev and SkipBy use the active traversal order, not only visual queue indices. Previous-cycle restoration was added by `702719cb`. |
| `crates/rustamp-audio/src/decode.rs:540-559`, `1120-1154` | The worker adopts the engine plan and derives its position from its current decoder track. Plan handover/fallback reporting was repaired by `6a606ef9`. |
| `crates/rustamp-playlist/src/lib.rs:36-40` | Playback traversal is explicitly audio-owned now. `702719cb` deleted the orphan playlist `PlaybackOrder`; `order.rs:27-42` only randomizes the visible list. S15 section 5 distinguishes this from shuffle playback. |

Therefore the old universal placebo diagnosis is refuted for both flags and
both traversal orders. This does **not** prove immediate mid-run policy changes
or truthful lights in every lifecycle state.

## 2. Narrower real gap: a mode can be lit after its consumer has exited

Deterministic source-level counterexample, no sleep or timing race needed:

1. Queue two 1,000-frame test tracks, shuffle OFF, repeat OFF. Default ring
   capacity is 16,384 frames (`engine.rs:171-189`), so both tracks fit without
   rendering any audio. Play and wait on the actual `end_of_stream().is_some()`
   condition. The existing harness exposes exactly that condition as
   `tests/common/mod.rs:496-500` (`wait_buffered`). Transport is still Playing.
2. The worker has selected no successor, sent EndOfStream and broken out
   (`decode.rs:699-709`). Its wrapper sends Stopped and Returned and exits
   (`decode.rs:425-438`). These are decoder lifecycle reports, not evidence
   that the output ring has finished playing.
3. `on_decode_out` latches EndOfStream and clears the command sender on Stopped
   (`engine.rs:1390-1414`). `poll` deliberately keeps transport Playing until
   all real buffered frames are consumed (`engine.rs:1279-1290`).
4. Set Repeat ON now, before draining the ring. `set_repeat` sets the flag,
   calls a best-effort `send` and unconditionally publishes the ON modes
   (`engine.rs:621-630`, introduced by `ade10e64`). `send` silently does nothing
   without a sender (`engine.rs:1684-1689`, originating in `c73d33d8`). No new
   worker is started and the latched EndOfStream is not revoked.
5. Drain the original 2,000 frames and poll: `finish_run` still stops this run.
   Repeat remains true in `state()` and the ON event has already been sent.
   Its `queue_exhausted` check even suppresses PlaylistEnded because repeat is
   now true (`engine.rs:1453-1465`); no PlaylistEnded is not proof of looping.
   This contradicts S15 section 4.3: enabling repeat on the final playing track
   must loop at its end. The counterexample works with shuffle ON as well once
   a finite permutation has been fully decoded.

Shuffle has the same missing-consumer window: `set_shuffle` publishes a plan
and ON flag (`engine.rs:586-612`) but cannot reorder already buffered successor
audio or send the plan to an exited worker. Do not claim that a changed plan
alone demonstrates a changed audible order. In the two-track example, enabling
shuffle while track 0 is still audible must preserve track 0 then visit track 1
once (S15 section 4.1), not anchor a new plan to decoder-prefetched track 1.
`on_decode_out(TrackStarted)` eagerly updates `current` (`engine.rs:1298-1325`),
even before the device reaches that track's boundary. The meaning of "current"
for mode changes needs particular care in the repair.

This is an implementation gap in the **audible** queue-boundary rule and the
engine/worker lifecycle, not absence of any shuffle/repeat algorithm. Existing
history commits fix other real defects; none of the cited setters repairs this
already-decoded/unconsumed state.

## 3. Application handoff required (not edited)

Exact changes needed from the application owner:

1. In `crates/rustamp-app/src/app.rs:216-228`, apply configured shuffle/repeat
   to the constructed engine before the initial snapshot / first Play. The
   constructor currently seeds only app/UI fields (`255-256`, `308-309`), then
   overwrites them with the engine's false/false snapshot (`323-324`,
   `1008-1014`, `1033-1034`). This is a separate restored-settings failure,
   **not** proof that a visible ON light always lies. Constructor seeding is
   still from `efd1d884`; EQ configuration was subsequently added by `b6d1d5f7`.
2. Remove independent shuffle/repeat mutation/publication in `apply_app`
   (`1516-1530`) and classify these commands as engine-owned in `routing.rs`.
   Preserve the existing unconditional engine forward (`app.rs:884`). Use the
   engine's accepted mode state as the sole authority for app/UI/host state.
   Update the stale ownership comments (`routing.rs:13-16`,
   `app.rs:1042-1051`) and associated ownership tests, not just their names.
3. Until the buffered-boundary repair is delivered, unsupported live mode
   changes must not appear to succeed. Any disabled/unavailable presentation
   must cover mouse, keyboard/menu and external command routes, and must not
   merely turn a light off while old repeat audio continues playing. This
   requires coordination with the UI owner; no UI change was made here.

No workaround was added in audio that reaches into application config or
pretends an unsupported live update is applied. No second orphan planner was
restored in the playlist crate. Source layout should be agreed with the owner:
the current integration deliberately moved traversal to audio (`702719cb`),
whereas this task requests the playlist next-item rule as the policy home.
Any extracted shared policy must actually be consumed by both live and stopped
navigation; testing only an unconsumed helper would not close this card.

## 4. Spec decisions and unanswered questions

S15 settles permutation without replacement; all four natural queue-end
combinations; Prev history; relative movement in traversal order; keeping the
audible track uninterrupted during toggles; immediate repeat boundary effects;
not reordering the visible playlist; and empty/single-item queues (sections
1-6). PRNG choice and cross-cycle anti-repeat are explicitly Rustamp choices.

Still open, and **not decided by this lane**:

- Section 7.1 / section 3.1: exact manual Next at the last non-repeating track
  (stop, restart or clamp). Which outcome should acceptance require?
- Section 7.2: whether elapsed time changes Prev from previous-track to restart,
  and, if so, the threshold. No two-second rule was invented.
- Section 7.3: the reference player's seed source remains unmeasured; Rustamp's
  explicitly permitted seeded choice is not a measured reference fact.

The buffered repeat counterexample does not require selecting among those open
alternatives: it uses natural completion, whose required looping is settled.
Resolving manual-navigation acceptance and the cross-lane truthfulness changes
is nevertheless required before claiming the entire readiness item closed.

## 5. Test coverage, proposed regressions and VM acceptance

**Tests added: none. Tests changed/weakened: none.** This is an evidence and
blocked-handoff card, not a fix with passing-test claims. No cargo test, test
binary, player, VM, broker, GUI or reference process was invoked.

Existing `crates/rustamp-audio/tests/shuffle_repeat.rs` exercises finite queue
completion (`28-62`, `95-125`), manual order/history (`154-225`) and mid-run
events (`228-289`). Its repeat tests (`65-92`, `128-151`) pump without first
proving real audio crossed a cycle; the latter does not actually assert the
anti-repeat property named in its title. They cannot certify this gap closed.
These assertions were not weakened or reported as a new passing run.

Required regressions for the repair, to be executed by the orchestrator only:

| Regression | Construction and what makes it fail |
| --- | --- |
| Repeat ON after prebuffered EOS, both shuffle states | Wait for decoded EOS while output remains unconsumed, enable via `Command::SetRepeat(true)`, drain first cycle, require second-cycle PCM and Playing with no PlaylistEnded. Current code stops after the first cycle. |
| Repeat OFF near an audible final boundary | Prefill enough audio to include a future cycle, turn repeat off while final current-cycle audio is audible; require exactly its remaining PCM, then stop. Fails on leaked prebuffered repeat audio or unbounded continued play. |
| Shuffle ON/OFF after successor prefetch | Hold rendering until successor is decoded; toggle while first-track PCM is still audible. Require no interruption/duplication of current PCM and the correct subsequent PCM sequence under S15 section 4. Fails if decoder current is mistaken for audible current. |
| All four natural modes | Compare patterned rendered samples/track boundaries against the published permutation across a full cycle (and a second repeat cycle), not only flags. Fails on silence, skipped/duplicated tracks, wrong successor or same-track shuffled loop seam. |
| App modes / restore / host snapshot | Start from all config flag pairs, then issue set/toggle commands through UI and external paths; compare engine, lights and host snapshot after acceptance. Fails on config reset, independent flag drift or false success for unavailable behavior. App/UI ownership required. |

Use channel barriers or observed engine/ring conditions, never added sleeps as
ordering. Bound waits only as stall escapes. VM-only proof still required:
all regressions and the complete suite in the interactive session, native
light/menu/key behavior, real output continuity at cycle/toggle boundaries and
the unresolved reference observations exported through the clean-room pipeline.

## 6. Gate and boundary receipt

Local compile-only gates completed; each exit code was captured into a variable
immediately after its cargo command:

| Gate | Exit | Worktree-local log |
| --- | --- | --- |
| `cargo fmt --all -- --check` | 0 | `target/beta08-fmt.log` |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 0 | `target/beta08-clippy.log` |
| `cargo build --workspace` | 0 | `target/beta08-build.log` |

`CARGO_NET_OFFLINE=true`; Cargo home was a private existing-cache snapshot in
`target/beta08-cargo-home` (copy exit 1 = files copied, no download). Target,
TEMP and TMP were explicitly inside this worktree. Build/cache/log artifacts
are ignored under `target/`; no shared target or external writable cache used.
Machine-readable exit receipt: `target/beta08-gate-exits.txt`.

No test run was authorized or performed. Initial worktree `git status --short`
was empty. Final source delta is only this new card; production file delta and
added test count are both zero. `git diff --check` exited 0, and the new card
was separately checked for trailing whitespace (zero matching lines).
Canonical-tree `git -C D:\code\Rustamp status --short` exited 0 with **zero
output lines**. The final boundary check used `GIT_OPTIONAL_LOCKS=0`.

`git merge-base --is-ancestor <revision> HEAD` returned 0 independently for
the original reviewed revision `f9012dc6` and fixes `ade10e64`, `6a606ef9` and
`702719cb`: these are actual ancestors, not fixes seen only on another branch.
