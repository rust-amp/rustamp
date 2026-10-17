# RFC 0006: Out-of-process decoder and DSP hosting (draft)

Status: **Accepted** on 2026-10-15 by the project owner. Implementation waits
for RFC 0003 Q1 (lossless delivery) and HANDOFF-3 (non-UTF-8 paths).
Owner: audio/engine lane + licensing. Implementation (later card): `rustamp-ipc`
(new `Message` variants), `rustamp-proto` (wire types), one new crate
`rustamp-mediahost` (binaries `rustamp-dechost`, `rustamp-dsphost`), engine-side
client in `rustamp-audio`.
Depends on: RFC 0001 (core contracts), RFC 0002 (decoder/audio surface), RFC 0003
(shared-memory rings), RFC 0004 (licensing split), open point HANDOFF-16
(sidplayfp), HANDOFF-3 (Unicode paths over IPC), RFC 0003 Q1 (blocking consumer).

## 1. Motivation

Two kinds of audio code cannot live in the player's processes:

- **Copyleft decoders.** `sidplayfp` (GPL-2.0+) is the planned SID decoder
  (HANDOFF-16). Linking third-party GPL code into a crate also removes that
  crate from the §7 plugin exception (only the copyright holder may grant it;
  this is why `rustamp-decoders` is already excluded). A separate helper
  process whose whole binary is GPL keeps the copyleft contained and the
  exception story intact.
- **Copyleft or crash-prone DSP code.** A GPL DSP implementation linked into
  the engine has the same problem, and any native DSP bug today takes down the
  player. Isolation fixes both.

The mechanism already exists in outline: RFC 0003 moves PCM between processes
through shared-memory rings, and `rustamp-ipc` carries a versioned control
channel. This RFC turns that into a concrete hosting contract. It does **not**
change any existing contract; it only adds to it.

## 2. Scope

In scope: the helper processes and their licenses, the control messages, which
rings flow which way, the open/seek/flush/EOS lifecycle, format changes,
latency accounting, crash behaviour, and the `PROTOCOL_VERSION` bump.

Out of scope (deliberately not decided here):

- Bridging classic 32-bit `dsp_` DLLs out of `plughost32`. That is S10/S11
  territory (call order, lifetime, embed windows). This RFC hosts DSP code
  compiled into the helper, behind the existing `DspStage` semantics.
- Which decoders or DSP stages move out of process first (policy for the
  implementation card; `sidplayfp` is the obvious first decoder).
- Resampling, buffering policy and latency compensation inside the engine
  (unchanged from today).
- A blocking consumer / event handle (RFC 0003 Q1). This RFC **depends** on it
  (see §4) but does not design it; Q1 gets its own small RFC first.

## 3. Facts and decisions

| Item | Kind | Source |
|---|---|---|
| `Decoder` reads interleaved `f32`, seeks by `Duration`, reports `AudioSpec`, `duration`, `metadata`, `stream_info`, `skipped_packets`, and `FormatChanged` (RFC 0002 C3) | fact | `rustamp-core::audio` (RFC 0001/0002) |
| `DspStage` processes interleaved `f32` in place; `prepare`/`reset`; `latency_frames` | fact | `rustamp-core::audio` (RFC 0001) |
| PCM rings are SPSC, one-way, fixed `frame_size` at creation, producer drops on overrun, consumer polls (RFC 0003) | fact | RFC 0003 §§5–7 |
| A new format means a new ring (RFC 0003 §12) | fact | RFC 0003 |
| A `PathBuf` that is not valid UTF-8 cannot be encoded over IPC yet (HANDOFF-3) | fact | HANDOFF open point 3 |
| New `Message` variants need a `PROTOCOL_VERSION` bump (precedent) | fact | RFC 0003 §9 |
| Protocol types the helper speaks must live in `rustamp-proto`, never in a GPL-only crate | decision (RFC 0004 invariant) | RFC 0004 |
| The helper binary is GPL-3.0-or-later **without** the §7 exception | decision | this RFC (§8) |

## 4. The lossless-delivery requirement (and why Q1 comes first)

