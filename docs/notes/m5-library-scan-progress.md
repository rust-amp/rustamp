# The media library shows the scan's progress (M5, wave229)

## Scope

`rustamp-app` gains the **status line** of the media library window: while a
scan runs, the window says what the scan is doing, and a scan that was
stopped says what it committed.

The index's side of it already existed - `rustamp_library::Library::
scan_with_progress`, `ScanProgress`, `ScanPhase`, `Cancel` and the batch
commits (`wave174`). The window ran that scan since `wave177`, but it showed
the progress **in the caption only** and it kept the newest event in a shared
slot that the interface thread polled.

`rustamp-library` and `rustamp-ui` are untouched: the crate that owns the
library window's renderer is owned by another lane, so the app owns the
status line itself.

Files:

- `crates/rustamp-app/src/library_window.rs` - the progress channel, the
  status line, the painting of the status band and the caption.
- `crates/rustamp-app/src/library_window_tests.rs` - the headless tests with a
  fake scan (6 new).
- `crates/rustamp-app/tests/library_window.rs` - the headless test of the app's
  own path (1 new).

## What existed

| | |
| --- | --- |
| the scan | `LibraryWindow::start_scan` runs the roots on the app's own worker thread |
| the events | one shared slot (`Arc<Mutex<Option<ScanProgress>>>`), written by the worker and read by the interface thread |
| `Escape` | `Key::Escape` -> `cancel_scan()` -> the `Cancel` flag; the scan stops at the next file boundary |
| the caption | `LibraryWindow::title()` showed the phase and the number of files seen |
| the status line | **nothing**: the renderer drew the view's status line (`1-20 of 1234`) and nothing else |

So the scan's progress never reached the window's status line, and the
committed report of a cancelled scan was thrown away.

## The status-line rules

The status line is the app's own text now
(`LibraryWindow::status_line()`):

| the state | the status line |
| --- | --- |
| no scan | the visible range of the list (`1-20 of 1234`) |
| a scan that runs | `scanning: 120 seen, 30 added, 5 updated, 2 skipped (Esc stops the scan)` |
| a scan that is over | the visible range of the list again |
| a scan that was cancelled | `cancelled: 12 added, 3 updated, 1 removed, 2 skipped (committed)` |

The rules behind the table:

* **the phase** is the scan's own (`scanning`, `removing`); the counters are
  the **live** ones of the newest event,
* the progress belongs to a scan that **runs**: the worker is alive and the
  scan did not reach its end yet. The `Finished` phase ends it, and the
  visible range is the status line again,
* a **cancelled** scan says so and shows the **committed** counters - the
  ones its `ScanReport` carries. The live counters include the batch that is
  in flight, so they can be ahead of the index by one batch, and that batch
  was never written.

The caption carries the same text while a scan runs, so the progress is
visible even when the window is not in front.

## The cancelled case

`Escape` asks the scan to stop. The scan finishes the file it is on, drops
the batch in flight and returns the report of what it **committed**. The
worker gives that report back with the index, and the window keeps it: the
status line shows the cancelled state with the committed numbers until the
next scan starts.

The report is the honest one: `ScanReport::added`, `updated`, `removed` and
`skipped` count the batches that were written, and `cancelled` says that the
scan was stopped. The live counters of the scan are never shown for a
cancelled scan.

## The events travel over a channel

The shared slot became a **channel** (`std::sync::mpsc`): the worker sends
the events and the interface thread takes them in `LibraryWindow::poll()`,
which the app's pump calls every tick. Neither side waits for the other -
the worker never blocks on the interface, and the interface never blocks on
the worker. Only the newest event is kept: the older ones are worthless.

The scan itself is behind the `ScanJob` trait (`RootScan` in production), so
that a test can run a **fake** scan: a real scan of a directory is fast and
hard to steer, and the status line must be proven for every phase and for
the cancelled case.

## The band is painted by the app

The renderer draws the status line from the view
(`LibraryView::status_line()`), and the view's status line is the visible
range: it cannot carry the scan's counters. The renderer has no other public
text path, and `rustamp-ui` is owned by another lane, so the app paints the
band itself: it asks the renderer for the glyphs - a scratch frame whose
search field holds the text, rendered with the same font and the same size as
the rest of the window - and paints the ink of that field into the band, in
the status line's own colour and over the band's own background.

The scratch frame holds the search band alone: the rest of the window is of
no use there, and a whole frame per paint would cost more than the text it
carries. The band is only painted while the scan has something to say.

## Tests

`crates/rustamp-app/src/library_window_tests.rs` (6 new, all headless, with a
fake scan):

- a running scan shows its phase and its counters,
- the `Finished` phase restores the normal status line,
- a cancelled scan shows the cancelled state with the committed numbers (and
  not with the live ones),
- without a scan the status line is the visible range,
- the status band is painted with the progress, at the left edge like the
  visible range,
- the caption carries the scan's progress.

`crates/rustamp-app/tests/library_window.rs` (1 new, headless): without a
scan the app's status line is the visible range (`1-3 of 3`).

## Remaining limits

- The status line is the app's text, and the app has to paint it: the
  renderer would be the better home for it, but `rustamp-ui` is owned by
  another lane.
- The counters of a running scan are the live ones, so they can be ahead of
  the index by one batch. The report of a finished scan is the honest
  number, and it is not shown: a finished scan shows the visible range
  again.
- The cancelled state stays until the next scan starts; there is no "dismiss"
  for it.
- The scan's progress is still not a **progress bar**: the window has no
  percentage and no estimate, because the scan does not know how many files
  the tree holds.
