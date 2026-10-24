# W19 (root cause): the ring signals on every advance

Follow-up to [`W19-twoproc-stall.md`](W19-twoproc-stall.md) §5, which fixed the
consequence and named the cause. The consequence was that one skipped wake cost
a player-side waiter its whole deadline; that card sliced every PCM wait in
`rustamp-mediahost` so a skipped wake now costs one 20 ms slice. The cause was
still in the tree: the wake signal is **edge-triggered**, and each side decides
the edge from the counter the *peer* owns, loaded before its own operation and
never ordered after the peer's last store to it. A plain thread interleaving
lets both sides evaluate a stale edge, neither signals, and both ends sleep on
a ring that is neither closed nor failed (that card's §1 has the interleaving,
and this card's RFC change restates it).

This card removes the edge. Owned: `crates/rustamp-ipc/**`,
`docs/rfc/0007-blocking-consumer.md`, this file.

---

## 1. The ring: every advance signals

`crates/rustamp-ipc/src/shm/ring.rs`. The rule is now stated once, in the
module docs (**12–30**, `# The signalling rule`), and the six call sites point
at it instead of repeating it. All six guards are gone — the card's §5 named
all six line numbers, while its §1 table listed only the four PCM ones; the two
vis sites carried the same unsound edge, so they are fixed too.

| site | was | now |
| --- | --- | --- |
| `write_from` **465–470** | `if was_empty && let Some(wake)` | `if let Some(wake)`, inside the existing `accepted > 0` block |
| `write_slot` **511–514** | `if was_empty && let Some(wake)` | `if let Some(wake)`, after the only `store_write_seq` that is reached when the slot was accepted |
| `write_from_blocking` **594–595** | `if was_empty { signal }` | `let _ = wake.signal(Wake::Data);` unconditional |
| `read_into` **846–851** | `if was_full && let Some(wake)` | `if let Some(wake)`, after the early `count == 0` return |
| `read_slot_into` **890–893** | `if was_full && let Some(wake)` | `if let Some(wake)`, after the early `available == 0` return |
| `read_into_blocking` **967–968** | `if was_full { signal }` | `let _ = wake.signal(Wake::Space);` unconditional |

The four `was_empty`/`was_full` locals are removed; `occupancy`/`available`
stay, because `free` and `count` are derived from them. The doc comments of
`write_from` (**436–438**), `write_slot` (**485–486**), `read_into`
(**819–821**) and `read_slot_into` (**863–864**) now say "after every
write/read that advanced the counter" instead of "when the write started from
empty".

### What I kept of the fast path, and why

Two guards survive, and both are facts about **this** side, so neither can be
evaluated against a stale peer counter — which is the whole defect:

1. **`self.wake.is_some()`** — a wake-less ring (every vis ring in production,
   RFC 0007 §9) still does not even look at an event. Unchanged behaviour,
   byte for byte.
2. **"this side actually stored"** — a call that accepted or read zero frames
   changed nothing, so it signals nothing. That keeps the two hot no-op paths
   syscall-free: `write_from` into a full ring (the drop path, which only
   advances `dropped_seq`) and `read_into`/`read_slot_into` on an empty ring.
   In the two blocking methods this guard needs no code at all: the signalling
   branch is entered only when `free > 0`/`available > 0` with `offered > 0`/
   `want > 0`, so it always stores — the comments at **590** and **963** say so
   rather than asserting it.

What I did **not** do: keep a syscall-free path for "nobody is waiting". The
two ways to have it are store-then-recheck behind a `SeqCst` fence (re-read the
peer's counter *after* your own store and signal if the peer's condition now
holds) and a waiter flag in the ring header. The first is sound but costs a
fence on the hot path and is one review mistake away from reintroducing exactly
this bug; the second needs a new header word, i.e. a ring-ABI change and an
RFC. Against one `SetEvent` on a path that already does a shared-memory copy,
neither is worth it today. Recorded as RFC 0007 open item **O15** for whoever
wants it and is prepared to prove it.

The cost of the change is therefore one extra `SetEvent` per advance on a wake
ring, in exactly the case the old code skipped: an advance that did not cross
the empty or full boundary. RFC 0007 §6.2 always allowed this ("costs one extra
`SetEvent` per advance and is always correct"), §6.1's own argument already
assumed it ("that end always signals when it advances that counter"), and §6.3
makes the extra signal harmless: the waiter re-derives its condition from the
counters, so a spurious wake costs one wait return and one re-check, never a
frame.

---

## 2. RFC 0007 §6.2: **withdrawn**, not corrected

`docs/rfc/0007-blocking-consumer.md`. I withdrew the refinement.

**Why not correct the proof.** The proof's load-bearing step is "the producer's
next write reads its *own* `write_seq` and **that same `read_seq`**". For the
code the refinement describes, that premise is false: nothing orders the
producer's load of `read_seq` after the consumer's last store to it, and the
interleaving in which it does not hold needs nothing exotic — four single
atomic operations in a plain thread schedule. A proof cannot be repaired by
rewording when its premise is false; only the *rule* could be changed (see O15
above), and a different rule is not this one. Correcting would also leave the
unsound rule standing as the recommendation while the implementation does
something else, which is how the document and the code drifted apart in the
first place.

What changed in the document:

- **Top (6–20):** a dated `Correction (2026-10-18)` block that says the
  refinement is withdrawn, that the rule of record is the unconditional one,
  and that nothing on the wire, in the header or in the API moves.
- **§6.2 (321–388):** the rule first ("every advance signals; the only guard is
  that your own store happened"), then a `#### The withdrawn refinement`
  subsection that keeps the old text and its proof verbatim for the record,
  marks the skipped ordering step, gives the capacity-3 interleaving, notes
  that the occupancy invariant the proof leaned on holds throughout and simply
  says nothing about which snapshot each side used, and that the auto-reset
  latch buys the consumer exactly one extra re-check and no more. Then why it
  is withdrawn rather than corrected, and the two sound alternatives.
- **§8's signalling table (563–578):** the four "if the write started from
  empty" / "if the read started from full" conditions become "whenever the
  store happened", with a note that the withdrawn conditions were there until
  2026-10-18 and that the two blocking methods carry the same rule, making
  seven sites rather than five.
- **§12 (667–671):** the cost bullet no longer says "only when the advance
  could have made the peer's condition true".
- **§14.3 (713–727):** see below.
- **§15:** new open item **O15**.

RFC 0007 is `Accepted`, so I did not rewrite its history: every change is dated
and the withdrawn text is still readable in place. No `Message`, header,
`RING_VERSION` or API change is involved, so §11's version accounting is
untouched.

---

## 3. The acceptance test: a bound that fails

Acceptance criterion 3 ("no lost wakeup at the check/wait boundary") was
satisfied by a test that could not fail. `blocking_no_lost_wakeup_under_jitter`
(now **2082–2160**) waited `TEST_WAIT` (10 s) on both sides and *looped* on a
zero read: the producer retried its write in a `loop … spin_loop`, the consumer
let `read == 0` fall through its `while received < FRAMES`. A skipped signal
made the test ten seconds slower per occurrence and green either way.

**Now `Ok(0)` is a failure on both sides.** It is a sound assertion, not a
flake risk: while the peer thread still has work, the ring cannot be empty for
the consumer and full for the producer at the same time, and with every advance
signalling a live peer always sets the latch the waiter is on. So a 10 s
`TEST_WAIT` that expires here means the signal was skipped — and the messages
print the counters that prove it (`frames_available`, `frames_published`,
`frames_read` at **2135–2142**; `free_frames`, `frames_written` and the derived
consumed count at **2116–2124**), the same discriminator the two host suites
now print at their stall escape. The producer's retry loop is gone with it,
which is why the net change is a strengthening and not a trade.

That is still a race, so it is only half of the criterion. The other half is
**deterministic** and is new, in the shape `deaf_wake.rs` uses — ignore the
signal as a liveness mechanism and inspect it directly:

- `pcm_ring_with_wake_probe` (**1716–1740**) hands the test a *third* handle on
  the ring's two latches (`RingWake::create` then `clone`, so all three handles
  are the same two kernel events — `create_and_clone_share_the_same_latches`
  in `shm/wake.rs` already pins that). `pcm_ring_with_wake` is now a two-line
  wrapper over it, so the existing tests keep their exact ring.
- `drain_latch` (**1742–1748**) consumes whatever is already latched on one
  direction, bounded by RFC 0007 §6.4's 1 ms floor. An auto-reset event
  releases exactly one waiter, so the probe is a peer waiter, not an observer —
  the three tests below are the only thing waiting on either direction, and the
  helper's doc says so.
- `every_advance_signals_not_only_the_edge` (**2162–2201**): write one frame,
  drain the `Data` latch, write a second frame **into a non-empty ring**, and
  require that the latch is set again; then drain `Space`, read one frame
  **from a non-full ring** (2 of 4), and require the same. `PROBE_WAIT` is
  250 ms and is a how-fast-does-it-fail bound only: the signal is a `SetEvent`
  the ring made before returning, so a correct ring returns from the wait
  immediately.
- `every_blocking_advance_signals_not_only_the_edge` (**2203–2244**): the same
  two assertions through `write_from_blocking`/`read_into_blocking`, which
  carried their own copy of the edge. Neither call ever waits here; each finds
  its condition already met.
- `every_vis_advance_signals_not_only_the_edge` (**2246–2279**): the same for
  `write_slot`/`read_slot_into`, which also carried it. Vis rings are created
  without a wake in production (RFC 0007 §9), so this pins the rule for the
  shape, not for a live caller.

Under the old code all six of those assertions fail, each with the site named.
`§14.3` of the RFC now demands both shapes and says why the old wording was
not enough.

**No existing test got weaker.** The two retry loops that disappeared were the
only tolerance removed, and both became assertions. `TEST_WAIT`, the ring
helpers' behaviour, and every other test's ring are unchanged.
`blocking_two_thread_lossless_delivery` is deliberately left alone: its
`assert!(written > 0, "producer stalled")` already fails on a skipped `Space`
signal, its consumer side still loops on zero, and its subject is criterion 2
(losslessness), which a skipped wake cannot break. Making it fail fast as well
would duplicate the jitter test without adding a property.

---

## 4. Files

| file | change | +/− |
| --- | --- | --- |
| `crates/rustamp-ipc/src/shm/ring.rs` | the signalling rule in the module docs; six guards removed; four doc comments; the jitter test's bound; `pcm_ring_with_wake_probe`, `drain_latch`, `PROBE_WAIT` and three deterministic tests | +238 / −36 |
| `docs/rfc/0007-blocking-consumer.md` | §6.2 withdrawn with the counterexample, the correction block, §8's table, §12's cost bullet, §14.3, O15 | +105 / −28 |
| `docs/tasks/W19-ring-wake-root.md` | **new** (this card) | +248 |

Not touched, and why:

- `crates/rustamp-mediahost/src/pcm.rs` **26–31** describes the edge as live
  code ("`rustamp-ipc` takes RFC 0007 §6.2's edge-triggered refinement"). It is
  now history, and the `RING_POLL` slice it justifies is still justified
  without it: the slice also covers an abrupt peer exit, which sets no state
  and signals nothing (RFC 0007 §6.5 `Drop`), and a genuinely wedged peer. That
  crate belongs to the W19 card's owner; the comment needs one sentence, not a
  behaviour change.
- `shm/wake.rs` already states the unconditional rule ("signaled by the
  **producer** after `write_seq`") and its §6.2 reference at **389** stays
  accurate.

## 5. Gates

Run on the host, from the worktree:

| gate | exit |
| --- | --- |
| `cargo fmt --all -- --check` | **0** |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | **0** |
| `cargo build --workspace` | **0** |

`cargo test` was **not** run: the suite belongs to the orchestrator's VM.

## 6. What only the VM can prove

1. **Do the three new deterministic tests pass?** They are the claim of §1 and
   they carry no race: three tests, milliseconds each. A failure names the
   signalling site and the direction.
2. **Does `blocking_no_lost_wakeup_under_jitter` still pass, and how long does
   it take?** It is now an assertion rather than a retry loop, so a pass is
   worth something it was not worth before. Its runtime should be
   indistinguishable from before (5 000 one-frame handoffs); a timing change
   would be the extra `SetEvent` per advance showing up, which is the one cost
   of this change.
3. **Does the rest of the `rustamp-ipc` suite stay green with more signals in
   flight?** The extra signals are all in wake rings, and no test asserts an
   upper bound on a wait whose peer advanced (`no_alloc.rs` uses wake-less
   rings; the wall-clock bounds in `blocking_timeout_*` are on idle rings), so
   this should be a no-op — but "should be" is a host read of the tests, not a
   run.
4. **Do `dsp_roundtrip` and `real_decode` stay green over a measured number of
   runs?** The W19 card made the stall cost one slice instead of 30 s; this
   card removes the skipped signal that caused the slice to be needed. A stall
   at the 30 s escape now means a third mechanism, and the counter snapshot
   those suites print at the escape says which: `available > 0` at a read
   escape would mean a wake is *still* being skipped somewhere, which after
   this change could no longer be the ring's own edge.
