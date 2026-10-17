# A4: Ring Drain & Honest Backlog (Finding F2 Fix)

## 1. The Hole (Finding F2)

In the audio engine lifecycle, the output backend's render callback advances the device clock (`played_frames`) by the total requested callback buffer length, even for frames filled with silence during an underrun (e.g., when the decode thread is stalled, pre-empted or slow).

Previously, the playback completion check in `Engine::poll()` relied on comparing the device clock's timeline projection against the stream end:
```rust
// OLD (WRONG)
if let Some(end) = self.end_of_stream
    && self.timeline_frames() >= end
    && self.transport == TransportState::Playing
{
    self.finish_run();
}
```
When an underrun occurred, `played_frames` outran `RingSync::written`, causing `timeline_frames()` to pass `end_of_stream` while real, unrendered audio frames were still sitting in the ring buffer. Consequently, the playback run terminated prematurely, cutting off the audio.

Furthermore, `buffered_frames()` was previously defined as:
```rust
// OLD (DISHONEST)
self.sync.written().saturating_sub(self.timeline_frames())
```
When `timeline_frames()` outran `written`, this calculation saturated to `0`, disguising the fact that a real backlog existed in the ring buffer.

An earlier attempted fix—requiring `written >= timeline_frames()` to finish—was fundamentally flawed: at the end of the stream, `written == end_of_stream`, so it demanded an exact clock hit on the device clock, causing runs to stall indefinitely.

---

## 2. The Design

To decouple playback completion from the device clock and obtain an honest backlog metric:

1. **Consumer-side audio frame counter (`RingSync::consumed`):**
   An atomic counter (`AtomicU64`) updated with `Ordering::Relaxed` inside the render callback ([`crates/rustamp-audio/src/ring.rs`]). It counts **only device frames actually read out of the ring**, completely ignoring silence frames written during underruns.

2. **Honest backlog reporting:**
   `Engine::buffered_frames()` reports:
   ```rust
   self.sync.written().saturating_sub(self.sync.consumed())
   ```
   This reflects the exact number of frames buffered in the ring, never saturating a real backlog to zero.

3. **Robust finish condition:**
   The run terminates if and only if all frames produced by the decode thread have left the ring:
   ```rust
   if let Some(end) = self.end_of_stream {
       let written = self.sync.written();
       let consumed = self.sync.consumed();
       if written >= end && consumed >= written && self.transport == TransportState::Playing {
           self.finish_run();
       }
   }
   ```
   Because `consumed` counts only real frames read from the ring, `consumed < written` holds until the very last audio frame has been read into the output buffer. Underrun silence does not advance `consumed`, preventing premature stops. Once the ring is empty, `consumed == written == end`, satisfying the condition without demanding an exact device clock hit.

4. **Seek-drain alignment:**
   On a seek-drain handshake (`request_epoch` / `drain_ack`), the render callback discards leftover pre-seek frames and sets `consumed` to `sync.written()`. Dropped frames count as consumed, preserving the invariant `consumed <= written` across seek epochs.

5. **Clean run re-basing:**
   On `start_run()` (including subsequent runs of the same or another track) and `stop()`, `consumed` is reset to 0 alongside `written`. The render callback updates `sync.consumed()` directly without stale closure-local state.

---

## 3. Exact Invariants

Let:
- $W \equiv \text{written}$: absolute timeline position (device frames) written by the decode thread up to the current point.
- $C \equiv \text{consumed}$: device frames actually read out of the ring by the render callback (excluding silence).
- $P \equiv \text{played\_frames}$: raw device clock frames reported by the output backend.
- $P_0 \equiv \text{clock\_origin}$: value of $P$ captured at the start of the current run or seek.
- $T_0 \equiv \text{timeline\_origin}$: timeline base position at the start of the current run or seek.
- $T \equiv \text{timeline\_frames}$: projected timeline position according to device clock:
  $$T = T_0 + (P - P_0)$$
- $S \equiv \text{track\_start}$: timeline position where the current track started.
- $D \equiv \text{duration\_frames}$: duration of the track, if known.
- $E \equiv \text{end\_of\_stream}$: final timeline position published by decode thread upon queue completion.

### Fundamental Invariants:

1. **Backlog (Ring buffered frames):**
   $$B = W - C \ge 0 \quad (\text{i.e. } C \le W)$$
   `buffered_frames()` $= W - C$. It never underflows and accurately measures unrendered frames.

