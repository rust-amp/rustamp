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
  stream. The oracle is the payload the container states: when the stream
  ends before that payload was delivered, `read` fails with
  `Error::Decode("truncated stream: ...")` in a call that writes no frames, so
  the frames the file does hold are still delivered. A file that holds
  everything its container declares ends with `Ok(0)` as before. The check
  applies to the containers that state their payload exactly:
  - **RIFF/WAVE**: the `data` chunk states the number of samples it holds.
    A damaged sample is still a sample, so a shortfall of frames can only
    mean that the file was cut.
  - **MPEG audio with a Xing/Info header**: the tag states the number of MPEG
    frames *and* the number of bytes the audio occupies (see below). Both
    are needed: a damaged MPEG frame is resynchronised by the reader, so a
    damaged file delivers fewer frames than it declares as well - but it holds
    all of its bytes. A file that is short of frames *and* of bytes was cut; a
    file that is short of frames only is damaged and keeps playing.
  - **AIFF**: the `COMM` chunk states the number of sample frames and the
    `SSND` chunk the number of bytes of sound data (see below). The same
    two conditions as for MPEG audio: a file that is short of frames *and* of
    bytes was cut, a file that is short of frames only (a last frame the
    chunk size cut in half) keeps playing.
  - **MP4/M4A**: the media data box states the number of bytes of media data
    the file holds, and the media header together with the sample tables state
    the number of frames and the bytes of the audio itself (see below). The
    claim of the media data box is the one that survives the loss of the movie
    box, so it is the one that names a cut file.
  - **Ogg** states the end of its stream with a **flag**, not with a count: the
    last page of a logical bitstream carries the end-of-stream flag. The
    oracle is that flag (see the Ogg header below).
  FLAC is not on that list, and the reason is worth naming: its `STREAMINFO`
  block states the number of samples exactly, but the format has no field for
  the number of bytes the audio occupies, and a damaged FLAC frame is dropped
  by the reader (its CRC-16 does not match) while the reader resynchronises at
  the next frame. A corrupt file is therefore short of samples exactly like a
  cut one, and it holds all of its bytes - the shortfall proves nothing. A
  truncated FLAC file still ends with an end of stream. The *metadata* of a
  FLAC file is a different matter - see below.
- **The FLAC metadata** (`flac_header.rs`). A FLAC file is the `fLaC` marker,
  a chain of metadata blocks and then the audio frames. Every block starts
  with a header of four bytes: one type byte (bit 7 marks the last block of
  the chain, bits 0 to 6 are the type) and a 24 bit big endian length of the
  payload. The chain is read from the file itself, because a block can be
  larger than any header window, and it says two things:
  - The chain holds the **`STREAMINFO` block** (type 0), the one that states
    the audio parameters. A file without it has nothing to decode: the reader
    cannot recognise a frame without the parameters and runs into the end of
    the file, which it reports as an end of stream - for a file that is not
    at its end. The decoder refuses such a file when it is opened and names
    the missing block: `Error::Decode("the FLAC file has no STREAMINFO
    block")`.
  - The chain states the **length of every block**, so a file that ends
    inside one was cut. That is a truncation and not an end of stream, and
    the decoder names it when the file is opened:
    `Error::Decode("truncated stream: the FLAC file ends inside its metadata
    blocks (...)")`. The message states where the cut is and what the cut
    block claims.
  What the chain does **not** state is the length of the audio, so the audio
  behind it keeps the honest end of stream (see the named collapse below).
- **The MPEG audio header** (`mpeg_header.rs`). An MPEG audio file may carry a
  **Xing** (variable bitrate) or **Info** (constant bitrate) tag in its first
  frame, behind the side information. The tag states the number of MPEG frames
  (the frame that carries the tag is not counted: it holds the tag, not
  audio) and the number of bytes the audio occupies. Both are read from the
  file itself, and the frame count is turned into a sample count the way the
  container reader does it: frames times the samples per frame, less the
  encoder delay and padding of the LAME extension. The reader's own count has
  to agree with that: a reader that fell back to its bitrate estimate states
  another number, and an estimate is not evidence of a cut file - then there is
  no oracle and the file ends as it always did.
  The byte count is not an exact length (some encoders count the tag frame
  with the audio, some do not), so it is only used as evidence that bytes are
  missing: a file that holds fewer bytes than the tag declares was cut.
