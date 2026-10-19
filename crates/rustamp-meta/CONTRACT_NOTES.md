# Contract notes (rustamp-meta)

Decisions this crate makes where the frozen contracts leave room, plus the
`rustamp-core` and spec requests that follow from them.

## Rules chosen

- **Merge / precedence.** A file may carry several tags (an MP3 can have
  ID3v2, ID3v1 and APE at once). Tags are consulted in a fixed order: the
  format's *primary* tag type first (`Id3v2` for MP3/AAC/WAV/AIFF,
  `VorbisComments` for FLAC/Ogg/Opus, `Mp4Ilst` for MP4/ALAC, `Ape` for
  APE/MPC/WavPack), then the remaining types in the order ID3v2,
  Vorbis comments, MP4/ILST, APE, ID3v1, RIFF INFO, AIFF text. For every field
  the **first tag that supplies a non-empty value wins**; ID3v2 therefore
  beats ID3v1, and ID3v1 only fills the fields the others leave empty. The
  rule lives in one place (`read::TAG_PRECEDENCE`).
- **"3/9" track and disc numbers.** The number item is read as a string and
  split on `/`, so both the combined form (`TRACKNUMBER="3/9"`, ID3v2 `TRCK`
  before lofty splits it) and the split form (`TRACKNUMBER="3"` +
  `TRACKTOTAL="9"`) work. A bare number yields `track_no` only. Disc numbers
  follow the same rule.