RFC 0003 rings drop on overrun by design: the right rule for live taps (vis,
output mirrors). A decoder feed and a DSP round-trip are **not** live taps —
a dropped frame is an audible gap. Polling plus drops cannot carry them.

So the hosting contract requires what RFC 0003 Q1 asks for: a way for the
consumer to keep up without loss (an event/semaphore the producer can wait on,
or a credit scheme with the same guarantee). This RFC freezes the message
shapes and the lifecycle now, but the implementation card is blocked until Q1
is answered. If Q1's answer changes the ring API, this RFC is amended, not
reinterpreted.

Until then the single hard rule stands: **no PCM cross a process boundary
under a drop policy, ever.** Anything lossy stays in-process.

## 5. The decoder host (`rustamp-dechost`)

One helper process per open decoder (one decoder, one control channel, one PCM
ring at a time; a second track means a second process — no multiplexing, no
shared state to get wrong).

**Lifecycle (control channel, engine = M, host = H):**

1. `M → H: DecoderOpen { location, prefetch_frames }` with `location` a
   `MediaLocation` wire form (path as UTF-8 string, or URL). `MediaSource::Reader`
   cannot cross processes: callers that hold a reader keep decoding in-process;
   the host only opens paths and URLs it can reach itself. Non-UTF-8 local paths
   are refused with `DecoderError` until HANDOFF-3 is fixed (recorded dependency,
   not silently mangled).
2. `H → M: DecoderOpened { spec, duration_ms, seekable, metadata }` where `spec`
   is a wire struct `{ sample_rate_hz: u32, channels: u8 }` in `rustamp-proto`
   (not `AudioSpec` itself, which lives in core). Then `H` creates the PCM ring
   (`frame_size = channels * 4`, capacity from `prefetch_frames`) and offers it
   with the normal `RingOffer`/`RingReady` handshake, direction host-to-main.
3. Decode loop: `H` decodes ahead into the ring. `M`'s decode thread consumes
   from it exactly as if it were reading a local `Decoder`. `skipped_packets`
   and current bitrate arrive as `H → M: DecoderStats { skipped_packets,
   bitrate_kbps }` on metadata-worthy changes only, not per block.
4. Seek: `M → H: DecoderSeek { to_ms }`. The host seeks its decoder, **closes
   the ring and offers a fresh one with a bumped generation** (RFC 0003 §8:
   a generation change is how the consumer learns the old data is dead), then
   `H → M: DecoderSeeked { reached_ms }`. No in-band flush marker that a
   consumer could misread as data.
5. Format change (RFC 0002 C3): same as seek — new ring, plus
   `H → M: DecoderFormat { spec }` before the first frame on it.
6. End of stream: host drains the ring, then `H → M: DecoderEos`, then closes.
   `Ok(0)` semantics of `Decoder::read` map to "ring closed and consumed".
7. Any failure: `H → M: DecoderError { message }`, then close. The engine
   surfaces it as an event exactly like a local decode error.

**Engine side.** The implementation wraps the whole conversation in a struct
implementing `rustamp_core::audio::Decoder`, so the engine does not learn a
second code path: `read` pulls from the ring consumer, `seek` runs the
seek handshake, `spec`/`duration`/`metadata` replay the `DecoderOpened`
values, `skipped_packets` replays the last `DecoderStats`. Spawning,
supervision and restart policy live in `rustamp-audio`, not in the contract.

## 6. The DSP host (`rustamp-dsphost`)

One helper process per DSP chain (the chain inside is ordered `Vec`-like, same
as the future in-process chain from S07; order of stages is engine policy).

**Data plane: two PCM rings, strict FIFO, same `frame_size` on both.**
`M → H` carries unprocessed blocks, `H → M` carries them back. The host
processes strictly in order, so block *n* out is block *n* back — sequencing
is implicit in the FIFO order on both rings, and no sequence numbers cross the
boundary. Partial blocks are a programming error on both ends
(`NotFrameAligned`, same as RFC 0003 §7).

**Lifecycle:**