- **The AIFF header** (`aiff_header.rs`). An AIFF file states its payload in
  two chunks: the `COMM` chunk names the number of sample frames, the `SSND`
  chunk the number of bytes of sound data. The container reader counts the
  whole `SSND` chunk as audio, so the frame count it reports is the chunk size
  divided by the block align - up to `8 / block_align` frames more than the
  file holds (four frames for 16 bit mono, one for 24 bit stereo). That is the
  correction: the count the file states (`COMM`) is the one to compare with,
  and the reader's count has to be the one the correction expects - otherwise
  the reader read another chunk and the two counts are about different things.
  The byte count is the `SSND` chunk without its own eight byte header (the
  `offset` and the `blockSize` field), and it is compared with the bytes that
  are really there. The header is read from the file itself, not from a fixed
  window: the chunks before the sound data can be larger than any window.
  A file that states no frame count (`COMM` with 0) falls back to the frames
  its sound data holds.
  The **duration** still comes from the reader's count, so it is up to
  `8 / block_align` frames too long (0.09 ms for 16 bit mono at 44.1 kHz).
- **The MP4 header** (`mp4_header.rs`). An MP4 file states its payload in its
  boxes, and it states it in two places:
  - The **media data box** (`mdat`) states how many bytes of media data the
    file holds. A file that is shorter than that was cut. This claim is the
    one that survives the loss of the movie box, so it is the one that names
    a cut file: a file whose movie box sits behind the media data (the layout
    ffmpeg writes) has no movie box left once it is cut, but the media data
    box is still there with its claim.
  - The **media header** (`mdhd`) states the duration of a track in units of
    the media timescale, and the **sample tables** (`stsz`, `stsc`, `stco` /
    `co64`) state where the samples of the track are and how many bytes they
    occupy. Together they are the frame count and the byte count of the
    end-of-stream rule, the same two conditions as for MPEG audio and AIFF.
  Two things have to line up for the end-of-stream rule to apply. The
  duration is a number of *frames* only when the media timescale is the sample
  rate; otherwise it is a number of timescale units, which is not what the
  reader delivers. And the reader's own count has to be the one the media
  header states, otherwise the two counts are about different tracks - a file
  with two sound tracks that state the same number of frames is therefore no
  oracle at all.
  The **where** the truncation is reported is different from the other
  containers as well: the MP4 container reader scans the whole file before it
  plays anything, so a cut MP4 file is refused when it is *opened*, not at
  the end of the stream. The decoder names the cut
  (`Error::Decode("truncated stream: the file holds N of the M bytes its
  media data box declares")`) instead of letting the bare "end of stream" of
  the reader stand. A file whose boxes are all there but whose sample table
  points behind the end of the file is the case the end-of-stream rule
  catches: it opens, and the stream ends before the media header's count is
  reached.
  The **sample tables** are read from the file itself, and the length of the
  file bounds the read: a table that claims more samples than the file holds
  is not there, so the file states nothing about its samples and is no
  oracle. The bound keeps the probe of a hostile file from spending the
  memory of the player: a box that claims four billion samples would ask for
  four gigabytes otherwise.
- **The Ogg header** (`ogg_header.rs`). An Ogg file is a sequence of **pages**,
  and it states the end of its stream with a **flag** instead of with a count:
  the last page of a logical bitstream carries the end-of-stream flag (bit 2 of
  the header type byte), so an intact file ends with a page that has it. The
  pages are walked - the capture pattern, the stream structure version, the
  flags, the granule position, the serial number, the sequence number, the
  segment table and the checksum - and the walk yields one of three verdicts:
  - **The file ends at the end of a page.** The last page is intact (its
    checksum matches) and carries the end-of-stream flag: the stream ends
    where the file ends, and the file ends with a plain end of stream. The
    last page is intact and does *not* carry the flag: the bitstream itself
    says that more is to come, so the file was cut and the decoder says so
    (`Error::Decode("truncated stream: the Ogg file ends with a page that has
    no end-of-stream flag")`).
  - **The file ends inside a page.** An intact file always ends at the
    boundary of a page, so a file that ends inside one was cut
    (`Error::Decode("truncated stream: the Ogg file ends inside the page at
    offset N")`).
  - **The pages show damage.** A page whose checksum does not match, or a gap
    in the page sequence numbers of a logical bitstream, means that pages are
    missing or broken. That is damage, not a cut: the page that would say
    "the stream ends here" cannot be read, so nothing is claimed and the file
    keeps playing as it always did.
  The walk reads every page and checks its checksum, so it reads the whole
  file once when the decoder is opened - like the MP4 container reader, which
  scans the whole file before it plays anything. A file whose length is
  unknown (a reader that cannot report its end) is walked the same way: the
  walk stops at the end of the file itself.
  The **granule position** of the last page is the second half of the check.
  The container reader reports a frame count only when it has seen an
  end-of-stream page, and that count is derived from the granule position of
  that page (measured on the repository fixture: the last page states 66150
  samples at 44.1 kHz and the reader reports 66150 frames). The count and the
  granule position therefore have to agree; a count that is larger than the
  granule position of the last page means that the reader knows more about the
  stream than the pages do, and then nothing is claimed. (The OggFLAC mapping
  is the one exception: it states the sample count in its `STREAMINFO` block
  instead of in the end-of-stream page.)
  What the granule position cannot do is tell a cut file from a damaged one
  on its own: the granule position of the last page states the samples *up to
  that page*, which is exactly what a cut file holds. It is the flag, not the
  count, that says that more is to come.
