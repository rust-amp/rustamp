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
- `Library::scan_with_progress(root, &Cancel, on_progress) -> ScanReport`,
  `Library::scan_root_with_progress(root_id, &Cancel, on_progress) -> ScanReport`
- `Library::add_root(path) -> Root` (idempotent, stored as given, need not
  exist yet), `Library::remove_root(path) -> usize` (tracks dropped;
  `NotFound` when the path is not a root), `Library::roots() -> Vec<Root>`
- `Library::query(&Query) -> TrackPage { rows, total }`, `Library::count()`,
  `Library::get(id)`
- `Library::write_tags(id, &TrackMetadata) -> Result<TrackRow>` (writes the
  tags into the real file and refreshes the row from it, see below)
- `TrackRow { id, path, metadata: TrackMetadata, mtime_ns, size }`
- `Root { id, path, added_at }`
- `Query { path_prefix, artist, album, title, genre, text, sort, descending,
  limit, offset }`; field filters are exact and case-sensitive, combined with
  AND; `path_prefix` is a literal byte prefix.
- `Cancel { new, cancel, is_cancelled }`, `ScanPhase { Scanning, Removing,
  Finished }`, `ScanProgress { phase, path, seen, added, updated, skipped,
  removed }`, `SCAN_BATCH_SIZE` (64).

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

## Background scan with progress (wave174)

`scan_with_progress` is the scan for a background thread. The crate owns no
thread: the caller decides where the scan runs and how the events reach the
user interface.

- `Library::scan_with_progress(root, &Cancel, on_progress) -> ScanReport`
  and `Library::scan_root_with_progress(root_id, &Cancel, on_progress)` are
  the scanning entries; `scan`/`scan_root` are the same code path with a
  flag that is never set and a callback that does nothing.
- The event is `ScanProgress { phase, path, seen, added, updated, skipped,
  removed }` with the coarse `ScanPhase { Scanning, Removing, Finished }`.
  The callback runs on the thread that drives the scan: once before the
  first file, once per considered file and once per phase change. The
  counters are the live ones; they can be ahead of the index by the batch in
  flight.
- `Cancel` is a shared flag (`Arc<AtomicBool>`): the caller keeps one clone
  and hands another to the scan, so the interface thread can stop a scan
  that runs on a worker thread. The scan checks the flag between two files
  and before the removal pass.

### The batch rule

A scan commits its work in batches of at most `SCAN_BATCH_SIZE` (64)
considered files, and every batch is one transaction. The index therefore
never holds half a batch: it holds the batches that were committed, in
order. The batch size bounds two things at once: the work a cancelled scan
drops (at most one batch) and the time a write transaction is open.

### The cancel semantics

- The scan stops at the next file boundary; the file it is on is finished.
- The batch in flight is **not** written: it is dropped, so the index keeps
  the batches that were committed before the cancel. At most
  `SCAN_BATCH_SIZE` files of work are lost; the next scan picks them up.
- The removal pass does **not** run after a cancel, so a cancelled scan
  never deletes a row. A cancel that arrives during the removal pass rolls
  the removal transaction back, so it deletes nothing either.
- `ScanReport` describes the *committed* state: `added`, `updated` and
  `removed` are the changes the index really holds, `scanned`/`skipped` are
  the files of the committed batches, and the new `cancelled` flag says
  whether the scan was stopped.

The scan rules are unchanged: the same `(mtime_ns, size)` change detection,
the same "unreadable file is skipped and keeps its row", and the removal
pass only under a fully walked root.

## Tag writing (wave169)

`Library::write_tags(id, &TrackMetadata) -> Result<TrackRow>` edits the tags of
an indexed track. The values go into the **real file** through
`rustamp_meta::write_tags`, which replaces the file atomically (the file is
either the old one or the new one). A field that is `None` is left as the file
has it (the `rustamp-meta` contract), so one field can be edited without the
others being lost.

