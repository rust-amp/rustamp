# S06i2 — Playlist-window interactions (scrollbar, selection, buttons, keys)

Status: implemented in the worktree, headless tests written, **not run** (the
suite runs in the Hyper-V test VM). Verified on the host by `cargo fmt --all --
--check` (clean), `cargo clippy --workspace --all-targets --all-features -- -D
warnings` (exit 0) and `cargo build --workspace --offline` (exit 0).

## Goal

Close the playlist-window interactions that **S06** settles, and record the
rest as spec gaps. Behaviour the spec leaves open is not changed.

## Owned paths

- `crates/rustamp-ui/src/playlist.rs` (model)
- `crates/rustamp-app/src/playlist_window.rs` (app glue and headless tests)
- `docs/tasks/S06i2-playlist-interactions.md` (this card)

Not touched: `rustamp-playlist`, `rustamp-meta`, `rustamp-audio`,
`rustamp-plughost`, and `crates/rustamp-app/src/window.rs` (see §4).

## Sources

- Spec: `specs/S06-playlist.md` (§§1-8), `specs/S01-window-geometry.md` (§4.1,
  §4.2), `specs/S03-shortcuts.md` (§2 playlist table, §5).
- No dirty-room material was read.

## 1. Scrollbar

Spec (S06 §3): "One page of four rows" per arrow step; "Up-arrow click at
(263,81) ... Down-arrow click at (263,87) ... 1501 px back". Spec (S01 §4.2):
"y = 20 at the scroll top, 40 at the middle and 60 at the bottom for the
captured list, which shows four rows at 13 px pitch, i.e. 40 px of travel."
Spec (S01 round 3b): "the playlist view is at the bottom, showing rows 57–60."

- **Existed:** arrows → `hit_test_scroll` (`playlist_window.rs` in ui,
  `SCROLL_SPLIT_Y`), `scroll_page` (`playlist.rs`), handle drawn at
  `scroll_handle_y` (ui `playlist_window.rs`).
- **Gap found and changed:** the model clamped the offset at `len - 1`
  (`playlist.rs` `scroll_page`, `scroll_to`, `clamp_scroll`), but
  `scroll_handle_y` assumes `len - VISIBLE_ROWS`. At the bottom the model could
  show a single row while the handle sat at 60.
- **Change:** new `PlaylistWindow::max_first_visible()` = `len - 4`
  (saturating); `scroll_page`, `scroll_to` and `clamp_scroll` use it.
- **Tests:** `playlist.rs` `scrolling_clamps_at_both_ends` (now 2, was 5),
  `a_list_that_fits_one_page_never_scrolls`,
  `the_scroll_range_follows_adds_removals_and_a_clear`,
  `the_bottom_page_matches_the_handle_at_its_lowest_point`; the existing
  `syncing_entries_keeps_live_selection_and_clamps_the_view` (now 53) and
  `rem_removes_the_selected_entry_as_a_plain_command` (now 55) were updated.
- **Spec gap (left alone):** the handle drag and track clicks. S06 §3 says a
  dragged handle "moves the view" (visual reading, Q6 provisional). The
  pixel-to-row mapping is not in any spec. `playlist_mouse_down` does not route
  clicks on the handle (`hit_test_scroll` covers only the arrow pair).

## 2. Selection

Spec (S06 §2): "Single posted click at (60,28) selects the first visible row"
(settled). "SEL ... selects all visible rows" (settled). Spec (S06 §5.3): "Without
a staged selection the button is a no-op."

- **Existed and agrees:** the mouse path (`playlist_mouse_down` →
  `click_list_first_visible`, `press_sel`) and the keyboard path both write the
  same `selected` set. Selection survives a sync (`set_entries`) and drops
  entries that were removed.
- **Keyboard added (§4):** Ctrl+A → `select_all` (`SelectOp::All`),
  Ctrl+I → `invert_selection` (`SelectOp::Invert`).
- **Spec gap:** whether "select all" (Ctrl+A) means the whole list or the
  visible page is not settled. S06 §2 settles only SEL = visible rows. The
  code uses the whole list for Ctrl+A (the menu label "Select all", S03 §2,
  `40205`), and this is recorded as an own reading. The behaviour of the
  SEL submenu (`INV SEL` / `SEL ZERO` / `SEL ALL`) was never driven (S06 §8 Q11).