- **Seek.** `SeekMode::Accurate` plus a 4096-frame pre-roll that is decoded and
  discarded, because MP3/Vorbis/AAC need previous blocks for exact output.
  Returns the requested position clamped to the duration (it is sample exact).
  A stream with no known duration has nothing to clamp to, so the target is
  bounded by `MAX_SEEK_FRAMES` (2^53) instead of overflowing the frame count.
  After a seek the truncation check counts from the new position, so a seek
  cannot turn a complete file into a truncated one.
- **Matroska.** The container counts its timestamps in the units of its
  timestamp scale (a millisecond by default) and not in frames, so two
  conversions happen: the frame count the container states is turned into
  frames at the rate of the track, and the timestamps of the packets are
  turned into frames before they are compared with the decoded ones. A seek
  therefore lands on the position the container states, with the resolution of
  the timestamp scale - a millisecond - and not on the sample.
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
- **Duration** is `None` when the container gives no frame count (any source
  whose length cannot be read). Where the container only *estimates* the count
  (an MP3 without a Xing/Info header, an estimate from the bitrate), the
  estimate is used for the duration: good enough for a display, and no
  evidence at all that the file was cut.

## Opus decoder (`opus_dec.rs`)

Symphonia ships no Opus codec: no published version of the crate has an `opus`
feature, and `symphonia-codec-opus` does not exist on crates.io. What symphonia
does have are the mappings: its Ogg demuxer reads the Opus identification header
and states the codec parameters, and its Matroska demuxer hands the
identification header of the track to the codec as the codec private data. So
only the codec itself was missing. This crate brings its own (`opus-rs`, a
pure-Rust port of the reference implementation) and registers it in a codec
registry of its own, next to the codecs symphonia enables.

- **The codec** decodes mono and stereo streams at 48 kHz, the rate Opus always
  runs at. A stream that states another rate, or more than two channels, is
  refused by name when it is opened
  (`Error::UnsupportedFormat("opus: only mono and stereo streams are
  supported")`). A packet the codec cannot decode is skipped and counted like
  any other damaged packet.
- **The pre-skip** is the number of samples at the start of an Opus stream that
  are the encoder's look-ahead and not audio; the identification header states
  it. Symphonia's Ogg reader leaves those samples in the stream: the granule
  position of an Ogg Opus stream counts the pre-skip (RFC 7845), and the
  reader's timestamp arithmetic does not account for that. The codec drops
  them, so a file delivers exactly the samples of its audio - the same 72000
  samples a reference decoder delivers for the repository fixture.
- **The duration of an Ogg file** follows from the same correction: the granule
  position of the last page counts the pre-skip, so the number of audio frames
  is the granule minus the pre-skip. The duration of the fixture is 1.5 s, not
  the 1.5065 s the granule alone would state.
- **The timestamps** of an Opus stream count the pre-skip samples, so they are
  ahead of the audio by exactly that many samples. A seek asks for the
  timestamp of the sample and not the one of the audio before it, and lands on
  the exact sample it asked for.
- **The end of the stream** is the end-of-stream flag of the last Ogg page, as
  for every other Ogg file: the oracle of the Ogg container applies to an Opus
  file unchanged (see the Ogg header above).
