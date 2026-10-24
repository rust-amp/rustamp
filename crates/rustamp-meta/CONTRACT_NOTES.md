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
- **Removing a field.** `write_tags_with(path, &metadata, &removals)` takes a
  field *out* of the file: the ID3v2 frame, the Vorbis comment or the MP4
  atom is deleted. The list is the one the writer supports (`TagField`:
  title, artist, album, album artist, year, genre, comment, track, disc).
  `None` still means "leave this tag as it is" — the two never mean the same
  thing — and a removal wins over a value in `metadata`. A field is removed
  as a whole: the track takes the track number and the total with it, the
  disc takes the disc number and the total, and the comment takes the
  `DESCRIPTION` item with it (the reader takes a description as the comment
  when a file has no comment of its own). Only the *primary* tag is
  written, so a value in a second tag (an ID3v1 tag next to an ID3v2 tag,
  the `RIFF INFO` of a WAV, the text chunks of an AIFF, the ID3v2 tag of a
  FLAC file) survives a removal; `WriteReport::unremoved` names every
  removal that is in that state, so the limitation is never silent.
- **Writing is atomic.** The new file is built next to the original (same
  directory, so the rename stays on one volume) and renamed over it. A
  failed or interrupted write therefore leaves the original byte for byte
  as it was; there is no half-written media file. The replacement keeps the
  permissions of the original. A symlinked file is written *through*: the
  link stays a link and the file it points at is replaced.
- **A write that changes nothing does not touch the file.** A request whose
  values the file already has leaves the file alone: its bytes and its
  modification time stay the ones it had, and the write report says so
  (`WriteReport::unchanged`). The comparison is made on the *decoded*
  values — the values a reader would see — and never on the bytes, because
  the byte layout of an ID3v2 tag is not deterministic (`lofty` keeps the
  text frames in a hash map, so two writes of the same metadata produce the
  same frames in a different order). A no-op save that rewrote the file
  would move its modification time for nothing, and every tool that watches
  the file (a library scanner, a file index) would see a new file. A removal
  wins over a value: a field that is in both the metadata and the removal
  list is removed, so the value of such a field cannot decide whether the
  write changes anything. The rule is in the `unchanged` module of the crate.
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
  where lofty has a matching key (not enumerated); a date or number that does
  not parse (`"01.01.2001"`, `"3/x"`) becomes `None`; `ParsingMode::BestAttempt`
  drops a malformed tag item rather than failing the file; a file with a media
  extension but unparseable body may read as empty metadata
  (`non_media_files_error_or_stay_default` accepts both outcomes). Making those
  errors needs a decision on strict parsing (`ParsingMode::Strict`) and a
  "report skipped items" API; no spec asks for it.
- **A field is removed with `write_tags_with`** (the `TagField` list); the
  removal is applied to the primary tag and is reported when a second tag
  still carries the value (`WriteReport::unremoved`).
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
- **Not supported yet:** writing a tag into a file that has none of the
  format's primary tag type but has a secondary one (an MP3 with only an
  ID3v1 tag gets an ID3v2 tag, the ID3v1 tag stays).

## Audit (removing a value)

- **The gap.** A field of `TrackMetadata` that is `None` means "leave this
  tag as it is", so the writer could not say "this value goes away": a
  cleared text field was written as an empty value and a cleared number
  kept the old one. `None` still means "keep" — every existing caller
  depends on it — so the removal is a separate, additive list.
- **The API.** `TagField` names the nine fields the writer supports;
  `write_tags_with(path, &metadata, &removals)` writes the metadata and
  removes the fields; `write_tags_with_report` is the honest form and names
  the removals the file still carries. `write_tags` and `write_tags_report`
  are the same functions with an empty removal list, so no existing caller
  changed.
- **What the removal does per format.** The removal deletes the item of the
  *primary* tag: the ID3v2 frame (`TIT2`, `TPE1`, `TDRC`, `TRCK`, …) for
  MP3/AAC/WAV/AIFF, the Vorbis comment (`TITLE`, `ARTIST`, `DATE`,
  `TRACKNUMBER`, …) for FLAC/Ogg/Opus/Speex, the `ilst` atom (`©nam`,
  `©ART`, `©day`, `trkn`, …) for MP4/ALAC, the APEv2 item for
  APE/MPC/WavPack. The Vorbis removal is spelled per key, so every
  spelling of a key goes (`ALBUMARTIST` and `ALBUM ARTIST`, `TRACKTOTAL`
  and `TOTALTRACKS`, `DISCTOTAL` and `TOTALDISCS`).
