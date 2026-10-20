# The media library window (M5, wave177)

## Scope

`rustamp-app` gains the **media library window**: the app side of the window
that the index (`rustamp-library`), the view model and the renderer
(`rustamp-ui`) were waiting for.

`rustamp-library` and `rustamp-ui` are untouched. `rustamp-platform` is
untouched as well: the modern window path needs `winit`'s
`WindowAttributes` and `rustamp_ui::shell::TaskbarRole`, and the platform
crate has neither - it is the layer below `winit`, not the layer above it.

Files:

- `crates/rustamp-app/src/library_window.rs` - the app's index, the view, the
  glue between them, the input map and the background scan.
- `crates/rustamp-app/src/modern_shell.rs` - the attributes of a window that
  is **not** skinned.
- `crates/rustamp-app/src/window.rs` - the presenter: the OS window, its
  surface, its input and the folder dialog it asks for.
- `crates/rustamp-app/src/app.rs` - the command arm, the `Alt+L` binding and
  the window's place in the focus cycle.
- `crates/rustamp-app/src/menu.rs` - the `Library` menu row.
- `crates/rustamp-app/src/focus.rs` - the focus cycle.
- `crates/rustamp-app/tests/library_window.rs` - the headless tests (16).
- `crates/rustamp-app/tests/library_window_os.rs` - the VM-only probe (1).

## Where the database lives

`library.sqlite3` **beside the configuration file** - the same directory as
`rustamp.toml`, which is `%APPDATA%\Rustamp` by default. The index is found
where the rest of the player's state is, travels with it and is removed with
it.

A player without a configuration file (no `%APPDATA%`) has nowhere to keep
one and opens an **in-memory** index instead, which lives exactly as long as
the run.

The index is opened when the window opens for the first time and kept for the
rest of the run; closing the window hides it, it does not throw it away.

## The command id

**Own choice.** S04 records the `Library` menu row (S04 §2 row 8) and its
accelerator hint `Alt+L` (S04 §3.3), but no public command id: S03 has no
row for it. The window therefore takes an id of its own, in the `Gen` range
that `WindowId` reserves for "generic / plugin window (`gen_*` plugins,
library, ...)".

| | |
| --- | --- |
| the id | `WindowId::Gen(0x4C49_4252)` (`rustamp_app::library_window::LIBRARY_WINDOW_ID`) |
| the number | the ASCII of `LIBR` - it cannot be mistaken for a plugin's own window number |
| the command | `Command::ToggleWindow(LIBRARY_WINDOW_ID)` |
| the keys | `Alt+L` (the S04 §3.3 accelerator) and the `Library` menu row |

The menu row is the one place where Rustamp invents a behaviour: S04 §9 does
not cover the click-through of the rows it does not measure, and the row now
opens the library window. The title row and `Minibrowser` keep their
draw-only treatment.

## The window

The library window is a **modern** window: it has a native title bar, a
system font and its own chrome, so no skin sheet applies to it. The classic
shell (`rustamp_ui::shell::skinned_window_attributes`) builds borderless,
fixed-size windows, so the library needs its own path.

`modern_window_attributes` starts from the shared shell and replaces the two
decisions that make a window modern (the decoration and the resizability).
It does **not** fork it: `winit::window::WindowAttributes` has no other
constructor, and `crates/rustamp-app/tests/window_shell.rs` fails if a
second `Window::default_attributes()` appears in the three window-owning
crates.

| | |
| --- | --- |
| the frame | the operating system's (decorated) |
| the size | 800 x 480 physical pixels when it opens, resizable afterwards |
| the taskbar | `TaskbarRole::Secondary` - no entry of its own |
| the position | the operating system's; it is not part of the docked stack |
| the focus | the last window of the focus cycle (own choice, like the rest of the cycle) |

A resize reaches the view: the number of rows the window shows is the
height of the list band, and the page is loaded again at the new offset.

## The glue

```
LibraryView::request()  ->  Query  ->  Library::query()  ->  TrackPage
                                                                  |
                                 LibraryView::apply_page(rows, total, offset)
```

The request is **scroll-aligned**: the view asks for the rows at its scroll
offset, and the app loads exactly those. The app adds nothing to the query:
the search term, the sort column, the direction and the window of rows are
the view's own. A match set that shrank clamps the scroll, and the clamped
offset is asked for again (at most twice).

## The background scan

The app owns the thread. The index travels to the worker and comes back with
it: a scan writes to the database, and the app must not query it while that
happens. While the scan runs the view keeps the page it has; when the
worker returns, the app takes the index back, reloads the page and shows the
result.

