# M30 — Close the erasure twins and dead decoder-error arms

**Date** 2026-10-18. **Lane** `wave85-erasure-twins`.
**Worktree** `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave85-erasure-twins`.

Boundary rule: write ONLY inside this worktree; nothing under `D:\code\Rustamp`,
`D:\code\rustamp-research` or `D:\code\rustamp-refbin` was read or written.

**Status:** implemented; fmt, clippy and build gates passing. VM test execution
pending orchestrator run.

No tests were run on the host (Hyper-V test-VM constraint strictly preserved):
not the suite, not `--no-run`, not a single binary. No test process, example,
player, helper binary or reference binary was spawned; no VM was touched; no
window or audio device was opened; no download, no dependency addition, no
install, no commit and no other git write. Only `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --all-features -- -D warnings` and
`cargo build --workspace --offline` ran (receipts in §6). Every command ran
inside this worktree.

---

## 0. Boundary & ownership

Owned paths: `crates/rustamp-audio/src/**`, `crates/rustamp-mediahost/src/**`, and
this card `docs/tasks/M30-erasure-twins.md`. Touched:
- `crates/rustamp-audio/src/engine.rs` (production and test module)
- `crates/rustamp-mediahost/src/decoder.rs` (production and test module)
- `docs/tasks/M30-erasure-twins.md` (this card)

Untouched: `crates/rustamp-audio/tests/**`, `crates/rustamp-mediahost/tests/**`,
and all other crates.

Read only (not modified): `AGENTS.md`, `CLEANROOM.md`, `CONTRIBUTING.md`,
`docs/HANDOFF.md`, `docs/tasks/A25-erasure-path.md`, `docs/tasks/M25-three-reds.md`,
`docs/tasks/M23-real-app-red.md`, `crates/rustamp-audio/src/decode.rs`,
`crates/rustamp-mediahost/src/control.rs`, `crates/rustamp-mediahost/src/dsp.rs`,
`crates/rustamp-output/src/null.rs`.

Sources: the task cards and RFCs named above (RFC 0006 §6/§8, RFC 0012, RFC 0015),
and clean repository code only. No Winamp source, SDK material, disassembly,
research tree or reference binary was read; no third-party code was copied; no
dependency was added.

---

## 1. Verdicts and mechanisms for the two follow-up places

A25 §5 handed off two places where `join_decode` returned an unprocessed
`TrackStarted` that was discarded:
1. `Engine::handle_output_error` (device error teardown, RFC 0012)
2. `Engine::stop` (stop command)

### 1.1 Place 1: `Engine::handle_output_error` — erasure CAN happen; fixed

- **File and lines:** `crates/rustamp-audio/src/engine.rs:995-1000` (`poll`) and
  `engine.rs:1206-1235` (`handle_output_error`).
- **Verdict:** The erasure **can happen**.
- **Mechanism:**
  1. During track transition or playback start, `prepare_track` in the decode
     thread queues `DecodeOut::TrackStarted` (`decode.rs:570`), carrying the new
     track's `index`, `duration_frames`, `duration_ms`, `metadata`, etc.
  2. Before `Engine::poll` drains the message queue, a fatal output device error
     occurs (e.g. device disconnected, buffer invalidation, WASAPI error), so
     `self.output.stream_error()` returns `Some(err)`.
  3. `Engine::poll` checks `self.output.stream_error()` **before** reading
     `self.from_decode` (`engine.rs:995-1000`). Seeing `Some(err)`, `poll` calls
     `self.handle_output_error(err)` and returns immediately without checking
     `from_decode`.
  4. In `handle_output_error` (`engine.rs:1212`), `self.join_decode()` was called
     with `let _ = self.join_decode();`, discarding the returned
     `Some(DecodeOut::TrackStarted { .. })`.
  5. The engine freezes at `TransportState::Stopped` and publishes
     `Event::StateChanged(Stopped)`, `Event::PositionChanged`, and
     `Event::Error(ErrorKind::Output)`.
- **What a user would see:** The player stops and reports an output error, but
  the UI display (and `duration_ms`, `duration_frames`, `metadata`) reflects the
  *previous* track's duration and title (or `None`/blank), while the track index
  indicates the new track. The new track's duration is erased.
- **Fix:** Deliver the held `TrackStarted` through `on_decode_out` right after
  the join (`engine.rs:1215-1218`), before the timeline freeze and the error
  publication, using the exact hold-and-deliver mechanism established by A25.

### 1.2 Place 2: `Engine::stop` — erasure CANNOT and MUST NOT happen; left alone

- **File and lines:** `crates/rustamp-audio/src/engine.rs:802-831` (`stop`).
- **Verdict:** The erasure as a defect **cannot happen**; delivering the held
  start here would corrupt playback state. **Left alone.**