The row is then rebuilt from the file: the tags are **read back** after the
write, and it is those values that land in the index - never the values the
caller passed in. A format that stores a value differently, or cannot store it
at all, therefore shows up in the index as the file really is; the index can
never claim a value the file does not carry. The row id does not change (the
row is updated in place), and the stored `mtime_ns`/`size` are refreshed from
the written file, so the next scan sees no change and does not read the file
a second time. The refreshed row is returned.

Error cases:

- `NotFound` if no row has the id, and `NotFound` if the file the row points
  at is gone - a file that vanished between the scan and the write is
  reported, never written into the row silently.
- The error of the write itself: an unreadable file, a format whose tags
  cannot be written (`UnsupportedFormat`), a year a tag cannot hold
  (`InvalidArgument`).
- The error of reading the tags back, if the written file cannot be read any
  more.

A failed write leaves the row exactly as it was, and the file untouched (the
write is atomic). A write that succeeded but could not be read back leaves the
row as it was, too: the row is then stale and the next scan repairs it,
because the file's stat no longer matches the stored one.

## Tests (38, all headless on temp directories)

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

Tag writing (8, wave169): `writing_tags_updates_the_row_without_a_rescan`
(the row follows the write, the id survives, the file carries the new value
and the next scan finds no change), `the_row_follows_the_file_and_not_the_caller`
(a value the format cannot store never reaches the row),
`a_partial_write_keeps_the_tags_the_caller_did_not_set`,
`a_failed_write_leaves_the_row_unchanged` (a year a tag cannot hold),
`a_write_to_a_file_that_is_no_media_leaves_the_row_unchanged`,
`a_vanished_file_is_reported_and_leaves_the_row_alone`,
`writing_to_an_unknown_track_is_an_error`,
`a_write_keeps_the_extra_tags_of_the_file`.

Background scan with progress (6, wave174):
`progress_events_arrive_in_order` (the phases arrive in order, one event per
considered file, the counters rise, the ignored file produces no event),
`a_file_that_cannot_be_read_is_reported_as_skipped`,
`a_cancelled_scan_stops_early_and_keeps_the_committed_rows` (the cancel
arrives in the second batch: the first batch is committed, the batch in
flight is dropped, so the index holds exactly `SCAN_BATCH_SIZE` rows),
`a_cancel_before_the_delete_pass_leaves_the_index_untouched` (the stale row
survives and the changed row keeps its old tags),
`a_normal_scan_still_behaves_as_before` (the plain scan and the scan with
progress report the same thing),
`a_scan_cancelled_before_it_starts_does_nothing`.

Fixtures are 44-byte WAV headers written with `rustamp_meta::write_tags`, so no
audio is decoded and no window is opened.

## What a UI needs next

- A background scan trigger (own thread plus a progress channel); the crate
  now has the two halves the app needs: `scan_with_progress` /
  `scan_root_with_progress` report the progress, and `Cancel` stops a scan
  that runs on a worker thread. The app owns the thread and the channel, and
  it loops over `roots()` calling `scan_root_with_progress(id, &cancel, cb)`
  per root. `ScanReport` is the summary value to publish; `cancelled` says
  whether the scan was stopped.
- Display columns: title, artist, album, track, duration, genre, path; the
  index has no column for artwork or for the file name.
- Tag editing is available now: `Library::write_tags(id, &metadata)` writes
  the file and returns the refreshed row. A caller that needs to know which
  `extra` entries the target format refused uses
  `rustamp_meta::write_tags_report` directly.
- Artwork via `rustamp_meta::read_artwork`; not stored in the index.
- A per-root last-scan timestamp (the `roots` table has only `added_at`).
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
- A tag write cannot ask for a field to be removed: `write_tags` reads a
  `None` as "leave the value the file has".
- The scan progress is not stored: there is no per-root last-scan time and
  no progress row in the index, so a scan that was interrupted is resumed by
  running it again.
- A cancelled scan drops the batch that was in flight (at most
  `SCAN_BATCH_SIZE` files of work); the next scan picks those files up.
- The scan reports one event per considered file, so a caller that forwards
  the events over a channel should throttle them if it cannot keep up.

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