1. `M → H: DspConfigure { spec, stages }` with the same wire spec struct as §5
   and `stages` an ordered list of `{ id: String, params: Vec<u8> }`. `params`
   are opaque to the transport (a stage's own serialization); unknown stage ids
   are refused with `DspError` before any ring exists.
2. Rings are offered (out-ring by main, back-ring by host, normal handshake),
   then `H → M: DspReady { latency_frames }`. The engine adds the reported
   latency to its own compensation exactly as for `DspStage::latency_frames`.
3. Run: engine writes blocks, host processes, engine reads back. Either side
   dropping would corrupt the stream, so §4 applies in full: the DSP path runs
   only under the Q1 lossless guarantee.
4. Reconfigure (new spec or new params): `M → H: DspConfigure` again, which
   implies close-and-reoffer of both rings (same generation-bump rule as §5).
   `M → H: DspReset` clears stage state without touching the rings (seek /
   track change). `H → M: DspError { message }` on failure.

**What this RFC does not move:** the EQ, the resampler and the limiter stay
in-process until S07 says otherwise; classic `dsp_` DLLs stay in `plughost32`
until S10/S11. The first DSP-host resident is whichever GPL or unstable stage
needs isolation first.

## 7. Protocol changes

`PROTOCOL_VERSION` becomes **2**. The additions are additive `Message`
variants, but the version bumps for the same reason as in RFC 0003 §9: a v1
peer would mis-decode them.

```rust
/// M: open a decoder in the helper. Answers: DecoderOpened | DecoderError.
DecoderOpen {
    location: MediaLocationWire, // path (UTF-8) or URL; see §5
    prefetch_frames: u32,         // ring capacity hint
},
/// H: the decoder is open; a RingOffer for its PCM follows.
DecoderOpened {
    sample_rate_hz: u32,
    channels: u8,
    duration_ms: Option<u64>,
    seekable: bool,
    metadata: TrackMetadata,     // already in rustamp-proto
},
/// M: seek the open decoder. Answer: DecoderSeeked (after the new RingOffer).
DecoderSeek { to_ms: u64 },
/// H: seek done, `reached_ms` is what the decoder reports.
DecoderSeeked { reached_ms: u64 },
/// H: the stream changed format; a new ring follows. (RFC 0002 C3)
DecoderFormat { sample_rate_hz: u32, channels: u8 },
/// H: damage/bitrate telemetry, on change only.
DecoderStats { skipped_packets: u64, bitrate_kbps: Option<u32> },
/// H: end of stream; the ring is drained, then closed.
DecoderEos,
/// Either: `DecoderOpen`/`DecoderSeek` failed, or the decoder died.
DecoderError { message: String },

/// M: (re)configure the DSP chain. Answers: DspReady | DspError.
DspConfigure {
    sample_rate_hz: u32,
    channels: u8,
    stages: Vec<DspStageDesc>,    // { id: String, params: Vec<u8> }
},
/// H: chain is live; both rings are offered; engine compensates this latency.
DspReady { latency_frames: u32 },
/// M: clear stage state; rings untouched.
DspReset,
/// Either: configure failed, a stage died, or the chain broke.
DspError { message: String },
```

`MediaLocationWire`, `DspStageDesc` and the wire spec fields live in
`rustamp-proto` (new module or alongside `playlist`/`state`; the
implementation card decides). `TrackMetadata` is already there. Nothing in
these variants may reference a GPL-only crate — the RFC 0004 invariant is
load-bearing here: the permissive `rustamp-ipc` must stay GPL-free, and these
variants are part of it.

`WinampIpc`, `LoadPlugin` and friends are untouched: classic plugins keep
flowing through `plughost32` exactly as before.

## 8. Processes, licenses, crash behaviour

**Erratum (2026-10-16, owner decision).** This section originally licensed the
whole helper crate GPL-3.0-or-later without the §7 exception. The owner split it:
**`rustamp-mediahost` is only the protocol framework** (the message loop and
supervision) and links no decoder, so it is **MIT OR Apache-2.0** and belongs to
the permissive set. **Each resident gets its own crate and binary**
(`rustamp-mediahost-<resident>`, e.g. `rustamp-mediahost-sidplayfp`), and that
crate carries **the resident's license**: a sidplayfp host is GPL because
sidplayfp is. **License correction (2026-10-17):** Symphonia 0.5.5 declares
**MPL-2.0**, a weak-copyleft license, not a permissive license. A future
Symphonia resident must preserve those obligations; the permissive framework
must not acquire it as a normal dependency. The
copyleft therefore sits in the resident crate rather than in the framework, and
the permissive closure of the framework stays GPL-free (it depends only on
`rustamp-ipc`, `rustamp-proto` and `thiserror`).

Consequences of the erratum:

- The **permissive set is five crates**, not four: `rustamp-proto`,
  `rustamp-plugin-abi`, `rustamp-ipc`, `rustamp-plughost` and
  `rustamp-mediahost`. The RFC 0004 acceptance check covers all five.
- A **resident crate carries no §7 exception**: only the copyright holder of the
  covered code may grant one, and a resident crate links someone else's. The same
  rule already excludes `rustamp-decoders` and `rustamp-skin`.
- Nothing else in this section changes: the helper is still spawned, supervised
  and killed by the engine, talks only over the documented protocol plus shared
  memory, and its death fails the track, never the player.

- Dependency direction is one-way: the helper depends on `rustamp-ipc` +
  `rustamp-proto` (permissive) plus whatever decoder/DSP code it hosts. No
  permissive crate ever depends on the helper. The player talks to it only over
  the documented protocol plus shared memory — the same arm's-length relation
  as with `plughost32`, which is what keeps the combination from pulling
  copyleft into the player.
- Supervision: the engine spawns the helper, runs `Hello`/`Ping` liveness on
  the control channel, and kills and reaps it on close. A dead helper looks
  like a stalled peer at the ring layer (RFC 0003 §12: crash detection is the
  control channel's job); the engine turns a missed `Pong` or an unexpected
  close into `DecoderError`/`DspError` semantics and fails the track, never the
  player. Restart policy (retry once, then fail the track) is engine policy,
  not contract.
- Install/packaging: the helpers ship next to `rustamp.exe`; if a helper is
  missing, its features degrade to "format unsupported" / "stage unavailable",
  never to a crash. (`cargo xtask package` learns the new binaries when it
  stops being a stub.)

## 9. What is deliberately not decided here

- The Q1 mechanism itself (separate RFC, blocks implementation).
- Non-UTF-8 path transport (HANDOFF-3; refused for now, see §5).
- Whether `DecoderOpen` needs auth/credentials for URLs (today: no; the host
  inherits the user's network context like the player would).
- A late-joiner consumer (RFC 0003 Q4) for the back-ring; the engine attaches
  before the first block, so it is not needed.
- Multi-tenancy (one host, many decoders): deliberately one decoder per
  process until someone measures the spawn cost as a problem.

## 10. Acceptance (for the implementation card)

1. All RFC 0006 message shapes exist in `rustamp-proto`/`rustamp-ipc` with
   layout/round-trip tests, and `PROTOCOL_VERSION` is 2 with a clean
   `Hello`-rejection test against v1.
2. `rustamp-dechost` decodes a reference file (the card names it) through the
   ring bit-identically to the same decoder in-process; seek lands within the
   decoder's coarse tolerance; EOS, format-change (where the codec allows a
   test), error and kill-the-host paths all behave per §§5/8.
3. `rustamp-dsphost` round-trips a passthrough chain sample-identically and a
   gain stage within `1e-6`; reported latency matches measured delay;
   reconfigure and reset behave per §6.
4. The engine-side `Decoder` wrapper passes the existing engine tests
   unchanged (no second code path in the engine).
5. The permissive-closure proof re-runs clean: no GPL crate in the
   dependencies of `rustamp-proto`, `rustamp-plugin-abi`, `rustamp-ipc` or
   `rustamp-plughost`.
6. `cargo xtask ring-cross` still passes (unchanged rings), full gate green.
