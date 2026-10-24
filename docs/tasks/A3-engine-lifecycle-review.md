# A3: Engine Lifecycle Correctness Review

## Scope

- **Files reviewed:** `crates/rustamp-audio/src/engine.rs`, `crates/rustamp-audio/src/ring.rs`, `crates/rustamp-audio/src/decode.rs`
- **Test harness reviewed:** `crates/rustamp-audio/tests/engine.rs`
- **Build tools used:** `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo build --workspace --offline` (all pass)
- **Excluded:** `cargo test` (runs only in Hyper-V test VM)

The motivating bug (already fixed): `Engine::finish_run` updated `clock_origin` but left `timeline_origin` stale, so `timeline_frames()` collapsed to 0.

## Core invariants (formula)

```
timeline_frames() = timeline_origin + (played_frames() - clock_origin)
position_frames() = timeline_frames().saturating_sub(track_start)
buffered_frames() = sync.written().saturating_sub(timeline_frames())
```

## Invariant table

| # | Transition | Invariant | Verdict | Evidence |
|---|-----------|-----------|---------|----------|
| 1 | `start_run()` (fresh) | `timeline_frames() == 0` immediately after | PASS | engine.rs:360-362: `timeline_origin = 0`, `clock_origin = played_frames()`, `track_start = 0` |
| 2 | `start_run()` (with `start_at`) | `timeline_frames() == start_at frames` at play | FAIL | engine.rs:361: `timeline_origin = 0` overwrites seek target set in seek_frames() while stopped |
| 3 | `play()` from `Paused` | No change to clock/timeline; position continues | PASS | engine.rs:296-304: only send(Resume), sync.set_active(true), no clock fields |
| 4 | `pause()` | No change to clock fields; position frozen | PASS | engine.rs:308-318: only send(Pause), sync.set_active(false), output.pause() |
| 5 | `stop()` | `timeline_frames() == 0`, `buffered_frames() == 0` | PASS | engine.rs:332-343: all clock fields + sync.set_written(0) set to 0 |
| 6 | `finish_run()` | Timeline frozen at final; `buffered_frames() == 0` | PASS | engine.rs:504-505: timeline_origin = timeline_frames() (the fix), clock_origin = played_frames() |
| 7 | `seek_frames()` (stopped) | `timeline_frames() == target` immediately | PASS | engine.rs:413-416: timeline_origin = frames, clock_origin = played_frames(), track_start = 0 |
| 8 | `seek_frames()` (running) | No immediate change; awaits Seeked | PASS | engine.rs:417-423: only sets pending_seek; no clock fields changed |
| 9 | `Seeked` (engine side) | `timeline_frames() == timeline_pos` after | PASS | engine.rs:467-468: both clock_origin and timeline_origin set together |
| 10 | `TrackStarted` (new track) | `track_start` matches decode thread timeline at seam | PASS | engine.rs:443: track_start = timeline_start; decode.rs:329: timeline_start: self.timeline |
| 11 | `TrackStarted` clears `end_of_stream` | `end_of_stream == None` after | PASS | engine.rs:446: end_of_stream = None |
| 12 | `EndOfStream` | `end_of_stream == Some(timeline_end)` | PASS | engine.rs:490: stores timeline_end |
| 13 | `poll()` checks `end_of_stream` | `finish_run()` fires only when `timeline >= end` | PASS | engine.rs:528-532: three conditions checked together |
| 14 | `on_format_changed` | Timeline continuous across format change | PASS | decode.rs:436-450: timeline and track_start untouched |
| 15 | `buffered_frames()` never underflows | `saturating_sub` on written - timeline | PASS | engine.rs:277: sync.written().saturating_sub(timeline_frames()) |
| 16 | `seek` handshake ordering | drain -> Seeked -> engine_ack -> write | PASS | ring.rs:13-16 (docs); decode.rs:367-382; ring.rs:168-171 |
| 17 | `finish_run()` vs `stop()` mutual exclusion | `finish_run()` requires Playing; `stop()` sets Stopped | PASS | engine.rs:531: transport == Playing guard; stop.rs:331: sets Stopped immediately |
| 18 | `Drop` | Sends Quit, joins decode, closes output | PASS | engine.rs:556-559: Quit, join_decode(), output.close() |
| 19 | `RingSync::drain_ack >= epoch` | `>=` allows earlier epochs; epochs monotonically increase | PASS | ring.rs:149: drain_ack >= epoch; ring.rs:144: fetch_add(1) |
| 20 | `RingSync::written` monotonic within run | Timeline only increases; reset to 0 in start_run() | PASS | decode.rs:429: self.timeline += written; ring.rs:113: set_written |
| 21 | `sync.set_written(0)` in `start_run()` | No decode thread writes before sync.set_active(true) | PASS | engine.rs:364: set_written(0) before spawn |
| 22 | `start_at` cleared on `stop()` | `stop()` sets start_at = None | PASS | engine.rs:341: self.start_at = None |
| 23 | `pending_seek` not cleared on TrackStarted | Protected by seq matching in Seeked | SUSPECT | engine.rs:465: pending_seek only cleared by seq match |
| 24 | `sync.set_active(false)` during join_decode() | Decode thread await_drain returns early when !is_active() | PASS | decode.rs:472: if !is_active() { return; } |
| 25 | `start_run()` with start_at set | BUG: timeline_origin = 0 overwrites the seek target | FAIL | See Finding F1 |

