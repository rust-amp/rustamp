# M14a: The kill-the-host two-process acceptance test

## Goal and scope

M14 §4 listed "abrupt resident kill mid-stream" among the things that cannot
be verified without two real processes. This lane writes that acceptance test
for the **decoder** path (the media host itself) and splits it into a
headless part that runs everywhere and a two-process part that stays
`#[ignore]`d.

Owned paths: `crates/rustamp-mediahost/**` and this card. Not touched:
`rustamp-ipc`, `rustamp-audio`, `rustamp-app`, `rustamp-dechost`,
`rustamp-dsphost`. No dependency added, no manifest or lockfile change, no
download, no window, no audio device, no plugin execution, no git write or
commit.

Sources: the repository's own code, `specs/` and the M14 review. No Winamp
source, SDK header or disassembly was read; no reference binary was run.

## 1. What was added

| File | Change |
|---|---|
| `crates/rustamp-mediahost/tests/support/frame_pattern.rs` | New. The fixture's deterministic frame pattern: frame `index` is `[index / 8, -index / 8]`. Shared by the in-process fake host and the staged VM fixture, so both kill checks compare against one expectation. |
| `crates/rustamp-mediahost/tests/pcm_transfer.rs` | New headless test `a_dead_host_delivers_whole_frames_and_then_fails_loudly` (runs on the host). |
| `crates/rustamp-mediahost/tests/pcm_process.rs` | New VM-only test `killing_the_helper_mid_stream_is_a_loud_error_and_never_a_torn_frame` (`#[ignore]`d), plus the `read_pcm` / `check_frames` helpers. |
| `docs/notes/vm-run-tests.ps1` | The W12 ignored-test inventory now names the new test (Group C: 4 -> 5 entries). No staging change. |

No production code changed: `rustamp-mediahost/src/**` is untouched, so the
wire, the protocol version and every public API are unchanged.

## 2. The headless part (runs on the host)

`a_dead_host_delivers_whole_frames_and_then_fails_loudly` stands in for the
helper process: a fake host speaks the wire, publishes three frames into the
real ring and then vanishes - it sends no terminal message, drops its control
transport and never closes the ring, which is exactly what a killed process
leaves behind.

Pinned:

1. **No torn frame.** The consumer receives exactly the three published
   frames, each equal to `[index / 8, -index / 8]` and in order. A frame that
   was torn, dropped or reordered breaks the pattern and fails the test.
2. **No silent end of stream.** Every read that follows the death either
   delivers frames or fails; `Ok(0)` (the session's EOS answer) fails the
   test.
3. **A loud, typed error.** The terminal outcome is
   `MediaHostError::Disconnected` and its text names the control channel
   (`control stream ended: ...`), i.e. the death was seen on the control
   stream, not guessed.
4. **No hang.** The wait that follows the death is bounded by the suite's
   30 s stall escape; a timeout is a loud failure that names the step.
5. **The failure sticks.** A second read after the failure reports the same
   loud error and never turns into end of stream (M14 L6 had no test).

### The assertions can fail (mutation evidence)

Two deliberate mutations of the *test's own* fake host were run and reverted:

* the host publishes a shifted pattern (`frame_samples(index + 1)`) -> the
  test fails with "frame 0 was torn, missing or out of order";
* the host says "end of stream" instead of dying -> the test fails with
  "a dead host must not look like end of stream (0 frames)".

Both mutations were reverted (the file was restored from the copy taken
before the mutations) and the suite is green again. 25 consecutive runs of
the test passed (no flake).

A finding worth recording: the consumer **cannot** observe a `DecoderEos`
while the ring is still open - `DecoderSession::read` drains the control
queue before the ring wait and only reads further control messages after the
ring closed. A host that announces end of stream and then dies is therefore
reported as the loud control failure, not as end of stream. That is the
wanted direction ("first failure sticks"), but it means the EOS path is only
reachable through the ring close, which the normal host does before it sends
`DecoderEos`.

## 3. The two-process part (VM only, `#[ignore]`d)

`killing_the_helper_mid_stream_is_a_loud_error_and_never_a_torn_frame` uses
the crate's own harness: `DecoderProcess::spawn` starts the staged
`decoder_fixture` helper (`RUSTAMP_DECODER_FIXTURE`), which completes the
Hello / `DecoderOpen` / `DecoderOpened` / `RingOffer` / `RingReady`
handshake inside `spawn`.

Then:

1. four frames are read while the helper streams, each checked against the
   fixture's pattern;
2. `DecoderProcess::terminate()` kills the helper from the outside
   (`TerminateProcess`: no clean exit, no terminal message, no ring close)
   while it is mid-stream;
3. the next read must fail with `MediaHostError::Disconnected` whose text
   names the control channel; `Ok(0)` fails the test, a timeout fails the
   test with the stall escape;
4. the frames that were published before the death are still delivered whole
   and in order;
5. a later read must report the same loud error (the failure sticks) and
   the transport's drop counter must still be zero.

### Staging

No new staging is needed: the test lives in the already staged `pcm_process`
binary, the `decoder_fixture` example is already built and copied by
`docs/notes/vm-run-tests.ps1`, and the guest runner already exports
`RUSTAMP_DECODER_FIXTURE`. The runner's session-0 escape hatch
(`-DesktopFreeResidentFixtures -Filter '--ignored:pcm_process'`) picks the
binary up unchanged, because the new test is in the same file as the two
existing ones. The only runner change is the W12 inventory entry, so the VM
report names the new test instead of silently omitting it.

## 4. What this proves and what it cannot prove

Proven on the host (headless): the consumer side of the contract - a dead
host yields whole frames only, then a loud typed error, never end of stream,
never a hang, and the error sticks.

**Not proven without the VM**: that a helper killed with `TerminateProcess`
mid-stream is reported the same way across a real process boundary. The
in-process fake host reproduces the *observable* shape of a killed process
(control EOF, no ring close, no terminal message), but it cannot show that
the real helper's stdout pipe EOFs, that the real helper leaves its shared
memory ring in the state a killed process leaves it in, or that the
`TerminateProcess` path reaches the same monitor branch. Those are exactly
the claims the `#[ignore]`d test makes, and it has **never been executed**:
this lane had no VM, no guest staging and no permission to run a helper.

Still open after this lane (M14 findings not addressed here): F1 (a DSP
stage cannot report a mid-run fault at all - the DSP kill analogue would
have nothing to assert), F2 (a truncated file may present as clean end of
stream), F3 (reset latency while idle), F4/F5 (the drift and prose-only
gaps, of which F4 was closed by wave184). The DSP-path kill acceptance (the
helper blocked in *read* instead of *write*) is still unwritten: the same
consumer mechanism is shared, but the blocked direction differs, so it needs
its own test.

## 5. Receipts (this worktree)

- `cargo fmt --all -- --check`: exit 0.
- `cargo clippy -p rustamp-mediahost --all-targets --all-features -- -D warnings`: exit 0.
- `cargo test -p rustamp-mediahost --all-features`: all pass, 3 ignored
  (the two-process tests, including the new one).
- `cargo check --workspace --all-features`: exit 0.

The headless test was run 25 times in a row without a flake. The
`#[ignore]`d test was not executed - it needs the VM.

Boundary: only this worktree was written. No file under `D:\code\Rustamp`
was touched, no git write or commit was made, and no forbidden tree was
read.
