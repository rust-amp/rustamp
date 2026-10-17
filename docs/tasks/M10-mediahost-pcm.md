# M10: Media-host PCM transfer

> **Orchestrator note (2026-10-17, after the VM runs).**
> 1. The first VM run after this merge failed one test,
>    `format_change_drains_old_pcm_before_new_channel_count`, with
>    `unexpected error: media host wait timed out` at a **five second** bound. The
>    next run passed it unchanged, so it is **flaky**, not a deadlock: the resident
>    fixture starves under load and the read's fixed timeout expires. A flaky test
>    in the suite is a defect of its own — it should wait on a condition rather than
>    on a wall-clock bound (or use a bound so generous that only a real stall reaches
>    it, with the message saying which). Until then, one red run of that test is not
>    evidence of a broken transfer.
> 2. The golden suite could not run in the guest at all until the runner learned to
>    stage `tests/golden` (`e9eda30`). Unrelated to this card, found by the same run.

## Boundary and scope

Write only `crates/rustamp-mediahost/**`, `crates/rustamp-dechost/**`,
`crates/rustamp-dsphost/**` and this card in the assigned worktree. Nothing under
`D:\code\Rustamp` may be read or written. No dirty-room/reference material.
Sources: RFCs 0003, 0006 and 0007 and the existing clean repository code only.
No copied third-party code, dependency additions, downloads, commits, windows,
audio playback or host test execution.

## Design decisions

- Follow RFC 0006 §5: the **host** creates/offers the decoder ring; the player
  opens/validates it and returns `RingReady`. The task's opposite allocation
  wording is not silently substituted for the accepted lifecycle.
- Keep the permissive framework independent of `rustamp-core` and real codec
  libraries. A resident supplies the decoder interface. No concrete codec is
  linked into the shipped placeholder binary.
- Use the existing named wake events and unchanged v3 messages. Blocking PCM
  calls are worker-only. A separate control monitor closes the ring on control
  loss, liveness failure, seek or shutdown, waking an infinite PCM wait.

## Outcome

Implemented a codec-independent decoder worker, player-side worker session,
Windows named PCM mappings/wakes, and a supervised child-process owner. This is
**framework completion, not full RFC 0006 acceptance**: no real codec resident or
`rustamp-core::Decoder`/engine adapter was added. Both helper binaries actually
live under `rustamp-mediahost/src/bin/`; there are no separate dechost/dsphost
crates in this checkout. The framework dechost explicitly reports that no
resident is linked instead of pretending to decode a real file.

## Requirement audit (before and after)

Path keys: **MH** = `crates/rustamp-mediahost`, **IPC** = `crates/rustamp-ipc`,
**PROTO** = `crates/rustamp-proto`. `HEAD:` references the unchanged baseline
before this lane; other references are the final working files. Verdicts describe
code inspection/compilation, **not executed test evidence**.