2. **Render callback advance:**
   For a callback of $N$ output frames where $K \le N$ frames are read from the ring and $N - K$ frames are silence:
   $$\Delta P = N$$
   $$\Delta C = K$$
   Silence never increments $C$.

3. **Track Position:**
   $$\text{position\_frames}() = \min(T - S, D)$$

4. **Seek-Drain Invariant:**
   Upon an epoch bump to $e$, pre-seek frames in the ring are discarded, and the callback aligns:
   $$C \leftarrow W$$
   So immediately after drain:
   $$B = W - C = 0$$

5. **Playback Completion Invariant:**
   The run terminates when:
   $$\text{end\_of\_stream} = \text{Some}(E) \land W \ge E \land C \ge W \land \text{transport} = \text{Playing}$$
   Since $C \le W = E$, this fires precisely when $C = W = E$ (all decoded frames rendered).

6. **Post-Run Freeze Invariant:**
   Upon `finish_run()`:
   $$T_0 \leftarrow E$$
   $$P_0 \leftarrow P$$
   $$T = T_0 + (P - P_0) = E$$
   $$\text{position\_frames}() = E - S = D$$
   $$B = W - C = 0$$
   The timeline is frozen exactly at the track boundary $E$, never overrunning into silence device frames.

---

## 4. Lifecycle Transitions & Proofs

| Transition | Action on $W$ and $C$ | Resulting State |
|---|---|---|
| Fresh `start_run()` | $W \leftarrow 0$, $C \leftarrow 0$ | $C = W = 0$, $B = 0$ |
| Normal decoding | $W \leftarrow W + \Delta W$ | $W \ge C$, $B = W - C > 0$ |
| Normal rendering | $C \leftarrow C + \Delta K$ ($\Delta K \le \text{buf}$) | $C$ approaches $W$, $B$ decreases |
| Underrun rendering | $\Delta P = N$, $\Delta C = K < N$ | $C$ advances only by $K$; silence ignored; $W \ge C$ holds |
| Seek (`apply_seek`) | $W \leftarrow T_{\text{seek}}$, epoch bumped | $W$ set to seek target; consumer drains |
| Drain ack | $C \leftarrow W$ | $C = W = T_{\text{seek}}$, $B = 0$; pre-seek backlog cleared |
| Gapless track seam | $W$ and $C$ monotonic; no reset | Invariant $C \le W$ holds smoothly across tracks |
| End of stream | $W = E$, wait for $C \ge W$ | Fires when $C = W = E$; ring is empty |
| `finish_run()` | $T_0 \leftarrow E$, $P_0 \leftarrow P$ | $T = E$, $B = 0$, position frozen at content end |
| `stop()` | $W \leftarrow 0$, $C \leftarrow 0$, $T_0 \leftarrow 0$ | All reset to 0 |
| Second `start_run()` | $W \leftarrow 0$, $C \leftarrow 0$ | Clean slate; no stale counter leaks from prior run |

---

## 5. Audio Thread Rule Compliance

Per `CONTRIBUTING.md`:
- No heap allocations.
- No locks (`Mutex`, `RwLock`).
- No blocking I/O or system calls.
- No logging.

Implementation in `make_render()`:
- `sync.consumed().saturating_add(frames_read)`: atomic load with `Ordering::Relaxed`.
- `sync.set_consumed(consumed)`: atomic store with `Ordering::Relaxed`.
- No locks or allocations are used in the audio callback. On x86_64, relaxed loads and stores compile to single machine `mov` instructions.

---

## 6. Null Device Parity

The headless integration test harness uses `NullOutput` / `NullClock`:
- `NullClock::pump()` and `NullClock::pump_record()` invoke the exact same `RenderFn` closure produced by `make_render()`.
- The device clock `played` advances by $N$ frames, exactly simulating backend hardware.
- Headless tests exercise the identical atomic counter and drain logic used in production.

---

## 7. What Could Not Be Verified Headless

1. **Host-specific audio driver thread scheduling:** Real WASAPI shared-mode or CPAL buffer periodicity jitter under high CPU load.
2. **Audio hardware sample-rate conversion:** Hardware DAC clock drift relative to CPU system time.
(Both are covered by the VM-only integration tests: `loopback.rs` and `real_playback.rs` running in the Hyper-V test VM).
