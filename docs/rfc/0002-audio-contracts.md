# RFC 0002: Decoder and reader contracts (M1, audio lane)

Status: **Accepted** on 2026-10-14 by the integration lead (all four changes
approved; C1–C4 are in effect).
Owner: Integration Lead. Written by: task card
[`docs/tasks/A1-audio-engine.md`](../tasks/A1-audio-engine.md).
Bundles exactly the three changes that card lists, plus one consequential
addition (C4 below) that is needed to surface the third change to the UI.

Scope: `rustamp-core` only. Nothing here changes `rustamp-ipc` or
`rustamp-plugin-abi`. The changes are additive except for `ReadSeek`, which
narrows a bound (see C2 for why nothing in the tree breaks).

## Why

Lane A (audio engine) has to (a) show damage in a stream without downcasting a
decoder, (b) build a byte stream over a reader that symphonia requires to be
`Sync`, and (c) react when a stream changes its format in the middle. The
decoders lane already documented the same three gaps in
`crates/rustamp-decoders/CONTRACT_NOTES.md`; this RFC freezes their shape.

## C1 — `Decoder::skipped_packets() -> u64`, defaulting to `0`

```rust
fn skipped_packets(&self) -> u64 { 0 }
```

A provided method on `Decoder`; existing implementations keep working
unchanged. It is a damage counter: how many packets/units the decoder threw
away because they could not be decoded since the stream was opened (not reset
by `seek`, matching the current `SymphoniaDecoder` behaviour).

Why a trait method and not a concrete method: the engine holds
`Box<dyn Decoder>` and must not downcast. Why not on `StreamInfo`: it is a
monotonic counter, not a snapshot field, and `StreamInfo` is a `Copy` wire
type.

Engine use: read once when a track ends, keep it in the engine snapshot and
publish it with the track's report. It is deliberately **not** an automatic
`Event::Error` — a file with one damaged block must not look like a failure.

## C2 — `ReadSeek` requires `Send + Sync`

```rust
pub trait ReadSeek: Read + Seek + Send + Sync {}
impl<T: Read + Seek + Send + Sync> ReadSeek for T {}
```

Today `MediaSource::Reader` stores a `Box<dyn ReadSeek>` with only `Send`.
Symphonia's `MediaSource` is `Send + Sync`, so the decoder crate wraps the
reader in a `Mutex` (`ReaderSource`). With `Sync` in the bound, that wrapper
becomes unnecessary.

Consequences:

- `MediaSource` becomes `Send + Sync` too (it contains the box). The engine
  moves sources to the decode thread; `Send` was enough, `Sync` costs nothing.
- A caller that wants to pass a non-`Sync` reader must wrap it itself. No
  public API changes shape.
- **Follow-up for the `rustamp-decoders` lane** (not part of this card, that
  crate is not owned by lane A): drop the `Mutex` in
  `symphonia_dec::ReaderSource` and the `Send`-only wording in
  `CONTRACT_NOTES.md`. The current wrapper keeps compiling and is still
  correct, it is just redundant. Every `MediaSource::Reader` in the tree
  (`rustamp-core/src/mock.rs`, `rustamp-decoders/tests/decode.rs`) already
  passes a `Cursor`/`Vec`, which is `Send + Sync`.

## C3 — `Error::FormatChanged` as the mid-stream format-change signal

Chosen shape: a new error variant, **not** a new `Event` and not a silent
`spec()` change.

```rust
/// The stream changed format mid-stream (chained Ogg, an MPEG layer change,
/// a stream whose parameters were only learned after the first packets).
FormatChanged {
    /// Format before the change.
    from: AudioSpec,
    /// Format after the change, now also reported by `Decoder::spec()`.
    to: AudioSpec,
},
```

Semantics (this is the part the decoders lane must implement):

1. The decoder detects that the parameters of the stream changed. It switches
   its internal state to the new format and returns
   `Err(Error::FormatChanged { from, to })` from `read`.
2. The failed `read` reports **no** frames (`out` is untouched).
3. From then on `spec()`, `stream_info()` and subsequent `read` calls use the
   new format. The same call site can be retried; the stream continues.
