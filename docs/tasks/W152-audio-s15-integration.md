# W152 — The audio engine drives the S15 playback order model

Date: 2026-10-19. Base: `7e0df14` (Merge wave149-external-control).
Worktree: `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave152-audio-s15-integration`.
Status: implemented and verified; **uncommitted in the worktree** (no git write, no
commit, no download, no window, no audio device).

## 1. Scope, boundary, sources

Owned paths: `crates/rustamp-audio/**`, `crates/rustamp-playlist/src/order.rs` (one
additive method, see §7) and this report. `rustamp-app`, `rustamp-ui` and the
protocol crates were not touched; the canonical tree `D:\code\Rustamp` was not
written to.

Sources: `specs/S15-shuffle-repeat.md` (the audited spec), the S15 model in
`crates/rustamp-playlist/src/order.rs`, the existing engine tests, and the local
git history of this repository. No Winamp source, SDK header, disassembly, no
`D:\code\rustamp-research`, no reference binary.

Delta: `crates/rustamp-audio/{Cargo.toml,src/decode.rs,src/engine.rs,tests/shuffle_repeat.rs}`,
`crates/rustamp-playlist/src/order.rs`, `Cargo.lock` (one workspace dependency
edge) and this report. The five source files: 545 insertions, 642 deletions.

## 2. What was replaced

The engine and the decode thread each carried their **own** shuffle/repeat
machinery. Two planners, two PRNGs, two sets of state, and a handover of a
copied plan between them. All of it is gone; the traversal now lives in exactly
one place, the `PlaybackOrder` model of `rustamp-playlist` (S15 §1–§4).

| Removed from the engine (`engine.rs`) | Replaced by |
| --- | --- |
| fields `shuffle`, `repeat`, `shuffle_seed`, `shuffle_plan`, `prev_shuffle_plan` | one field `order: Arc<Mutex<PlaybackOrder>>` (the model) plus `order_stale` |
| `rng_step` (a private xorshift64*) | the model's generator (`rustamp_playlist::rng::Rng`) |
| `replan_shuffle_engine`, `replan_shuffle_new_cycle_engine` | `PlaybackOrder::set_shuffle` / `next_index` (the model plans) |
| `ensure_shuffle_plan`, `shuffle_pos_of_current` | the model owns the plan *and* the position in it |
| index arithmetic in `next_track`, `prev_track`, `skip_by` | `order.next_index()`, `order.prev_index()`, `order.skip_by()` |
| `DecodeMsg::SetShuffle { on, plan, prev_plan }` (a copied plan handed to the worker) | `DecodeMsg::ModesChanged { shuffle, repeat }`; the worker reads the same model |
| `DecodeOut::ShufflePlanned { plan, prev_plan }` (the worker reporting its plan back) | removed: there is one plan, and both sides read it from the model |

| Removed from the decode worker (`decode.rs`) | Replaced by |
| --- | --- |
| fields `shuffle_plan`, `shuffle_pos`, `prev_shuffle_plan`, `rng_state`, `cycle_boundaries` | the shared model; the worker keeps only the *modes it last saw* |
| `rng_next`, `rng_below`, `shuffle_slice` | the model's generator |
| `init_shuffle_plan`, `replan_shuffle_new_cycle` | `PlaybackOrder::next_index` (the model draws the next cycle) |
| the hand-written traversal in `Worker::run` for `Ended`/`SkipNext`/`SkipPrev`/`SkipBy` | `advance_in_order`, `next_in_order`, `prev_in_order`, `skip_in_order` |
| the `clamped_index` helper (dead after the switch) | `PlaybackOrder::skip_by` |
| `Config { shuffle, repeat, shuffle_seed, shuffle_plan, prev_shuffle_plan }` | `Config { order: Arc<Mutex<PlaybackOrder>> }` |

## 3. What the engine delegates

