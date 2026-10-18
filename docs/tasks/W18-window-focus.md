# Task card W18 — Window focus: `FocusNextWindow` and a real `ShowMainWindow`

Date: 2026-10-18. Status: implemented, formatted, clippy-clean and built;
**tests written, not run** (the suite runs only in the orchestrator's VM).
Desktop acceptance is pending, see the last section.

## Boundary and sources

- Write only this worktree (`wave91-focus`). Owned paths:
  `crates/rustamp-app/**` and this card. No other crate, manifest or lockfile
  was touched; no change outside the app crate is needed.
- Origin: a never-committed lane (`wave26-windows`) preserved as a patch over
  `app.rs`, `lib.rs`, `window.rs` plus a new `focus.rs`. It was re-applied by
  hand to the tree after the F5 tray/hotkey work, not with `git apply`.
- Sources: S03 (ids 40063, 40258), S05 §3.1, `WindowId` and its `Ord` in
  `rustamp-proto`, F5. No Winamp source, SDK material or reference binary was
  read; no dependency added.

## The gap

`C::FocusNextWindow` only noted "is not implemented yet". `C::ShowMainWindow`
only set `main_visible = true`; F5's presenter does un-minimize and focus on a
hidden→shown transition, but a window that is **minimized and still visible**
never changes `main_visible`, so Alt+W did nothing to it.

## What was built

- `focus.rs` (new, pure): `FOCUS_ORDER = [Main, Equalizer, Playlist]` and
  `next_focus(current, is_open)`, which wraps and skips closed windows.
- `App` (`app.rs`): `focused_window`, a pending `focus_request`, a pending
  `restore_request`; `window_open`, `focused_window`, `take_focus_request`,
  `take_restore_request`; `focus_next_window`, `show_main_window`,
  `maintain_focus`.
- Presenter (`window.rs`): `Windowed::apply_focus_requests`, called at the end
  of the `about_to_wait` window syncs.

## Adaptation to the current tree (what differs from the old patch)

- `window_open(Main)` is `main_visible`, not constant `true`: since F5 the main
  window can be hidden, and a hidden window cannot take focus. If nothing is
  open the cycle gives no request instead of focusing a hidden window.
- `maintain_focus` leaves a focused/hidden Main alone (it is the fallback) and
  only requests focus when Main is visible.
- **The old tests assumed the playlist starts closed; it now starts open**
  (`playlist_open` defaults true). The tests use a `set_open` helper that
  states the wanted open/closed set explicitly instead of relying on defaults.
- `App::playlist_open` already exists; the patch's copy was dropped.
- `ShowMainWindow` now sets `main_visible = true` as well (the old patch
  predates `main_visible`), so it still recovers a window hidden by the tray.
- `Minimize` now clears a pending restore and a pending main-focus request, so
  the later of Minimize/ShowMainWindow wins in both directions (the patch only
  covered Show-after-Minimize).
- The presenter takes both requests on every call whether or not an OS window
  exists, so a request for a closed or absent window is dropped, not kept.
  It runs after `sync_eq_window` / `sync_playlist_window`, so a window opened
  in the same frame already exists when it is asked to take focus. The patch's
  `focus_window` helper is folded into it.
- The patch's `#[ignore]` VM test only re-asserted the constant array; it
  proved nothing and was dropped.
- Module comment corrected against the tree's S03: 40063 is `provisional`
  (no menu label), only 40258 is `menu`; the patch said both were `menu`.
  Also noted that for the three windows the app has the order agrees with
  `WindowId`'s `Ord`; the choice is still not taken from it.

## Honesty about the order

The focus order is an **own choice, not a measurement**. No capture measures
which window the reference picks. `focus.rs` says so and keeps the recipe for
a measurement: post `WM_COMMAND` 40063 to each of the three windows in turn,
read back the keyboard-focus window, and repeat with each window closed to
learn the skip and the restart point. The closed-window skip and the choice to
exclude a hidden main window are also own choices.

## Tests (written, not run)

`app.rs`: `focus_next_window_visits_the_open_windows_in_the_stated_order_and_wraps`
(six steps over three windows), `focus_next_window_skips_a_closed_window`,
`focus_requests_are_consumed_once`, `closing_the_focused_window_falls_back_to_main`,
`a_hidden_main_window_is_not_focusable`,
`show_main_window_shows_restores_and_focuses_once`,
`the_later_of_minimize_and_show_main_window_wins`,
`focus_commands_keep_the_main_window_geometry`.
`focus.rs`: seven pure tests on the order, skip, wrap, fallback and
out-of-cycle ids. `window.rs`:
`the_presenter_consumes_focus_and_restore_requests_without_leaking_them`
(headless; no OS window exists in it).

## Verification receipt

| Command | Result |
| --- | --- |
| `cargo fmt --all -- --check` | exit 0 |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | exit 0 |
| `cargo build --workspace` | exit 0 |

`cargo test` was **not** run; no window was opened, no audio touched, no VM
used. The test results are unknown until the orchestrator runs them.

## What only a real desktop can decide

Headless tests decide the model: the cycle, the skip, the wrap, the single
consumption of requests. They cannot decide:

1. Whether Windows really gives the **keyboard** to the window the model picks.
   `Window::focus_window` is a request; Windows' foreground-lock rules may turn
   it into a taskbar flash instead, and the model's `focused_window` is then
   wrong until the next `Focused` event. (Nothing here feeds OS activation back
   into the model; that is open.)
2. Whether the restore actually **un-minimizes** a minimized main window and
   raises it, including after the tray hid it.
3. That Ctrl+Tab and Alt+W reach the command at all when a secondary window
   (equalizer, playlist) is the focused OS window.
4. Which order the reference uses (the measurement above).

Checklist for the VM: open equalizer and playlist, press Ctrl+Tab repeatedly
and watch the active title bar and the typing target; close one window and
repeat; minimize the main window and press Alt+W; hide it via the tray and
press Alt+W.