- **Comment.** Read from the comment item, falling back to the description
  item, because some taggers (for example ffmpeg's FLAC muxer) store the
  comment as the Vorbis `DESCRIPTION` field.
- **Encoding / non-UTF-8 bytes.** Parsing uses lofty's default
  `ParsingMode::BestAttempt`; ID3v1 text is decoded as Latin-1 (every byte
  maps to a character) and ID3v2 text uses the encoding declared in the
  frame. `rustamp-meta` never touches raw tag bytes, so it cannot panic on
  them: a malformed string is replaced by an empty string (or the item is
  dropped) and the rest of the file still reads. Text values are trimmed;
  empty strings become `None`.
- **Absent tags.** A media file without tags yields exactly
  `TrackMetadata::default()` (all tag fields `None`, empty `extra`), never an
  error. A file that is not media, an empty file or an unreadable path yields
  an error; nothing panics.
- **Duration and bitrate stay `None`.** `read_tags` sets
  `lofty`'s `read_properties(false)` and never asks for audio properties.
  Reading properties is not decoding, but for some containers (a CBR MP3
  without a Xing header) it means scanning the whole file, and the contract
  says the decoder fills these fields during playback anyway. This keeps the
  read tags-only, cheap (all five fixtures together take ~3 ms) and robust on
  truncated files. If a caller ever needs the cheap properties, it is a small
  additive option; none is exposed yet.
- **`extra`.** A fixed list of well-known items that have no
  `TrackMetadata` field is copied into `extra` under their conventional
  upper-case name (ReplayGain, including the Vorbis-only `R128_*_GAIN`
  loudness variants, ISRC, MusicBrainz IDs, composer, label, ...).
  `DISCTOTAL` is put there too, because there is no `disc_total` field yet.
- **Pictures.** `TrackMetadata` has no field for cover art, so pictures are
  returned by `read_artwork` (the front cover) and `read_artworks` (every
  picture) as `Artwork` values: the role (`PictureKind`), the MIME type, the
  description and the raw bytes. The bytes are exposed verbatim; nothing is
  decoded, resized or validated. See "Artwork" below.
- **Writing.** `write_tags` sets only the fields that are `Some` and leaves
  the rest of the tag untouched, so read → edit → write preserves what the
  caller does not set. A missing primary tag is created. The year is written
  as a recording date (ID3v2 `TDRC`, Vorbis `DATE`, MP4 `©day`). `extra`
  entries are written best-effort (only keys that map to the target format).
  Clearing a field is not supported yet; a tag editor (S06 era) will need an
  explicit "remove" form.
- **Writing is atomic.** The new file is built next to the original (same
  directory, so the rename stays on one volume) and renamed over it. A
  failed or interrupted write therefore leaves the original byte for byte
  as it was; there is no half-written media file. The replacement keeps the
  permissions of the original. A symlinked file is written *through*: the
  link stays a link and the file it points at is replaced.
- **Only the primary tag is written.** A file may carry more than one tag
  (an MP3 with ID3v2 and ID3v1). The write replaces the format's primary
  tag and leaves the other tags byte for byte as they were; the old code
  rewrote all of them.
- **Unknown frames and keys.** "Unknown" means the tag format allows it but
  this crate has no field for it: a `TXXX` frame with a private
  description, a FLAC `CUSTOM_KEY`, a freeform MP4 atom. ID3v2 and MP4 keep
  them — `lofty` holds them next to the generic tag, and the write path
  turns its `preserve_format_specific_items` option on so that they do not
  depend on the global state. The Vorbis family keeps them because the
  *concrete* tag is written, not the generic one (the generic model cannot
  represent a key it does not know, so a write used to drop every
  `CUSTOM_KEY` of a FLAC file). APEv2 has no such place in `lofty`'s generic
  model: its unknown items are dropped. The cover art of a FLAC file lives
  in its own metadata block and is kept as well.
- **What a write replaces.** The tag container is rewritten, nothing else:
  the audio frames are copied byte for byte (the Ogg writer is the
  exception — it rewrites the page headers, so the sequence numbers and
  their checksums change, but the audio packets do not). An ID3v2 tag is
  written as ID3v2.4 with 1 KiB of padding, whatever the file held before.
  A FLAC file without a padding block gets one.

## Artwork

The artwork reader exists so a library view can show album art. It reads the
pictures that are *inside* the file and nothing else.

- **API.** `read_artwork(path) -> Result<Option<Artwork>>` returns the front
  cover, `read_artworks(path) -> Result<Vec<Artwork>>` returns every picture
  of the file in the tag precedence order (primary tag first). `Artwork`
  carries the role (`PictureKind`), the MIME type, the description and the
  raw bytes. The bytes are the stored ones: no decoding, no resizing, no
  validation — the caller hands them to an image decoder.
- **Picture kinds.** `PictureKind` mirrors the ID3v2 `APIC` picture types
  (`0..=20`, anything above is `Undefined(u8)`). MP4 has no picture type: the
  `covr` atom *is* the album art, so an MP4 picture is reported as
  `FrontCover`.
- **The front cover** is the first picture that claims the role
  (`FrontCover`). A picture that names no role (`Other`, the type `0`) is the
  cover only when no picture claims the role — that is what a tagger writes
  when it does not know the picture types. Pictures that name another role (a
  back cover, the media label) are not the front cover: a file with only
  those has no front cover and `read_artwork` answers `None`.
- **Containers covered.** The ID3v2 `APIC` frame (MP3, AAC, WAV, AIFF), the
  FLAC `PICTURE` block, the Vorbis `METADATA_BLOCK_PICTURE` (Ogg Vorbis,
  Opus, Speex) and the MP4 `covr` atom. APEv2 cover art is *not* covered:
  `lofty` keeps it as a binary item, not as a picture.
- **Embedded only.** No network, no sidecar lookup: a `cover.jpg` next to the
  file is never read. The reader touches the file it is given and nothing
  else.
- **Strict.** The artwork reader parses strictly, the tag reader stays
  best-effort. A picture that cannot be parsed is an error, not a picture
  that is silently dropped — a half-there cover is worse than none. The price
  is that a file whose tag container is malformed in another way is an error
  here as well, even when `read_tags` still reads its text fields. A file
  without any picture is `Ok(None)`, never an error.
- **Not written.** The tag layer cannot write pictures: `TrackMetadata` has no
  field for them. Writing cover art needs the `rustamp-core` change below
  (or a separate write API in this crate).

## Requests

### `rustamp-core` proposal (needs an RFC before it lands)

1. **Add `disc_total: Option<u32>` to `TrackMetadata`.** The card asks for the
   "3/9" disc form, but the struct only has `disc_no`, so the total is parked
   in `extra["DISCTOTAL"]`. A first-class field would match `track_total` and
   remove the workaround. Additive, `#[serde(default)]` already applies.
2. **Add an artwork field to `TrackMetadata`** (for example
   `artwork: Vec<TrackArtwork>`), or bless a sibling type in `rustamp-meta`
   as the contract for cover art. The sibling type exists now (`Artwork` in
   `rustamp-meta`, read with `read_artwork`), but the playlist/library lane
   still has no agreed way to carry a picture from a tag read to the UI.
   Keeping raw bytes in `TrackMetadata` would make it heavier to clone on
   the hot path, so a side-channel type is probably better; that is exactly
   why this is a proposal and not a change.
3. **Note (no change requested):** `TrackMetadata::duration_ms` /
   `bitrate_kbps` are deliberately left `None` here; the decoder fills them
   from the container. If the library ever wants tag-only durations without
   opening a decoder, that is a separate decision.

### Spec request: **S14 (title formatting)**

- S14 does not exist, so `rustamp-meta` exposes raw fields only and does
  **not** implement `%1`–`%7`. The placeholder `"Artist - Title"` string is
  `TrackMetadata::display_title` in `rustamp-core`; it is clearly marked as a
  fallback and is the single place a real formatter will replace.
- **Open question for S14:** is `PlaylistEntry::title` the *raw* tag title or
  a *formatted* string? This is left undecided (the field is not touched
  here). The playlist lane and S14 must agree before the playlist shows
  formatted titles.
- When S14 lands it should say which fields are used, the fallbacks for
  missing fields, and whether the format string is per-entry or global. The
  raw inputs it needs (title, artist, album, album artist, track/disc
  numbers, year, genre, comment, extra) are all available from this crate.

## Audit (card E2)

- `write_tags` refuses `year > 9999` with `InvalidArgument` before opening the
  file; before, `year as u16` wrapped (70000 -> 4464) and was written.
- Still silent, by documented design: `extra` keys the target format has no
  field for (best-effort write, e.g. ReplayGain in MP4, tested); `DISCTOTAL`
  has no field in `TrackMetadata`, so it travels in `extra` and is written only
  where lofty has a matching key (not enumerated); clearing a field is not
  possible; a date or number that does not parse (`"01.01.2001"`, `"3/x"`)
  becomes `None`; `ParsingMode::BestAttempt` drops a malformed tag item rather
  than failing the file; a file with a media extension but unparseable body
  may read as empty metadata (`non_media_files_error_or_stay_default` accepts
  both outcomes). Making those errors needs a decision on strict parsing
  (`ParsingMode::Strict`) and a "report skipped items" API; no spec asks for it.
- Round trip losses: total/track strings are numbers only.

## Audit (tag writing)

- **The write is atomic.** The new file is built next to the original and
  renamed over it (`src/atomic.rs`). Before, `lofty` wrote into the file in
  place: a write that failed half-way left a damaged media file. A
  read-only file cannot be written (Windows refuses to replace it), and the
  original stays as it was.
- **Only the primary tag is written.** Before, every tag of the file was
  rewritten, so an ID3v1 tag next to an ID3v2 tag was rewritten as well.
  Now the second tag stays byte for byte as it was.
- **The Vorbis family keeps its unknown keys.** Before, the write went
  through `lofty`'s generic tag model, which drops every key it does not
  know: a FLAC file's own `CUSTOM_KEY` was lost on every write. The
  concrete Vorbis comments are written now, so the key survives.
- **The FLAC cover art survives.** The concrete comments alone would have
  dropped it — a FLAC file keeps its pictures in their own metadata block —
  so the whole FLAC file is written, comments and picture blocks.
- **The ID3v2 unknown frames survive.** They were already kept (the
  generic model holds them in a companion tag), but only while `lofty`'s
  global `preserve_format_specific_items` option is on. The write path
  turns it on for the calling thread, so the behaviour does not depend on
  what the rest of the process did to the global options. The option is
  thread-local and `rustamp-meta` is the only crate in the workspace that
  uses `lofty`.
- **Still lost, by documented design:** the unknown items of an APEv2 tag
  (`lofty`'s generic model has no place for them), and the ID3v2 tag of a
  FLAC file (the format does not allow it; the copy in the file is left
  alone).
- **Not supported yet:** clearing a field (a tag editor needs an explicit
  "remove" form), and writing a tag into a file that has none of the
  format's primary tag type but has a secondary one (an MP3 with only an
  ID3v1 tag gets an ID3v2 tag, the ID3v1 tag stays).

## Album grouping (album values)

An album value is a property of a *group* of files, not of one file. The
`album` module draws that group and computes the two album values from it.

- **The grouping key** (`album::album_key`, `album::AlbumKey`): a track
  **with** an album tag is keyed by the album title and the *effective
  album artist* — the `album_artist` tag, or, when that tag is missing, the
  `artist` tag. Both are trimmed and lower-cased, so `"Abbey Road"` and
  `"abbey road "` are one album and a differently cased album artist does
  not split an album. A track **without** an album tag is **its own
  album**, keyed by its path: nothing says two untagged files belong
  together, and merging them would give one of them the wrong album gain.
- **The fallback is per track.** A track that names the album artist and a
  track that names only the artist land in the same album when that artist
  *is* the album artist; a track whose artist differs from the album's
  artist starts its own group instead of dragging the album's gain onto
  it. This is a deliberate own choice: it keeps the grouping predictable
  from the tags alone and never merges two albums that merely share a
  title.
- **The album gain** is the negative of the 95th percentile of the pooled
  per-block loudness values of the album's tracks, clamped to ±51 dB — the
  track rule applied to the album as a whole, not an average of the track
  gains. The blocks are pooled through their loudness histograms
  (`replaygain::album_gain_from_histograms`), so the percentile is resolved
  on the histogram's 0.01 dB grid and can differ from the exact percentile
  by at most 0.005 dB. A one-track album therefore reproduces that
  track's gain to within one bin.
- **The album peak** is the largest track peak of the album.
- **Silence.** An album whose blocks all sit at or below -99 dB measures as
  the neutral 0.0 dB, not as the saturated +51 dB: a silent album must not
  ask for a huge boost. (The *track* rule is stricter — exact zero samples
  — so a track that is merely very quiet still asks for the clamped +51 dB.
  That difference is inherited from the track estimator and is unchanged.)
- **The grouping is pure.** It reads no file and touches no audio: the
  caller reads the tags, analyzes the audio and hands the result in
  (`album::AnalyzedTrack`). Writing the values back is the caller's job
  (`replaygain::write_replaygain`); the crate does not scan and does not
  write on its own.

## Audit (card E4 — ReplayGain)

- **MP4 loss fixed.** `extra` used to be written through
  `ItemKey::from_key(tag_type, name)`, whose ilst table only knows the
  freeform `----:com.apple.iTunes:replaygain_*` form, so the four classic
  ReplayGain keys were silently dropped on MP4. They are now looked up in a
  canonical table (`replaygain::EXTRA_ITEM_KEYS`) and inserted by `ItemKey`,
  which writes exactly those freeform atoms; the round-trip test asserts all
  four keys on all five fixture families.
- **Silence is now opt-in, not default.** `write_tags` keeps its best-effort
  signature (existing callers unchanged), but `write_tags_report` names every
  refused `extra` key in `WriteReport::unwritten_extra`, and
  `replaygain::write_replaygain` returns which keys landed and which were
  skipped. Callers that must know whether a write happened use those.
- **Per-format honest answers** (from `lofty` 0.25.4's key tables, verified by
  reading them, not by guessing): the four classic keys read and write on
  ID3v2 (MP3/AAC/AIFF/WAV via `TXXX`), Vorbis comments (FLAC/Ogg/Opus/Speex),
  MP4 ilst (freeform) and APEv2 (APE/MPC/WavPack). `R128_TRACK_GAIN` /
  `R128_ALBUM_GAIN` exist only in the Vorbis map, so they round-trip there
  and are reported as skipped everywhere else. `REPLAYGAIN_REFERENCE_LOUDNESS`
  has no `ItemKey` at all (unmapped keys are discarded on read), so the tag
  layer cannot carry it on any format; it is a named constant
  (`replaygain::REFERENCE_LOUDNESS_KEY`) and every write lists it as skipped.
  RIFF INFO, AIFF text and ID3v1 carry no ReplayGain keys, and no file format
  uses them as its primary (write-target) tag.
- **Analysis is a documented estimator, not a reference implementation.**
  `replaygain::analyze_replaygain` is pure (samples + channels + rate in,
  gain + peak out, no I/O) with the 89 dB SPL pink-noise reference anchored
  so a full-scale sine is 0 dB. It is RMS-only: no K-weighting, no gating,
  no channel weights, no equal-loudness contour, and the rate argument is
  validated but does not change the number. Silence measures 0 dB gain by
  own choice; gain clamps at ±51 dB by own choice; non-finite samples and
  ragged frames are refused loudly. Not EBU R128, not bit-identical to
  mp3gain/metaflac/foobar2000/loudgain.

## Audit (artwork reading)

- **The API changed shape.** `read_artwork` used to answer
  `Result<Vec<Artwork>>` (every picture). It now answers
  `Result<Option<Artwork>>` — the front cover, which is what a library view
  needs — and the old form is `read_artworks`. The picture type is a
  `PictureKind` instead of a bare `u8`, so a caller can ask
  `is_front_cover()` instead of comparing numbers. No crate outside this one
  used the old signature.
- **Tested, headless, on temp files.** The front cover is read back from all
  four containers: the ID3v2 `APIC` frame, the FLAC `PICTURE` block, the
  Vorbis `METADATA_BLOCK_PICTURE` and the MP4 `covr` atom — bytes, MIME type
  and description survive. A file without a picture answers `None`, not an
  error; a picture that cannot be parsed is an error, never a panic (a
  truncated FLAC picture block and a truncated `APIC` frame are both tested).
  A back cover is not the front cover, and a real front cover wins over an
  untyped picture.
- **The test writes the pictures with `lofty`.** The tag layer of this crate
  cannot write pictures, so the test writes them with `lofty`'s own API
  (`Tag::push_picture` + `Tag::save_to`) into a copy of a fixture.
- **Still open:** writing cover art (needs the `rustamp-core` proposal
  below), a sidecar `cover.jpg` lookup (deliberately out of scope), and
  decoding the image (the caller's job — the bytes are handed over as they
  are stored).