| RFC requirement | Before this lane (evidence) | After this lane (evidence/verdict) |
|---|---|---|
| §4 no dropped PCM; lossless producer and consumer wake | Ring primitives already implemented: IPC `src/shm/ring.rs:532,908`; hosting absent: HEAD:MH `src/lib.rs:81` | Implemented worker partial-write loop using infinite `write_from_blocking`: MH `src/decoder.rs:102,145`; worker client uses blocking read: `src/decoder.rs:442,460`. No lossy write call in the hosting path. |
| §5 one process, decoder, channel, active PCM ring per open decoder | Only stub state/pid: HEAD:MH `src/lib.rs:50` | Implemented owned process/session: MH `src/decoder.rs:593,602`; one resident opened at `src/decoder.rs:193`, one active producer at `src/decoder.rs:215`. |
| §5.1 path/URL open, no reader handles, no lossy non-UTF-8 conversion | Wire type exists: PROTO `src/host.rs:16`; existing conversion is lossy at `src/host.rs:24` | Resident receives only MediaLocationWire: MH `src/decoder.rs:53`; strict local-path helper at `src/decoder.rs:583`. Existing lossy proto helper was not changed; callers must use the strict helper before constructing file messages. Real file/URL decoding remains a resident task. |
| §5.2 DecoderOpened specification, duration, seekability, metadata | Wire exists: IPC `src/message.rs:186`; open is a stub: HEAD:MH `src/lib.rs:81` | Implemented resident snapshot/validation and client replay: MH `src/decoder.rs:196,307,383`. |
| §5.2 **host** allocates/offers, channels×4 frame bytes, prefetch capacity, HostToMain | Generic mapping exists: IPC `src/shm/region.rs:141`; no hosting call | Implemented: MH `src/pcm.rs:67,90,100`; player validates kind, direction, size, format, generation before opening at `src/pcm.rs:116`. No player-allocated decoder ring was substituted. |
| §5.2 RingOffer/Ready/Refused handshake before first PCM | Message shapes exist: IPC `src/message.rs:120,137,144` | Implemented: MH `src/decoder.rs:217,221,351`; opener refuses mapping/wake failures before Ready at `src/decoder.rs:361`. |
| RFC 0007 wake before offer, wake opened before Ready, no render blocking | Wake primitive exists: IPC `src/shm/wake.rs:311,333`; no hosting use | Implemented: MH `src/pcm.rs:96,158`; no render/UI/audio crate changed. Close-only supervisor views never consume frames/counters: `src/pcm.rs:50`. |
| §5.3 real resident fills ring; player reads like local decoder | No transfer: HEAD:MH `src/bin/dechost.rs:7` | Generic resident-driven transfer implemented: MH `src/decoder.rs:38,102,442`. **Partial:** deterministic test resident only; no real decoder library linked and no core Decoder implementation. Explicit unavailable resident: `src/decoder.rs:59`. |
| §5.3 skipped packets/current bitrate only on change | Wire only: IPC `src/message.rs:212` | Implemented comparison and client telemetry replay: MH `src/decoder.rs:131,387,397`. |
| §5.4 seek closes old ring, fresh generation/ring, reached-ms reply | Seek stub: HEAD:MH `src/lib.rs:87` | Implemented interruptible seek/reoffer and stale-prefetch discard: MH `src/control.rs:185`, `src/decoder.rs:207,219,247,512`. Generations are fresh nonzero IPC epochs, required to differ from the preceding ring; not an arithmetic old+1 written into a new mapping. |
| §5.5 format change requires new ring and DecoderFormat before frames | Wire only: IPC `src/message.rs:207` | Implemented: MH `src/decoder.rs:256,469`; old PCM drains before client changes channel count; new producer waits for matching Ready. |
| §5.6 drain/EOS/close, never confuse empty with EOS | Ring can drain after close: IPC `src/shm/ring.rs:928`; no hosted EOS | **Partial/lifecycle deviation:** MH `src/decoder.rs:237` closes after publishing the complete final block, then sends DecoderEos; client exposes zero only after old PCM drains and that marker arrives (`src/decoder.rs:460,484`). Host does not wait for consumer-drained before sending EOS, and stays alive for seek/shutdown. The current public ring API has no wait-until-drained primitive; literal RFC drain→EOS→close needs IPC-lane coordination, not a silent new wire acknowledgement. |
| §5.7 errors close ring, send DecoderError, surface like local errors | Binary propagated errors and exited without DecoderError: HEAD:MH `src/bin/dechost.rs:33` | Ring RAII close and bounded terminal diagnostic flush: MH `src/decoder.rs:70,160`. Resident errors stay loud/sticky at `src/control.rs:181`. **Partial:** engine event conversion is outside owned paths. |
| §5 engine-side rustamp-core::Decoder adapter | Absent: HEAD:MH `src/lib.rs:15` | **Deferred:** framework DecoderSession is available (`src/decoder.rs:277`); core trait implementation and engine registration belong to rustamp-audio, which this lane may not change. |
| §8 Hello/version gate, Ping/Pong, unexpected close/death must fail track not player | Passive Hello/Ping handler only: HEAD:MH `src/lib.rs:73,77`; ring drop does not close: IPC `src/shm/region.rs:242`, `src/shm/ring.rs:393` | Independent framed reader/writer/monitor: MH `src/control.rs:76,142,155,200`. Missed Pong, EOF, malformed/wrong-version control or queue saturation closes/signals the registered ring and yields Disconnected, never EOS (`src/control.rs:58,222,233`). Engine event wiring/retry policy still deferred. |
| §8 spawn, kill and reap on close; missing helper degrades | No spawning: HEAD:MH `src/lib.rs:15` | Implemented framework owner with CREATE_NO_WINDOW and unavailable-feature error: MH `src/decoder.rs:602`; startup-error cleanup at `src/decoder.rs:632`, Drop cleanup at `src/decoder.rs:664`. No process/window/audio was actually launched by this lane. |
| §6 DSP two-ring round trip, format/reset/latency | Protocol only; stub: HEAD:MH `src/lib.rs:126` | **Deliberately deferred**. MH `src/lib.rs:143`; stub binary now sends DspError instead of exiting on an unsupported operation (`src/bin/dsphost.rs:36`). |
| §7 wire and version accounting | Hosting variants already v3: IPC `src/message.rs:25,29,177`; wire spec in PROTO `src/host.rs:64` | **Unchanged** Message/proto/ring layout and PROTOCOL_VERSION=3. RFC text's v2 was already superseded by the checked-in v3 rationale. No bump, new message, field, refusal reason or dependency. |
| §8 permissive framework/resident licensing split | Manifest already correct: MH `Cargo.toml:6,18`; old crate docs incorrectly said GPL: HEAD:MH `src/lib.rs:11` | Manifest/normal dependency closure unchanged; crate docs corrected: MH `src/lib.rs:11`. No codec, core, copied code or dependency entered the permissive framework. |

