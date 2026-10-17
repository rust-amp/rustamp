# Task card N3 — GUI drives playback end to end

**Goal**
Establish, with evidence, which of the chains from app/GUI/playlist actions to audio engine playback are wired, close the ones that are not, and provide thorough headless unit and integration tests.

**Lane** N — application.
**Owned paths**
- `crates/rustamp-app/src/**`
- `crates/rustamp-app/tests/**`
- `crates/rustamp-ui/src/playlist*.rs`
- `docs/tasks/N3-gui-drives-playback.md`

Do NOT touch `crates/rustamp-plughost` or `crates/rustamp-audio`.

---

## Findings and Implementation per Chain

### 1. Opening a file (the Open Media path) enqueues it and playing it starts playback
- **Already wired:**
  - `crates/rustamp-app/src/app.rs:1051`: `Command::OpenMedia { kind, play }` routes to `self.open_dialog_for(command)`.
  - `crates/rustamp-app/src/dialogs.rs:137-144`: Confirming the dialog (`dialog_key(Key::Enter)`) calls `confirm_open_media()` which produces `Command::AddItems { items, at: None, play }`.
  - `crates/rustamp-app/src/app.rs:1027`: `Command::AddItems` forwards to `self.add_items()`.
  - `crates/rustamp-app/src/app.rs:1109-1131`: `add_items` updates `engine.set_queue()`, calls `self.sync_entry_ids()` (which updates `self.playlist.set_entries()`), and if `play` is true, calls `self.engine.play_index(first_new)`.
  - `crates/rustamp-ui/src/behaviour.rs:430` / `crates/rustamp-audio/src/engine.rs:446`: Clicking Play or submitting `Command::Play` invokes `self.play()` -> `self.start_run()`, which sets `current = Some(0)` and starts playback.
- **What was missing:**
  - The existing test (`confirming_the_files_dialog_adds_the_picks_and_closes` at `app.rs:2028`) only tested that confirming the dialog with a dummy path grew the queue length without ever attempting playback. There was no test proving that opening via Open Media enqueues a playable item and that playing it transitions the engine to `TransportState::Playing` with queue length (1), `current_index` (`Some(0)`), and engine transport in agreement.
- **What changed:**
  - No production logic change needed for this chain; added comprehensive headless test `chain1_opening_file_via_open_media_enqueues_and_playing_starts_playback` in `crates/rustamp-app/src/app.rs` testing both `play: false` followed by `Play`, and `play: true` directly.
- **Test:**
  - `crates/rustamp-app/src/app.rs:chain1_opening_file_via_open_media_enqueues_and_playing_starts_playback`.

---

### 2. Double-clicking a playlist entry plays that entry
- **Already wired:**
  - `crates/rustamp-proto/src/command.rs:143`: `Command::PlayEntry(EntryId)` defined.
  - `crates/rustamp-app/src/routing.rs:58`: `Command::PlayEntry` owned by `app`.
  - `crates/rustamp-ui/src/behaviour.rs:72`: `MainWindow` handles `Command::PlayEntry`.
  - `crates/rustamp-ui/src/playlist_window.rs:171-177`: `hit_test_list_row(x, y)` resolves playlist coordinates to a visible row `0..VISIBLE_ROWS`.
- **What was missing:**
  - `PlaylistWindow` in `crates/rustamp-ui/src/playlist.rs` had no method to double-click a row and emit `Command::PlayEntry`.
  - `App` in `crates/rustamp-app/src/playlist_window.rs` had no `playlist_double_click` method.
  - `crates/rustamp-app/src/app.rs:1039-1045`: `Command::PlayEntry` was completely unwired, only recording an unhandled note: `"the playlist model is not wired to the app yet (S06/S14/S15)"`. It never mapped the entry ID to a queue index or called `self.engine.play_index()`.
  - `crates/rustamp-app/src/window.rs:1073-1090`: The windowed presenter had no double-click tracking for the playlist window.
- **What changed:**
  - In `crates/rustamp-ui/src/playlist.rs`: Added `PlaylistWindow::double_click_row(&mut self, row: usize) -> Option<Command>` and `double_click_index`, which select the row and return `Some(Command::PlayEntry(id))`.
  - In `crates/rustamp-app/src/playlist_window.rs`: Added `App::playlist_double_click(&mut self, x: i32, y: i32) -> Result<Vec<Command>>`.
  - In `crates/rustamp-app/src/app.rs`: Implemented `App::play_entry(&mut self, id: EntryId)` in `apply_app`: maps `id` to index in `self.entry_ids` and invokes `self.engine.play_index(index as u32)`.
  - In `crates/rustamp-app/src/window.rs`: Added double-click detection (consecutive presses <= 500 ms within 4px) in `playlist_event` and routed to `self.app.playlist_double_click(x, y)`.
- **Tests:**
  - `crates/rustamp-ui/src/playlist.rs:double_click_row_selects_and_emits_play_entry`
  - `crates/rustamp-app/src/playlist_window.rs:double_clicking_a_row_emits_play_entry`
  - `crates/rustamp-app/src/app.rs:chain2_double_clicking_playlist_entry_plays_that_entry`

---

