# Contract and spec notes for `rustamp-playlist` (task card E1)

**Nothing in this file has been applied.** It records (a) the facts this card
had to invent because no spec exists yet, and (b) proposals for `rustamp-core`.
`rustamp-core` is frozen by `docs/rfc/0001-core-contracts.md`, so any change
listed under *Proposals* needs a new RFC.

## 1. Spec requests

### B4S — blocking, format description needed

`PlaylistFormat::B4s` exists and is detected (an XML root named `winamp`), but
reading and writing it return `PlaylistError::UnsupportedFormat`. Reason: B4S
is a Winamp-specific XML dialect and this card requires a **public format
description that was actually used**. No such description is available in this
repository, and downloading one is forbidden by the card. Guessing the element
and attribute names would be inventing a format.

Requested: a spec (or a pointer to a freely licensed description, e.g. a
DeaDBeeF parser) covering

* the root element and namespace,
* how an entry stores its location (element name, absolute vs relative, any
  `file:` URL convention),
* where title and length live and in which unit,
* whether entries have stable indices that must be preserved on write.

### S14 — title formatting: what is the input?

The readers fill `PlaylistEntry::title` with what the file itself contains
(`#EXTINF` text, PLS `TitleN`, XSPF `title`), verbatim, trimmed, without
formatting.

Requested decisions from S14:

* Is the cached `PlaylistEntry::title` the **formatted** display string or the
  raw title from the file?
* Which of the fields that `%1`–`%7` can reference are available for an entry
  loaded from a playlist? Concretely: does an M3U `#EXTINF` title become the
  input for `%1`, and is `Artist - Title` in an `#EXTINF` split into two fields
  or kept as one?
* Should loading a playlist call the formatter at all, or is formatting applied
  when the row is painted?

Until S14 answers this, `rustamp-playlist` stores the file's text unchanged and
performs no splitting or formatting.

### S15 — shuffle sequence: relationship to `randomize`

`order::randomize` is a **one-shot** Fisher-Yates permutation of the current
order, from a seeded in-crate PRNG, applied through `Playlist::apply_order`. It
is the "Randomize" list operation, not the shuffle mode.

Requested decisions from S15:

* Is "Randomize" the same operation as re-seeding the shuffle sequence, or a
  separate one-shot action?
* When the shuffle sequence is built and when it is invalidated: on toggle, on
  add/remove/reorder, on sort, on randomize, on playlist load?
* Does the current entry stay current when the order changes (this crate leaves
  it untouched), and does the sequence guarantee "no immediate repeat"?
* Where does the random seed come from, and must a session be reproducible?

### S06 — playlist editor: which operations call into this crate

Requested decisions from S06:

* The exact "Misc / Sort" and "Misc / List" menu entries, so the right
  `SortKey` (`Title`, `FileName`, `PathAndFileName`, `Duration`) is chosen and
  the "Randomize" entry maps to `order::randomize`.