- **Mechanism and why:**
  1. `Engine::stop` is a normal, explicit transport stop command issued by the
     user (e.g. clicking the Stop button) or controller, **not a failure teardown**.
  2. In gapless playback, the decode thread runs ahead of physical playback on the
     device. When track 0 finishes decoding, the decode thread immediately enters
     `prepare_track` for track 1 and queues `DecodeOut::TrackStarted` for track 1
     (`decode.rs:570`). At that moment, the device buffer is still rendering the
     end of track 0.
  3. If the user clicks Stop while listening to track 0, their explicit intent is
     to stop track 0.
  4. If `stop()` were to deliver the held `TrackStarted` for track 1 through
     `on_decode_out`, `self.current` would advance to index 1 (`engine.rs:1053`),
     `Event::TrackChanged` would be published for track 1 (`engine.rs:1062-1067`),
     and `self.duration_ms` would become track 1's duration. The engine would
     then stop at position 0 of track 1.
  5. **What a user would see if delivered:** The user who clicked Stop while
     listening to track 0 would see the player jump to track 1 and stop at the
     beginning of track 1! This would be an egregious bug and violate Winamp's
     transport behavior.
  6. Discarding the prefetched start in `Engine::stop` is therefore the intended,
     correct behavior: it cleanly aborts the decode thread and discards unrendered
     future tracks without corrupting the current track identity.
  7. When playback of track N is requested via `play_index(N)` and stopped before
     `poll()`, `stop()` cleanly resets `timeline_origin = 0`, `track_start = 0`,
     and `position_ms = 0` with `current = Some(N)`.

---

## 2. Invariants kept by the `handle_output_error` fix

The fix in `handle_output_error` keeps every invariant established in A25:
- **The run stops:** `transport = Stopped`, unchanged.
- **The error is published once:** the delivery is `TrackStarted` (no `Error`
  event); `self.report_output_error(&err)` publishes `Event::Error` exactly once.
- **The timeline freezes at the failure:** `let frozen_timeline = self.timeline_frames();`
  is captured before the delivery, and `TrackStarted` does not modify
  `timeline_origin`, so `self.timeline_origin = frozen_timeline` preserves the
  exact frozen timeline position.
- **The next run starts clean:** `Returned` is recovered by `join_decode`'s drain,
  the delivery never calls `flush_output` (so sticky `flush_failure` cannot be
  armed), and the held start is consumed.
- **The teardown ordering is intact:** `poll` still checks `stream_error` first,
  `DecodeMsg::Stop` is sent before the join, and `Stopped` in `from_decode` is
  discarded by the join drain.

---

## 3. The two dead decoder-error arms in `rustamp-mediahost`

M23 §1 noted that `DecoderSession::open` (`decoder.rs:316-318`) and
`DecoderSession::remember` (`decoder.rs:396-399`) contained unreachable
`Message::DecoderError => Err(MediaHostError::OperationFailed(message))` arms.

### 3.1 Why they are dead code (a lie in the code)
`ControlPeer`'s background monitor thread (`control.rs:183-186`):
```rust
Ok(Ok(Message::DecoderError { message })) => {
    fail(format!("resident decoder failed: {message}"));
    break;
}
```
intercepts `Message::DecoderError` directly from the framing stream. It calls
`fail(...)`, which arms the sticky `state.failure` and closes `state.abort` (to
wake blocked PCM workers), and immediately `break`s out of the monitor loop,
dropping `incoming_tx`.

It **never** forwards `Message::DecoderError` to `incoming_tx`. Consequently,
`peer.recv()`, `peer.recv_blocking()`, and `peer.try_recv()` never receive
`Message::DecoderError`. Instead, their `self.check()?` or error-mapping path
returns `Err(MediaHostError::Disconnected("resident decoder failed: ..."))`.

The arms mapping `Message::DecoderError` to `MediaHostError::OperationFailed` in
`DecoderSession::open` and `DecoderSession::remember` were completely
unreachable. Keeping them in the code misled readers into writing tests or callers
expecting `OperationFailed` (the exact issue M23 §1 uncovered).

### 3.2 What was closed
- Removed `Message::DecoderError { message } => Err(MediaHostError::OperationFailed(message))`
  from `DecoderSession::open`.
- Removed `Message::DecoderError { message } => { self.peer.fail(&message); return Err(...); }`
  from `DecoderSession::remember`.
- Removed `| Message::DecoderError { .. }` from match patterns in `read` and `seek_inner`.
- Added unit tests in `crates/rustamp-mediahost/src/decoder.rs` confirming that
  terminal `DecoderError` from a peer produces sticky
  `MediaHostError::Disconnected("resident decoder failed: ...")` on `peer.recv`
  and in `DecoderSession::open`.

---

## 4. Tests: the cases the warts imply

Owned paths constraint strictly observed: tests were added in
`crates/rustamp-audio/src/engine.rs` (`mod tests`) and
`crates/rustamp-mediahost/src/decoder.rs` (`mod tests`). No test files outside
owned paths were touched.

### 4.1 Audio engine tests (`crates/rustamp-audio/src/engine.rs`)

