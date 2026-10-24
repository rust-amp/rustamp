# W35: RFC 0006 remainder — the DSP end-of-transfer evidence and the retired stub facade

Status: implemented; gates green in the lane worktree.
Date: 2026-10-19. Lane: `wave135-mediahost-pcm`.

## Boundary and sources

Worktree: `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave135-mediahost-pcm`.
Nothing under `D:\code\Rustamp` was written; `D:\code\rustamp-research`, Winamp
source, SDK headers and disassembly were never read. No window, no audio
device, no download, no dependency, no git write or commit.

Sources: `docs/rfc/0006-out-of-process-hosting.md` (read completely),
`docs/rfc/0007-blocking-consumer.md`, `docs/tasks/W22-mediahost-pcm.md`,
`docs/tasks/M10-mediahost-pcm.md`, the mediahost crate, the IPC ring/wake
implementation and the resident crates. Only freely licensed project sources.

## What was already real (verified, not rebuilt)

RFC 0006's PCM transfer is implemented in `crates/rustamp-mediahost`: the
decoder conversation (`src/decoder.rs`), the two-ring DSP conversation
(`src/dsp.rs`), the named mappings and wake events (`src/pcm.rs`, RFC 0007
§5), the supervised control channel (`src/control.rs`), the resident
boundary and the real residents in `rustamp-dechost` / `rustamp-dsphost`
(RFC 0006 §8 erratum). The framework's own binaries keep reporting that no
resident is linked; that is the licensing boundary, not an unfinished
transfer.

## What this lane changed

### 1. The DSP end-of-transfer evidence (RFC 0006 §4, §6)

The DSP path has **no** end-of-stream marker (unlike the decoder path's
`DecoderEos`), so the only end-of-transfer evidence was the back-ring's
close. A close was accepted as a clean end of stream even when the host had
failed mid-transfer and returned fewer frames than it was given: the player
read `Ok(0)`, the engine took it for a finished track, and the loss was
invisible. W22 §"Unsettled remainder" item 1 named exactly this hole.

The evidence is the RFC's own data-plane contract (RFC 0006 §6): "the host
processes strictly in order, so block *n* out is block *n* back". Every
frame written into the chain has to come back, so at the close the session
compares the frames written into the in-ring with the frames read back from
the back-ring. A mismatch is a truncated transfer: a loud, sticky failure
that names both counts, never an end of stream. The counters are per ring
generation, so a reconfigure starts the comparison over.

No wire change, no new field, no new message: the rule is derived from the
RFC's FIFO guarantee, and it can only turn a silent loss into a loud
failure — a complete transfer still ends in `Ok(0)`.

Changed: `crates/rustamp-mediahost/src/dsp.rs` (the check in
`DspSession::read`, the module and method docs, the two counter accessors'
docs).

### 2. The legacy stub facade is gone (`crates/rustamp-mediahost/src/lib.rs`)

`MediaHost`, `DecoderHost` and `DspHost` were the crate's remaining stub:
a single-message facade that answered RFC 0006's own `DecoderOpen`,
`DecoderSeek`, `DspConfigure` and `DspReset` with `NotImplemented`. Such a
facade cannot express the RFC 0006 §5/§6 conversation at all — it has no
ring, no handshake and no lifecycle — and the crate's module docs had to
apologise for it. Nothing in the workspace called it: the only references
were its own definition and its own four unit tests (`grep` over the whole
tree). The RFC's contract surface is `serve_decoder` / `serve_dsp` plus
`DecoderSession` / `DspSession`, and it is unchanged.

This is the one public-API removal of the lane; it is a single file and
revertible on its own.

## Tests

Both new tests are in-process (one process, threads, named mappings, no
child process, no device), in `crates/rustamp-mediahost/tests/dsp_control.rs`
next to the sibling test for the pre-finish case.

| Test | What it forces | Mutation witness |
| --- | --- | --- |
| `a_truncated_transfer_after_finished_writes_is_a_loud_failure_not_eos` | A scripted host completes the real two-ring handshake, takes four frames from the in-ring, publishes only three and closes the back-ring. The control channel stays healthy (no `DspError`, no EOF, no missing Pong), so the frame counts are the only evidence. | With the count check disabled the read returns `Ok(0)` and the test fails with "a truncated transfer must not turn into EOS" (verified by disabling the check and re-running). |
| `a_complete_transfer_after_finished_writes_is_the_end_of_stream` | The same scripted host publishes every frame; the drained back-ring is the end of the stream and it sticks. | Guards the rule against a false positive: without the counts matching, a complete transfer would fail. |

The existing suites stay unchanged and green: `dsp_control` (9 tests),
`pcm_transfer` (11), `deaf_wake` (2), `pcm_named` (3), `dsp_named` (3), the
crate's unit tests, plus the four `#[ignore]`d two-process suites, which
stay VM-only (they need a staged fixture binary and a second process).

## Deliberately left, and why

1. **RFC 0006 §5.6's literal "drain, then `DecoderEos`, then close" order.**
   The host closes the ring and then sends the marker. RFC 0007 §6.5 (the
   later accepted RFC) states that the ring's close-then-drain order *is*
   the drain-then-EOS order §5.6 needs, and the client only accepts the
   marker after the ring is drained, so no frame is lost. A literal
   producer-side drain would need a lossless, cancellable `wait_drained`
   primitive in `rustamp-ipc` (an IPC-lane change); a spin or sleep poll is
   not allowed.
2. **A host-to-main "successful completion" marker for the DSP path.**
   W22 asked for one, but RFC 0006 names no such message, and a new
   `Message` variant needs its own RFC, a `PROTOCOL_VERSION` bump and
   round-trip tests in `rustamp-ipc`. The frame-count rule gives the same
   unambiguous positive evidence without touching the wire.
3. **RFC 0006 §7's "`PROTOCOL_VERSION` becomes 2".** The protocol is at 4
   (RFC 0005 → 2, hosting → 3, RFC 0015 → 4). It is not decremented; the
   intent of §10.1 (a clean `Hello` rejection against an old peer) is met
   by the existing version gate.
4. **The engine-side `Decoder` wrapper** (RFC 0006 §5, "Engine side"),
   **resident selection** (§§2/9), **URL decoding** (M4), **non-UTF-8
   paths** (HANDOFF-3) and **packaging** (§8) belong to other crates and
   lanes: `rustamp-audio`, the resident crates and `xtask`.
5. **The two-process acceptance paths** (kill-the-host, named mapping
   across processes) stay `#[ignore]`d and VM-only; they are not faked and
   were not run here.

## Gates

Run in the lane worktree; cargo output and `TEMP`/`TMP` under the
worktree's `target`.

| Gate | Exit |
| --- | --- |
| `cargo fmt --all -- --check` | **0** |
| `cargo clippy -p rustamp-mediahost --all-targets --all-features -- -D warnings` | **0** |
| `cargo test -p rustamp-mediahost --all-features` | **0** (all suites, the four two-process tests ignored) |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | **0** (the removed facade had no caller) |
| `cargo test --workspace --all-features` | **0** (112 suites, 1560 passed, 19 ignored, 0 failed) |

## Files

| File | Change |
| --- | --- |
| `crates/rustamp-mediahost/src/dsp.rs` | the end-of-transfer evidence in `DspSession::read` and the docs that describe it |
| `crates/rustamp-mediahost/src/lib.rs` | the legacy `MediaHost`/`DecoderHost`/`DspHost` stub facade removed |
| `crates/rustamp-mediahost/tests/dsp_control.rs` | the scripted-host pair and the two new tests |
| `docs/tasks/W35-mediahost-transfer-evidence.md` | this card (new) |
