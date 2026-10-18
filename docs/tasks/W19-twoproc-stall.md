# W19: the intermittent 30 s stall in `dsp_roundtrip` and `real_decode`

**Symptom (docs/notes/lane-jobs.md, r49–r53).**
`dsp_roundtrip::reset_keeps_rings_and_continues` (`dsp_roundtrip.rs:180`) and
`real_decode::end_of_stream_comes_only_after_the_ring_is_drained`
(`real_decode.rs:155`) intermittently reach their **30 s** stall escape with
"the resident stopped delivering without a close or an error", while the same
binaries finish in 0.26 s / 0.85 s when they run. Green in r49 and r51,
stalled in r50 and (dsp only) r53, stalled once and then green three times in
an isolated single-binary run.

**Verdict: a real defect, not a starved guest.** The delivery *can* lose the
PCM path. The wake signal that makes a blocked reader or writer look again at
the ring is **edge-triggered** in `rustamp-ipc`, and an ordinary thread
interleaving can skip it; the framework then spent its whole caller deadline
inside **one** wait on that signal, so a skipped signal cost the entire bound
instead of being noticed at the next look at the counters. Both halves end up
asleep and nothing wakes either: no close, no error, exactly the escape
length.

Two corrections to the premises in the lane notes, both load-bearing:

- **The two tests are not two-process.** Neither spawns a resident. Both run
  the host worker on a `thread::spawn` in the *same* process
  (`dsp_roundtrip.rs:153`, `real_decode.rs:131`) over real named sections and
  named events. There is no child, no pipe and no second scheduler involved,
  which makes "the guest was starved" a much stronger claim than it looks: it
  would have to starve a runnable thread *inside one process* for thirty
  seconds while its 30 s timer ran out on time.
- **Both panics are on the player side, and only the player side has an
  unsliced wait.** That is the cheap discriminator — see candidate 1.

---

## 1. The defect: an edge-triggered wake skipped by a stale counter read

### Where

