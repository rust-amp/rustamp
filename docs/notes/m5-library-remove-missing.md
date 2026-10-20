# The media library removes the missing entries (M5)

## Scope

`rustamp-app` gains the **user action** for the index's "remove the rows
whose file is gone": the media library window can now drop the rows whose
file was deleted, moved or is on a drive that is not attached.

The index's side of it already existed - `rustamp_library::Library::
remove_missing` (`wave220`) drops the rows, keeps the ones whose file cannot
be checked and reports them. This lane is the **wiring**: the chord, the
guard against a running scan, the reload of the page and the report.

`rustamp-library`, `rustamp-ui` and `rustamp-skin` are untouched. No
contract changes: the action is not a `Command` (there is no command id for
it, and S03 has no row for the media library), so nothing in
`rustamp-core` moves.

Files:

- `crates/rustamp-app/src/library_window.rs` - the chord, the action, the
  guard and the report note.
- `crates/rustamp-app/tests/library_window.rs` - the headless tests (5 new).
- `crates/rustamp-app/Cargo.toml`, `Cargo.lock`, `THIRD_PARTY.md` - the test's
  dev-dependency (see below).

## The chord

| | |
| --- | --- |
| the chord | `Ctrl+M` ("M" for **missing**) |
| the action | removes every row whose file is gone, under every registered root |
| the scope | `Library::remove_missing(None)` - the rows of all roots |

**Own choice:** S20 (the media library) is not started, so no spec measures
the library's chords. `Ctrl+M` is in neither the main window's table nor the
global table, and it collides with nothing the library window binds already
(`Ctrl+F` focuses the search field, `Ctrl+O` asks for a folder, `F5`
rescans). The plain letter `M` does nothing: it is a printable character
that belongs to the search field.

The action is a chord and **not** a click: dropping rows is not undoable
from the window, and a stray press on a band of the list must not do it.

## A running scan refuses the action

The scan worker **holds the index** while it scans: it writes to the
database, and the app must not query it while that happens. The removal
therefore cannot run beside a scan.

The window **refuses and says so**; it does not stop the scan. Two reasons:

* stopping the scan would throw away the batches it has not committed yet -
  the scan commits in batches of 64 files, and a scan of a big tree would
  lose its work;
* the user asked for a removal, not for an abort. `Escape` stops the scan,
  and the action works once the worker is back.

The refusal is a note: *"a scan is still running: stop it (Escape) before
the missing entries are removed"*.

The guard is the window's own state (`worker.is_some()`): while the worker
holds the index, the window cannot query it, so the action has nothing to
work on.

## The report

The action reloads the page afterwards - the rows that are gone are not in
the list any more, and the total is smaller - and leaves a **note** for the
app to show (the app's notes are the player's user-facing messages; the
presenter prints them).

| situation | the note |
| --- | --- |
| the index holds no rows | `nothing to remove: the index holds no rows` |
| rows were checked | `removed 1 of 2 entries` |
| a path could not be checked | `; 1 kept, the file could not be checked: <path>` |

The report's three numbers are the index's own ([`RemoveReport`]): how many
rows were looked at (`checked`), how many were dropped (`removed`) and which
rows were kept because their file could not be checked
(`kept_unreadable`). The paths of the kept rows are named, because a row
that could not be checked is a row the user has to look at: the removal
cannot say whether it is dead.

The note is prefixed with `Library: ` by the app, like every other note of
the window.

## Tests (5 new, all headless)

`crates/rustamp-app/tests/library_window.rs`:

- `ctrl_m_removes_the_row_whose_file_is_gone` - a temp library with two
  tracks, one file deleted: the chord drops exactly that row, the view is
  reloaded (the total is smaller) and the report is a note.
- `ctrl_m_reports_a_path_it_cannot_check_and_keeps_the_row` - a row whose
  path the system refuses to name: the path is reported, the row stays in
  the index and in the list.
- `ctrl_m_refuses_while_a_scan_is_running` - the chord during a scan is
  refused with a note; the same chord works once the scan is over.
- `ctrl_m_on_an_empty_library_is_a_no_op_with_a_note` - an empty library
  changes nothing and says so.
- `the_plain_letter_removes_nothing` - the letter without the modifier does
  nothing at all.

The tests are headless: null output, the built-in skin, no OS window and no
child process.

### The dev-dependency

The test with the unreadable path needs a row whose path the file system
cannot even name - a path with a NUL byte in it. **No scan can produce such
a path**, so the row is written straight into the index's own database with
`rusqlite`, the crate `rustamp-library` already uses.

`rusqlite` is therefore a **dev-dependency of `rustamp-app`** (the same
version and the same features as the library crate uses, so the build is
shared). No new package enters `Cargo.lock` - the lock file grows by exactly
one line, the dependency edge - and no license changes: `rusqlite` is MIT
and already recorded in `THIRD_PARTY.md`.

## Remaining limits

- The removal is not undoable from the window: the rows are gone from the
  index, and a rescan does not bring them back.
- The scope is every registered root. The window cannot select a single
  root for the removal (the preferences page can remove a whole root).
- The note reaches the user through the app's note channel (the presenter's
  output), not through the window's caption: the caption carries the scan
  and its progress, and a note with a list of paths does not belong in a
  title bar.
- The window does not clear the selection when the selected row is removed.
  The selection is anchored to a row id, and a row that is not on the page
  is simply not selected any more - the same rule every other page change
  follows.