- **The Matroska mapping** states the channel count of an Opus track as a
  channel layout and not as a count, so the parameters the container reader
  hands to the codec do not describe the stream. The identification header the
  track carries states the count (RFC 7845, §5.1), and that is the one that is
  used: the parameters are completed from it before the codec is built
  (`opus_dec::complete_parameters`), so an Opus stream in a Matroska file is
  decoded like a stream in an Ogg file. The rate is not touched: the Matroska
  mapping states the rate of the codec (48 kHz, as the mapping requires), and
  a track that states another rate is still refused by name.
- **The duration of a Matroska file** is the duration the container states,
  less the look-ahead. The container states it in the units of its timestamp
  scale - a millisecond by default - and the container reader reports that
  count as the frame count, so it is turned into frames at the rate of the
  track. The repository fixture states 1508 ms for the encoded stream; the
  duration of the audio is 1.5015 s, the difference being the encoder's
  look-ahead and the resolution of the container.
- **The padding at the end of a Matroska file** is not dropped: the Matroska
  reader ignores the `DiscardPadding` of the last block, so the decoder
  delivers the padding as well. The fixture delivers 72648 frames instead of
  the 72000 frames of its audio - 648 samples (13.5 ms) of the encoder's
  flush, which is silent. The Ogg mapping does not have that problem: the
  granule position states the end of the audio exactly.
- **A seek in a Matroska file** lands on the position the container states,
  and the container states the position of a block with a resolution of a
  millisecond: the seek of the fixture lands 53 frames (1.1 ms) after the
  sample it asked for. A seek in an Ogg file is sample exact.


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
   decoders). The decoders compare the frames they delivered with the payload
   the container states and report a shortfall as `Error::Decode`, so a cut
   file fails loudly instead of playing short. The remaining limit is where no
   such oracle exists: the count is only evidence of a cut file when the
   container states it exactly. RIFF/WAVE does (its `data` chunk states the
   number of samples), and so does MPEG audio whose file carries a Xing/Info
   header - there the byte count of the header tells a cut file from a
   damaged one. AIFF does as well: its `COMM` chunk states the number of
   sample frames and its `SSND` chunk the number of bytes of sound data, and
   the two together tell a cut file from a damaged one (the reader counts the
   eight bytes of the `SSND` header as audio, so its own count is corrected
   before the comparison - see above).
   The MPEG audio reader only estimates its frame count when the file carries
   no such header, and the FLAC reader resynchronises damaged frames without
   counting them - so a truncated FLAC file still ends with an end of stream.
   An AIFF file whose length is unknown (a reader that
   cannot report its end) has no byte count and therefore no oracle either.
   The wave decoder has no such limit: it parses the `data`
   chunk itself and compares the declared length with the bytes that are there.
   MP4/M4A states its payload exactly as well - its media data box states the
   bytes of the file, its media header and sample tables state the frames and
   the bytes of the audio - but its container reader scans the whole file
   before it plays anything, so the cut is named when the file is *opened*
   rather than at the end of the stream (see above).
   Ogg is the one container that states its end with a **flag** instead of a
   count: the last page of a logical bitstream carries the end-of-stream flag,
   and a file that does not end with such a page was cut (see above and item
   10 below).
   The FLAC case is item 8 below: it is the one where the container states the
   sample count exactly and still cannot say whether the file was cut.
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
6. **A cut MPEG file is only a loud failure when its header states both
   counts** (symphonia). A Xing/Info tag that states the frame count but not
   the byte count is no oracle: a damaged MPEG file delivers fewer frames
   than it declares too, and without the byte count the two cannot be told
   apart. The same holds when the reader and this crate do not read the same
   header (a first frame with a CRC, for example): the reader's estimate is
   not evidence, so such a file ends as it always did. A header that lies
   about the frame count while the file holds every byte it declares is not
   a cut file either - it ends cleanly, like the damaged file it resembles.
7. **A cut AIFF file is only a loud failure when its two chunks agree with
   the reader** (symphonia). The `COMM` chunk states the frame count and the
   `SSND` chunk the byte count; the reader's own count has to be the one the
   `SSND` chunk implies (its size divided by the block align, the eight bytes
   of its own header included), otherwise the two counts are about different
   chunks and there is no oracle. A file that holds every byte its `SSND`
   chunk declares is not a cut file either, even when its `COMM` chunk claims
   more frames than the file holds - it ends cleanly, like the damaged file it
   resembles. A block-aligned file (a non-zero `offset` or `blockSize` in the
   `SSND` chunk) is one the reader refuses, so it never reaches this rule.
