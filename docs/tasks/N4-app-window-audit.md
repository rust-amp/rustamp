# Task card N4 — Application window and command layer audit

**Goal**
Audit the application layer of the Rustamp project against its specifications (S02, S03, S04, S05, S06), fix what the specifications settle, and document the findings across window/dialog reachability, command routing, focus/window state, dialog lifecycles, and unimplemented notes.

**Lane** N — application.
**Owned paths**
- `crates/rustamp-app/src/**`
- `crates/rustamp-app/tests/**`
- `crates/rustamp-ui/src/**` (accessors / behavioural fixes)
- `docs/tasks/N4-app-window-audit.md`

---

## 1. Window and Dialog Reachability and Behaviour (S04 Menu Table & S02 Behaviour List)

### S04 Menu Table Walk
Every item in S04 §1 / §2 / §3:

| Spec Item / Menu Entry | Command / Action | Status | Location / Notes |
|---|---|---|---|
| Nullsoft Winamp... (About) | `Command::OpenAbout` | Implemented | `crates/rustamp-app/src/dialogs.rs:66`, `crates/rustamp-ui/src/dialogs.rs:56`. Opens `DialogKind::About` (260x120). |
| File -> Play file... | `Command::OpenMedia { kind: Files, play: true }` | Implemented | `crates/rustamp-app/src/dialogs.rs:66`, `crates/rustamp-ui/src/dialogs.rs:64`. Opens `DialogKind::OpenMedia` (320x180). |
| File -> Play directory... | `Command::OpenMedia { kind: Directory, play: true }` | Implemented | `crates/rustamp-app/src/dialogs.rs:66`. Opens `DialogKind::OpenMedia` (320x180). |
| File -> Play location... | `Command::OpenMedia { kind: Location, play: true }` | Implemented | `crates/rustamp-app/src/dialogs.rs:66`. Opens `DialogKind::OpenMedia` (320x180). |
| File -> View file info... | `Command::OpenFileInfo` | Implemented | `crates/rustamp-app/src/dialogs.rs:66`, `crates/rustamp-ui/src/dialogs.rs:58`. Opens `DialogKind::FileInfo` (280x150). |
| File -> Jump to file... | `Command::OpenJump { target: File }` | Implemented | `crates/rustamp-app/src/dialogs.rs:66`. Opens `DialogKind::Jump(JumpTarget::File)` (300x160). |
| File -> Jump to time... | `Command::OpenJump { target: Time }` | Implemented | `crates/rustamp-app/src/dialogs.rs:66`. Opens `DialogKind::Jump(JumpTarget::Time)` (240x80). |
| Play -> Previous / Play / Pause / Stop / Next | Transport commands | Implemented | `crates/rustamp-ui/src/menu.rs:188-192`, `crates/rustamp-app/src/routing.rs:32-38`. Dispatched to `Engine`. |
| Options -> Preferences... | `Command::OpenPrefs` | Implemented | `crates/rustamp-app/src/dialogs.rs:66`. Opens `DialogKind::Preferences` (360x240). |
| Options -> Skins -> Skin Browser... | `Command::OpenSkinBrowser` | Implemented | `crates/rustamp-app/src/dialogs.rs:66`. Opens `DialogKind::SkinBrowser` (280x200). |
| Options -> Skins -> (Select skin) | `Command::LoadSkin(path)` | Implemented | `crates/rustamp-app/src/app.rs:1072`. Loads skin and updates windows. |
| Options -> Equalizer | `Command::ToggleWindow(Equalizer)` | Implemented | `crates/rustamp-app/src/app.rs:992`. Toggles visibility in app and UI. |
| Options -> Playlist Editor | `Command::ToggleWindow(Playlist)` | Implemented | `crates/rustamp-app/src/app.rs:996`. Toggles visibility in app and UI. |
| Options -> Double Size | `Command::ToggleDoubleSize` | Implemented | `crates/rustamp-app/src/app.rs:1003`. Toggles 2x scaling on Main and EQ. |
| Options -> Always on Top | `Command::ToggleAlwaysOnTop` | Implemented | `crates/rustamp-app/src/app.rs:1020`. Toggles always-on-top level. |
| Bookmarks -> Bookmark Editor / Add Current | `Command::OpenBookmarkEditor`, `Command::AddBookmark` | Missing dialog | `crates/rustamp-app/src/app.rs:1053`. Records note (dialog not specified in S04). |
| Visualisation -> Select / Toggle | `Command::ConfigureVisPlugin`, `Command::ToggleVisPlugin` | Missing plugin host | `crates/rustamp-app/src/app.rs:1064`. Records note (requires M3 plugin host). |
| Exit | `Command::Quit` | Implemented | `crates/rustamp-app/src/app.rs:983`. Sets quit flag. |

