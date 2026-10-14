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

1. `Decoder` could expose a decode-error/skip counter (`fn skipped_packets(&self) -> u64`
   with default 0) so the engine can show or log damage without downcasting.
2. `MediaSource::Reader` requires only `Send`; symphonia needs `Sync`, so the
   reader is wrapped in a `Mutex`. Making `ReadSeek: Send + Sync` would avoid it.
3. A way to report "format changed mid-stream" (e.g. `Error::FormatChanged`
   or an `Event`) would let the engine chain the remainder as a new track.