### 3. Next/Previous (button, menu or key) move through the queue and the engine follows, including at the ends
- **Already wired:**
  - Buttons: `crates/rustamp-ui/src/behaviour.rs:429, 433` maps `Widget::Previous` -> `Command::Prev`, `Widget::Next` -> `Command::Next`.
  - Keys: `crates/rustamp-ui/src/shortcuts.rs:1326-1329` maps `Key::letter('B')` / `Key::Numpad(6)` -> `Command::Next`, `Key::letter('Z')` / `Key::Numpad(4)` -> `Command::Prev`.
  - Menu: `crates/rustamp-ui/src/menu.rs:463-474` defines "Previous track" and "Next track".
  - Engine: `crates/rustamp-audio/src/engine.rs:456-457, 578-598` implements `next_track` and `prev_track`.
- **What was missing:**
  - Existing test `next_and_prev_move_through_the_queue_and_report_back` (`app.rs:1565`) only tested raw `Command::Next` advancing from 0 to 1. It did not test:
    - Button click on `Widget::Next` / `Widget::Previous`.
    - Key presses ('B' and 'Z').
    - Menu actions ("Next track" and "Previous track").
    - Boundary behavior at both ends: Previous at track 0 (stays at track 0) and Next at the end of the queue (stops playback and emits `Event::PlaylistEnded`).
- **What changed:**
  - No production logic change needed (wiring was intact); added comprehensive test `chain3_next_and_previous_move_through_queue_across_button_menu_key_and_ends` covering all 3 invocation methods and both queue ends.
- **Test:**
  - `crates/rustamp-app/src/app.rs:chain3_next_and_previous_move_through_queue_across_button_menu_key_and_ends`.

---

### 4. The playlist window's own model (its rows) stays in step with the engine's queue when entries are added or removed
- **Already wired:**
  - Adding entries: `crates/rustamp-app/src/app.rs:1109-1131` added to `engine.set_queue()`, synced `entry_ids` and updated `self.playlist.set_entries()`.
- **What was missing:**
  - Removing entries: `Command::Remove` and `Command::Clear` in `crates/rustamp-app/src/app.rs:1035-1046` only logged `"the playlist model is not wired to the app yet (S06/S14/S15)"`.
  - `self.engine.queue()` was never modified on removal.
  - In `crates/rustamp-app/src/playlist_window.rs:185`, REM button removed IDs from `self.entry_ids` locally without touching `engine.queue()`, causing an immediate desync.
  - External `Command::Remove` or `Command::Clear` did not update `self.entry_ids`, `self.playlist`, or `self.engine.queue()`.
  - In `crates/rustamp-app/src/app.rs:920-935`, `entry_id_for` erroneously grew `self.entry_ids` even if `index >= engine.queue().len()`.
- **What changed:**
  - In `crates/rustamp-app/src/app.rs`:
    - Implemented `remove_entries(&mut self, ids: &[EntryId])`: filters both `self.entry_ids` and `self.engine.queue()`, calls `self.engine.set_queue(new_queue)`, and updates `self.playlist` via `self.sync_playlist_entries()`.
    - Implemented `clear_entries(&mut self)`: clears `self.entry_ids`, clears `self.engine.set_queue(Vec::new())`, and updates `self.playlist`.
    - Implemented `SelectOp` forwarding via `self.playlist.apply_select(op)`.
    - Fixed `entry_id_for`: returns `None` for out-of-bounds indices without allocating phantom entries.
  - In `crates/rustamp-app/src/playlist_window.rs`:
    - Removed premature local `entry_ids.retain` in REM arm so removal is handled centrally and atomically in `apply_app`.
- **Tests:**
  - `crates/rustamp-app/src/playlist_window.rs:rem_removes_the_staged_selection`
  - `crates/rustamp-app/src/app.rs:chain4_playlist_window_model_stays_in_step_with_engine_queue`

---

### 5. Removing the playing entry, or clearing the list while playing, leaves the engine in a defined state
- **Defined state:**
  - **Removing the playing entry:** When an entry that is currently playing is removed, playback of that track cannot continue. The engine is stopped via `self.engine.stop()`, resetting transport to `TransportState::Stopped` and playback position to 0 ms. The engine queue and playlist window model are updated to contain the remaining entries.
  - **Clearing the list while playing:** When the playlist is cleared while playing, the engine is stopped via `self.engine.stop()`, resetting transport to `TransportState::Stopped` and playback position to 0 ms. The engine queue is empty (`0`), the playlist model is empty (`0` rows), and `entry_ids` is empty.
  - **Removing a non-playing entry while playing:** Playback continues undisturbed (`TransportState::Playing`); the engine queue and playlist model are updated to the remaining entries.
- **Already wired:**
  - Nothing: removal was completely unwired from the engine.
- **What was missing:**
  - Active playback check and graceful stop transition on removal of the active playing entry or on list clear.
- **What changed:**
  - In `remove_entries`: checks if `self.engine.current_index()`'s ID is in the removed set. If so, calls `self.engine.stop()`.
  - In `clear_entries`: if transport is not `Stopped`, calls `self.engine.stop()`.
- **Test:**
  - `crates/rustamp-app/src/app.rs:chain5_removing_playing_entry_or_clearing_leaves_defined_state`
