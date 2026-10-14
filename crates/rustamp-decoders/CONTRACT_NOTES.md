# Contract notes (rustamp-decoders)

Behaviour the core contract leaves open, and what the symphonia decoder does:

- **Mid-stream format change.** `Decoder::spec` is fixed, so: a channel-count
  change is remapped to the opened count (mono duplicated, extras dropped,
  missing zero-filled); a sample-rate change or container reset (chained Ogg)
  ends the stream (`read` returns `Ok(0)`).
- **Seek.** `SeekMode::Accurate` plus a 4096-frame pre-roll that is decoded and
  discarded, because MP3/Vorbis/AAC need previous blocks for exact output.
  Returns the requested position clamped to the duration (it is sample exact).
- **Corrupt data.** Packets that fail to decode are skipped
  (`SymphoniaDecoder::skipped_packets`); 512 consecutive failures raise
  `Error::Decode`. FLAC/MP3 readers resynchronise internally and do not count.
- **Bitrate** is the file average (file size / duration), not instantaneous.
- **Duration** is `None` when the container gives no frame count (e.g. MP3
  without a Xing/Info header).

## Proposals

All three were accepted by RFC 0002 (`docs/rfc/0002-audio-contracts.md`); item 2
is in effect here.

1. `Decoder` exposes a decode-error/skip counter (`fn skipped_packets(&self) -> u64`
   with default 0) so the engine can show or log damage without downcasting.
   *Accepted in core (RFC 0002, C1): the trait method exists. This crate's count
   is still the inherent `SymphoniaDecoder::skipped_packets` and is not wired to
   the trait method yet, so a `dyn Decoder` call returns the default 0
   (follow-up).*
2. `ReadSeek` requires `Send + Sync` (RFC 0002, C2), which is what symphonia
   needs, so the reader is held directly and no `Mutex` wrapper is required.
   *Done (RFC 0002, C2): `ReaderSource` holds the `Box<dyn ReadSeek>` itself.*
3. A way to report "format changed mid-stream" (e.g. `Error::FormatChanged`
   or an `Event`) would let the engine react to new parameters.
   *Accepted in core (RFC 0002, C3/C4) as `Error::FormatChanged`; this crate
   still ends the stream on a sample-rate change, so implementing it here is a
   follow-up.*