- **A field is a whole field.** The track is the number *and* the total, the
  disc is the number *and* the total: half the formats keep the two in one
  place (the ID3v2 `TRCK` frame, the MP4 `trkn` atom), so only the pair can
  be removed as a unit. The comment takes the `DESCRIPTION` item with it,
  because the reader takes a description as the comment when a file has no
  comment of its own — the ffmpeg FLAC muxer writes the comment that way.
- **The limits.** Only the primary tag is written, so a value in a second
  tag (an ID3v1 tag next to an ID3v2 tag, the `RIFF INFO` of a WAV, the text
  chunks of an AIFF, the ID3v2 tag of a FLAC file) survives a removal. The
  report names every such removal (`WriteReport::unremoved`) — the
  limitation is loud, not silent. The `extra` map is not a field list: a key
  of `extra` is not removed by a `TagField`.

## Audit (wave253 — the write report names what the format cannot store)

- **The gap.** The MP4 `trkn` and `disk` atoms hold a pair of 16-bit
  numbers, and a zero is "no number at all". A track or disc number of zero,
  or one above 65535, was therefore written into the tag and dropped by the
  format on the way out — and because the writer *replaces* the number, the
  number the file had before was gone with it. The write report named the
  refused `extra` keys and the removals the file still carries, but not the
  value the format could not hold: the loss was silent.
- **The API.** `WriteReport` has one additive field, `unstoreable:
  Vec<TagField>`: the fields whose value the target format could not store.
  `WriteReport::is_complete()` includes it, so a caller that asks "did the
  write land?" hears "no" when a value is missing. The field is
  meta-internal: `rustamp-core` and `rustamp-proto` are untouched, so no RFC
  is needed.
- **How the loss is detected.** A per-format *range check*, not a read-back
  of the written file. The writer knows what each format can hold, the check
  costs nothing, and it has no error path and no false positives (a
  read-back would have to know the reader's trimming rules, would have to
  look at the primary tag alone so a second tag cannot hide a loss, and
  could fail after a write that succeeded). The check runs on every write
  path — the generic path (ID3v2, MP4, APEv2), the FLAC path and the Vorbis
  path — and the table is the one below.
- **The table.**

  | Tag written | Files | Track/disc number | `0` | `> 65535` |
  |---|---|---|---|---|
  | ID3v2 | MP3, AAC, WAV, AIFF | any `u32` | kept | kept |
  | Vorbis comments | FLAC, Ogg Vorbis, Opus, Speex | any `u32` | kept | kept |
  | MP4 `ilst` | M4A, ALAC | 1..=65535 | lost | lost |
  | APEv2 | APE, MPC, WavPack | any `u32` | kept | kept |

- **What the app must surface.** A field of `unstoreable` is a value the
  user asked for that the file does not have. The app should say which
  field it was and that the format cannot hold it — "the track number is not
  in the file: MP4 stores 1..=65535" — and not present the write as a
  success. The value itself is not in the report: the app has it in the
  metadata it sent, and the report names the field.
- **What is not reported.** The year: a year above 9999 is refused with
  `Error::InvalidArgument` before the file is opened, so it is loud
  already. An empty text value: the reader drops an empty value in every
  format, so no format loses anything the caller could read back. A field
  of the removal list: the caller asked for it to be gone, so the format is
  not the reason it is not in the file.

## Audit (wave274 — a write that changes nothing does not rewrite the file)

- **The gap.** The byte layout of an ID3v2 tag is not deterministic: `lofty`
  keeps the text frames of a tag in a hash map, so two writes of the same
  metadata produce the same frames in a different order. A save that
  changed nothing therefore still changed the file: the bytes were another
  arrangement of the same frames and the modification time moved. Every
  tool that watches the file — the library scanner, the file index, a
  sync client — saw a new file for a save the user had not changed
  anything in.
- **The API.** `WriteReport` has one additive field, `unchanged: bool`: the
  request would not have changed the file, so the file was not touched.
  `WriteReport::is_complete()` does **not** include it: a write that
  changed nothing is complete, because the values the caller asked for are
  in the file — they were there already. The field is meta-internal:
  `rustamp-core` and `rustamp-proto` are untouched, so no RFC is needed.
  Nothing outside this crate constructs a `WriteReport`, so the field is
  additive for every caller in the workspace.
