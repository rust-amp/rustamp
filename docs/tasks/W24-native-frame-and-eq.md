# W24 — The native window frame, and the five defects the real machine showed

Lane: `paseo-wave122-frame`. Owned paths: `crates/rustamp-app/**`,
`crates/rustamp-ui/**`, `crates/rustamp-platform/**` and this card.

Context: the packaged player now **opens** on the owner's machine (the audio
backend probe no longer takes the main thread's COM apartment, commit
`17a26b2`). Everything below is what the owner then saw on the real window,
with a screenshot — defects that no automated check could have caught in the
shape the checks had.

Gates run in this lane (all three, after the last change):

| Gate | Exit |
|---|---|
| `cargo fmt --all -- --check` | 0 |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 0 |
| `cargo build --workspace` | 0 |

`cargo test` was **not** run: the suite belongs to the orchestrator VM. Every
test added below is headless by construction (no window, no desktop); the
VM-only probes stay `#[ignore]`d and VM-gated.

---

## 1. The native title bar (done)

**What made the windows decorated.** Nothing in the code said they should not
be. `winit`'s `WindowAttributes::default()` sets `decorations: true`
(`winit-0.30.13/src/window.rs:148`), which on Windows is
`WS_CAPTION | WS_BORDER | WS_SYSMENU` plus the minimise and maximise boxes
(`platform_impl/windows/window_state.rs:258-268`). All six creation sites
built their attributes from `Window::default_attributes()` and never touched
`decorations`, so every window — main, equalizer, playlist, each dialog, the
menu popup — came up with the standard caption.

**The change.** One shared shell, `crates/rustamp-ui/src/shell.rs`
(`skinned_window_attributes`): borderless, non-resizable, sized by its
**client** area. It is now the only place in `rustamp-ui` and `rustamp-app`
that calls `Window::default_attributes()`. `rustamp-app` routes every window
through `Windowed::window_attributes`, which adds the saved position and
nothing else; the repeated per-window config lookups collapsed into one
`Windowed::saved_position`.

**Why the client geometry the tests pin is unaffected.** The frame lives
*outside* the client area. The shell names the client size
(`with_inner_size`), which is exactly what the geometry tests measure
(`window.inner_size()`), and dropping the frame does not touch it: 275 x 116,
550 x 232 and the dialog sizes are unchanged, and `with_position` still sets
the same outer position. What *does* change is the outer size, which now
equals the client size — so `Windowed::place_eq_below_main` finally stacks
client edges gap-free (S05 §3.3) instead of being a caption's height too low.
That approximation's doc comment is retired.

**Spec backing.** S01 measures the three classic windows with the origin at
"the window's client top-left" and puts a skinned title bar at (0,0,275,14)
inside that client area; `MainWindowLayout::title_bar`,
`EqualizerLayout::title_bar` and `playlist_window::TITLE_BAR_HEIGHT` all draw
it from the skin with the skin's own close and shade buttons. A native caption
is a second title bar.

**Checks added.**

* `crates/rustamp-ui/src/shell.rs` unit tests: the shell refuses the frame,
  keeps the exact client size of every window, and the builders a caller
  chains on (position, window level) do not bring the frame back. One
  assertion guards the premise — it fails if `winit` ever stops defaulting to
  a decorated window, which would make the whole module pointless.
* `crates/rustamp-app/src/window.rs`:
  `every_window_the_presenter_opens_is_borderless_at_its_measured_client_size`
  — per window: borderless, not resizable, client size still the measured one.
  Fails if a window is created decorated **and** if a frame change moved the
  client size.
* `crates/rustamp-app/tests/window_shell.rs`:
  `only_the_shared_shell_builds_window_attributes_from_winits_defaults` scans
  the three crates' `src/` and `examples/` and fails if a second site builds
  attributes from `winit`'s defaults — that is how the defect would come back.
  A companion test fails if the shell itself loses `with_decorations(false)`,
  so the scan cannot go green over a broken shell.

**What needs the owner's machine.** That the window the OS shows has no
caption. The VM probe `tests/eq_window.rs` now also asserts
`outer_size() == inner_size()`, which is only true without a frame — that is
the closest observable, and it still needs a desktop.

---

## 2. One taskbar entry for the whole player (done)

**Cause.** Every `winit` window is an ordinary top-level window, and the shell
gives each of those its own taskbar button. Nothing said which window
represents the application.

