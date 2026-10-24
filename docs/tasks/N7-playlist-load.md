# Task card N7 — Playlist file loading and skip reporting (wave 54)

**Scope**: `crates/rustamp-app/src/**`, `crates/rustamp-app/tests/**`, `docs/tasks/N7-playlist-load.md`.

## 1. Problem and Root Cause Analysis (What Happened Today)

Before this task, playlist loading in `rustamp-app` was an absolute dead end. Specifically:

1. **LIST OPTS button mapping**:
   - In `crates/rustamp-app/src/playlist_window.rs:222-230`, clicking `PlaylistButton::ListOpts` called `self.playlist.press_list_opts()`, returning `PlaylistDialog::LoadPlaylist`, which emitted `Command::OpenPlaylist`.
   - `Command::OpenPlaylist` was routed to the app (`crates/rustamp-app/src/app.rs:1336`), calling `self.open_dialog_for(Command::OpenPlaylist)` to open `DialogKind::LoadPlaylist`.

2. **Empty dialog body**:
   - In `crates/rustamp-app/src/dialogs.rs:350`, `dialog_body(DialogKind::LoadPlaylist)` was hardcoded to:
     ```rust
     DialogKind::LoadPlaylist => vec!["Playlist file:".to_string(), String::new()],
     ```
     The second line was an unconditionally empty string; it displayed neither typed text nor staged/dropped paths.

3. **No input or confirmation mapping**:
   - In `crates/rustamp-app/src/dialogs.rs:135-164`, `dialog_key(Key::Enter)` handled `Key::Enter` for Jump, SkinBrowser, and OpenMedia, but had no branch for `LoadPlaylist`.
   - In `crates/rustamp-ui/src/dialogs.rs:498-503`, `accepts_text()` returned `false` for `LoadPlaylist`.
   - In `crates/rustamp-ui/src/dialogs.rs:744-746`, `confirm_load_playlist()` returned `None`.
   - Pressing Enter simply consumed the key without closing the dialog or initiating any action.

4. **CLI startup path bug**:
   - In `crates/rustamp-app/src/app.rs:189-191`, starting Rustamp with a file via CLI (`rustamp <file>`) directly called `engine.set_queue(vec![TrackSource::path(file.clone())])`. If `<file>` was a playlist (`.m3u`, `.pls`, `.xspf`), it was treated as a single audio file and immediately failed to decode.

5. **No connection to reader skip reports**:
   - `rustamp-app` never invoked `rustamp_playlist::load_playlist_file_report` or `load_playlist_file`. The skip diagnostics created in task card E3 existed only in `rustamp-playlist` and were completely unreached.

## 2. Implementation: Connecting the Dead End

### 2.1 Dialog Text, Staging, and Confirmation

In `crates/rustamp-app/src/dialogs.rs`:
- Implemented `playlist_dialog_text: String` and `playlist_dialog_path: Option<PathBuf>` on `App`.
- Updated `dialog_accepts_text()` and `dialog_text()` to accept text input for `DialogKind::LoadPlaylist`.
- Supported `Key::Backspace`, `Key::Digit`, `Key::Letter`, `Key::Space` for typing paths.
- Supported drag-and-drop file staging via `dialog_drop_file()`.
- Implemented `confirm_load_playlist(&mut self) -> bool`:
  - If no path is staged or typed, returns `false` (keeps dialog open so user can retry, preserving test `the_load_playlist_dialog_keeps_its_confirmation_open`).
  - If a path is provided, calls `self.load_playlist_file(&path, true)`, clears the dialog fields, closes the dialog, and returns `true`.
- Updated `dialog_body` via `load_playlist_lines(&self)`: shows `"Playlist file:"`, the staged/typed path tail, and the hint `"Type a path or drop a file, Enter loads."`.

### 2.2 App-Level Playlist Loading

In `crates/rustamp-app/src/app.rs`:
- Added `pub fn load_playlist_file(&mut self, path: &Path, play: bool) -> Result<rustamp_playlist::PlaylistReport, rustamp_playlist::PlaylistError>`:
  - Invokes `rustamp_playlist::load_playlist_file_report(path)`.
  - Replaces current engine queue with surviving entries in order (`TrackSource::path` or `TrackSource::Url`).
  - Calls `self.sync_entry_ids()` to synchronize UI playlist representation.
  - If `play` is true and at least one entry survived, initiates playback of track 0 via `self.engine.play_index(0)`.
  - Records the report on `self.last_playlist_report`.
