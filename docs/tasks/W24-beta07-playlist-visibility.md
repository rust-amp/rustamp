# W24 — BETA-07 Verification: Playlist Visibility and Check Marks Control

Status: **Refuted / Stale finding verified**. No production code changes were required because the gap was already resolved in commit `4bf7026` (N5 lane) and refined in `b6d1d5f`. Pinned regression test added in `crates/rustamp-app/tests/playlist_visibility.rs`. Verified by `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `cargo build --workspace`.

---

## 1. The Readiness Document Finding (BETA-07)

Card `docs/tasks/BETA-readiness.md` (pinned to snapshot `f9012dc63b35b92edb83da5f19609002e41a94f8`) states at lines 74–79:

> **### P2 — BETA-07: Playlist visibility/check marks do not control its OS window (S05/S06/N4)**  
> *User impact: the playlist opens even though the initial model says closed; its toggle changes a flag/check but cannot hide it, and its native close explicitly keeps it open (`crates/rustamp-ui/src/state.rs:145-146`, `crates/rustamp-ui/src/behaviour.rs:410-413`, `crates/rustamp-app/src/window.rs:1198`, `crates/rustamp-app/src/window.rs:1071-1075`). There is no playlist counterpart to EQ's visibility-controlled creation/destruction (`crates/rustamp-app/src/window.rs:376-385`, `crates/rustamp-app/src/window.rs:1001-1037`, `crates/rustamp-app/src/window.rs:1336-1354`).*

And in Section 1 (Table row 2, line 21):
> `| Launch without arguments | On a working device, construct an empty stopped engine, then open the main window and an unconditional playlist window; output opening happens before either window. | crates/rustamp-app/src/app.rs:160-182, crates/rustamp-audio/src/engine.rs:223, crates/rustamp-app/src/lib.rs:110-145, crates/rustamp-app/src/window.rs:1149-1198 |`

---

## 2. Refutation Evidence: File, Line, and Commit

The finding was valid at snapshot `f9012dc`, but was **completely resolved** prior to this task:

### A. The Core Fix: Commit `4bf70262f417ff9f3e16a79ccdb20dddffecde58`
**Author:** N5 lane ("Deliver the first minute: open a file from the GUI, stop phantom playback, make the visible controls act", Sat Oct 17 17:03:46 2026)

1. **Window synchronization counterpart to EQ (`sync_playlist_window`):**
   - Implemented in `crates/rustamp-app/src/window.rs` (lines 1135–1145):
     ```rust
     fn sync_playlist_window(&mut self, event_loop: &ActiveEventLoop) {
         if self.app.playlist_open() {
             self.open_playlist_window(event_loop);
         } else if self.playlist_window.is_some() {
             self.playlist_surface = None;
             self.playlist_window = None;
             self.last_playlist_click = None;
         }
     }
     ```
   - Dropping `self.playlist_window` and `self.playlist_surface` destroys the OS window when `playlist_open == false`.

2. **Unconditional open replaced:**
   - In `resumed()` (`crates/rustamp-app/src/window.rs:1506`), unconditional `self.open_playlist_window(event_loop)` was replaced by `self.sync_playlist_window(event_loop)`.

3. **Native close (`WindowEvent::CloseRequested`) fixed:**
   - The stub at lines 1071–1075 (`"rustamp: playlist close is not implemented yet; the window stays open"`) was replaced at `crates/rustamp-app/src/window.rs:1232–1239`:
     ```rust
     WindowEvent::CloseRequested => {
         if let Err(error) = self.app.close_playlist_window() {
             eprintln!("rustamp: playlist close failed: {error}");
         }
         self.sync_playlist_window(event_loop);
         self.request_redraw();
     }
     ```
   - Calling `self.app.close_playlist_window()` submits `Command::ToggleWindow(WindowId::Playlist)`, toggling the model to closed, and `sync_playlist_window` drops the OS window.

4. **Frame synchronization hook:**
   - In `about_to_wait()` (`crates/rustamp-app/src/window.rs:1705`), `self.sync_playlist_window(event_loop)` is executed on every frame pump. Any toggle (menu, shortcut, PL button, right-click, IPC) is immediately reflected in the OS window.

5. **Initial state alignment:**
   - `crates/rustamp-app/src/app.rs` initialized `playlist_open: true` on launch, resolving the mismatch where the presenter opened the window while the model said closed.

### B. Configuration Persistence: Commit `b6d1d5f`
**Author:** Configuration lane ("Make the configuration real: honoured settings, a written file, loud malformed input")
- In `crates/rustamp-app/src/app.rs:259`:
  ```rust
  playlist_open: pl_layout.map(|l| l.visible).unwrap_or(true),
  ```
- If the saved configuration specifies `visible = false`, the model and OS window start closed and unchecked. Otherwise, they default to open and checked.

### C. Prior Documentation Acknowledgments
- `docs/tasks/BETA-readiness-2.md:40`:
  > `| BETA-07 | Playlist visibility/check marks don't control its OS window | **FIXED (by the N5 lane, confirmed by reading)** | sync_playlist_window creates/drops the OS window to follow App::playlist_open (window.rs:1089-1101); PL button, menu row, skinned and OS close all flip the one flag (window.rs:1091-1092,1187-1191). The follow-up W10 review of the playlist lane confirms BETA-07 needed no change. Not re-verified: the initial playlist_open default value. |`