**Change.** `rustamp_ui::shell::TaskbarRole` (`Application` / `Secondary`),
applied in the shell via `with_skip_taskbar` on Windows. The per-window
decision lives in one place, `PresenterWindow::taskbar_role` in
`crates/rustamp-app/src/window.rs`: only the main window is `Application` —
it lives for the whole run (closing it quits), so the entry never vanishes
under the user. The equalizer, the playlist and the dialogs are `Secondary`.

**Check.** `exactly_one_window_carries_the_players_taskbar_entry` enumerates
`PresenterWindow::ALL` and fails if more or fewer than the main window claim
the entry.

**Needs the owner's machine.** That the shell then shows one button.
`WindowAttributes::platform_specific` is private in `winit`, so the applied
flag itself cannot be read back headlessly — only the decision can.

**Not done, not asked:** alt-tab still lists the secondary windows. Making
them owned windows (`with_owner_window`) would remove that too, but it also
forces them permanently above the main window, which changes z-order
behaviour the specs do not cover.

---

## 3. The visualization only moved while focused (done, half-provable)

**What was focus-driven.** Not the model: `App::pump` →
`MainWindow::refresh` → `Visualization::advance` takes `now_ms` and the
transport and nothing else, and the only thing `App::set_focused` changes is
the title-bar sprite. The frame *delivery* was the message-driven half: the
tick called `request_redraw()` (i.e. `RedrawWindow` → `WM_PAINT` →
`RedrawRequested` → `paint`), so a frame reached the screen only if and when
that window message came back.

**Change.** `Windowed::present_frames`: the tick paints directly. The frames
are drawn into `softbuffer` surfaces the player owns and presented with an
ordinary blit, which needs no `WM_PAINT`, so the animation no longer depends
on a window message being delivered. `RedrawRequested` is still handled for
repaints the OS itself asks for (an uncovered or resized window), and the
input paths still request one so a press shows without waiting for the tick.

**Check.** `the_visualization_advances_with_the_clock_while_the_window_is_unfocused`
pushes one analyser frame, sets the window explicitly unfocused, lets time
pass and asserts the envelopes aged. It fails if anyone makes the animation
depend on focus or activation.

**Needs the owner's machine.** That the frame now reaches the screen on every
tick. That is the half the defect actually lived in and it cannot be observed
without a window. Stated plainly: no test here proves the owner's symptom is
gone — it proves the model is focus-free and removes the message dependency
from the delivery.

---

## 4. Drag and drop onto the playlist (done)

**Cause.** Only the dialog window had a `WindowEvent::DroppedFile` arm. The
playlist window had none, so a drop on it was discarded. (`winit` registers an
OLE drop target per window by default, and `OleInitialize` now succeeds since
`17a26b2`, so the events do arrive — nothing was listening.)

**Change.** `App::drop_on_playlist` (`crates/rustamp-app/src/playlist_window.rs`)
turns the paths into one `Command::AddItems { at: None, play: false }` through
the shared `rustamp_ui::media_items_for_paths` — the same append the ADD
button performs, so a drop and a pick cannot drift apart. The presenter's
`playlist_event` routes `DroppedFile` into it; `winit` delivers one event per
file, so a multi-file drop arrives as several calls and keeps the shell's
order.

**Stated simplification.** A dropped `.m3u`/`.pls`/`.xspf` is added as an
ordinary entry, not loaded: the reference's drop-a-playlist behaviour is
unmeasured (S06 leaves it open) and the load path is LIST OPTS (RFC 0010).

**Check.** `dropping_files_on_the_playlist_appends_them_in_order`: appends in
order, never replaces, empty drop is a no-op.

**Needs the owner's machine.** That the OS delivers a shell drag to that
window.

---

## 5. ADD opens the standard file-open dialog (done)

**Cause.** `DialogKind::OpenMedia { kind: Files }` was satisfied by a window
of ours that asked for a typed path or a drop onto it. `crates/rustamp-ui` is
`forbid(unsafe_code)` and had no Win32 binding, and the dialogs module says so
explicitly.

