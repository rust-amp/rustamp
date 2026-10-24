# Task card M1 — Tag reading and writing (`rustamp-meta`)

**Goal**
`rustamp-meta` reads and writes media tags through `lofty` and maps them onto
the frozen `rustamp_core::TrackMetadata`, so the playlist can show titles,
artists, albums and durations without decoding the audio. When the card is done,
a folder scan yields metadata for tagged files of every supported family, and a
write round-trips.

**Lane** B — decoders/formats, meta part (`crates/rustamp-meta/`).

**Owned paths**
- `crates/rustamp-meta/**`
- Do **not** edit `Cargo.toml` at the repository root and do not touch any other
  crate. The workspace dependency `lofty` is already prepared and wired into
  this crate, and `tempfile` is available as a dev-dependency.

**Allowed sources**
- `rustamp_core` (`TrackMetadata`, `Result`/`Error`) and
  `docs/rfc/0001-core-contracts.md`.
- The public documentation of `lofty` (in the local cargo registry).
- Public knowledge of the tag formats themselves (ID3v1/v2, Vorbis comments,
  MP4/ILST, APE, FLAC picture blocks).
- `docs/PLAN.md` section 2 for scope.

**Forbidden sources**
- Winamp 5.x source, Winamp SDK headers or documentation, disassembly, the
  Winamp 2.95 binary, `D:\code\rustamp-research`. Nothing from them.
- No downloads (no `cargo add`, no git clone, no web fetching). `lofty` is
  already in the workspace; if you need another crate, ask first.

**Spec dependencies**
- **S14 (title formatting) does not exist.** Do **not** implement the Winamp
  `%1`–`%7` format, and do not invent a display-title rule. Expose the raw
  fields; if you add a convenience display title, mark it clearly as a
  placeholder that S14 will replace, and keep it in one place.
- **Open question from the playlist lane:** is `PlaylistEntry::title` the raw
  tag title or a formatted string? That is part of S14. Leave the decision
  visible (a doc comment plus a note in `CONTRACT_NOTES.md`), do not decide it.
- S06 (playlist editor) does not exist. No UI behaviour here.

**Interface contract**
- Consumes `rustamp_core::TrackMetadata`; anything you need added to it is a
  contract change and needs an RFC — write the proposal in
  `crates/rustamp-meta/CONTRACT_NOTES.md` instead of editing `rustamp-core`.
- Suggested surface (adjust if `TrackMetadata` dictates otherwise):
  - `read_tags(path) -> Result<TrackMetadata>` — tags only, **no audio
    decoding**;
  - a batch/scan entry point that takes an iterator of paths and returns
    per-path results, so one unreadable file does not abort the scan;
  - `write_tags(path, &TrackMetadata) -> Result<()>` (or a builder) that
    preserves the fields it does not set;
  - a probe/extension list for the formats `lofty` handles, to sit next to
    `rustamp-decoders`' registry.

**What must work**
- Read: ID3v1 and ID3v2 (v2 takes precedence, with a documented rule for the
  merge), Vorbis comments (FLAC/Ogg), MP4/ALAC atoms, APE tags, and WAV/AIFF
  info chunks where `lofty` supports them.
- Fields: title, artist, album, album artist, track/disc number (with the
  "3/9" form), year/date, genre, comment, and embedded pictures (expose them,
  do not decode audio for them).
- **Absent tags** yield `TrackMetadata::default()`-shaped values with the
  fields `None`, never an error; a file that is not media at all yields an
  error, not a panic.
- Non-UTF-8 / mixed-encoding tag bytes must never panic; document the lossy
  rule you chose.
- Duration and bitrate: only report them if `lofty` can read them cheaply.
  Never decode audio in this crate; if a value is unknown it stays `None` (the
  decoder fills it during playback).
- Write: setting the common fields round-trips (read → write → read), and
  writing to a file whose tags are absent creates them.
- A scan of many paths must be usable from a background thread: no global
  state, no panics, and per-file errors are values.

**Acceptance tests**
- [ ] `cargo fmt --all -- --check` clean.
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean.
- [ ] `cargo test --workspace --all-features` green, with new tests for:
  - [ ] reading a tagged file of each supported family (use the small fixtures
        already in `crates/rustamp-decoders/tests/fixtures/` by relative path —
        they are generated with ffmpeg and documented in `FIXTURES.txt` — or
        generate your own with the same ffmpeg commands);
  - [ ] absent tags → `None` fields, no error;
  - [ ] a non-media file (random bytes, a text file, an empty file) → an error
        or default metadata, never a panic;
  - [ ] write → read round-trip on a **temp copy** of a fixture (never modify
        the fixture in place);
  - [ ] ID3v1/ID3v2 precedence with a file that carries both;
  - [ ] the "3/9" track-number form and a bare number;
  - [ ] embedded artwork is exposed and its bytes are unchanged;
  - [ ] a batch scan with one broken path among good ones returns the good
        results and reports the broken one;
  - [ ] unicode titles and artists survive a round-trip.
- [ ] State in the report how long reading tags for the fixtures takes, and
      confirm no audio decoding happens (e.g. the tag read of a deliberately
      truncated file still works).
- [ ] `cargo test -p rustamp-plugin-abi --target i686-pc-windows-msvc` and
      `cargo build -p rustamp-plughost --target i686-pc-windows-msvc` still green.
      (`rustamp-meta` itself is only built for the 64-bit player.)
- [ ] `cargo xtask golden` reports no mismatch.

**Provenance notes**
- `lofty` is an ordinary dependency, not copied code; no `THIRD_PARTY.md` entry
  is needed. If you copy any code from a free project, stop and report it.

**Report back**
Files changed, the public surface you chose, the encoding/precedence rules you
decided, test results and timings, what you deliberately left out (S14 title
formatting), the spec requests, and any `rustamp-core` proposal.
