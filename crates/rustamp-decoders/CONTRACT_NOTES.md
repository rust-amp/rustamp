# Contract notes (rustamp-decoders)

Behaviour the core contract leaves open, and what the decoders do.

## Symphonia decoder

- **Mid-stream format change.** A channel-count change is remapped to the opened
  count (mono duplicated, extras dropped, missing zero-filled). A **sample-rate**
  change is reported once as `Error::FormatChanged { from, to }` (RFC 0002, C3):
  the failing `read` writes no frames, `spec()` reports the new rate from that
  call on, and the next `read` continues in it. A container reset (chained Ogg)
  cannot be continued, so the stream ends (`read` returns `Ok(0)`) as C3.4
  requires.
- **Truncation.** A file that was cut short is a decode failure, not an end of
  stream. The oracle is the frame count the container states: when the stream
  ends before that count was delivered, `read` fails with
  `Error::Decode("truncated stream: ...")` in a call that writes no frames, so
  the frames the file does hold are still delivered. A file that holds
  everything its container declares ends with `Ok(0)` as before. The check
  applies to the containers that state their payload exactly - RIFF/WAVE, whose
  `data` chunk states the number of samples - and to no other (see the named
  limits below).
- **Seek.** `SeekMode::Accurate` plus a 4096-frame pre-roll that is decoded and
  discarded, because MP3/Vorbis/AAC need previous blocks for exact output.
  Returns the requested position clamped to the duration (it is sample exact).
  A stream with no known duration has nothing to clamp to, so the target is
  bounded by `MAX_SEEK_FRAMES` (2^53) instead of overflowing the frame count.
  After a seek the truncation check counts from the new position, so a seek
  cannot turn a complete file into a truncated one.
- **Empty input.** A source of zero bytes is rejected before probing with
  `Error::Decode("empty source: 0 bytes to decode")`, not with the probe's "no
  suitable format reader found" (which named the wrong problem).
- **Bounded read loop.** Every iteration of `fill_pending` produces samples,
  counts a skipped packet, or ends the stream. The two arms that do none of
  those (a foreign track id, a buffer that decodes to zero frames) are counted
  and fail with `Error::Decode` after `MAX_NO_PROGRESS_PACKETS` (65536) in a
  row, so a source that keeps handing out unusable packets cannot spin forever.
- **Corrupt data.** Packets that fail to decode are skipped
  (`SymphoniaDecoder::skipped_packets`); 512 consecutive failures raise
  `Error::Decode`. FLAC/MP3 readers resynchronise internally and do not count.
- **Failure kind by phase.** `open_err` maps a pre-open `Unsupported` to
  `Error::UnsupportedFormat` ("not my format"); `stream_err` maps a post-open
  `Unsupported` to `Error::Decode` while keeping the message, because a
  container that was accepted is ours, damaged — not foreign.
- **Bitrate** is the file average (file size / duration), not instantaneous.
- **Duration** is `None` when the container gives no frame count (e.g. MP3
  without a Xing/Info header, or any source whose length cannot be read).

## Wave decoder (`wave_dec.rs`)

The RIFF/WAVE PCM decoder is the second factory (see below). What it does where
the core contract leaves room:

- **Claimed formats.** `WAVE_FORMAT_PCM` (8-bit unsigned, 16/24/32-bit signed
  little-endian), `WAVE_FORMAT_IEEE_FLOAT` (32/64-bit) and
  `WAVE_FORMAT_EXTENSIBLE` whose KSDATAFORMAT sub-format GUID resolves to PCM or
  IEEE float. Anything else - `RIFX` (big-endian), `RF64`/`BW64`, MS/IMA ADPCM,
  µ-law/A-law, and any unknown tag - is refused with `Error::UnsupportedFormat`,
  both at probe time (no signature match) and at open time (the tag is checked).
- **Valid bits below the container.** `WAVE_FORMAT_EXTENSIBLE` may carry a
  `wValidBitsPerSample` below the container width (e.g. 20 in 24). The sample is
  left-justified in the container, so the decoder shifts the unused low bits off
  before scaling. A float container with a smaller valid-bit count is refused
  (there is no such encoding).
- **Chunk walking.** `fmt ` must precede `data`; unknown chunks (`LIST`, `fact`,
  `bext`, ...) are skipped with the RIFF pad byte that follows an odd size. The
  RIFF size field itself is not trusted - the chunk walk is authoritative - so
  trailing bytes after `data` are ignored rather than an error.
- **`data` size.** The declared size is the claim the file has to keep: a source
  that ends before the declared length was cut, and the decoder says so - the
  frames the file does hold are delivered, and the next `read` that writes no
  frames fails with `Error::Decode("truncated WAV data: ...")`. The duration is
  still computed from the bytes that are really there, so it never claims more
  than the file has. The streaming marker `0xFFFFFFFF` means "to EOF" in a
  source that cannot report its length; there is no claim to keep, so that
  stream ends cleanly and its duration is `None`.