- **The rule.** The comparison is made on the **decoded values** — the
  values a reader would see — and never on the bytes: the order of the
  frames of an ID3v2 tag is not ours to compare, and two tags that hold the
  same values are the same tag. The decoded values are the reader's (a
  text value is trimmed, an empty one is "no value", a date is its year, a
  number pair is split on the `/`), so the value that is compared is the
  value a reader would see.
  - **Only the fields the request names are compared.** A field that is
    `None` in the metadata means "keep", so it cannot decide anything. A
    removal of a field the primary tag does not hold changes nothing
    either.
  - **The comparison is made against the primary tag**, the tag the writer
    writes. A value that only a second tag holds (an ID3v1 tag next to an
    ID3v2 tag) is not in the primary tag, so a request for it *is* a
    change: the write would put the value into the tag the reader prefers,
    and the effective value of the file would change with it.
  - **An `extra` entry the target format cannot store is not a change.** It
    cannot be in the file and cannot be put there; it stays in
    `WriteReport::unwritten_extra`, which is reported either way.
- **What the rule does not decide.** The rule looks at the values, not at
  the bytes: a write that would put the tag of the file in another shape
  (an ID3v2 tag is written as ID3v2.4 with padding, whatever the file held
  before) is a no-op when the values are the same. The file keeps the shape
  it had, which is the point.
- **The proof.** `tests/unchanged.rs` (headless, on temporary copies of the
  fixtures): an identical write leaves the bytes *and* the modification time
  of the file untouched; a one-field change is written; a removal of a field
  the file does not have is not written; a value that only the ID3v1 twin
  holds is a change; an `extra` entry the format refuses is not a change; the
  R128 keys of an ID3v2 tag (which live in `TXXX` frames the generic model
  cannot see) are compared through the concrete tag.

## What each format can carry

The property suite (`tests/tag_properties.rs`) writes generated metadata into
every format the writer supports, reads it back and compares the result with
what the format can hold. The table is the outcome; the notes below are the
differences a caller has to know about.

| File | Tag written | Numbers | Empty value |
|---|---|---|---|
| MP3, AAC, WAV, AIFF | ID3v2 | any `u32` | an empty item |
| FLAC, Ogg Vorbis, Opus | Vorbis comments | any `u32` | not stored |
| MP4 (AAC, ALAC) | `ilst` | 1..=65535 | an empty item |
| APE, MPC, WavPack | APEv2 | any `u32` | an empty item |

Every format carries the title, the artist, the album, the album artist, the
year, the genre and the comment. The `extra` map is narrower:

| Key | ID3v2 | Vorbis comments | `ilst` |
|---|---|---|---|
| `REPLAYGAIN_TRACK_GAIN` | yes | yes | yes |
| `R128_TRACK_GAIN` | yes | yes | no |
| `DISCTOTAL` | no | yes | no |

- **The reader trims.** A value of `"  Song  "` is read back as `"Song"`, and
  a value of blanks is read back as "no value". The trim belongs to the
  reader, not to the writer: the bytes of the file keep the blanks.
- **The MP4 numbers are 16-bit.** The `trkn` and `disk` atoms hold a pair of
  `u16`, and a zero is "no number at all". A number above 65535 and a zero
  are therefore lost — and because the writer *replaces* the number of the
  file, the old number is lost with them, not kept. The write report names
  the field whose value is lost (`WriteReport::unstoreable`), so the loss is
  loud.
- **ID3v2 keeps the track number and the total in one frame.** A total
  without a number is written as the pair `0/total`, so the file gains a
  track number of zero. The Vorbis family keeps the two in comments of their
  own and MP4 in one atom, so the same write leaves the number alone.
- **The Vorbis family does not store an empty value.** The key is gone.
  ID3v2 and MP4 store an empty item instead. The reader cannot tell the two
  apart, because it drops an empty value either way.
- **An `extra` key can overwrite a field of the metadata.** On the Vorbis
  family `extra["ALBUM"]` is the `ALBUM` comment, so it replaces the album
  the metadata named. ID3v2 and MP4 have no such key, so the entry is
  refused and reported instead.
- **The byte layout of an ID3v2 tag is not stable.** `lofty` keeps the text
  frames of a tag in a hash map, so two writes of the same metadata produce
  the same frames in a different order. The content of the tag is stable,
  the bytes are not. FLAC, Ogg, Opus and MP4 are byte-stable. A write that
  would not change the values of the tag does not happen at all (see the
  wave274 audit above), so a file the user did not change keeps its bytes.
- **The audio is never rewritten.** The property suite compares the audio of
  every file before and after every write: the frames behind the MP3 and AAC
  tags, the frames behind the FLAC metadata blocks, the audio packets of an
  Ogg stream (its page headers change), the `mdat` payload of an MP4 file,
  the `data` chunk of a WAV file and the `SSND` chunk of an AIFF file.

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