### S02 Behaviour List Walk
- **Title bar buttons**: Menu (`Widget::WindowMenu` -> `Command::OpenMainMenu`), Minimize (`Widget::WindowMinimize` -> `Command::Minimize`), Shade (`Widget::WindowShade` -> `Command::ToggleWindowShade(Main)`), Close (`Widget::WindowClose` -> `Command::Quit`). Implemented (`crates/rustamp-ui/src/behaviour.rs:443`, `crates/rustamp-app/src/window.rs:412-425`).
- **Clutterbar**:
  - `O`: momentary click pops options menu. Implemented (`behaviour.rs:451`).
  - `A`: latches always on top (`Command::ToggleAlwaysOnTop`). Implemented (`behaviour.rs:438`).
  - `I`: left click opens File Info box for current track; if no track loaded, nothing happens (S02 §2). **Fixed:** Previously returned `None`. Now checks `if self.track_title.is_some() { Some(Command::OpenFileInfo) } else { None }` in `behaviour.rs:440`.
  - `D`: latches double size (`Command::ToggleDoubleSize`). Implemented (`behaviour.rs:439`).
  - `V`: momentary click pops visualisation menu. Implemented (`behaviour.rs:451`).
- **Time display**: Click toggles elapsed / remaining (S02 §4). Implemented in `behaviour.rs:448`.
- **Marquee**: Dragging with autoscroll off scrolls manually; controls display hints while held. Implemented in `behaviour.rs:186-218`.
- **Transport controls**: Prev, Play, Pause, Stop, Next, Eject. Implemented in `behaviour.rs:428-435`.
- **Sliders**: Volume (0..100%), Balance (-127..127), Position/Seek. Implemented in `behaviour.rs:455-463`.
- **EQ & Playlist toggles**: Toggle child windows. Implemented in `behaviour.rs:436-437`.

---

## 2. Command Routing and Double Application Audit

Every `Command` variant routes through `routing::app_owns(command)`. Variants owned by the engine are forwarded to `self.engine.apply(command)`. Variants owned by the app are handled in `App::apply_app(command)`.

### Double Application Shapes Identified:
1. **`SelectOp::Set` in `crates/rustamp-ui/src/playlist.rs` (`click_list_first_visible` lines 240–245, `press_sel` lines 298–302)**:
   - In `click_list_first_visible` and `press_sel`, the producer mutates its local state (`self.selected = ...`) *and* produces `Command::Select(SelectOp::Set(target))`.
   - When this command reaches `App::apply_app` (`app.rs:1033`), it calls `self.playlist.apply_select(op)` on the app's playlist model, applying the selection a second time.
   - For idempotent operations like `Set` and `All`, this double application is functionally invisible, but for `Invert` it is fatal (invert twice = no-op). The codebase previously fixed `invert_selection` to not mutate locally, but `click_list_first_visible` and `press_sel` still exhibit this dual-mutation pattern.
2. **`Command::Remove` in `crates/rustamp-ui/src/playlist.rs:330–339` (`press_rem`)**:
   - `press_rem` removes entries locally from `self.entries` and clears `self.selected`, *and* emits `Command::Remove(ids)`.
   - In `App::apply_app` (`app.rs:1031`), `remove_entries(ids)` is executed, which mutates `self.entry_ids`, syncs the engine queue, and replaces `self.playlist.set_entries(...)`.
   - Local mutation in the producer precedes the engine / app state transition.
3. **`confirm_jump_time` in `crates/rustamp-app/src/dialogs.rs:131–134`**:
   - `dialog_key(Key::Enter)` executed `let _ = self.submit(seek);` without closing the dialog (`close_any()`).
   - Consequently, the dialog remained open, and pressing Enter again would resubmit `Command::Seek`, applying the seek a second time instead of closing.
   - **Fixed:** Updated `confirm_jump_time` to call `if self.submit(seek).is_ok() { self.dialogs.close_any(); }`.

---

## 3. Focus and Window State Audit

- **Focused Window**:
  - The main window tracks focus via `MainWindow::set_focused(bool)` (`behaviour.rs:161`), updating title bar graphics.
  - S05 and S02 specify title bar focus variants (active vs inactive title bar sprite).
- **Closing the Focused Window**:
  - Main window `Close` quits the application (`app.rs:983`, S02 §1).
  - Child windows (EQ, Playlist): S05 §7.8 notes hiding and reopening child windows; EQ close hides EQ, Playlist close records a note while keeping the window open.
  - Dialog windows: Closing the focused dialog window via OS (`WindowEvent::CloseRequested`) calls `app.close_dialog(kind)`, cleaning up dialog state without closing parent windows or quitting (`window.rs:782-786`).