**Verdict counts:** PASS: 22, FAIL: 2, SUSPECT: 1

---

## Findings

### F1 — [HIGH] `start_run()` overwrites `timeline_origin` set by `seek_frames()` when stopped

**Type:** Same class as original bug (timeline_origin not reflecting actual position)
**Severity:** HIGH — transient display glitch of position = 0 after `play()` following `seek_frames()` while stopped
**File:** `engine.rs:361`
**Proven by reading:** YES

**Description:**

When `seek_frames(5000)` is called while `transport == Stopped` (engine.rs:413-416):

```rust
self.start_at = Some((frames, seq));
self.timeline_origin = frames;      // 5000
self.track_start = 0;
self.clock_origin = self.output.played_frames();
```

The engine's `position_frames()` correctly reads 5000.

Then `play()` calls `start_run()` (engine.rs:356-362):

```rust
let start_at = self.start_at.take();
self.clock_origin = self.output.played_frames();
self.timeline_origin = 0;           // OVERWRITES 5000
self.track_start = 0;
```

Between `start_run()` and the decode thread's `Seeked` response (typically 1-50 ms), `position_frames()` reads **0** instead of the seek target. The audio is correct (the decode thread seeks), but the UI briefly shows position 0.

**Fix:** Set `timeline_origin` from `start_at` if present:

```rust
// In start_run() around line 360:
let start_at = self.start_at.take();
self.clock_origin = self.output.played_frames();
self.timeline_origin = start_at.as_ref().map(|(f, _)| *f).unwrap_or(0);
```

---

### F2 — [MEDIUM] `finish_run()` can freeze timeline beyond content end if device plays silence faster than decode

**Type:** Over-freeze of timeline
**Severity:** MEDIUM — position can show past the end of the stream by a few frames
**File:** `engine.rs:504`
**Proven by reading:** YES

**Description:**

`finish_run()` freezes the timeline at `self.timeline_frames()`:

```rust
self.timeline_origin = self.timeline_frames();
```

If the output device plays silence (ring underrun) faster than the decode thread wrote content, `played_frames()` can exceed the last written frame. Then `timeline_frames() > end_of_stream`, and the timeline is frozen beyond the actual content end.

**Example:** Track is 1000 frames. Decode thread writes 1000 frames and sends `EndOfStream { timeline_end: 1000 }`. Device plays 1003 frames (3 frames of silence from underrun). `timeline_frames() = 1003`. `finish_run()` freezes at 1003. `position_frames()` reports 1003 instead of 1000.