8. **A truncated FLAC file still ends with an end of stream** (symphonia),
   because FLAC states no byte count and the sample count alone cannot tell
   a cut file from a damaged one. The `STREAMINFO` block states the number of
   samples exactly - the reader takes its frame count from that block, so the
   two agree without a correction, unlike MPEG audio where the encoder delay
   and padding have to be subtracted - but the format has no field for the
   number of bytes the audio occupies, so the second condition of the MPEG
   audio rule cannot be built. The sample count cannot take its place: a
   FLAC frame whose CRC-16 does not match is dropped by the reader and the
   reader resynchronises at the next frame, so a corrupt-but-complete file
   delivers fewer samples than it declares while holding all of its bytes
   (measured on the repository fixture: 61542 of 66150 samples). The MD5 in
   the `STREAMINFO` block cannot take its place either - it covers the
   *decoded* samples, so it fails for damage and for a cut alike, and the
   reader only computes it when verification is asked for.
   What the format does offer is a *proven lower bound* on the number of
   audio bytes: the declared minimum frame size times the number of frames
   the declared sample count needs at the declared maximum block size. A file
   below that bound cannot hold the samples it declares, so it was cut. That
   bound is not an oracle, though: it depends on the size of the last frame,
   which depends on the audio (a file that ends in silence has a very small
   last frame and a bound near zero, a file that ends loudly has a bound
   close to its own size), so it fires for some cuts and not for others, and
   a loud failure on such a threshold would be an unstable promise. It is
   therefore not implemented, and the honest end of stream stays.
   The limit covers the **audio** only. The metadata chain in front of it
   states the length of every block, so a file that ends inside the chain is
   named as a cut when it is opened, and a file without a `STREAMINFO` block
   is named as undecodable.
9. **An MP4 file is only a loud failure when its own boxes agree** (symphonia).
   MP4/M4A states its payload exactly, so it has an oracle - but the oracle
   needs the boxes to agree with each other and with the reader:
   - The **media data box** names the cut when the file is shorter than the
     box claims. That claim survives the loss of the movie box, so it is the
     one that names a cut file in either layout.
   - The **end-of-stream rule** (the media header's frame count and the
     sample tables' byte count) needs the media timescale to be the sample
     rate (otherwise the duration is a number of timescale units and not a
     number of frames) and the reader's own count to be the one the media
     header states (otherwise the two counts are about different tracks). A
     file with two sound tracks that state the same number of frames, a
     fragmented file without a movie box, and a file whose length is unknown
     are therefore no oracle: they end as they always did.
   - A file that holds every byte its media data box declares is not a cut
     file, even when its media header claims more frames than the file
     delivers: a damaged AAC or ALAC frame is dropped by the reader, so a
     corrupt-but-complete file is short of frames exactly like a cut one
     (measured on the repository fixtures: 62054 of 66150 frames for the
     damaged ALAC file, 64512 of 67584 for the damaged AAC file) while it
     holds all of its bytes. It ends cleanly, like the damaged file it
     resembles.
10. **An Ogg file is only a loud failure when its last page can be read**
    (symphonia). The end-of-stream flag is an exact oracle: a file whose last
    page is intact and does not carry it was cut, and the bitstream says so
    itself. Two limits follow from the way the format stores the flag:
    - The flag lives in the **page header**, and the header is covered by the
      page checksum. A last page whose checksum does not match cannot be read
      at all, so it cannot say whether the stream ended there. Such a file is
      damage, not a cut: it keeps playing and nothing is claimed.
    - The **segment table** of the last page states the length of its body,
      and it is covered by the checksum as well. A file whose last page has a
      damaged segment table therefore cannot be told from a file that was cut
      inside that page: the walk believes the segment table and names the
      cut. This is the same trust model the other containers use (an AIFF
      `SSND` chunk or an MP4 `mdat` box with a damaged size field gives a
      wrong verdict in the same way), and it is the price of the loud
      failure: a half-downloaded track must not play short as if it were
      complete.
    A file that was cut **before** its end-of-stream page is named in both
    cases (the last page is intact and has no flag, or the file ends inside a
    page), and a file whose pages show damage anywhere - a page with a wrong
    checksum, a gap in the page sequence numbers - is never named.
    A **chained** Ogg file (several logical bitstreams in one file) is not
    judged either: the container reader ends the stream at the chain boundary
    (C3.4), so the end of the file is never reached and no verdict is
    reported.

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