**Change.** `crates/rustamp-platform/src/file_dialog.rs`: `pick_media_files`
over `GetOpenFileNameW` with `OFN_EXPLORER | OFN_ALLOWMULTISELECT`
(plus `FILEMUSTEXIST`, `PATHMUSTEXIST`, `HIDEREADONLY` and `NOCHANGEDIR`, so
the dialog cannot move the process's current directory). It takes the owner
window's raw handle exactly as `TrayIcon::attach` does — the established
convention in this codebase, and the only existing native-dialog precedent
(the tray's `TrackPopupMenu`). The `windows` crate gained two **features**
(`Win32_UI_Controls`, `Win32_UI_Controls_Dialogs`); no new dependency, and
`Cargo.lock` is unchanged.

The presenter decides the route, the app keeps its model:
`DialogRoute::for_dialog` says the files face is native and every other dialog
keeps its own window. The request is recorded and drained once per tick in
`Windowed::drain_media_pick`, never from a window procedure — the modal dialog
runs its own message loop, the same rule the tray popup follows. The chosen
files become one `Command::AddItems` carrying the requesting command's `play`
flag untouched, so `Play file` still starts and the playlist's ADD still only
appends. A cancelled dialog adds nothing. Multiple selection is always on.

**Checks.**

* `only_the_files_face_of_open_media_is_the_native_file_dialog` — the routing
  decision, including that the `play` flag survives, and that the directory
  and location faces and all other dialogs are untouched.
* `a_pending_file_dialog_is_drained_exactly_once` — one request, one dialog.
* `rustamp-platform`: `parse_selection` is unit-tested on the two shapes
  `GetOpenFileNameW` returns (one full path; directory followed by bare
  names), on the cancelled/empty buffer, and the filter string's
  NUL/double-NUL form. This is where multi-select is actually easy to get
  wrong, and it is pure, so it is tested on every platform.

**Needs the owner's machine.** That the dialog appears, is modal to the player
and multi-selects.

**Consistency note.** The directory and location faces keep their own windows:
a folder pick is a different native dialog (`IFileOpenDialog` with
`FOS_PICKFOLDERS`), which no spec covers and which was not asked for.

---

## 6. The selected playlist row's colour — diagnosed, change is outside this lane

**What the skin provides, and what the code does with it.** `pledit.txt`
provides `SelectedBG`, and the renderer already uses exactly that:
`crates/rustamp-ui/src/playlist_window.rs::draw_rows` fills the row with
`skin.pledit().selected_bg`, confined to the settled list band (`row_rect`,
S06 §2: x 12–254, 13 px pitch). Nothing is hardcoded.

**So where the brown and orange come from.** The bundled default skin is the
Rust theme (`BuiltinDefaultSkin` → `rustamp_skin::rust_theme`), and its
`crates/rustamp-skin/assets/rust-theme/pledit.txt` says:

```
Normal=#DF7B38      <- orange row text
SelectedBG=#6A2610  <- dark brown selection band
```

That is the brown-and-orange bar in the screenshot. It is the skin's own
choice, faithfully rendered.

**`crates/rustamp-skin` is outside this lane's owned paths, so nothing was
changed there.** The exact change, if the owner wants a different selection
colour, is in that one file — for example the classic reading S06 §101
records, a blue band:

```
[Text]
Normal=#DF7B38
Current=#FFFFFF
NormalBG=#141517
SelectedBG=#0000C6
Font=Arial
```

(S06 §101 reads the reference highlight as (0,0,198) = `#0000C6`, as
corroboration only. The webamp-derived default the parser falls back to is
`#0000FF`.) Changing it will move the playlist golden images in
`crates/rustamp-ui/tests/golden_cases`, which also need regenerating — that is
a second reason it does not belong in a lane that cannot run the suite.

**Second finding, same file, not changed.** `draw_rows` honours `NormalBG`
and `SelectedBG` but **not** `Normal` and `Current`: the row labels are
stamped from the `text.bmp` glyph sheet in whatever colour that sheet bakes
in, because `rustamp_gfx::BitmapFont` has no tint (`rustamp-gfx` is also
outside this lane) and the golden images pin the current output. So the
playing entry does not turn white, and a skin that sets `Normal` sees it
ignored. The fix needs either a tint parameter on `BitmapFont` or a recolour
pass in `rustamp-ui`, plus regenerated goldens.

**Check added.**
`the_selected_row_is_filled_with_the_skins_own_selectedbg` builds a skin whose
`pledit.txt` names a colour no default uses, renders, and asserts that exact
colour inside the band and *not* outside it. It fails if anyone hardcodes a
highlight, and it fails if the band spills onto the side rails or the
scrollbar.

---

## 7. The main menu is now the operating system's own popup (done)

**What was established first, from the code and the specs.** S04 §1 settles
the window class of **every** popup it measured as `#32768` — the class
Windows itself uses for menus. The reference menus therefore *are* system
menus. The player's own `MAIN_MENU_CLASS` constant already recorded that
class, and `window.rs` admitted in a comment that `winit` cannot set a window
class, "so this is an ordinary small window hosting the rendered menu model".

**Both defects had that one cause.** The popup was an ordinary top-level
`winit` window, so it carried the standard caption (defect 1 again, in one
more place). And it was painted by `render_main_menu` with the `text.bmp`
bitmap font: a small fixed-cell glyph sheet drawn into rows the measured
geometry makes 35 px tall (S04 §8's provisional split: 15 × 35 + 4 × 11 =
569). No scale of that sheet reads like a menu, which is exactly what the
owner saw.

**Change.**

* `crates/rustamp-platform/src/popup_menu.rs`: a `PopupRow` tree
  (separator / entry / submenu) and `show_popup_menu` over `TrackPopupMenu`
  with `TPM_RETURNCMD | TPM_NONOTIFY`. Separators are `MF_SEPARATOR`,
  submenus are `MF_POPUP` (so the OS draws the arrow and opens them), ticks
  are `MF_CHECKED`, and an accelerator is joined to its label with a tab —
  which is what makes Windows right-align it in the accelerator column. This
  generalises what the tray popup already did.
* `crates/rustamp-ui/src/menu.rs`: `native_main_menu(state, flags)` turns the
  existing table into that tree, with `MenuChoice` and `choice_for_id` for the
  id the OS reports. The S04 §2 row order, the four separators, the six
  submenu parents (S04 §3.2), the measured accelerators (S04 §3.3, §3.4) and
  the three checkable window rows resolved against live state (S04 §3.1) all
  carry over unchanged. Submenu contents stay the provisional S03-backed
  grouping — and are now reachable: the self-drawn popup could only note that
  a submenu was "deferred", because it had no way to draw one.
* `crates/rustamp-app`: the menu window is gone (fields, attributes, sync,
  paint, key and pointer handling). `Windowed::drain_main_menu` shows the
  system popup from the tick — outside every window procedure, like the tray
  popup — and routes the chosen id through `App::activate_menu_choice`, which
  reuses the existing `App::activate_menu_item` dispatch. No second
  activation path was invented.

**Why this implementation, and what it costs.** It is what the spec settles
(`#32768`), it is what the owner's reference screenshot shows, and the
readability problem disappears by construction because no font of ours is
used. The costs, stated plainly:

* **No colouring.** The menu is the plain system menu. That is deliberate per
  the owner's correction: the colouring in the Winamp 5 screenshot is not
  2.95, and 2.95's menu is the ordinary light-grey system menu. Owner-drawn
  colours would be `MF_OWNERDRAW` plus `WM_MEASUREITEM`/`WM_DRAWITEM`
  handling, and are not wanted.
* **The player is modal while the menu is up.** `TrackPopupMenu` runs its own
  message loop, so the 60 Hz tick does not run and the visualization holds
  still until the menu closes. The tray menu already behaved this way, and so
  does the reference.
* **Retired, not deleted:** `render_main_menu`, `render_submenu`,
  `menu_item_at`, `row_bands`, `App::menu_click` and `App::render_menu` are
  no longer on any live path. They are kept with their S04 geometry receipts
  and their golden tests, and are now documented as retired. Deleting them
  would also delete those receipts and would move golden images this lane
  cannot re-verify; it is a follow-up.
* Keyboard navigation of the menu is now the OS's (arrow keys, accelerator
  letters, Escape), which is more than the self-drawn popup had.

**Checks added** (`crates/rustamp-app/tests/main_menu.rs`, headless):

* `the_main_menu_is_a_native_menu_tree_with_the_measured_shape` — every S04 §2
  row reaches the popup; the four separator rules are separator **rows**; the
  six arrow rows are real **submenus** and none is empty; the measured
  accelerators are carried *separately from* the label (so the OS
  right-aligns them) and no label smuggles a tab.
* `the_native_menus_tick_marks_follow_the_open_windows` — the tick flag
  follows live state.
* `every_id_the_native_menu_can_report_maps_back_to_one_action` — one choice
  per selectable entry, no zero ids, every id maps to its own row, and
  out-of-range ids map to nothing. A mis-mapped id is the one way this path
  can be silently wrong (it would run the wrong action).
* `a_chosen_native_leaf_runs_the_existing_key_dispatch` — the native path
  reuses the existing activation.

**The probe that passed while the defect shipped is gone.** Both
`the_main_menu_popup_opens_at_the_measured_client_size`
(`tests/main_menu.rs`) and `the_menu_popup_opens_at_the_measured_size`
(`src/window.rs`) measured a 323 x 569 client area and passed while that
window carried a caption and drew unreadable text. There is no window of ours
left to measure, so both were removed.

**Needs the owner's machine.** That the popup appears frameless, in the system
font, at the menu-button anchor, with the arrows, ticks and right-aligned
accelerators the OS draws.

---

## Changes needed outside this lane's owned paths (not made)

1. **`docs/notes/vm-run-tests.ps1`** (lines ~525 and ~528) lists the two
   removed VM-only menu probes in its expected-test table:
   `the_menu_popup_opens_at_the_measured_size` and
   `the_main_menu_popup_opens_at_the_measured_client_size`. Both tests no
   longer exist. **Exact change:** delete those two `[pscustomobject]` rows
   from the `Group A (VM GUI)` list. Leaving them makes the runner look for
   tests that are gone.
2. **`crates/rustamp-skin/assets/rust-theme/pledit.txt`** — the selection
   colour the owner called ugly. Exact replacement content in §6 above.
3. **`crates/rustamp-gfx`** (`BitmapFont`) — a text colour/tint, so the
   playlist rows can honour `pledit.txt`'s `Normal` and `Current` (§6,
   second finding). Needs regenerated playlist goldens.