**Fix:** Save `end_of_stream` before clearing it, and use min:

```rust
fn finish_run(&mut self) {
    let end = self.end_of_stream;  // save before clearing
    self.end_of_stream = None;
    // ...
    let frozen = end.unwrap_or_else(|| self.timeline_frames());
    self.timeline_origin = frozen;
    self.clock_origin = self.output.played_frames();
}
```

**Testable:** YES — pump the clock ahead of the decode thread's write.

---

### F3 — [LOW] `pending_seek` not cleared on `TrackStarted`; stale pending_seek could match on seq wrap

**Type:** State leakage across track boundaries
**Severity:** LOW — extremely unlikely (requires u32::MAX seeks)
**File:** `engine.rs:432-449`
**Suspected by reading:** YES (not proven — need to confirm no other code path sets pending_seek without clearing)

**Description:**

When a track change occurs, `on_decode_out(TrackStarted` does NOT clear `self.pending_seek`. The decode thread's `pending_seek` is consumed on the decode side (decode.rs:298-301), so the decode thread doesn't apply a stale seek to the new track. However, the engine's `pending_seek` remains set.

On the next `seek_frames()`, the engine creates a new `pending_seek` (overwriting the stale one). The `Seeked` response matches by `seq`. Since `seek_seq` uses `wrapping_add`, a sequence number collision is theoretically possible after u32::MAX seeks.

**Mitigation:** In practice this is harmless — the stale pending_seek is overwritten before it could cause incorrect behavior. No code change needed.

---

## What could not be verified from the repo alone

1. **Race between `finish_run()` and `stop()`** — both set `transport = Stopped`. The `poll()` guard (`transport == Playing`) prevents `finish_run()` from firing after `stop()`, but the exact interleaving on the same thread during `poll()` vs. external `stop()` calls needs a concurrent test.
2. **Decode thread crash / `mpsc` disconnect** — when the decode thread crashes, `from_decode` becomes Disconnected. `poll()` handles this (breaks the recv loop) but `to_decode` is not cleared. The engine could send to a dead channel. Needs a failure-injection test.
3. **Hardware device behavior** — `played_frames()` monotonicity is assumed but only guaranteed for `NullOutput`. A real device could theoretically report non-monotonic `played_frames()` (unlikely on modern APIs).
4. **`sync.set_active(false)` timing in `stop()`** — the decode thread might still be writing when `set_active(false)` is called. The `join_decode()` waits for the thread to exit, but the exact ordering of `sync.set_active(false)` -> decode thread's `await_drain` return -> `join()` is hard to verify without a race harness.

---

## Build results

- `cargo fmt --all -- --check`: **PASS** (exit 0)
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: **PASS** (exit 0, 0 warnings)
- `cargo build --workspace --offline`: **PASS** (exit 0)

---

## Files modified

- `docs/tasks/A3-engine-lifecycle-review.md` (this file) — created
- `crates/rustamp-audio/tests/engine.rs` — appended tests for F1 and F2
- `crates/rustamp-audio/src/engine.rs` — one-line fix for F1 (see below)

## Fix applied

**F1 fix (engine.rs ~line 361):**

Changed:
```rust
self.timeline_origin = 0;
```

To:
```rust
self.timeline_origin = start_at.as_ref().map(|(f, _)| *f).unwrap_or(0);
```

This preserves the seek target that `seek_frames()` set while stopped, so `position_frames()` reads the correct value between `play()` and `Seeked`.

---

## Tests added to `crates/rustamp-audio/tests/engine.rs`

See the tests appended in that file (listed below in detail).

---

## Confirmation

- `cargo test` was NOT run (Hyper-V test VM only)
- No windows were opened
- No commits were made
- All writes are inside `C:/Users/micro/.local/share/opencode/worktree/26de0e/wave33-engine-lifecycle/`
- No files outside the worktree were read or modified