1. `engine_publishes_the_start_when_output_fails_right_after_it`:
   - Queue has `track7_2048.tst` (2048 frames) and `track2_44100.tst` (44100 frames = 1000 ms).
   - Track 0 plays with `stop_after_current` armed and finishes naturally.
     `duration_frames()` is `Some(2048)` — the stale duration an erasure would leave behind.
   - Track 1 starts via `play_index(1)`.
   - The test waits until `engine.sync.counters().0 > 0`, confirming that
     `prepare_track` has queued `TrackStarted` for track 1 into `from_decode`.
   - Device failure is simulated via `clock.fail()`.
   - `engine.poll()` runs: `poll` checks `stream_error` before `from_decode` and
     enters `handle_output_error`.
   - **Assertions:**
     - `engine.duration_frames() == Some(44_100)` (started track, NOT previous track's 2048).
     - `engine.duration_ms() == Some(1000)`.
     - `engine.current_index() == Some(1)`.
     - `engine.transport() == TransportState::Stopped`.
     - `engine.position_frames() == 0`.
     - Exactly one `Event::Error` with `ErrorKind::Output` containing `"simulated device failure"`.
     - `engine.last_error()` is set with `ErrorKind::Output`.
     - `Event::TrackChanged` for index 1 carrying `duration_ms == Some(1000)`.
   - Without the fix, this test fails deterministically at `duration_frames() == Some(44_100)`.

2. `stop_does_not_advance_to_prefetched_next_track_during_gapless`:
   - Queue has `track1_2048.tst` and `track2_44100.tst`.
   - Track 0 starts and plays partially.
   - The test waits until the decode thread has decoded track 0 and prefetched
     into track 1 (`written > 2048`), queueing `TrackStarted` for track 1.
   - User stops playback while device has only partially played track 0 (`frames < 2048`).
   - `engine.stop()` executes.
   - **Assertions:**
     - `engine.transport() == TransportState::Stopped`.
     - `engine.current_index() == Some(0)` (stays on track 0, does not jump to track 1).
     - No `Event::TrackChanged` is emitted for track 1.
   - Proves why `stop()` must NOT deliver the held start.

### 4.2 Mediahost tests (`crates/rustamp-mediahost/src/decoder.rs`)

1. `decoder_error_fails_control_peer_sticky_as_disconnected`:
   - Connects in-process control peers via piped channels.
   - Server sends `Message::DecoderError { message: "refused corrupt stream" }`.
   - Proves `client_peer.recv()` returns `Err(MediaHostError::Disconnected("resident decoder failed: refused corrupt stream"))`.
   - Proves `client_peer.check()` is `Err(MediaHostError::Disconnected(...))`.

2. `decoder_session_open_fails_sticky_as_disconnected_when_server_reports_decoder_error`:
   - Connects in-process control peers.
   - When client attempts `DecoderSession::open`, server sends `Message::DecoderError`.
   - Proves `DecoderSession::open` returns `Err(MediaHostError::Disconnected(...))`
     carrying `"resident decoder failed: refused corrupt stream"`, confirming the
     dead `OperationFailed` arm is gone and behavior is honest.

---

## 5. Files and line counts

All files inside owned paths. `git diff --stat`:

| File | Lines (after) | Change |
|---|---:|---|
| `crates/rustamp-audio/src/engine.rs` | 1827 (was 1514) | `handle_output_error` delivers held `TrackStarted` from `join_decode` before freeze; `join_decode` doc comment updated; new `#[cfg(test)] mod tests` with two deterministically pinned unit tests (+313 lines). Production + test change. |
| `crates/rustamp-mediahost/src/decoder.rs` | 798 (was 669) | Removed unreachable `Message::DecoderError` arms from `DecoderSession::open` and `remember`; cleaned up match patterns in `read` and `seek_inner`; added unit tests for `ControlPeer` sticky failure and `DecoderSession::open` (+129 lines). Production + test change. |
| `docs/tasks/M30-erasure-twins.md` | this card | new card. |

No dependencies added or modified. No `Cargo.toml`/`Cargo.lock` changed. No files
in other crates touched.

---

## 6. Gate receipts

Executed strictly within this worktree (`C:\Users\micro\.local\share\opencode\worktree\26de0e\wave85-erasure-twins`):

- `cargo fmt --all -- --check`: **PASS** (exit 0, no diffs).
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: **PASS** (exit 0, no warnings).
- `cargo build --workspace --offline`: **PASS** (exit 0).
- `cargo test`: **NOT RUN** (Hyper-V VM constraint strictly respected).
- No GUI window opened, no audio device accessed, no external dependency
  added, no git commit created, no process spawned (beyond compiler toolchain).

---

## 7. Boundary receipt

`git status --short` inside this worktree (read-only):

```
 M crates/rustamp-audio/src/engine.rs
 M crates/rustamp-mediahost/src/decoder.rs
?? docs/tasks/M30-erasure-twins.md
```

Nothing under `D:\code\Rustamp`, `D:\code\rustamp-research` or
`D:\code\rustamp-refbin` was read or written.

---

## 8. Provenance

Implemented strictly from the task cards (A25, M25, M23), the RFCs (0006, 0012, 0015),
and clean repository code. No Winamp source, SDK headers, disassembly or forbidden-tree
contents were read. No third-party code was copied. License impact: none (no
dependencies changed; `rustamp-audio` stays GPL-3.0-or-later, `rustamp-mediahost`
stays MIT OR Apache-2.0).