- **Truncated tail.** A `data` size that is not a whole number of frames leaves a
  partial frame. It is dropped and counted in `skipped_packets()`; the stream
  ends cleanly (the file holds everything its header declares).
- **Seek.** Sample exact: a byte offset from the start of the sample data, no
  pre-roll. The target is clamped to the duration when that is known; half a
  frame of slack makes a target derived from `duration_of(n)` land on frame `n`
  rather than the frame before it.
- **Bitrate** is the uncompressed sample format
  (`rate * channels * container_bits`), not a file average.

## Named collapses

Distinctions the decoders still cannot make, named so they are not mistaken for
correct behaviour:

1. **A truncated stream is no longer indistinguishable from a clean end** (both
   decoders). The decoders compare the frames they delivered with the frame
   count the container states and report a shortfall as `Error::Decode`, so a
   cut file fails loudly instead of playing short. The remaining limit is
   where no such oracle exists: the count is only evidence of a cut file when
   the container states it exactly (a RIFF/WAVE `data` chunk does). The MPEG
   audio reader only estimates its frame count when the file carries no
   Xing/LAME header, the AIFF reader counts the `SSND` header into its frame
   count, and the FLAC and MP4 readers resynchronise damaged frames without
   counting them - so a truncated file of those kinds still ends with an end of
   stream. The wave decoder has no such limit: it parses the `data` chunk
   itself and compares the declared length with the bytes that are there.
2. **A damaged file whose *content signature* matches is reported as
   `UnsupportedFormat`** (symphonia). The signature tells the factory to hand the
   file to symphonia; when the probe then rejects the body, nothing is open yet,
   so `open_err` can only say "not my format". Telling "signature matched but the
   container is broken" apart from "no reader at all" needs a third error kind.
   The wave decoder has the same shape: a `RIFF`/`WAVE` header whose `fmt ` chunk
   is unusable is `UnsupportedFormat`.
3. **A container we can read but whose codec we cannot** is folded into the same
   `UnsupportedFormat` (nothing was opened, so it is the closest honest kind).
4. **The registry resolves ties by registration order (RFC 0013).**
   `DecoderRegistry::probe` picks the highest confidence and breaks a tie
   deterministically by registration order (earlier registrations win); it does
   not refuse when two factories claim the same media with identical score.
   Content signatures strictly beat extension hints (`Confidence::CONTENT`
   outranks `Confidence::EXTENSION`), and unclaimed inputs are refused with
   `Error::UnsupportedFormat`.
   This is no longer dead policy: `default_registry` registers two real
   claimants (`symphonia`, then `wave`) that both score `Confidence::CONTENT` on
   a `RIFF`/`WAVE` signature and `Confidence::EXTENSION` on a `.wav`/`.wave`
   name, so a real WAV file takes the tie-break. Symphonia is first on purpose:
   for the media both claim it is the more complete decoder, so the default
   behaviour for WAV is unchanged. The guarantees are pinned by
   `tests/robustness.rs` (fake claimants, the rule itself) and
   `tests/selection.rs` (the two real factories).
5. **Zero channels** (symphonia). The conversion path and `read` use
   `channels.max(1)`, so a hostile buffer cannot reach `chunks_exact(0)`; a
   stream that really carries no audio is reported as
   `Error::Decode("stream contains no audio")`. The wave decoder refuses a
   `fmt ` chunk with zero channels at open time instead.

## Proposals

All three were accepted by RFC 0002 (`docs/rfc/0002-audio-contracts.md`); item 2
and item 3 are in effect here.

1. `Decoder` exposes a decode-error/skip counter (`fn skipped_packets(&self) -> u64`
   with default 0) so the engine can show or log damage without downcasting.
   *Accepted in core (RFC 0002, C1): the trait method exists, and
   `SymphoniaDecoder` overrides it, so a `dyn Decoder` call sees the real count.
   The inherent method stays as a convenience for callers that hold the concrete
   type; both read the same field. `WaveDecoder` overrides it too (its counter is
   a dropped partial frame).*
2. `ReadSeek` requires `Send + Sync` (RFC 0002, C2), which is what symphonia's
   `MediaSource` needs, so the reader is held directly and needs no interior
   synchronisation.
   *Done (RFC 0002, C2): `ReaderSource` holds the `Box<dyn ReadSeek>` itself.
   The `ReadSeek` bound is what carries the guarantee: the wrapper inherits
   `Send + Sync` from the boxed reader, and the crate's tests pin concurrent
   shared access and the move to another thread.*
3. A way to report "format changed mid-stream" (e.g. `Error::FormatChanged`
   or an `Event`) would let the engine react to new parameters.
   *Accepted in core (RFC 0002, C3/C4) as `Error::FormatChanged` and
   **implemented here** for a sample-rate change: `fill_pending` queues the new
   spec, `read` reports it from a call that writes no frames, and `spec()`
   switches at report time. `rustamp-audio` handles it by reconfiguring and
   continuing (`decode.rs::on_format_changed`).*