`crates/rustamp-ipc/src/shm/ring.rs` (RFC 0007 §6.2's "safe and cheap
refinement"):

| site | rule |
| --- | --- |
| `write_from` 433–447 | `was_empty = (write_seq − read_seq) == 0` decided **before** the write; `Wake::Data` is signaled only if it holds |
| `write_from_blocking` 561–571 | same, inside the check-then-wait loop |
| `read_into` 812–825 | `was_full = (write_seq − read_seq) == capacity` decided before the read; `Wake::Space` only if it holds |
| `read_into_blocking` 933–941 | same |

Each side decides its edge from the counter the **peer** owns: the producer
reads `read_seq` (435, 562), the consumer reads `write_seq` (812, 933).

### The interleaving that skips the `Data` signal

Capacity 3, one frame in the ring (`write_seq = 10`, `read_seq = 9`). P is the
producer, C the consumer; every step is a single atomic operation, so this is
a plain thread interleaving and needs nothing exotic from the memory model.

1. **P** `write_from_blocking`: loads `write_seq = 10` (561), loads
   `read_seq = 9` (562) → occupancy 1 → `was_empty = false` (567). P is now
   descheduled anywhere before its `store_write_seq` (569).
2. **C** `read_into_blocking`: loads `write_seq = 10`, `read_seq = 9` →
   available 1, `was_full = (1 == 3) = false` → copies the frame, stores
   `read_seq = 10` (940), **no** `Space` signal (941).
3. **C** returns 1 frame; the caller wants more and calls again: loads
   `write_seq = 10` (P still has not stored), `read_seq = 10` → available 0,
   not closed → `wake.wait(Wake::Data, …)` (949).
4. **P** resumes: stores `write_seq = 12` (569) and **sends no signal**,
   because step 1 already decided `was_empty = false`.

C is now asleep on data the ring holds. P loops, finds the ring full, and
sleeps on `Wake::Space` — which C will never send, because C only signals
`Space` from a *full* ring and it is not looking at the ring at all. Both ends
asleep, ring neither closed nor failed.

A leftover latch does not rescue this: the auto-reset event gives C one extra
re-check at step 3, after which the latch is clear and the next wait is
permanent.

The `Space` direction is the mirror image (C loads a `write_seq` from before
P's filling store, so `was_full` is false and no `Space` signal is sent, while
P has meanwhile observed a pre-drain `read_seq`, found `free == 0` and gone to
sleep).

### Why RFC 0007 §6.2's proof does not hold

> "the consumer waits only after observing empty, so its observed `read_seq`
> does not change while it waits; the producer's next write reads its *own*
> `write_seq` and that same `read_seq` […] so the producer's check must observe
> empty too — so it signals."

"That same `read_seq`" is the step the proof skips. Nothing orders the
producer's load of `read_seq` **after** the consumer's last store to it. In
the interleaving above the producer's load (step 1) happens *before* the
consumer's store (step 2), so the producer evaluates its edge against a
pre-drain `read_seq` and concludes the consumer still has work to do. The
occupancy invariant the proof leans on is intact throughout — it just does not
say anything about *which* snapshot each side used. The RFC's own escape
clause is the fix: "An implementation that is unsure may signal
unconditionally; it costs one extra `SetEvent` per advance and is always
correct."

### Why it reaches the escape instead of self-healing

Only the player-side waits were unsliced. Before this card:

| waiter | bound per single wait | effect of a skipped signal |
| --- | --- | --- |
| `DecoderSession::read` (decoder.rs) | the caller's whole timeout (30 s in the suite, 5 s in the engine) | **stall for the full bound** |
| decoder worker `pump` write | `None` → forever | **stall forever** |
| `DspSession::read` / `write_timeout` (dsp.rs) | the caller's whole timeout | **stall for the full bound** |
| DSP worker `pump` read/write | `CONTROL_POLL` = 20 ms | self-heals in ≤ 20 ms |

The DSP worker already sliced both of its waits (for control servicing), and
no reported stall is on that side. Both reported panics are player-side waits,
and the player side is exactly the side that slept the whole bound. The code
property and the observed side agree — that is the strongest single piece of
evidence in this card.

### Why these two tests and not the others

The exposure is one window per handoff, so it scales with the number of
drain-to-empty / fill-to-full boundaries, which is `frames / capacity`:

| test | ring | reads | handoffs per run |
| --- | --- | --- | --- |
| `real_decode::end_of_stream_comes_only_after_the_ring_is_drained` | 3 | 32 frames | ≈ 22 000 |
| `real_decode::real_file_opens_…bit_exact` | 7 | 1024 frames | ≈ 9 500 |
| `dsp_roundtrip::reset_keeps_rings_and_continues` | in-ring 3, back-ring written 1–3 frames at a time | 257 frames | thousands, on the back-ring's `Data` direction |
| the `seek_…` tests | 7 | short runs | hundreds |

The two named tests are the most exposed of their binaries (the dechost one by
~2.3×), and the guest runs four of them at a time on 4 vCPU with roughly eight
threads per test (each `ControlPeer` starts three), so a sub-microsecond window
is hit by a preemption often enough. This explains a rate, not a verdict: the
mechanism does not single these two tests out, it only ranks them first, which
is what a probabilistic race looks like. Nothing about "two processes" or
"recent code" enters the explanation, which matches the dependency argument the
lane notes already made.

### Why the ring's own suite never caught it

`blocking_no_lost_wakeup_under_jitter` (ring.rs 2029) is RFC 0007 acceptance
criterion 3, and it **cannot fail** on a skipped wake: both sides use
`TEST_WAIT` (10 s) and loop on `Ok(0)`, so a skipped signal costs ten seconds
and then recovers. `blocking_two_thread_lossless_delivery` can fail, but only
in the `Space` direction (`assert!(written > 0, "producer stalled")`) and only
after a 10 s wait. A skipped `Data` signal is invisible to the whole suite.

---

## 2. The other candidate mechanisms, with verdicts

- **A missed handshake — refuted.** Both handshakes are strictly ordered and
  every step is pinned by a generation equality check: decoder
  `offer → RingReady(gen) → pump` (decoder.rs `serve_inner`, `DecoderSession::open`),
  DSP `in-ring offer → RingReady(in) → back offer → RingReady(back) → DspReady`
  (`serve_inner`, `handshake`). A mismatch or a wrong order fails loudly with
  the message named, and `unknown_stage_is_a_loud_error_before_any_ring` plus
  the `abrupt_*` tests pin the refusal paths. A handshake defect would also be
  deterministic and would not wait exactly 30 s.
- **A ring or channel left unwritten — refuted.** The PCM path writes only
  through `write_from_blocking`, which never drops and returns the accepted
  count; `frames_dropped() == 0` is asserted in both suites. On the control
  path every queue is a bounded `sync_channel` whose overflow is a *loud*
  failure (`control.rs:202` "control consumer stalled", `:256` "control writer
  unavailable"), never silence. `peer.set_abort` is always registered *before*
  the peer is told its ring is live (decoder.rs `serve_inner`, dsp.rs
  `serve_inner`/`handshake`), so no live ring is ever unsupervised.
- **A wait armed after the thing it waits for — refuted for the control
  plane, and this is the one real instance on the PCM plane.** The control
  waits are `mpsc` receives, which cannot miss an earlier send. The ring waits
  are correct check-then-wait loops (ring.rs 885–890, 933–949) and the latch
  survives a signal that lands between the check and the wait. The defect is
  not that the wait is armed late; it is that **no signal is ever sent** (§1).
- **A control message consumed twice or dropped — refuted.** `Hello`, `Ping`,
  `Pong`, `DecoderError` and `DspError` are consumed by the monitor and never
  forwarded (control.rs 158–190), so no caller can see them; everything else
  is forwarded exactly once into one queue. `recv`, `recv_blocking` and
  `try_recv` all drain that one queue, so a message can be taken at a
  different site than intended — but every such site turns it into a *loud*
  error ("unexpected DSP message while reading/writing", "expected DspResetAck
  …"), never a wait. `DspReset`/`DspResetAck` are one-for-one
  (`poll_commands`, dsp.rs 147–150, and the post-EOS idle loop), and `reset`
  is bounded by `CONTROL_WAIT` and fails the peer sticky on expiry.
- **The guest starved of CPU — insufficient on its own, and refuted as the
  whole story by the record.** It stalled in r53 with no lane building and
  again in an isolated single-binary run, and a starved *thread in the same
  process* would have to lose thirty seconds while that thread's own 30 s
  timer expired on schedule. Load is still a genuine *amplifier*: it is what
  makes the sub-microsecond window in §1 get hit, which is exactly why r50
  (three lanes building) stalled both and r51 (quiet) neither. The one
  starvation variant that would fit the symptom on its own is the whole VM
  being paused (host swap, snapshot) for ≥30 s; §4 says how to tell.

---

## 3. What changed

Owned paths only. The root cause sits in `rustamp-ipc`, which this card does
not own (§5); what is fixed here is the framework's dependence on a signal for
*liveness*, which RFC 0007 §6.1/§6.3 never granted: "a lost, duplicated or
spurious signal can never turn into a wrong frame", because the waiter
re-derives its condition from the counters. A waiter that spends its whole
deadline in one `WaitForSingleObject` turns that hint into a dependency. After
this change every PCM wait in the framework re-derives its condition from the
counters at least every 20 ms, so a skipped signal costs one slice instead of
the caller's bound — and the DSP worker, which already did this, keeps doing
it unchanged.

| file | change | +/− |
| --- | --- | --- |
| `crates/rustamp-mediahost/src/pcm.rs` | new `RING_POLL` (20 ms), `next_slice`, `read_frames`, `write_frames`, and the `RingCounters` diagnostic type | +108 / −1 |
| `crates/rustamp-mediahost/src/decoder.rs` | worker write (141) and `DecoderSession::read` (451) go through the sliced helpers; new `DecoderSession::ring_counters` | +22 / −5 |
| `crates/rustamp-mediahost/src/dsp.rs` | `DspSession::write_timeout` (530) and `read` (570) go through the sliced helpers; the now-unused `remaining` helper removed; new `back_ring_counters`/`in_ring_counters`; one comment on `CONTROL_POLL` | +35 / −14 |
| `crates/rustamp-mediahost/tests/deaf_wake.rs` | **new** regression test (388 lines) | +388 |
| `crates/rustamp-dechost/tests/real_decode.rs` | the stall escape prints the ring counters; module docs | +29 / −4 |
| `crates/rustamp-dsphost/tests/dsp_roundtrip.rs` | the three stall escapes print the ring counters; module docs | +51 / −12 |

**Contracts kept.** `None` still means "until data, a closed ring or a
failure"; `Some(d)` still returns `TimedOut` only after the whole `d`, because
the slice is `min(remaining, RING_POLL)` and an expired slice is not an
outcome — it only sends the loop back to the counters. A caller bound shorter
than one slice (the 20 ms in `bounded_read_timeout_is_not_eos_and_retry_delivers_pcm`
and in the engine's DSP block timeout) therefore behaves exactly as before.
The cost is one event wait per 20 ms per idle ring, the rate the DSP worker has
always run at.

**The new test.** `deaf_wake.rs` does not try to reproduce the race; it
removes the signal entirely. `DeafRings` gives each of a ring's four endpoints
its **own** `RingWake::create`, so no `SetEvent` from any party — publish, free
slot, or `close()` — can ever reach a waiter, and everything the two tests
complete they complete from the header counters alone. Both a decoder transfer
and a DSP round trip must deliver every frame bit-exactly and reach EOS with
every wake lost. Before the slice, the first read runs to its (short, loud)
bound; after it, the run costs one slice per handoff. This pins the property
the two stalling suites silently depended on, deterministically and without a
race.

---

## 4. What only the VM can prove

Nothing here was executed: `cargo test` belongs to the orchestrator's guest.
Run on the host and reported below: `cargo fmt --all -- --check` **0**,
`cargo clippy --workspace --all-targets --all-features -- -D warnings` **0**,
`cargo build --workspace` **0** (and `--tests`, which is what compiles
`deaf_wake.rs`).

The guest has to answer three things:

1. **Does `deaf_wake.rs` pass?** It is the claim of §3 and it is deterministic:
   two tests, well under a second each, no race to catch. If it fails, the
   escape message names the step and prints the counters.
2. **Do the two stalling binaries stay green over a measured number of runs?**
   The rate the lane notes are still waiting for. The slice makes the stall
   impossible to reach *as a 30 s stall*, so a red here would mean a different
   mechanism, not a smaller rate of this one.
3. **If anything ever stalls again: which cause was it?** Both suites now
   print the ring counters at the escape, which is the measurement that
   separates the two hypotheses with no instrumentation run needed:
   - `available > 0` at a read escape (or `available < capacity` at a write
     escape) → the frames or the space were already there and the waiter slept
     on them: a skipped wake, i.e. this defect surviving in some form.
   - `available == 0`, `closed == false`, `published` unchanged for the whole
     30 s → the peer genuinely published nothing: a wedged or starved host.
     Add `Get-VM` / host-paging evidence from the same window to tell a paused
     VM from a wedged thread.

A sharper version of measurement 3, if it is ever wanted: log
`published`/`read` once per slice at `RUSTAMP_*`-gated verbosity and look at
whether the counters moved during the stall. Not added here — the escape
snapshot already discriminates, and a per-slice log on the PCM path is a cost
on every run for one bit of information.

---

## 5. The follow-up this card does **not** do

The root cause stays in the tree, one layer down:

- `crates/rustamp-ipc/src/shm/ring.rs`: drop the four `was_empty`/`was_full`
  guards (446, 489, 824, 866, 570, 941) and signal unconditionally after the
  counter store. That is one `SetEvent` per advance, which RFC 0007 §6.2
  itself calls "always correct". The slice in this card then only covers the
  `close()` direction and real stalls, as the RFC intends.
- `docs/rfc/0007-blocking-consumer.md` §6.2: the soundness proof of the
  refinement is wrong (§1 above) and must either be corrected or the
  refinement withdrawn.
- `ring.rs` 2029 `blocking_no_lost_wakeup_under_jitter`: acceptance criterion
  3 is not actually tested — a skipped wake only makes it slow. It needs a
  bound that fails, or the deaf-wake shape from `deaf_wake.rs`.

Both paths are outside this card's owned set (`rustamp-mediahost`,
`rustamp-dechost`, `rustamp-dsphost`), so they need an owner who holds
`rustamp-ipc` and `docs/rfc`.