## Failure and lifetime notes

- Without supervision an abruptly dead producer leaves a live-looking mapping
  and frozen counters. A consumer's infinite ring wait cannot detect process
  death. That was the pre-existing primitive behaviour, not a safe hosted path.
- Each side now holds a **close-only** extra ring view owned by the control
  monitor. It does not read/write PCM or advance sequence counters. On EOF or
  heartbeat failure it stores closed and wakes both directions independently of
  the blocked worker. Error text is retained even when EOF follows it.
- Default policy: 100 ms probes, 5 s Hello/Pong deadline; callers may supply a
  different policy. Explicit PCM read deadlines produce TimedOut, never Ok(0).
  Shutdown and seek also wake a producer blocked behind a full ring.
- A codec hung inside its native read while its monitor still answers Pong is
  not proven dead by heartbeat. Callers can use finite read deadlines and the
  process owner's terminate/Drop to kill/reap it. Retry/restart is engine policy.
- Generic blocking stream readers/writers cannot be cancelled by dropping a
  Rust thread. Their owner closes pipes by killing/reaping the child; no blocked
  I/O thread is joined on the player worker. Monitor shutdown is separately
  bounded by its 20 ms maximum polling slice.

## Tests and VM handoff

All tests below were **compiled by Clippy, not executed**. Existing four stub
unit tests remain. Added eleven default headless tests plus two ignored
two-process tests (Windows counts):