| Engine API | Delegates to |
| --- | --- |
| `Engine::shuffle()`, `Engine::repeat()`, `Engine::state()` | `order.shuffle()`, `order.repeat()` — the published state is read back from the model, so the lights and the traversal cannot disagree |
| `Engine::set_shuffle(on)` | `order.set_current_index(anchor)` + `order.set_shuffle(on)`; the model re-plans the permutation around the audible track (S15 §4.1) |
| `Engine::set_repeat(on)` | `order.set_repeat(on)`; the repeat boundary is the model's (S15 §2) |
| `Engine::set_shuffle_seed(seed)` | `order.set_seed(seed)` — the one additive change in the playlist crate (§7) |
| `Engine::shuffle_plan()` | `order.shuffle_plan()` (a copy; the signature changed from `&[usize]` to `Vec<usize>` because the model owns the plan) |
| `Engine::next_track()` (stopped) | `order.next_index()` |
| `Engine::prev_track()` (stopped) | `order.prev_index()` |
| `Engine::skip_by(delta)` (stopped) | `order.skip_by(delta)` |
| `Engine::set_queue()`, `Engine::set_current_index()` | `order.set_len()` / `order.invalidate()` / `order.set_current_index()` — a queue mutation invalidates the permutation (S15 §4) |
| the decode thread at every track boundary | `order.next_index()` — the same permutation the engine publishes |

The engine keeps exactly one thing of its own: `order_stale`, the flag that says
"the queue changed while a run was live". While a run is live, the decode thread
plays the queue snapshot that run started with, so the order keeps describing
*that* queue; the engine takes the order over again when the run ends (the
order is re-synced at the next run start and before every engine-side
navigation).

## 4. The engine ↔ worker protocol

One message, `DecodeMsg::ModesChanged { shuffle, repeat }`, replaces the two
mode messages. The engine writes the modes into the model **first** (so the
published state is current at once) and then sends the message; the worker
takes the modes over and notices what changed.

* A **shuffle** change makes the worker re-seat itself in the new permutation:
  `realign_order()` puts the track it is decoding where it sits in the new
  plan, so the track that is playing is not abandoned half-way (S15 §4.1).
* A **repeat** change takes effect at the next queue boundary check
  (S15 §4.3). At the end of the queue the worker revokes the
  `EndOfStream` and continues with the next cycle, as before.

### Ordering semantics of a navigation command

The control channel is first-in, first-out, so the worker knows in which order
the commands and the mode changes were issued. A manual `Next`/`Prev`/`SkipBy`
is therefore evaluated with the **repeat mode it was issued under**
(`Worker::in_issued_mode`): the worker temporarily puts the mode it last saw
into the model, evaluates the command and restores the published mode. A
command that was queued before the toggle is not wrapped by the toggle; a
command that was queued after it is.

This is what the existing regression `manual_advance_sequential_modes` pins
(prev with repeat off stays at track 0; after enabling repeat, prev wraps to
the last track) and it was verified by experiment: removing the issued-mode
handling makes that test fail.

## 5. Tests

New engine tests in `crates/rustamp-audio/tests/shuffle_repeat.rs` (17 tests
in the file now, 13 before):

| Test | What it pins |
| --- | --- |
| `shuffle_traversal_follows_the_published_plan` | the permutation the engine publishes is the permutation the traversal follows; the manual advance lands on `plan[pos]` |
| `repeat_changes_the_live_selection_at_the_queue_end` | without repeat the run ends at the last track and publishes `PlaylistEnded`; with repeat the manual advance wraps to the first track |
| `a_mid_run_shuffle_toggle_is_honoured` | the mid-run toggle re-plans around the audible track (`plan[0] == 0`) and the manual advance follows the new plan; the published state follows the model |
| `repeat_of_one_loops_the_same_track` | a single track with repeat on loops and the run never ends |

The manual advances are asserted with a **frame budget** (8 192 frames) rather
than an open wait: the long test tracks are a second long, so within the budget
the track cannot have ended and the automatic advance cannot have happened —
what the test sees is the *manual* advance.

The model's own behaviour (permutation without replacement, the anti-repeat
guarantee across a cycle boundary, the shuffle history, the mid-run toggle, the
empty and single-entry queue) is already covered by the unit tests in
`crates/rustamp-playlist/src/order.rs`; the engine only delegates, so those
tests are reused rather than duplicated.