* Whether "Sort" is stable (core's `sort` is) and whether sorting moves the
  current entry with the row (core keeps it selected by id).
* Whether the editor needs the *raw* file text of an entry (for an "Edit
  entry" style action) — this crate only keeps the resolved location, title and
  duration.
* Whether the playlist window needs to know which format a loaded file had
  (for "Save playlist" defaulting to the same format). **Answered by the
  import side:** `import_playlist_file` and `import_playlist_path` return an
  `ImportedPlaylist` that carries the detected `PlaylistFormat` next to the
  entries, so the caller no longer has to guess it. The plain readers
  (`read_playlist*`, `load_playlist_file*`) keep their signature and report
  the entries only. `rustamp-app` does not use the import side yet; that is
  the open end.

## 2. Proposals for `rustamp-core` (need an RFC)

### 2.1 `Playlist::apply_order` is O(n²)

```rust
for id in order {
    if let Some(slot) = old.iter_mut().find(|s| s.as_ref().is_some_and(|e| e.id == *id)) { ... }
}
```

One linear scan per id makes `apply_order` quadratic. `Command::Randomize`
exists in core, so a 50 000-entry playlist would take minutes. Proposal: index
the drained entries by `EntryId` (a `HashMap` or a sort by the current
position) and rebuild in O(n log n). This crate is written against
`apply_order` as it is, as the card requires.

### 2.2 `Playlist::insert_many` is O(k·n) when it is not appending

`insert_many` calls `insert(start + offset)` per entry, shifting the tail each
time. Inserting k entries at the front of an n-entry playlist costs O(k·n).
`append_loaded` inserts at `index == len`, which is O(k) amortized, so this
crate is not affected, but the UI's "enqueue at the top of a large playlist"
would be. Proposal: drain/splice once, as `move_entries` already does.

### 2.3 No place for artist/album from a playlist file

`#EXTART` and `#EXTALBUM` (M3U) and XSPF `<creator>`/`<album>` have no field in
`LoadedEntry`/`PlaylistEntry`, so they are parsed and dropped. If they should
survive, `PlaylistEntry` needs either dedicated fields or an
`extra: BTreeMap<String, String>` like `TrackMetadata`. Recommendation: decide
this together with S14 (which fields the title formatter needs) rather than
adding fields now.

### 2.4 Non-`http` URL schemes

RFC 0001 describes `MediaLocation::Url` as "a network stream or other URL". This
crate classifies every `scheme:` location other than `file:` as `Url`
(`http`, `https`, `ftp`, `mms`, `rtsp`, …). No change requested, but the
behavior is worth confirming in a spec: an unknown scheme is kept verbatim
rather than rejected.

### 2.5 Lossy paths (informational)

`file:` URLs and Windows-1252 playlist bytes are decoded to `String` with
replacement characters, then turned into a `PathBuf`. A non-UTF-8 path inside a
playlist is therefore not recoverable byte-exactly. This is the same limitation
RFC 0001 already lists for `PathBuf` over IPC; no action for E1.

## 3. Deterministic rules chosen in this crate

These are documented in each module and are candidates for a spec to override:

| Area | Rule |
|---|---|
| detection | content marker > empty input takes the extension's format > binary/UTF-16, XML with an unknown root, text behind an unknown or XML (`.xspf`/`.b4s`) extension: **rejected** > `.m3u8` > other text as M3U (card E2 tightened this; before, any `.m3u` was trusted and `.xspf` HTML parsed to an empty list) |
| marker search | the **whole file** is searched for a marker, not only its first kilobyte, so a PLS with a long comment preamble and an XSPF with a long XML comment in front of its root are still recognized; the markers are ASCII and are searched in the raw bytes, so the search does not depend on the encoding |
| explicit format | `read_playlist_as` rejects bytes that contradict the format (other marker, binary/UTF-16/XML for the line formats, truncated or rootless XSPF) |
| writer input | an empty or whitespace-padded location, a line break in an M3U/PLS location, or a leading `#` in an M3U location is `InvalidEntry`; it used to be rewritten silently |
| encoding | UTF-8 first (BOM stripped), Windows-1252 fallback for `.m3u`; `.m3u8` uses UTF-8 with replacement characters; UTF-16 is not decoded |
| M3U | most recent `#EXTINF` wins; leading number of the length; negative/unparseable length = unknown; empty title = `None`; `#` line = comment; `#EXTALBUM`/`#EXTART` recognized and dropped |
| PLS | keys before the first section and inside `[playlist]`; ascending index order; last occurrence wins (an empty value removes the key); `NumberOfEntries`/`Version` ignored; values verbatim |
| XSPF | every `<track>` in document order; a track without a location is dropped; `xml:base` is a directory prefix; names matched by local part, case-insensitively; a document cut off mid-track still yields that track |
| writing | CRLF for M3U/PLS, LF for XSPF, always UTF-8; unknown duration is `#EXTINF:-1`; a duration with a sub-second part is written with three decimals so that write → read is exact |
| paths | relative entries join the playlist directory; `/` and `\` are both separators; a drive prefix is never joined; `file:` → `File`, any other scheme → `Url` untouched; empty location = skipped |
| randomize | one Fisher-Yates permutation from a seeded xorshift64\* generator, applied through `apply_order`; ids, selection and current entry unchanged |

## 4. Round-trip honesty (card E2)

`write_playlist` then `read_playlist_path` is exact for locations (absolute
files, URLs), non-empty trimmed titles and durations (whole milliseconds), in
all four formats (`tests/formats.rs`, `tests/property.rs`). What is lost, and
why, pinned in `tests/audit.rs`. The same rules are pinned as *properties*
over seeded random playlists in `tests/format_properties.rs`: the round trip
and its documented losses, the format detection (including a long comment in
front of the file), the total behavior of the readers on malformed input, and
the stability of the writer.

| Field | Loss | Reason |
|---|---|---|
| title with leading/trailing whitespace | comes back trimmed | every reader trims (`normalize_title`) |
| `Some("")` title | comes back `None` | same |
| entry with an empty title and no duration | the `#EXTINF` line is gone when the file is written again | the reader turns the empty title into "no title"; an entry with neither a title nor a duration needs no `#EXTINF` line, so the second write drops it. The file converges after one round |
| title with CR/LF | M3U/M3U8/PLS: line break becomes a space; XSPF exact | one entry per line in those formats |
| artist, album | never stored | `LoadedEntry` has no field (2.3); `#EXTART`, `#EXTALBUM`, XSPF `creator`/`album` are parsed away. Needs a core RFC together with the S14 decision (is `title` raw or formatted; is `Artist - Title` split) |
| relative `File` location | written as-is, resolves against the reader's base; a relative location that looks like a PLS key (`FileN=…`, `[playlist]`) is detected as a PLS and loses the key | `LoadedEntry::location` is documented as resolved; callers must resolve before writing. A `FileN=` line is a PLS marker and a content marker wins over the file name (section 3) |
| XSPF `<title>` of the list, `<meta>`, `<extension>`, per-track `<image>`/`<info>` | dropped | no field |
| PLS/M3U unknown keys and comments | dropped | no field |

Not settled by any spec (S06 is silent on playlist files; S14 is missing), so
nothing was invented: raw-vs-formatted title, artist/album storage, how a
caller learns that N entries were skipped (settled by task card E3: `PlaylistReport`
and `SkippedEntry` provide surviving entries and detailed skip reasons, while
`read_playlist` retains its legacy `Vec<LoadedEntry>` signature for full workspace
compatibility), whether a marker-less text file named `.pls` should be an error
(today it is read as a bare M3U list, tested), B4S.