4. A decoder that cannot continue (for example because it cannot resynchronise
   the container) returns `Ok(0)` (end of stream) as it does today. It must not
   use `FormatChanged` for that case.

Why an error and not an `Event`:

- A `Decoder` has no `EventBus` and no thread of its own; the decoder cannot
  publish. Making it an `Event` would mean inventing an event sink inside the
  `Decoder` trait, which is a much bigger contract change.
- Only the engine can react: it owns the DSP chain, the resampler/remixer to
  the output format and the `PlayerState`. So the decoder must *tell* the
  engine, and `read -> Result<usize>` is the only channel it has.
- `Result` already distinguishes "no frames but fine" (`Ok(0)` = EOF) from
  "failed but recoverable", which is exactly the situation.
- The engine maps it to an `Event` itself (see C4), so the UI sees an event
  either way.

Why not "the caller just checks `spec()` every block": that costs a virtual
call and a spec comparison on the decode path every block for a case that
happens once. The error is raised exactly once per change.

Engine reaction (decision, documented in `rustamp-audio`): publish
`Event::Error(ErrorInfo { kind: ErrorKind::FormatChanged, .. })`, then
`Event::StreamInfoChanged(Some(new_info))`; re-`prepare` the DSP chain for the
new spec, reset the format converter, and continue the same decoder. The
engine's own output format does not change (the converter remaps engine format
to device format), so the render callback and the ring are untouched.

Doc-only consequence: the `Decoder::spec` doc comment ("Fixed for the lifetime
of the decoder") gets the exception for `FormatChanged` added.

## C4 — `ErrorKind::FormatChanged`

`Event::Error` carries `ErrorInfo { kind: ErrorKind, message: String }`, and
`ErrorKind` has no variant that fits. Add one, appended at the end so existing
serialized indices do not move:

```rust
pub enum ErrorKind { Io, UnsupportedFormat, Decode, Output, Other, FormatChanged }
```

This is the only reason C3 is visible to UI/platform/plugin lanes; without it
the event would be indistinguishable from a decode error.

**`crates/rustamp-core/src/event.rs` is deliberately not touched.** The three
existing events are enough: `Event::Error` + `ErrorKind::FormatChanged` carries
the signal, `Event::StreamInfoChanged` carries the new technical data, and
`Event::TrackChanged` is unaffected (the entry did not change). Adding a
`Event::StreamFormatChanged` would duplicate what these two already say.

## Deliberately left out

- **A `Decoder::position`/`consumed_frames` accessor.** The engine's position
  comes from the output clock (`OutputDevice::played_frames`), so a second
  clock on the decode side would be a second source of truth. Not needed.
- **Reset semantics for `skipped_packets` on `seek`.** Left as the decoder
  chooses (current symphonia behaviour: never reset). If a spec later needs
  per-seek counts, that is a new RFC.
- **Chained streams as separate tracks.** `FormatChanged` keeps one decoder
  alive; a decoder that must restart the container reports `Ok(0)` instead and
  the engine ends the track. Splitting one file into several entries is a
  playlist concern and needs S06/S14, not this RFC.
- **`OutputDevice::played_frames` reset on `flush`.** RFC 0001 says it is *not*
  reset, and A1 relies on that: the engine keeps its own
  `(clock_origin, timeline_origin)` pair per seek instead of asking the device
  to rewind its counter. No change needed.
- **`OutputDevice` access for tests.** The A1 tests need to drive a virtual
  device's clock. This is solved in `rustamp-output` with a
  `NullOutput::pair()`/`NullClock` split, not in `rustamp-core`. The core
  `mock::NullOutput` stays as it is (RFC 0001 mocks are not extended here). 
- **Removing `rustamp-core::mock::NullOutput`** even though `rustamp-output`
  now has a better one. Removing a mock breaks other lanes' tests; it is a
  cleanup for later.

## Compatibility

- `rustamp-plugin-abi` and `rustamp-ipc` are untouched; the i686 checks are
  unaffected.
- `rustamp-core` keeps its `mock` feature contract.
- Existing code compiles unchanged: C1 and C4 are additive, C2 only narrows a
  public bound that all current callers already satisfy, C3 adds a variant to a
  non-exhaustive-by-convention error enum that no caller matches exhaustively
  except the engine (which handles it).