**Mutation check.** The worker's `next_in_order` was temporarily replaced by a
sequential `current + 1`; 6 of the 17 tests failed (the four new ones except
`repeat_of_one_loops_the_same_track`, plus `manual_advance_sequential_modes`,
`manual_advance_shuffle_history` and
`shuffle_repeat_prev_across_cycle_boundary`), and all of them passed again
after the change was reverted. The new tests therefore pin behaviour, not the
shape of the code.

## 6. Gates

Run in the worktree, each with its exit code:

| Gate | Result |
| --- | --- |
| `cargo fmt --all -- --check` | 0 |
| `cargo clippy -p rustamp-audio -p rustamp-playlist --all-targets --all-features -- -D warnings` | 0 |
| `cargo test -p rustamp-audio -p rustamp-playlist --all-features -- --test-threads=1` | 0 failures |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 0 |
| `cargo test --workspace --all-features -- --test-threads=1` | 0 failures |

No window, no audio device, no spawned process, no download, no git write.

**One pre-existing flake, not from this lane.**
`crates/rustamp-audio/tests/resident.rs::resident_to_inprocess_source_change_is_gapless`
fails intermittently when the test binaries run in parallel (it waits 30 s for
the resident track to open and the named PCM rings of the parallel tests
collide). It was measured on the **unmodified** base: `git archive HEAD` was
extracted to a scratch directory and the binary was run four times there — it
failed 3 of 4 times, exactly as it does with this lane's code, and passes every
time with `--test-threads=1`. The S15 path is not involved (that test never
touches shuffle or repeat). It is reported here so the orchestrator does not
read the flake as a regression of this lane.

## 7. What still differs from S15 (and from the old engine)

1. **A queued manual command follows the *current* permutation, not the one it
   was issued under.** The shuffle toggle re-plans the single shared
   permutation, so a `Next` that was already in the control queue follows the
   new plan. Repeat keeps the strict issue-order semantics of §4 above. The
   old engine used its old plan for an in-flight command.
2. **A live run keeps its queue snapshot.** A queue mutation during a live run
   does not change the running traversal; the order is taken over again when
   the run ends. A gapless run cannot change the queue it is playing, so this
   is the only consistent reading of S15 §4 for the live case.
3. **The published permutation values changed.** The model draws from
   `rustamp_playlist::rng::Rng` (a seeded xorshift64*), the engine from its own
   private xorshift. For the same seed the permutation is a different one.
   The seed source itself remains an own choice (S15 §7.3 is open).
4. **Turning repeat off mid-run now ends the run at the queue-boundary check**
   (S15 §4.3). The old worker additionally stopped *decoding* at the moment
   the toggle arrived (the `cycle_boundaries` mechanism); the audible end of
   the run is the same place, the decoder simply runs a little further ahead.
5. **S15 §7's open cases stay open.** Manual `Next` on the last track without
   repeat ends the run and publishes `PlaylistEnded`; the elapsed-time
   threshold for `Prev` (restart the current track after N seconds) is not
   implemented — `Prev` always goes to the previous track; the reference
   player's seed source is unmeasured.
6. **The shuffle re-anchor uses the audible track, the worker re-seats itself
   at the track it is decoding.** If the decoder is ahead of the device (it
   prefetches into the ring), the tracks between the two positions are not
   visited in that cycle. This is the behaviour of the old engine as well.
7. **One additive change in the playlist crate:**
   `PlaybackOrder::set_seed(seed)`. The engine must be able to re-seed the
   model's generator (the deterministic tests need it), and the model had no
   way to do that. The active plan is deliberately left alone: a re-seed must
   not move the play cursor.
8. **`Engine::shuffle_plan()` returns `Vec<usize>`** instead of `&[usize]`,
   because the model owns the plan and the engine hands out a copy. The
   application does not use it.

## 8. Not in this lane

* `crates/rustamp-app/src/config.rs` still carries the warning that "playback
  shuffle/repeat is not supported yet (engine does not implement
  shuffle/repeat)". That text is now factually stale, but the app layer is
  owned by another lane and was not touched here.
* The app/UI ownership of the mode flags (the application keeps its own
  snapshot flags, see `docs/tasks/W23-beta08-shuffle-repeat.md` §3) is an
  application-lane question; the engine now publishes a single truth
  (`state().shuffle` / `state().repeat` read from the model) for it to use.