| Test coverage | File:line | Processes |
|---|---|---|
| 4099-frame bit-exact stub transfer; capacity 3, oversized blocks, wraps, zero drops, stats, drained/repeated EOS | MH `tests/pcm_transfer.rs:177` | One process, worker/control threads |
| Stereo→mono/rate change at frame 513; drain old generation, exactly-once samples | MH `tests/pcm_transfer.rs:202` | One process |
| Seek interrupts full capacity-1 ring and discards old prefetch | MH `tests/pcm_transfer.rs:236` | One process |
| Explicit bounded empty-ring wait is not EOS; retry produces PCM | MH `tests/pcm_transfer.rs:247` | One process |
| Resident decode error is diagnostic, never EOS | MH `tests/pcm_transfer.rs:261` | One process |
| Abrupt control EOF without producer.close wakes infinite read (disconnect surrogate, not a real process crash) | MH `tests/pcm_transfer.rs:282` | One process |
| Missing Pong on still-open stream closes ring and wakes infinite read under watchdog | MH `tests/pcm_transfer.rs:345` | One process |
| Old Hello rejected before decoder open | MH `tests/pcm_transfer.rs:383` | One process |
| Real named mapping/events transfer + drain; format/direction/generation checks; invalid size/capacity/spec refusal | MH `tests/pcm_named.rs:19,44,73` | One Windows process |
| Real named PCM across processes + seek, cap 3, exact sample checks | MH `tests/pcm_process.rs:24` | **Two; ignored, VM only** |
| Resident process exits mid-stream without close; infinite player read must fail within watchdog | MH `tests/pcm_process.rs:58` | **Two; ignored, VM only** |

For the two-process cases the VM runner must additionally build/stage the
`decoder_fixture` example (`cargo build --offline -p rustamp-mediahost --example
decoder_fixture`) and set **guest** `RUSTAMP_DECODER_FIXTURE` to its absolute
executable path. Run the ignored `pcm_process` cases through the existing VM
test runner. This example is a deterministic fixture, not a real codec resident
or a package/install deliverable. Spawn always uses CREATE_NO_WINDOW. A real
codec-file bit-identity comparison, engine tests through a core adapter and the
existing ring-cross suite still require the later resident/engine/IPC lanes and
VM evidence; none is claimed here.

## Deliberately left

- Real decoder residents (including sidplayfp/Symphonia) and their own licenses,
  codec/file/URL integration and reference-file comparisons.
- The GPL engine's core Decoder adapter, event publication, registration,
  restart/retry policy and packaging. No audio/app/UI/IPC/core/proto code touched.
- DSP chain residents, two-ring processing, latency/reset/reconfigure and classic
  32-bit plugin bridging. EQ/resampling/limiting stay in process.
- RFC deferrals: non-UTF-8 path wire extension, URL credentials/auth, late joining,
  multiplexing, access-control changes for sections/events, M6 non-Windows wake
  and mapping implementations, buffering/resampling/latency engine policy.
- Literal host-side drain-before-EOS ordering, as qualified in the audit table.

## Files and line counts

Counts include blank lines (not diffstat); `Cargo.toml` and Cargo.lock unchanged.

| File under MH | Final lines | Change |
|---|---:|---|
| `src/control.rs` | 305 | new |
| `src/decoder.rs` | 670 | new |
| `src/pcm.rs` | 167 | new |
| `src/lib.rs` | 209 | modified |
| `src/bin/dechost.rs` | 23 | modified |
| `src/bin/dsphost.rs` | 45 | modified |
| `examples/decoder_fixture.rs` | 23 | new |
| `tests/pcm_transfer.rs` | 396 | new |
| `tests/pcm_named.rs` | 95 | new |
| `tests/pcm_process.rs` | 87 | new |
| `tests/support/resident.rs` | 94 | new |

This task card is also new. Only these owned source/doc paths were changed.

## Compile-only gates and provenance

- `cargo fmt --all -- --check`: **exit 0** (final pass).
- `cargo clippy --offline --workspace --all-targets --all-features -- -D warnings`:
  **exit 0**, includes default/ignored integration tests and fixture example.
- `cargo build --offline --workspace`: **exit 0**, 47.75 s on the first final pass.
- Initial formatting/Clippy findings in new files were fixed; no foreign-lane
  files were modified to make a gate pass.
- **No cargo test (including --no-run), test binary, example binary, player,
  helper, reference binary, VM job, window or audio playback was run. No commit,
  git write, download, dependency addition or install.** Only allowed compile/
  formatting checks and read-only inspection commands were used.
- No Winamp source, SDK, disassembly or forbidden-tree contents were read. No
  third-party code copied. Worktree-local `git status --short` is the final
  boundary receipt; it cannot independently attest to another tree, which this
  lane was forbidden to inspect.
