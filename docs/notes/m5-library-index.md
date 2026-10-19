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
- `roots(id INTEGER PRIMARY KEY AUTOINCREMENT, path TEXT NOT NULL UNIQUE,
  added_at INTEGER NOT NULL)` lists the indexed directories.

Connections are configured with `PRAGMA foreign_keys = ON`, so deleting a track
also deletes its `track_extra` rows; and with a registered `rustamp_lower` scalar
function (Rust's Unicode lower-casing) that the free-text search and the text
sort use.

The stable row id is the SQLite rowid (`tracks.id`). Rows are upserted by
`path`, so ids survive re-scans. `duration_ms`/`bitrate_kbps` stay `NULL`
unless the tag container itself carries them: `rustamp-meta` reads tags, not
audio properties.

`mtime_ns` is stored in **nanoseconds** since the Unix epoch, not seconds:
second granularity would miss a tag rewrite that keeps the same size inside the
same second.

## Public API

- `Library::open(path)`, `Library::in_memory()`
- `Library::scan(root) -> ScanReport`, `Library::scan_root(root_id) -> ScanReport`
- `Library::add_root(path) -> Root` (idempotent, stored as given, need not
  exist yet), `Library::remove_root(path) -> usize` (tracks dropped;
  `NotFound` when the path is not a root), `Library::roots() -> Vec<Root>`
- `Library::query(&Query) -> TrackPage { rows, total }`, `Library::count()`,
  `Library::get(id)`
- `TrackRow { id, path, metadata: TrackMetadata, mtime_ns, size }`
- `Root { id, path, added_at }`
- `Query { path_prefix, artist, album, title, genre, text, sort, descending,
  limit, offset }`; field filters are exact and case-sensitive, combined with
  AND; `path_prefix` is a literal byte prefix.

### Query semantics

- `text` is the free-text search: one string, matched as a literal
  case-insensitive substring of `title`, `artist` or `album`
  (`LIKE ... ESCAPE '\'`, `%`/`_`/`\` literal, both sides folded with
  `rustamp_lower`, so `ÄRZTE` finds `Ärzte`). Whitespace-only means "no
  filter"; it covers one column at a time, not a concatenated haystack.
- `sort` is `SortKey::Id` (default), `Title`, `Artist`, `Album` or `Duration`.
  Text columns sort case-insensitively, `NULL` sorts last in both directions,
  and ties always break by ascending row id — a total order, which is what
  makes paging stable.
- `limit`/`offset` page the result; `total` in `TrackPage` is the number of
  matches *before* the window, so a UI can size a scrollbar without a second
  query. `limit: Some(0)` returns no rows and still reports the full `total`.

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

`scan_root(id)` is `scan` on the root's stored path — scanning is independent
of the roots table, so a path can be scanned without being registered.
`remove_root` deletes exactly the row set `scan`'s removal pass looks at
(`rows_under`), plus the `roots` row.

A re-scan of an unchanged tree reports zero `added`/`updated`/`removed`, zero
`skipped`, and returns identical row ids.

## Tests (24, all headless on temp directories)

Scan/query (14, wave145): `scan_indexes_and_queries`, `rescan_is_idempotent`,
`update_on_change`, `removed_files_dropped`, `query_by_album`,
`query_filters_combine_with_and`, `extras_round_trip`, `persistent_db_file`,
`get_by_id`, `non_audio_files_are_ignored`,
`unreadable_file_keeps_its_row_and_counts_as_skipped`,
`scan_outside_root_does_not_touch_other_roots`,
`like_wildcards_in_paths_are_literal`, `scanning_a_missing_root_is_an_error`.

Roots, search, sort and paging (10, wave153):
`roots_are_added_listed_and_idempotent`,
`scan_root_scans_the_registered_directory`,
`removing_a_root_drops_its_tracks_and_extras` (also checks the `track_extra`
cascade), `removing_an_unlisted_root_is_an_error`,
`removing_a_listed_but_unscanned_root_drops_nothing`,
`freetext_searches_title_artist_and_album_case_insensitively` (incl. `ÄRZTE`),
`freetext_treats_like_wildcards_literally`,
`sort_orders_by_column_case_insensitively_with_nulls_last` (durations are set
straight in the index, because a WAV tag cannot carry `duration_ms`),
`sort_descending_reverses_order_with_stable_ties`,
`pagination_reports_the_unpaged_total` (boundaries: first/next/last page, past
the end, `None` limit, `limit 0`, filter-scoped total).

Fixtures are 44-byte WAV headers written with `rustamp_meta::write_tags`, so no
audio is decoded and no window is opened.

## What a UI needs next

- A background scan trigger (own thread plus a progress channel) and progress
  reporting; `ScanReport` is already the summary value to publish, and the UI
  can loop over `roots()` calling `scan_root(id)` per root.
- Display columns: title, artist, album, track, duration, genre, path; the
  index has no column for artwork or for the file name.
- Tag editing: `rustamp_meta::write_tags`, then re-scan the file (or update
  the row directly) — no write API in this crate yet.
- Artwork via `rustamp_meta::read_artwork`; not stored in the index.
- A per-root last-scan timestamp (the `roots` table has only `added_at`) and
  scan progress.
- CD playback/ripping (M5 optional) is out of scope here.

## Remaining limits

- Paths are opaque strings as scanned; a differently spelled path is a
  different row, and non-UTF-8 paths are stored lossily. `add_root` does not
  canonicalise either, so callers must keep one spelling per directory.
- Roots may nest: `remove_root` on an outer root also drops the tracks of an
  inner root, while the inner root stays listed.
- Change detection is stat-based; a same-size, same-mtime rewrite is missed.
- Free-text search is a `LIKE` scan over three columns, not a tokenised
  full-text index; there is no ranking and no cross-column matching.
- Sorting by duration is mostly a stable order over `NULL`s today, because
  `rustamp-meta` does not read audio properties.

## Dependency added

- `rusqlite = "0.32"` with features `bundled` and `functions` — **MIT**,
  `libsqlite3-sys 0.30.1` is MIT, and the bundled SQLite itself is public
  domain. `functions` is a feature of the same crate and adds no dependency, so
  `Cargo.lock` is unchanged by wave153. Recorded in
  `THIRD_PARTY.md` (direct-dependency table, `library` consumer, plus the
  lockfile inventory: `rusqlite`, `libsqlite3-sys`, `hashlink`,
  `hashbrown 0.14.5`, `fallible-iterator`, `fallible-streaming-iterator`,
  `vcpkg`). `.github/scripts/license-audit.py` passes: 310 external packages
  reconciled, all permissive roots on both Windows targets.
- `rustamp-meta` (internal, GPL-3.0-or-later) for tag reading; `tempfile`
  (workspace, dev-only) for the tests.

