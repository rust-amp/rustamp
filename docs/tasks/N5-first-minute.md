# Task card N5 - The first minute: open a file, see errors, working controls

**Lane** N - application. **Findings addressed:** BETA-03, BETA-04, BETA-06
(and the visibility half of BETA-07, which closing the playlist needs) from
[BETA-readiness.md](BETA-readiness.md).

**Owned paths:** `crates/rustamp-app/src/**`, `crates/rustamp-app/tests/**`,
`crates/rustamp-ui/src/**`, this card. No engine, output, skin, IPC or
playlist-crate change: everything below is app/UI-side.

**Verification:** `cargo fmt --all -- --check`, `cargo clippy --workspace
--all-targets --all-features -- -D warnings` and `cargo build --workspace`
pass on the host. **No test was run on the host** (the suite runs in the
Hyper-V test VM); the new tests are headless (null output, no window) and
still need their VM run.

## 1. BETA-03 - one honest path from the GUI to a file

**Before.** Eject emitted nothing. ADD opened the open-media files face, but
its field took letters, digits and spaces only, so `C:\Music\song.mp3` could
not be typed; picks arrived only through the test-only `stage_media_paths`.

**Now.**

- **Eject** emits `OpenMedia { kind: Files, play: true }` (S03 §5 "Click the
  eject button: open file(s)", id 40029, RFC 0005 mapping)
  (`crates/rustamp-ui/src/behaviour.rs`, `release_command`).
- **ADD** still opens `OpenMedia { kind: Files, play: false }` (RFC 0005 O5
  unchanged).
- **Typing:** the presenter sends the OS text of each key press
  (`KeyEvent::text`: layout-aware, with case, `:`, `\`, `.`, AltGr
  characters) to `App::dialog_text` -> `DialogManager::type_text` whenever
  the focused dialog has a field; control characters fall through to the key
  path. `Backspace` is a new `Key` and deletes the last character. Ctrl or Alt
  alone stay shortcuts; Ctrl+Alt (AltGr) is text. The jump box takes typed
  `:` too, so `m:ss` is now typeable.
- **Dropping:** a file dropped from Explorer onto the open-media dialog
  (`WindowEvent::DroppedFile`) is staged as a pick (`App::dialog_drop_file`);
  `Enter` confirms.
- The face shows the field's tail when it is longer than the line, and a
  one-line hint (`Type a path or drop files, Enter adds.`).
- `Enter` confirms through the existing `AddItems` path; the entry appears
  in the playlist at once; `play: true` (Eject) starts it, `play: false`
  (ADD) waits for a double-click.

**Not delivered, said plainly:** no OS file picker and no clipboard paste.
Both need a Win32 binding (`IFileOpenDialog`, the clipboard API) that
`rustamp-app` does not have; adding the `windows` crate to the app is a new
dependency, which this lane may not add. Dropping a file is the no-typing
route. Dropping onto the main or playlist window stays unwired: S13
(drag-and-drop rules) is not started.

**Tests** (`crates/rustamp-app/tests/first_minute.rs`):
`eject_then_a_typed_path_queues_and_plays_the_file` (click Eject, type a real
temp path key by key with a Backspace correction, Enter: queue 1, playlist
row, engine and UI `Playing`, timeline moving, marquee names the track),
`add_then_double_click_plays_the_typed_file`,
`a_file_dropped_on_the_dialog_is_queued_and_played`. Unit tests in
`rustamp-ui/src/dialogs.rs`: `a_full_windows_path_can_be_typed_and_corrected`,
`the_jump_box_takes_typed_digits_and_colons_only`,
`dropped_files_become_picks_on_the_file_faces_only`; `rustamp-app/src/dialogs.rs`:
`a_long_path_shows_its_tail`.

## 2. BETA-04 - no phantom playback, errors reach the user

**Before.** An open failure (missing path, no decoder) made the decode worker
publish `Event::Error` and exit without an end of stream, so the engine stayed
`Playing` over silence; both the app's and the UI's event handlers ignored
`Event::Error`. Play on an empty queue lit `Playing` although the engine
refused it.

**Now** (`App::surface_error`, `crates/rustamp-app/src/app.rs`), on every
`Event::Error`:

- `FormatChanged` is a notice (the run continues): noted only.
- Anything else is noted as `error: ...` (the presenter prints notes to
  stderr) and kept as `App::last_error`, naming the entry:
  `cannot play C:\x\song.mp3: <reason>`. Play on an empty queue says
  `nothing to play: the playlist is empty (Eject or ADD opens a file)`.
- A running transport is **stopped**, unless the run has already reported
  its end of stream (it then finishes by itself). The UI's transport is
  re-synced to the engine's, so the optimistic `Playing` goes out.
- The error is cleared when a track actually starts (`TrackChanged` with an
  index).

**What a user sees:** the transport lights return to stopped and the time
display blanks; the main window's **native title bar** reads
`Rustamp - cannot play C:\Music\song.mp3: <reason>` until a track starts; a
console shows the same line. The title is an own choice (no spec gives the
player an error surface); it is legible whatever the skin, whereas the
built-in placeholder font is not (BETA-02).

**Deliberately broad, stated:** the app cannot tell an open failure from a
mid-track read failure (the engine reports both the same way, and
`rustamp-audio` is not this lane's). A mid-track read failure therefore also
stops the run instead of advancing to the next entry. That is a defined state
and Play retries; a precise rule needs an engine signal for "the worker
exited without an end of stream" (engine lane). Device loss (BETA-05/H4) is
not covered: the output layer raises no error event yet.

**Tests:** `a_missing_file_stops_the_transport_and_says_why`,
`an_unsupported_file_typed_into_the_dialog_stops_and_says_why` (also: a good
file afterwards plays and clears the title),
`play_on_an_empty_queue_does_not_light_playing`.

## 3. BETA-06 - the controls that look real

### Main window

| Control | Before | Now | Basis |
|---|---|---|---|
| Menu button | main menu | unchanged | S04 §1 |
| Minimize | dead | `Minimize` -> OS minimize | S03 §5 |
| Shade | dead | `ToggleWindowShade(Main)`, 275x14 and back | S03 §5, S05 §1.3 (measured) |
| Close | dead | `Quit` | S03 §5, RFC 0005 O3 (main close quits) |
| Eject | dead | open file(s) dialog | S03 §5, RFC 0005 (40029) |
| Right-click | nothing | main menu; on EQ/PL button toggles that window | S03 §5 (verified / menu) |
| Prev/Play/Pause/Stop/Next, volume, balance, seek, EQ, PL, time display, A, D, I | worked | unchanged; PL now really shows/hides the playlist window | S02, S03 |
| Shuffle / Repeat | flags only | unchanged (BETA-08, engine) | S15 open |

### Playlist window

| Control | Before | Now | Basis |
|---|---|---|---|
| Close (skinned and OS close box) | note / "stays open" | hides the window (`ToggleWindow(Playlist)`); PL button and menu row reopen it | RFC 0005 `CloseFocusedWindow` (toggle windows hide), S03 §2 Ctrl+F4 |
| ADD, REM, SEL, scroll, list click, double-click | worked | unchanged | S06 |

To make Close real, the playlist editor's OS window now follows
`playlist_open` like the equalizer's (`Windowed::sync_playlist_window`), and
the model starts **open**, matching the window the presenter opens at launch
(`tests/main_menu.rs` updated: the check mark used to say closed while the
window was up). Tests: `the_main_window_buttons_act`,
`a_right_click_opens_the_main_menu_or_toggles_eq_and_pl`,
`the_playlist_close_hides_it_and_the_pl_button_brings_it_back`,
`playlist_window.rs::the_title_bar_close_hides_the_editor_and_pl_reopens_it`,
`the_title_bar_shade_says_why_it_does_nothing`;
`behaviour.rs::every_interactive_widget_emits_its_command` extended.

### Left alone, and what each waits for

| Control | Why it is not wired | Waiting for |
|---|---|---|
| Playlist Shade | the editor's shade size and drawing are unmeasured; a note says so | S05 §1.5 / §9 item 3 (a capture of the PE shade) |
| Clutter O, V | their popups' contents are unmeasured | S04 §9 (submenu contents) |
| Main-menu submenu rows (Play, Options, Playback, Visualization, Skins, Bookmarks) | contents unmeasured; a note names the row | S04 §9 |
| Main-menu `Exit` | no S03 command id; click-through unmeasured | S04 §9 or an RFC 0005 row mapping Exit -> `Quit` |
| Shift+click Eject (open directory), Shift/Ctrl+click on transport | the UI machine gets no modifier state with pointer input; modifier clicks not exercised | S03 §6 Q3 (and a pointer-modifier path in `MainWindow`) |
| Drag the skinned title bar, double-click it to shade | settled by S03 §5 but needs presenter gesture work (`drag_window`, double-click timing, the shaded middle band); the native title bar still moves the window | presenter work, no spec gap |
| Right-click on the title, time display, seek bar; right-click a playlist entry | the menus exist (S03 §5) but their contents are S04's | S04 |
| MISC popup | contents are a visual reading only | S06 §4 |
| LIST OPTS confirm | replace vs append open | RFC 0010 (BETA-09) |
| Files dropped on the main/playlist window | rules unspecified | S13 (not started) |

## 4. Remaining risk

- None of the new tests has run; they need the VM suite.
- The phantom-playback fix is app-side and coarse (see section 2).
- `WindowEvent::DroppedFile` relies on `winit`'s default OLE drag-and-drop
  on Windows; not exercised outside the VM.