- **FocusNextWindow and ShowMainWindow**:
  - `Command::FocusNextWindow` and `Command::ShowMainWindow` are routed to the app (`routing.rs:84-85`).
  - S02 and S05 are silent on keyboard-driven inter-window focus cycling order (Z-order vs stack order) and `ShowMainWindow` semantics when docked.
  - Per instructions, because the specs are silent on these exact behaviors, the behavior was left alone and the notes retained.

---

## 4. Dialog Result Paths Audit

Every dialog reachable in Rustamp (`About`, `FileInfo`, `Jump(File)`, `Jump(Time)`, `OpenMedia`, `LoadPlaylist`, `SkinBrowser`, `Preferences`) follows defined lifecycle transitions:
1. **Confirmed**:
   - `Enter` consumes key.
   - For `Jump(Time)`: Produces `Command::Seek`. On success, closes dialog (`self.dialogs.close_any()`). Confirmed exactly once.
   - For `OpenMedia`: Produces `Command::AddItems`. On success, closes dialog. Confirmed exactly once.
   - For `SkinBrowser`: Loads selected skin and closes dialog. Confirmed exactly once.
   - For informational/unmeasured dialogs (`About`, `FileInfo`, `Preferences`, `LoadPlaylist`): Enter is either consumed or kept open per RFC 0010 without destructive side effects.
2. **Cancelled**:
   - `Escape` consumes key and calls `self.dialogs.close_any()`.
   - Closes dialog immediately and modifies no playback or playlist state.
3. **Closed by OS**:
   - `WindowEvent::CloseRequested` maps to `app.close_dialog(kind)`.
   - Cleans up `dialog_open_kind`, destroys the softbuffer surface/window handle, and changes no playback state.

---

## 5. "Not Implemented Yet" Notes Audit

Audited all 16 `self.note(` locations in `crates/rustamp-app`:

1. `crates/rustamp-app/src/app.rs:1011` (`Command::ToggleWindowShade` for non-Main window):
   - **Gap**: S05 §3 specifies shade mode for Main and EQ, but shaded EQ rendering is deferred.
2. `crates/rustamp-app/src/app.rs:1036` (`the playlist reordering is not wired to the app yet`):
   - **Gap**: Drag-and-drop / list reordering protocol between playlist UI and engine queue is left open in S06 / S14 / S15.
3. `crates/rustamp-app/src/app.rs:1054` (`Command::OpenBookmarkEditor | Command::AddBookmark`):
   - **Gap**: Bookmark dialog UI and persistence are unspecified in S04.
4. `crates/rustamp-app/src/app.rs:1062` (`Command::FocusNextWindow | Command::ShowMainWindow`):
   - **Gap**: S02 / S05 provide no specification for focus cycling order or focus stealing rules.
5. `crates/rustamp-app/src/app.rs:1065` (`Command::ConfigureVisPlugin | Command::ToggleVisPlugin`):
   - **Gap**: Requires milestone M3 plugin host and DLL execution architecture.
6. `crates/rustamp-app/src/app.rs:1075` (`skin load warning`):
   - **Genuine note**: Preserved to report skin parsing warnings to the presenter log.
7. `crates/rustamp-app/src/menu.rs:125, 142, 146` (Various menu actions: visualization, bookmarks, playback options):
   - **Gaps**: Require plugin host, bookmark manager, or unmeasured submenus.
8. `crates/rustamp-app/src/playlist_window.rs:245, 277, 305` (Playlist MISC / REM / ADD submenus):
   - **Gaps**: S06 §5 leaves secondary popup menus and dialog selections unmeasured.

All remaining notes represent genuine gaps where specifications have not yet settled the design.

---

## 6. Verification and Headless Tests Added

- **Added Unit Tests**:
  - `crates/rustamp-ui/src/behaviour.rs`: Extended `Widget::ClutterI` test verifying that clicking clutter I produces `[]` when no track is loaded, and produces `[Command::OpenFileInfo]` when a track is loaded.
  - `crates/rustamp-app/src/app.rs`: Updated `dialog_keys_navigate_and_confirm` to verify that Enter immediately closes the Jump to Time dialog.
  - `crates/rustamp-app/src/app.rs`: Added `clutter_i_opens_file_info_only_when_track_loaded` verifying end-to-end `App::mouse_down` / `App::mouse_up` behavior with and without loaded tracks.
  - `crates/rustamp-app/src/app.rs`: Added `dialog_result_paths_cancelled_confirmed_and_os_close` verifying that cancelled dialogs change nothing, confirmed dialogs apply exactly once and close, and OS close cleanly resets dialog state.
