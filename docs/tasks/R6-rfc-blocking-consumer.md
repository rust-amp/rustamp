# Task card R6 — Draft RFC 0007: blocking consumer for the shm rings (Q1)

**Goal**
Write the **draft** (proposed, not accepted) of `docs/rfc/0007-blocking-consumer.md`:
how an shm-ring consumer blocks instead of polling (event handle, semaphore,
or credit scheme), with lifecycle, wake-up/loss rules, and what it costs the
ring API and the protocol. RFC 0006 §4 depends on it (lossless decoder/DSP
delivery); this draft unblocks that implementation. No implementation — the RFC
is the deliverable.

**Lane** R — RFC planning (docs only).

**Owned paths**
- `docs/rfc/0007-blocking-consumer.md` (new file; the only file you may change).
- Everything else is **read-only**. If the RFC needs something you cannot
  change from the draft, write it as an open question.

**Allowed sources**
- RFC 0003 §§6–8 and open question Q1 (the exact gap), Q2 (DACL — your
  mechanism must not assume a security model Q2 has not given it), Q4
  (late joiner — say how blocking interacts with it).
- RFC 0006 §4 (the lossless-delivery requirement that motivates this) and §9
  (what stays undecided).
- `crates/rustamp-ipc/src/shm/**` (the ring API to extend: `write_from`,
  `read_into`, counters, `close`), `crates/rustamp-ipc/src/message.rs`
  (`PROTOCOL_VERSION` rules — note RFC 0005 and RFC 0006 both target v2; your
  proposal must say which bump it rides).
- The repository's existing RFCs for shape and tone.

**Forbidden sources**
- Winamp 5.x source, Winamp SDK headers or documentation, disassembly, the
  Winamp 2.95 binary, `D:\code\rustamp-research`, `D:\code\rustamp-refbin`.
  Nothing from them.
- Do not download anything. Do not run a reference binary.

**Scope**
- **In scope:** the mechanism choice with reasoning (named auto-reset event per
  ring? one event per direction? semaphore with count = free slots? pure
  credit messages on the control channel?); who creates/owns/closes the kernel
  object and how its name/lifetime ties to the ring generation rule (stale
  generations must not wake the wrong consumer); lost-wakeup-proof sequencing
  (check-then-wait with the generation/counter as the condition); spurious
  wakeups; timeout and shutdown semantics (`close()` wakes the waiter with
  `Closed`); which ring calls gain blocking variants and which stay
  non-blocking (the audio render path and vis polling must never block);
  whether `RingOffer`/`RingReady` change; the version-bump accounting with RFC
  0005 and RFC 0006; acceptance criteria for the implementation lane.
- **Out of scope:** implementing anything; accepting the RFC; Q2/Q3/Q4/Q5
  themselves (reference them, do not answer them); changing the slot layout or
  the publication ordering of RFC 0003 §6.
- The hot path stays allocation-free and lock-free; the blocking call itself
  may sleep in the OS, and the RFC must say exactly where sleeping is and is
  not allowed.

**Acceptance tests**
- [ ] The draft exists at `docs/rfc/0007-blocking-consumer.md` and follows the
  shape of the earlier RFCs (motivation, mechanism with rejected alternatives,
  lifecycle, wake-up/loss rules, API sketch, version accounting, backwards
  compatibility, acceptance criteria, open questions).
- [ ] Nothing outside the owned paths was modified (`git status --short` shows
  only the new file).
- [ ] No `cargo` build is required (docs only), but the draft must quote the
  current ring API names exactly.

**Provenance notes**
- None expected (design from our own RFCs and code).

**Report back**
The RFC path; the recommended mechanism with the decisive reason; the rejected
alternatives in one line each; the version-bump accounting; the open questions
left for the owner; anything you could not decide from the allowed sources.