The progress is a **single latest-value slot** (`Arc<Mutex<Option<ScanProgress>>>`),
not a queue: the interface shows the newest value and the older ones are
worthless. `Escape` asks the scan to stop; the cancel flag is checked between
two files, so the batches that were committed stay in the index.

The worker is stopped and joined when the window is closed and when the app
goes away, so no thread outlives the database.

## The input map

Own choice: S20 (the media library) is not started, so no spec measures it.
The map follows the shape of the window - a search field above a list.

| Input | Effect |
| --- | --- |
| a printable character | appends to the search text, while the search field has the focus |
| `Backspace` | deletes the last character of the search text, while the field has the focus |
| `Tab` | moves the focus between the search field and the list |
| `Up` / `Down` | moves the selection one row |
| `PageUp` / `PageDown` | moves the selection one screenful |
| `Home` / `End` | moves the selection to the first / last row |
| `Enter` | plays the selected row (`Command::AddItems { play: true }`) |
| `Escape` | stops the running scan |
| `F5` | scans the registered roots again |
| `Ctrl+F` | focuses the search field |
| `Ctrl+M` | removes the rows whose file is gone (see `m5-library-remove-missing.md`) |
| `Ctrl+O` | asks for a folder to add to the library |

The movement keys always move the selection, whether the search field has
the focus or not: the field holds one line of text, and the list is what the
window is for. With no selection the movement starts at the visible rows,
like the playlist editor does (`Down` takes the first visible row, `Up` the
last one).

The mouse: a press in the search band focuses the field, a press in a column
header sorts by that column (the same column again reverses it), a press in
the scrollbar scrolls to the place it sits at (`scroll_for_click`), and a
press in a row selects it. The wheel scrolls by rows.

`Enter` plays the selected row through the **existing** path: the app emits
`Command::AddItems { items: [MediaLocation::File(path)], at: None, play:
true }` and submits it, so the track is queued and started like any other.

## What is VM-only

Everything that needs a desktop:

| | |
| --- | --- |
| the window itself | `tests/library_window_os.rs`, `#[ignore]`d, gated by `require_test_vm()` |
| the native frame | the probe reads back the outer size: a modern window's outer size is **larger** than its client area (the classic windows' check is the opposite: borderless means equal) |
| the folder dialog | `Ctrl+O` asks the operating system; the app only records the request |
| the drawn frame | the renderer is covered by the headless render tests; the pixels on the screen are not |

The headless suite covers the model: the command, the glue, the input map
and the path from `Enter` to playback.

## Tests

`crates/rustamp-app/tests/library_window.rs` (16, all headless):

- the command opens and closes the window, and the database lands beside the
  configuration file,
- the window id is a `Gen` window of its own,
- `Alt+L` opens the library,
- the request becomes a query and the rows become the page,
- the search field narrows the match set, and `Backspace` widens it again,
- the text only reaches the field that has the focus,
- the movement keys move the selection,
- a press on a row selects it,
- a press on a column header sorts by that column,
- a press in the scrollbar scrolls to the place it sits at,
- `Ctrl+O` asks for a folder, and the request is taken exactly once,
- `Ctrl+F` focuses the search field,
- `Enter` plays the selected row through the `AddItems` path, and the track
  is in the queue,
- `Enter` without a selection plays nothing,
- the window renders a frame of its client size, and its caption names the
  library.

`crates/rustamp-app/tests/library_window_os.rs` (1, `#[ignore]`d, VM-only):
the real window, its client size and its native frame.

The fixture is the decoder crate's `sine.mp3`; the test copies it and writes
its own tags, so the rows can be told apart. Nothing is downloaded and no
file outside the temp directory is touched.

## Remaining limits

- The window's size, the column widths, the colours and the whole input map
  are **own choices**: S20 is not started, so nothing here is measured.
- The input map grew after this lane: `Ctrl+M` removes the rows whose file
  is gone (`docs/notes/m5-library-remove-missing.md`).
- The search is the index's free-text search (a case-insensitive substring
  of title, artist or album). There is no filter by artist, album or genre
  yet - the view does not own those.
- The index is opened lazily, so a player that never opens the library never
  creates the database file.
- The scan walks the whole tree of a root; there is no "watch the folder"
  mode and no incremental update.
- The window is not skinned and has no skin-aware behaviour: it does not
  follow the player's double-size mode, and it is not part of the docked
  stack.
- The scan's progress is shown in the window's caption and in its status
  line, not in a progress bar inside the window (see
  `m5-library-scan-progress.md`).