- `docs/tasks/S06i3-playlist-visibility.md:45–71`:
  > Section 1 confirms: *"The finding (BETA-07) predates the N5 lane: the playlist model said closed while the OS window was up, and its toggle/check could not hide it. That wiring already exists in this worktree, put there by N5 (see docs/tasks/N5-first-minute.md §3); nothing below redoes it."*

---

## 3. Two-Way Truth Verification

The check marks and window visibility tell the truth in both directions:

| Control / Input Path | Effect on Model | Effect on OS Window | Effect on Menu Check | Source Code Reference |
|---|---|---|---|---|
| **Default Launch** | `playlist_open = true` | Created (`open_playlist_window`) | `true` (checked) | `app.rs:259`, `window.rs:1506`, `menu.rs:170` |
| **Config `visible: false`** | `playlist_open = false` | Not created (`None`) | `false` (unchecked) | `app.rs:259`, `window.rs:1506`, `menu.rs:170` |
| **Skinned Close Button** | Toggled to `false` | Dropped (`None`) | `false` (unchecked) | `playlist_window.rs:271`, `window.rs:1309` |
| **Native OS Close Box** | Toggled to `false` | Dropped (`None`) | `false` (unchecked) | `window.rs:1232–1239` |
| **Main PL Button (Left click)** | Toggled (`true` <-> `false`) | Created / Dropped | Toggled (`true` <-> `false`) | `behaviour.rs:496`, `window.rs:1705` |
| **Main PL Button (Right click)** | Toggled (`true` <-> `false`) | Created / Dropped | Toggled (`true` <-> `false`) | `app.rs:540`, `window.rs:1650` |
| **Main Menu Item** | Toggled (`true` <-> `false`) | Created / Dropped | Toggled (`true` <-> `false`) | `menu.rs:170`, `window.rs:1705` |
| **Shortcut `Alt+E`** | Toggled (`true` <-> `false`) | Created / Dropped | Toggled (`true` <-> `false`) | `shortcuts.rs:1390`, `window.rs:1705` |

---

## 4. What the Specs Settle vs. What They Leave Open

