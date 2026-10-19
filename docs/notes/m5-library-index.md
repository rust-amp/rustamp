# Media library index foundation (M5)

## Scope

`rustamp-library` provides the storage/scan/query layer only. It owns no UI,
no player wiring and no playlist model. `rustamp-app` and `rustamp-ui` are
untouched.

## Schema

Defined in `crates/rustamp-library/src/lib.rs` (module docs and `migrate`):

- `tracks(id INTEGER PRIMARY KEY AUTOINCREMENT, path TEXT NOT NULL UNIQUE,
  title, artist, album, album_artist, genre, comment, year, track_no,
  track_total, disc_no, duration_ms, bitrate_kbps, mtime_ns, size, scanned_at)`
  with indexes on `artist` and `album`.
- `track_extra(track_id, key, value)` is the `TrackMetadata::extra` map.

The stable row id is the SQLite rowid (`tracks.id`). Rows are upserted by
`path`, so ids survive re-scans. `duration_ms`/`bitrate_kbps` stay `NULL`
unless the tag container itself carries them: `rustamp-meta` reads tags, not
audio properties.

`mtime_ns` is stored in **nanoseconds** since the Unix epoch, not seconds:
second granularity would miss a tag rewrite that keeps the same size inside the
same second.

## Public API

- `Library::open(path)`, `Library::in_memory()`
- `Library::scan(root) -> ScanReport`
- `Library::query(&Query) -> Vec<TrackRow>` (ordered by row id)
- `Library::count()`, `Library::get(id)`
- `TrackRow { id, path, metadata: TrackMetadata, mtime_ns, size }`
- `Query { path_prefix, artist, album, title, genre }`; all fields optional,
  combined with AND, text fields exact and case-sensitive, `path_prefix` a
  literal prefix.

## Scan rule

1. Walk `root` recursively; symlinks and junctions are not followed (no
   cycles). A file root indexes that one file. A missing root is
   `Error::NotFound`, not an empty report.
2. Consider only regular files whose extension
   `rustamp_meta::supports_extension` accepts (case-insensitive). Other files
   are ignored and count in no report field.
3. Stat each considered file and compare `(mtime_ns, size)` with the stored
   row:
   - missing or changed stat -> `rustamp_meta::read_tags`, then upsert by
     `path`; a new path is `added`, an existing one `updated`, and the id is
     preserved;
   - identical stat -> untouched;
   - stat or tag read fails -> `skipped`; the previous row is **kept**, because
     the file still exists and a transient failure must not delete data.
4. After the walk, delete rows under `root` whose file was not found on disk
   (`removed`). Rows outside `root` are untouched. If any directory could not
   be read, the removal pass is skipped for the whole scan.
5. Path matching escapes `%`, `_` and `\` for `LIKE ... ESCAPE '\'`, so a
   folder such as `100%_music` is treated literally.

A re-scan of an unchanged tree reports zero `added`/`updated`/`removed`, zero
`skipped`, and returns identical row ids.

## Tests (14, all headless on temp directories)

`scan_indexes_and_queries`, `rescan_is_idempotent`, `update_on_change`,
`removed_files_dropped`, `query_by_album`, `query_filters_combine_with_and`,
`extras_round_trip`, `persistent_db_file`, `get_by_id`,
`non_audio_files_are_ignored`, `unreadable_file_keeps_its_row_and_counts_as_skipped`,
`scan_outside_root_does_not_touch_other_roots`,
`like_wildcards_in_paths_are_literal`, `scanning_a_missing_root_is_an_error`.

Fixtures are 44-byte WAV headers written with `rustamp_meta::write_tags`, so no
audio is decoded and no window is opened.

## What a UI needs next

- A background scan trigger (own thread plus a progress channel) and progress
  reporting; `ScanReport` is already the summary value to publish.
- Query construction from UI filters, plus free-text search (`LIKE '%term%'`
  over title/artist/album) — not added here because no UI card defines it.
- Display columns: title, artist, album, track, duration, genre, path.
- Sorting and pagination; `query` currently returns every match ordered by id.
- Tag editing: `rustamp_meta::write_tags`, then re-scan the file (or update
  the row directly) — no write API in this crate yet.
- Artwork via `rustamp_meta::read_artwork`; not stored in the index.
- Multi-root library management: callers already can scan several roots, but
  there is no `roots` table or "last scan time" query.
- CD playback/ripping (M5 optional) is out of scope here.

## Dependency added

- `rusqlite = "0.32"` with feature `bundled` — **MIT**, `libsqlite3-sys 0.30.1`
  is MIT, and the bundled SQLite itself is public domain. Recorded in
  `THIRD_PARTY.md` (direct-dependency table, `library` consumer, plus the
  lockfile inventory: `rusqlite`, `libsqlite3-sys`, `hashlink`,
  `hashbrown 0.14.5`, `fallible-iterator`, `fallible-streaming-iterator`,
  `vcpkg`). `.github/scripts/license-audit.py` passes: 310 external packages
  reconciled, all permissive roots on both Windows targets.
- `rustamp-meta` (internal, GPL-3.0-or-later) for tag reading; `tempfile`
  (workspace, dev-only) for the tests.

## Remaining limits

- Paths are opaque strings as scanned; a differently spelled path is a
  different row. Non-UTF-8 paths are stored lossily.
- Change detection is stat-based; a same-size, same-mtime rewrite is missed.
- `query` has no limit/offset and does not use a full-text index.