4. Documentation that still describes the retired menu window:
   `docs/tasks/N12-window-ansi.md:87`, `docs/tasks/X5-smoke-window.md:38,48`
   (both mention the `Rustamp Menu` window title),
   `docs/tasks/W12-ignored-inventory.md`, `docs/tasks/W19-ignored-readiness.md`
   and `docs/tasks/X4-ignored-staging.md` (the two removed probes). These are
   historical task cards; only the smoke-window note (`X5`) matters
   operationally, and it only *excludes* the menu title from its wait
   condition, so it stays correct.

## Files touched in this lane

New files (total lines):

| File | Lines |
|---|---|
| `crates/rustamp-platform/src/popup_menu.rs` | 293 |
| `crates/rustamp-platform/src/file_dialog.rs` | 255 |
| `crates/rustamp-ui/src/shell.rs` | 195 |
| `crates/rustamp-app/tests/window_shell.rs` | 126 |
| `docs/tasks/W24-native-frame-and-eq.md` (this card) | 421 |

Modified files (`git diff --numstat`, added/removed):

| File | + | − |
|---|---|---|
| `crates/rustamp-app/src/window.rs` | 590 | 335 |
| `crates/rustamp-app/tests/main_menu.rs` | 188 | 78 |
| `crates/rustamp-ui/src/menu.rs` | 156 | 0 |
| `crates/rustamp-app/src/playlist_window.rs` | 93 | 2 |
| `crates/rustamp-ui/src/playlist_window.rs` | 72 | 0 |
| `crates/rustamp-app/src/menu.rs` | 52 | 7 |
| `crates/rustamp-app/tests/eq_window.rs` | 28 | 8 |
| `crates/rustamp-ui/src/lib.rs` | 12 | 3 |
| `crates/rustamp-ui/src/presenter.rs` | 7 | 4 |
| `crates/rustamp-platform/src/lib.rs` | 6 | 0 |
| `crates/rustamp-platform/Cargo.toml` | 3 | 0 |

Total across modified files: +1207 / −437. `Cargo.lock` is untouched (the two
additions are features of the already-vendored `windows` crate).

## What the equalizer part of this card's name is

The lane was originally also asked to fix a missing equalizer window. The
owner confirmed, before any change was made, that the EQ button **does** open
the equalizer. Nothing in that area was touched: no behaviour changed, no test
asserting it was added. The only EQ-adjacent edits are the shell (the window
is borderless now) and the VM probe's extra `outer_size == inner_size`
assertion.