### Settled:
1. **Geometry and Resizability:** S01 §1, §4.1: The playlist window client area is 275 x 116 pixels, non-resizable in classic skin mode.
2. **Button State Visuals:** S01 §161–162: The PL toggle button uses `shufrep.bmp` (23,61,23,12 when closed, 23,73,23,12 when open).
3. **Shortcuts & Right-Click:** S03 §2, §5: `Alt+E` toggles the playlist window; right-clicking the PL toggle button toggles the window.
4. **Menu Integration:** S04 §1: The main menu row 6 `Playlist Editor` displays a check mark when the window is visible and toggles it on activation.
5. **Docking Placement:** S05 §3.1, §4.2: On a fresh launch, the playlist editor docks below the main window / equalizer at client offset (26, 261).

### Left Open:
1. **Windowshade combination with Equalizer / Playlist docking:** S05 §1.5 explicitly marks equalizer and playlist editor windowshade interaction as open.
2. **Double-size effect on playlist:** S01 §90 and S05 §2.1 settle that double-size mode resizes the main window to 550 x 232 but does *not* resize the playlist editor (it stays 275 x 116); how non-adjacent docking behaves during resize remains partially open.

---

## 5. Added Tests and Failure Modes

Added integration test suite: `crates/rustamp-app/tests/playlist_visibility.rs`

### Tests:
1. `beta07_playlist_check_marks_and_visibility_tell_the_truth_in_both_directions`:
   - Validates initial launch state (`app.playlist_open() == true`, `menu_check == true`).
   - Validates skinned close button (`[X]` at client 264, 3) emits `Command::ToggleWindow(WindowId::Playlist)`, closes window, unchecks menu mark, does not quit.
   - Validates PL button click brings it back and re-checks menu mark.
   - Validates native `close_playlist_window()` closes window, unchecks menu mark, and is idempotent.
   - Validates right-click on PL toggle button flips state and check mark.
   - Validates main menu activation flips state and check mark.
   - Validates `Alt+E` shortcut dispatch flips state and check mark.

2. `beta07_configured_hidden_playlist_starts_closed_and_unchecked`:
   - Validates that launching with a config specifying `WindowLayout { id: Playlist, visible: false, .. }` starts with `playlist_open == false` and `menu_check == false`.
   - Validates that activating the menu item toggles it open and sets the check mark to `true`.

### How Each Test Assertion Can Fail:
- **Initial state failure:** Fails if `AppOptions::headless()` or default `App::new` reverts `playlist_open` to `false` or disconnects the menu check from `MainWindowState::playlist_open`.
- **Skinned close failure:** Fails if playlist title-bar hit-testing breaks or emits a quitting command instead of `ToggleWindow(Playlist)`.
- **Menu mark mismatch failure:** Fails if `menu_check(MenuCheck::PlaylistEditor)` reads a stale or hardcoded boolean instead of live `MainWindowState::playlist_open`.
- **Native close failure:** Fails if `close_playlist_window` fails to submit `ToggleWindow`, leaving the window open or failing on double-close.
- **Config persistence failure:** Fails if configuration parsing or layout extraction in `App::new` ignores `visible: false` on `WindowId::Playlist`.

---

## 6. Verification and Gates

- `cargo fmt --all -- --check`: Exit 0 (clean).
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: Exit 0 (clean).
- `cargo build --workspace`: Exit 0 (clean).
- `cargo test`: Not run on host (reserved for orchestrator Hyper-V VM per `AGENTS.md`).

---

## 7. What Only the Virtual Machine Can Prove

1. **Actual OS Window Destruction:** Headless and unit tests verify `self.app.playlist_open()` and `self.playlist_window.is_none()`. Only the Hyper-V VM with Win32 desktop API access can verify through `FindWindowEx` and `GetWindowRect` that the OS HWND is destroyed or unmapped from desktop rendering.
2. **Winit / Softbuffer Surface Lifecycle:** Proving that recreation of the `winit::window::Window` and `softbuffer::Surface` upon re-opening succeeds without leaking surfaces or crashing on Win32 display drivers.
3. **Window Stacking and Drag-Docking Follow:** Real OS mouse drag messages moving the docked windows together across screen pixels (as documented in `crates/rustamp-app/tests/playlist_window.rs`).
