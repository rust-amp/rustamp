# W33 — Resident-process test accounting and death preconditions

## Scope and provenance

Worktree: `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave133-resident`.
Owned: `crates/rustamp-audio/**`, `crates/rustamp-dechost/**`, this card.
Changed: only `crates/rustamp-audio/tests/resident_process.rs` and this card.
No product, resident fixture, IPC, shared contract or dependency changes.
No downloads, reference binaries, git writes or commits. No Winamp source,
SDK material, disassembly or research directory was read. Sources: the
repository's own code and RFC 0006 §§5/8; no third-party code was copied.

## Findings

### `the_stub_resident_decodes_through_the_engine_adapter`: test fault

The reported `3991` is an accounting error, not evidence of lost PCM:

1. The fixture serves 4099 frames at 1000 Hz, two channels.
2. The test seeks to frame 100, so exactly 3999 frames remain.
3. It reads up to eight post-seek preview frames and checks the first sample.
4. It then resets its count to zero and demands 3999 *additional* frames.
5. The observed 3991 plus the eight already consumed is exactly 3999.

The missing invariant is that **every successful post-seek read counts**,
including a partial preview read. No worker-readiness wait can fix that
arithmetic. The adapter's `convert_read` already blocks on `DecoderSession`;
these tests do not pump an unready engine output clock.

Fix: seed the total with the actual preview count, not an assumed eight.
Keep the exact 3999-frame assertion and sticky EOS assertion. Additionally
check every stereo frame against its absolute fixture index, both in the
preview and while draining, and reject overproduction immediately. Thus
missing, duplicated, reordered or torn frames cannot hide behind a correct
total.

### `a_helper_that_dies_mid_stream_is_a_loud_decode_error`: test fault

The reported `64` is `Ok(64)`, **not** `Ok(0)` and not an EOS signal.
`StubDecoder::read` produces up to 257 frames. The test's output buffer holds
only 64 frames, so the next consumer read can legitimately return another
64 frames from that same producer block. Producer calls, ring publications
and consumer reads are not one-to-one. The stub exits on its *next decoder
read* when its position is positive, after the pump has published the first
block; it does not exit after the consumer's first read.

There was also a scheduling precondition missing: with the adapter's default
8192-frame ring, the whole 257-frame block fits without a consumer. The
helper can die before a slow guest consumer takes any audio. With the
one-restart policy, an initial failure before any delivered frame can reopen
at zero and legitimately deliver audio on the retry too.

Fix: use a **one-frame prefetch ring**. The producer must wait for observable
consumer progress before it can finish publishing its block and reach the
exit. This establishes a genuinely mid-stream death without a new sleep or
assuming any process schedule. Drain partial reads until a terminal error,
with the existing per-read stall bound and an overall stall escape. Assert:

- positive delivered audio before the failure (the previous early-error
  acceptance branch is removed);
- every delivered frame exactly matches the ordered stereo pattern;
- no more than the original 257-frame block is delivered;
- `Ok(0)` is always a failure, never an accepted death;
- the terminal error is a nonempty `CoreError::Decode` and still names the
  resident restart;
- a later read remains a loud decode error, never audio or EOS.

The positive consumed position also makes the restart deterministic: the
fresh helper seeks there, and `fixture:exit` then exits without new audio.
The original error/restart guarantees are retained, with stronger frame and
sticky-error checks. The existing fixture's artificial 50 ms exit delay was
not changed or used as a readiness guarantee.

## Why the neighboring fixtures pass

- `pcm_process::named_pcm_two_process_transfer_and_seek` initializes its
  absolute frame index at 29 immediately after seeking and counts/checks
  *every* subsequent frame. There is no uncounted preview.
- `pcm_process::host_exits_mid_stream_and_infinite_read_survives` uses a
  three-frame ring and loops over successful partial reads until the typed
  disconnect; it never equates the second read with the exit.
- `pcm_process::killing_the_helper_mid_stream_is_a_loud_error_and_never_a_torn_frame`
  uses a one-frame ring, observes delivered audio before terminating the
  helper, and drains/checks any in-flight frames until the error.
- The DSP exit test also loops until the error. The DSP kill test observes
  explicit one-frame write/read round trips before killing the helper.
- Those mediahost tests drive `DecoderProcess`/`DspProcess` directly, not
  `ResidentFactory`'s additional one-restart wrapper. The passing Group D
  plugin tests likewise do not exercise either faulty assertion here; their
  success is not a proof of the adapter's frame accounting.

## Verification

Host gates, with each process exit code captured immediately after Cargo:

- `cargo fmt --all -- --check`: exit **0**.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: exit **0**.
- `cargo test --workspace --all-features`: pending.

Cargo ran offline (no downloads). The test run uses `RUST_TEST_THREADS=1`
to serialize the audio harnesses and redirects `TEMP`/`TMP`
inside this worktree. Logs: `target/w33-resident-{fmt,clippy,test}.log`.

The two changed tests remain ignored and were **not executed on the host**:
they spawn helper processes and are VM-only. Only the guest rerun with
`RUSTAMP_DECODER_FIXTURE` staged can confirm the actual named-ring,
cross-process EOF/wake, restart and sticky terminal behavior under its
scheduler. Headless workspace tests and compiling the ignored test targets
cannot substitute for that verdict.

`git -C D:\code\Rustamp status --short`: exit **0**, empty output (zero
status lines). `git diff --check`: exit **0**. The test file is 521 lines,
with **70 added / 45 removed**; this task card is new. No other tracked files
were changed.
