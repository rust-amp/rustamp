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
  upper-case name (ReplayGain, ISRC, MusicBrainz IDs, composer, label, ...).
  `DISCTOTAL` is put there too, because there is no `disc_total` field yet.
- **Pictures.** `TrackMetadata` has no field for cover art, so pictures are
  returned by `read_artwork` as `Artwork` values (picture type, MIME type,
  description, raw bytes). The bytes are exposed verbatim; nothing is
  decoded, resized or validated.
- **Writing.** `write_tags` sets only the fields that are `Some` and leaves
  the rest of the tag untouched, so read → edit → write preserves what the
  caller does not set. A missing primary tag is created. The year is written
  as a recording date (ID3v2 `TDRC`, Vorbis `DATE`, MP4 `©day`). `extra`
  entries are written best-effort (only keys that map to the target format).
  Clearing a field is not supported yet; a tag editor (S06 era) will need an
  explicit "remove" form.

## Requests

### `rustamp-core` proposal (needs an RFC before it lands)

1. **Add `disc_total: Option<u32>` to `TrackMetadata`.** The card asks for the
   "3/9" disc form, but the struct only has `disc_no`, so the total is parked
   in `extra["DISCTOTAL"]`. A first-class field would match `track_total` and
   remove the workaround. Additive, `#[serde(default)]` already applies.
2. **Add an artwork field to `TrackMetadata`** (for example
   `artwork: Vec<TrackArtwork>`), or bless a sibling type in `rustamp-meta`
   as the contract for cover art. Today the playlist/library lane has no
   agreed way to carry a picture from a tag read to the UI. Keeping raw bytes
   in `TrackMetadata` would make it heavier to clone on the hot path, so a
   side-channel type is probably better; that is exactly why this is a
   proposal and not a change.
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
- Round trip losses: see the `write_read_roundtrip_on_each_family` comment
  (MP4 lacks a plain-text ReplayGain key); total/track strings are numbers only.