- Updated `App::with_registry`:
  - When `options.file` is passed and has a playlist extension (`is_playlist_file`), loads the playlist into the queue using `app.load_playlist_file(file, false)`.
- Updated `add_items`:
  - If an item added via `Command::AddItems` is a playlist file, expands the playlist and appends surviving entries to the queue.

### 2.3 Skip Report Surfacing

When a playlist has skipped entries (`!report.is_clean()`):
1. The report is recorded in `App::last_playlist_report`, accessible via `pub fn last_playlist_report(&self) -> Option<&PlaylistReport>`.
2. Following the recommendation from task card E3 §4.2, the skips are surfaced via `self.note(...)`:
   - Summary line: `loaded N tracks, skipped M malformed entries from <path>`
   - Individual skip lines: `  - skipped: <reason>`
3. Notes are buffered in `self.notes`, accessible headlessly via `app.drain_notes()`, and printed directly to `stderr` in windowed mode by `window.rs`.

**Why this location was chosen**:
- Rustamp does not yet have a rich in-window toast/notification system or detailed status line for log diagnostics.
- Logging to `notes` (and thus `stderr`) ensures full visibility without altering existing skinned window layout geometry.
- Storing the structured report on `App` ensures automated tests, scripts, and future UI elements can inspect detailed skip metadata programmatically.
- **Spec status**: No spec settles skip report placement. S06 §4 settles only the title `"Load Playlist"`.

### 2.4 Error Handling for Malformed or Unsupported Files

When loading a file that cannot be parsed (e.g. invalid bytes, unknown format, or unresolvable structure):
- `load_playlist_file` receives `Err(PlaylistError)`.
- The current player queue is **left completely unchanged** (no destructive side-effects).
- The error is surfaced visibly:
  1. Set on `self.last_error = Some(msg)`.
  2. Surfaced in the native window title bar via `app.window_title()` (`Rustamp - <error>`).
  3. Logged to `self.note(format!("error: {msg}"))`.
  4. Published to the event bus as `Event::Error`.
- Returns `Err(err)` to the caller.

## 3. Headless Tests

Added headless unit tests in `crates/rustamp-app/src/app.rs`:

1. `good_playlist_file_loads_and_plays`:
   - Writes a valid M3U file with two entries (`track1.tst`, `track2.tst`).
   - Opens Load Playlist dialog, stages file path, presses Enter.
   - Verifies queue receives both entries in order, playback begins on track 0, dialog closes, and report is clean with 0 skips.

2. `playlist_file_with_bad_entry_loads_surviving_and_reports_skips`:
   - Writes an M3U file containing a dangling EXTINF directive and two valid entries.
   - Loads via dialog; verifies surviving entries reach queue in order and playback starts.
   - Verifies `last_playlist_report()` records 1 skip with reason `"dangling EXTINF"`.
   - Verifies `drain_notes()` contains skip summary and detailed skip reason.

3. `unsupported_playlist_file_reports_error_and_changes_nothing`:
   - Starts with 1 track in queue.
   - Attempts to load corrupted/binary file with `.m3u` extension.
   - Verifies queue remains unchanged with the original track.
   - Verifies `last_error` is set, `window_title()` reflects the failure, and note is logged.

4. `load_playlist_dialog_text_input_and_display`:
   - Tests typing characters into dialog, deleting via Backspace, and verifying dialog body lines and hint.

## 4. Own Choices

1. **Queue semantics (Replace vs Append)**:
   - `Load Playlist` (via dialog or `load_playlist_file`) replaces the queue, matching classic Winamp 2.95 behavior where loading a playlist starts a new playlist session.
   - `AddItems` appends to the queue and expands playlist files if given.
2. **Empty dialog confirmation**:
   - If Enter is pressed when nothing is typed or staged, `confirm_load_playlist` returns `false` and leaves the dialog open, avoiding accidental queue clears.
3. **Skip and error placement**:
   - Skips surface in `self.note(...)` (stderr in windowed runs) and in `app.last_playlist_report()`.
   - Malformed/unsupported file errors surface in `self.last_error` (native window title), `self.note(...)` (stderr), and `Event::Error`.
   - Dialog prompt wording: `"Playlist file:"` and hint `"Type a path or drop a file, Enter loads."`.
4. **CLI startup behavior**:
   - If CLI argument `options.file` has a recognized playlist extension (`.m3u`, `.m3u8`, `.pls`, `.xspf`, `.b4s`), it is loaded as a playlist rather than a single audio file.