## 3. Buttons

Spec (S06 §4): the five bottom-bar buttons are ADD, REM, SEL, MISC, LIST OPTS.
The dialog titles are settled from window metadata (`Add file(s) to playlist`,
`Load Playlist`). The MISC popup exists (`#32768`, 474 x 81), but its contents
are a visual reading.

| Button | Produces | Status |
|---|---|---|
| ADD | `Command::OpenMedia { Files, play: false }` + dialog | live (pre-existing); `play: false` is unmeasured (RFC 0005 O5) |
| REM | `Command::Remove` (no-op unstaged, S06 §5.3) | live (pre-existing) |
| SEL | `Command::Select(Set(visible))` | live (pre-existing) |
| MISC | a note only | **drawn but inert**: spec gives existence and size, not contents (§4 of S06, open Q15) |
| LIST OPTS | `Command::OpenPlaylist` + dialog | live (pre-existing, RFC 0010) |
| title Close / Shade | a note only | inert: hiding and shading are open (S05 §1.5) |
| title window-menu (6,3) | nothing | by spec: no window-menu button (S06 §1) |

No button change was needed in this card. The inert cases are the spec's own
gaps, and the code states them in notes.

## 4. Keyboard (S03 §2, playlist editor table)

Spec (S03 §2): "Delete | 1034 | Remove the selected item(s) | menu";
"Ctrl+A | 40205 | Select all | menu"; "Ctrl+I | 40171 | Invert selection | menu";
"Up | 40289 | Move the selection up | provisional"; "Home | 40264 | Jump to the
first entry | provisional"; "PgUp | 40256 | Scroll up one page | provisional".
Spec (S03 header): "How the tables combine when a non-main window has focus ...
is **provisional**."

- **Before:** `window.rs` `playlist_event` routes every key to the global
  `App::key_down` (the main table). In particular Ctrl+A reached the main
  table, which binds it to always-on-top (`40019`).
- **Change:** new `App::playlist_key_down` (`playlist_window.rs`). It handles
  only Delete (no modifiers), Ctrl+A and Ctrl+I, and passes every other key to
  `App::key_down`. Delete with nothing staged returns `ShortcutOutcome::Handled`.
- **Not done (spec gap):**
  - The arrow, Home, End and page rows are provisional, so they stay on the
    shared path. The table's provisional move and scroll rows are not
    implemented.
  - Enter (`40184`, "Play the selected item(s)", menu) is not wired. The spec
    does not say which item plays with several selected.
  - Crop (Ctrl+Delete, `1035`) and remove dead (Alt+Delete, `40222`) have no
    model support and no measured effect.
  - Sort, reverse and randomise rows (`40209`–`40213`) are not wired (Q3).
  - **Wiring needed outside this card:** `window.rs` `playlist_event` still calls
    `self.key_event(&event)`, which routes to `App::key_down`. To take effect, the
    call must go to `App::playlist_key_down`. `window.rs` is not on this lane's
    owned-path list, so this card does not edit it.

## 5. Tests (all headless; no window, no process, no audio)

- `rustamp-ui` `playlist.rs`: the scroll tests in §1, `ctrl_a_selects_every_entry_and_ctrl_i_inverts_the_selection`.
- `rustamp-app` `playlist_window.rs`: `delete_removes_the_selected_entry_in_the_playlist_window`,
  `delete_without_a_selection_is_handled_and_changes_nothing`,
  `ctrl_a_selects_all_and_ctrl_i_inverts_in_the_playlist_window`,
  `other_playlist_keys_take_the_shared_path_and_leave_the_list_alone`.
- `crates/rustamp-app/tests/playlist_window.rs`: unchanged. It is VM-only and
  `#[ignore]`d, and it still pins the 275 x 116 window and the docked origin.

## 6. Verification status

- Not run here: `cargo test` (by rule, the suite runs in the VM). The new
  tests have been **compiled** (`clippy --all-targets`) but **not executed**.
- Ran on the host: `cargo fmt --all -- --check` (clean), `cargo clippy --workspace
  --all-targets --all-features -- -D warnings` (exit 0), `cargo build --workspace
  --offline` (exit 0).
