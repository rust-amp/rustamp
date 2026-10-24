# The last scan of a media library root (wave237)

## What the page shows

The **Preferences** dialog's `Library` page lists the roots of the media
library. Every row now carries the time of the root's last **finished**
scan:

```
C:\Music — last scanned 2026-10-20 03:11 UTC
D:\Tunes — never scanned
```

The page is the place for it: the media library **window** shows tracks, not
roots, so the roots are only listed in the preferences dialog.

The row is built in one place, `rustamp_platform::library_root_text`, and the
dialog's list box shows exactly what it returns.

## The format

| | |
| --- | --- |
| the time zone | **UTC** - the text does not depend on the time zone of the machine that shows it |
| the resolution | the **minute** - the row says when the root was looked at, and a minute is what that means for the user |
| the order | the path first, then the time - the path is what the row is about |

`None` means "never scanned". A scan that was **cancelled** or that
**failed** does not count: it did not finish, so it must not claim that the
root was scanned. The index keeps the time of the last finished scan in that
case (`wave227`).

## Where the time comes from

The time is in the index: `Root::last_scanned_at` is the unix second of the
root's last finished scan, and `Library::roots()` returns it.

The app had to change one thing for the column to have something to show: the
scan ran `Library::scan_with_progress(path)`, which works on an arbitrary path
and therefore records no time. The scan now runs
`Library::scan_root_with_progress(id)`, the call that records the time of a
root that ran to the end. The scan of a registered root is the same scan; only
the call changed.

The app keeps the roots (`LibraryWindow::roots`) as the index's `Root`
records, not as bare paths, so the time travels with the path. The list is
read again when the worker gives the index back, because a finished scan
changed the time.

## The model

`rustamp_platform::PrefsSettings::library_roots` holds one
`LibraryRootRow` per root:

| | |
| --- | --- |
| `path` | the root's path, as it was added |
| `last_scanned_at` | the unix second of the last finished scan, `None` when there was none |

`PrefsSettings::library_root(row)` still answers the **path** of a row: the
"Remove" button names the root it removes.

## The long path

A row is one line, and a path can be longer than the list. The list box
carries a horizontal scroll bar, and its scroll range is now the width of the
widest row (measured with the list's own font), never less than the constant
that was there before. A long path therefore stays readable: the user scrolls
the row sideways.

## What is VM-only

| | |
| --- | --- |
| the list with the times | the dialog is a native window: `crates/rustamp-app/tests/library_window_os.rs` and the VM's own dialog run |
| the scroll range | the measurement needs the list's device context, so it happens with the window |
| the text of a row | the headless suite covers it: the formatting is a pure function |

## Tests

`crates/rustamp-platform/src/prefs_dialog.rs` (unit tests):

- `a_root_that_was_never_scanned_says_so`,
- `a_root_names_the_utc_minute_of_its_last_scan`,
- `the_row_text_carries_the_path_and_the_time`.

`crates/rustamp-platform/src/utc.rs` (unit tests): the calendar - the epoch, a
known moment, a leap day, the seconds before the epoch and every day of a
year.

`crates/rustamp-app/tests/library_prefs.rs` (integration tests, headless):

- `a_root_that_was_never_scanned_carries_no_time`,
- `a_scanned_root_carries_the_time_of_its_last_scan`,
- `the_time_of_a_root_survives_a_restart`.

## Remaining limits

- The time is in **UTC**, not in the time zone of the user. A relative time
  ("5 minutes ago") would need the clock of the machine that shows it, and
  the row would change every second; the absolute time is the same text on
  every machine.
- The minute is the resolution: two scans in the same minute are the same
  text. The index keeps the second.
- The row is one line, so the time of a long path is off the screen until the
  user scrolls the row sideways. A second column would show both at once; the
  list box is a plain one and has no columns.
- The time is only what the app's own scan recorded. A root that was scanned
  by another program (the index file is shared) keeps the time that program
  wrote, if it wrote one.
