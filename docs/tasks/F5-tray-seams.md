# Task card F5 — Apply the F3 tray and hotkey seams

Date: 2026-10-18. Status: implemented and compile-checked; test execution and
interactive-desktop acceptance are pending with the orchestrator.

> **See "Follow-up F5a — Close the three tray-icon gaps" at the end of this
> card.** A verification pass confirmed both seams below and found three gaps:
> the left click was inert, there was no shell-restart recovery, and the glyph
> was the stock application icon. F5a closes all three and records an executed
> test run. Where the sections below say "stock icon" or "no `TaskbarCreated`
> recovery", F5a supersedes them; everything else here still holds.

## Boundary and sources

- Write only this worktree:
  `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave85-tray-seams`.
- Owned paths: `crates/rustamp-app/**`, `crates/rustamp-platform/**`, this card.
  No edits to UI, audio, any other crate, manifests or lockfile.
- Nothing under `D:\code\Rustamp`, `D:\code\rustamp-research` or
  `D:\code\rustamp-refbin` was read or written by this lane.
- Sources: the repository's clean-room policy, plan and contribution rules;
  F3's existing model and explicit VM-checklist seams; F4's seam pattern;
  BETA-readiness-2 BETA-11; existing app/platform code; locally cached
  `windows` 0.62.2 and `winit` 0.30.13 API signatures/documentation.
- No Winamp source, SDK material, reference binary or disassembly was read.
  No third-party code was copied, dependency added or download performed.

## The two F3 seams, applied

1. **Hotkey drain, next to F2 in the UI pump:**
   `crates/rustamp-app/src/window.rs:1594`, `Windowed::about_to_wait`, calls
   `submit_hotkeys(take_pending_hotkeys())`. The consumer at `window.rs:1314`
   expands each existing `HotkeyAction::commands()` in queue order; the common
   submission path at `window.rs:1320` calls `App::submit` once per command.
   It runs before the existing null-clock/pump/window-state sync. The normal
   16 ms wake schedule continues even with the main window hidden.
2. **Attach when the main HWND exists, then route menu picks:**
   `window.rs:1422`, `Windowed::resumed`, calls `attach_tray` after storing the
   main window and surface. `attach_tray` (`window.rs:1331`) obtains the raw
   Win32 HWND through winit's safe raw-handle API and calls `TrayIcon::attach`.
   The existing `resumed` early return prevents duplicate attachment.
   `drain_tray_menu` (`window.rs:1354`) consumes callback requests in
   `about_to_wait`, passes unchanged `TRAY_MENU` to the native popup, and
   submits the selected `entry.commands()` through the same common path.

`rustamp-app` keeps `#![forbid(unsafe_code)]`. The necessary native presenter
adapter therefore lives alongside the existing attach backend in
`crates/rustamp-platform/src/tray.rs`: `tray_callback` (`:164`) subclasses the
owner and coalesces `TRAY_CALLBACK_MESSAGE` right-click/context-menu requests,
without calling App or storing Rust pointers in the HWND. `show_menu` (`:306`)
builds the Win32 popup from the supplied entries and their existing labels.
`TPM_RETURNCMD | TPM_NONOTIFY` returns a selection instead of sending an
unrelated `WM_COMMAND` through winit. One-based private popup ids are resolved
back to model entries (`menu_entry_for_id`, `:121`); zero/unknown ids dispatch
nothing. Every created native menu is destroyed on all return paths.

The guard is declared before the window/surfaces and explicitly removed in
`Windowed::exiting` (`window.rs:1638`). Its Drop removes both the icon and
subclass, and clears queued requests. The subclass also removes itself at
`WM_NCDESTROY`. No callback uses an App pointer or re-enters command routing.

## Model unchanged; one previously inert application effect completed

- `HotkeyRegistry`, hotkey vocabulary/defaults, `TrayMenuEntry`, `TRAY_MENU`,
  labels, public shortcut ids and existing command mappings are unchanged.
  `hotkeys.rs` is untouched. No core/wire/config contract changed.
- Backend-only additions: callback/popup handling and diagnostic `TrayError`
  variants. These are needed because winit's `WindowEvent` does not expose
  the private tray message and the app forbids unsafe Win32 bindings.
- `ToggleWindow(Main)` previously only logged an unimplemented-window note.
  App now owns runtime `main_visible`; the presenter applies it with
  `set_visible`, restoring/unminimizing and requesting focus on a hidden-to-shown
  transition. The existing `ShowMainWindow` command sets the same state true.
  No UI-crate model was extended. Only the main window is toggled; docked
  neighbours retain their independent open flags.
- Visibility starts true on each launch, as before, so an unavailable tray
  cannot make a saved hidden state strand the next launch. No persisted
  visibility semantics were added. Native popup placement/dismissal and this
  runtime show/hide policy are own choices, not new reference-player claims.

## Failure policy

- Existing `main.rs` session lifetime and registration warnings are retained:
  a refused chord warns; the player keeps the registrations it obtained.
- Raw-handle, owner-thread, callback-install, stock-icon and NIM_ADD failures
  warn in `attach_tray`, leaving the player running without an icon. An add
  refusal also removes the just-installed subclass (the local guard drops).
- Popup create/populate/cursor/track failures warn, release the menu and return
  to the pump. Cancellation dispatches nothing. Foreground activation and the
  post-popup no-op are best-effort, not reasons to abort playback/startup.
- Command-submission errors warn per command and do not discard later items.
  Both native request and existing hotkey queues recover poisoned locks.
- No attach retry loop or shell-restart (`TaskbarCreated`) recovery was added;
  a refused initial attachment stays absent for this owner/window lifetime.

## Tests and limits

Four new headless tests are written and type-checked, **not executed**:

- `window.rs:1865`: the actual drain consumer submits play/pause/resume,
  next/previous/stop and show/hide through a real App command channel with
  NullOutput and the existing FLAC fixture; checks order, a single toggle and
  an empty batch. No event loop, window, hotkey registration or host spawn.
- `window.rs:1892`: tray selections use that same submission path for
  transport, visibility/recovery and Quit (not merely popup dismissal).
- `window.rs:1919`: Play on an empty queue does not discard the following
  visibility action or prevent a subsequent toggle.
- `tray.rs:411`: every native popup position resolves to the exact model entry
  and command; cancellation, out-of-range ids and an empty menu resolve to none.

The existing F3 tests separately pin queue draining/FIFO and registration
refusal/unregister bookkeeping. The new consumer tests inject action batches;
they do not simulate a real WM_HOTKEY, execute `about_to_wait` with an event
loop, or prove delivery while another process has focus. No test-pass claim
is made from compilation.

**Orchestrator / interactive Hyper-V VM checklist (not run here):**

1. Run the headless suites, including these four new tests and F3's tests.
2. Launch the windowed player (`--no-audio` suffices for platform checks):
   observe one icon/tooltip, right-click it, select every row, cancel the menu,
   show/hide twice including a minimized window, and confirm Exit quits the
   player and removes the icon. Verify dismissal while the owner is hidden.
3. With another application focused, fire all six default chords and verify
   the mapped state changes. Registration alone is not delivery evidence.
4. Hold a default chord in another process before launch: verify its warning,
   successful startup and delivery of the other chords. Exercise a shell/icon
   refusal where feasible: verify startup continues and no inert icon remains.
5. Check clean exit and abrupt process loss for no orphaned icon/registration.
   Pixel visibility, real input, OS cleanup and foreground behavior require
   this desktop check; headless model/consumer tests cannot establish them.

## Verification receipt

- `cargo fmt --all -- --check`: exit 0 after correcting local formatting.
- `cargo clippy --offline --workspace --all-targets --all-features -- -D warnings`:
  exit 0 (2.29 s final run); all targets' test code type-checked only. The first
  pass caught a test index typed usize instead of the engine's u32; corrected.
- `cargo build --offline --workspace`: exit 0 (55.07 s).
- `git diff --check`: exit 0.
- No `cargo test`, test binary, player/example, window, audio device, reference
  binary or VM was run. No commit, staging or other intentional git write.
- Final worktree `git status --short` is recorded in the lane report. Build
  artifacts remain under this worktree's ignored `target` directory.

---

# Follow-up F5a — Close the three tray-icon gaps

Date: 2026-10-18. Status: implemented, formatted, clippy-clean, built **and
tested** (real numbers below). Interactive-desktop acceptance still pending.

This is a follow-up section on F5's own card rather than a sibling card: the
three gaps are defects in the seams the sections above describe, and splitting
them across two files would have duplicated the whole boundary and failure
policy. The F5 sections above are unchanged; everything new is here.

## Boundary and sources

- Write only this worktree:
  `C:\Users\micro\.paseo\worktrees\3ikyavhh\paseo-tray-click`
  (branch `paseo-tray-click`, base = `main` 3b5309b).
- Owned paths: `crates/rustamp-app/**`, `crates/rustamp-platform/**`, this
  card. Touched: `crates/rustamp-app/src/window.rs`,
  `crates/rustamp-platform/src/tray.rs`,
  `crates/rustamp-platform/src/lib.rs`, this card. **Nothing else** — no
  manifest, no lockfile, no `build.rs`, no `hotkeys.rs`, no other crate, no
  UI/audio/core/wire/config contract.
- Nothing under `D:\code\Rustamp`, any other worktree, or any
  research/reference tree was read or written by this lane.
- Sources: F5's own card and code, the repository's clean-room policy, and the
  locally cached `windows` 0.62.2 signatures (`CreateDIBSection`,
  `CreateBitmap`, `CreateIconIndirect`, `DestroyIcon`, `GetSystemMetrics`,
  `RegisterWindowMessageW`). No Winamp source, SDK material, reference binary,
  disassembly or third-party image was read. No dependency added, no download.

## Gap 1 — the icon answers the left click too

`tray_callback` queued only `WM_RBUTTONUP | WM_CONTEXTMENU`, so a left click
did nothing and a hidden window could only be recovered through the context
menu.

- **The mapping is a named model const, not an invented command:**
  `TRAY_LEFT_CLICK` (`crates/rustamp-platform/src/tray.rs:82`) is
  `TrayMenuEntry::ToggleMainWindow` — literally `TRAY_MENU[0]`. It lives next
  to `TRAY_MENU`, is `pub`-exported (`lib.rs:39`) and is headlessly testable
  on its own.
- **The callback still only queues:** `tray.rs:348` routes `WM_LBUTTONUP` and
  `WM_LBUTTONDBLCLK` into `LEFT_CLICKS` (`tray.rs:312`), a queue **separate**
  from `MENU_REQUESTS`. The icon keeps the original (not
  `NOTIFYICON_VERSION_4`) callback format, so the mouse message arrives in
  lParam; a double click arrives as down/up/dblclk/up and both `up` and
  `dblclk` coalesce into the one pending request, so the pump sees **one**
  toggle, not two or four. `queue_request`/`take_request`/`clear_requests`
  (`tray.rs:370`, `:381`, `:390`) are shared by all three queues and recover
  poisoned locks.
- **The app submits it through the menu's own path:**
  `Windowed::drain_tray_left_click` (`crates/rustamp-app/src/window.rs:1384`)
  takes the request (`TrayIcon::take_left_click`, `tray.rs:626`) and calls
  `submit_platform_commands(TRAY_LEFT_CLICK.commands())` — the same function
  `drain_tray_menu` (`window.rs:1397`) uses for a menu pick, which is the same
  `App::submit` channel F2's forwarded commands and F3's hotkeys use.
- **The split holds both ways.** A left click never opens the popup (it never
  touches `MENU_REQUESTS`, and only `drain_tray_menu` calls `show_menu`); a
  right click never toggles the window (it never touches `LEFT_CLICKS`, and
  the pick that toggles is the explicit show/hide row the user chose).

## Gap 2 — the icon comes back after an Explorer restart

Nothing listened for the shell's `TaskbarCreated` broadcast, so a shell restart
silently lost the icon for the rest of the run.

- **The id is registered once:** `SHELL_RESTART_MESSAGE: OnceLock<u32>`
  (`tray.rs:319`), filled by `shell_restart_message` (`tray.rs:321`) from
  `RegisterWindowMessageW("TaskbarCreated")`. `attach` primes it
  (`tray.rs:607`) before the subclass is installed, so the window procedure
  only ever reads the cached value.
- **Zero is guarded, and that guard is a pure function.**
  `RegisterWindowMessageW` returns 0 on failure and 0 *is* `WM_NULL` — the most
  common message there is, and one `show_menu` itself posts to the owner after
  the popup. `is_shell_restart(registered, message)` (`tray.rs:148`) is
  therefore `registered != 0 && message == registered`: an unregistered id
  matches nothing at all. It is a free function precisely so it can be tested
  headlessly.
- **The existing subclass sees the broadcast** (`TaskbarCreated` goes to every
  top-level window, and the owner HWND is one): `tray.rs:356` queues a
  coalesced request into `SHELL_RESTARTS` (`tray.rs:314`) and then **forwards
  the message on to `DefSubclassProc`** like every other unrelated message.
- **The re-add runs in the UI pump, through the attach path.** The
  `NOTIFYICONDATAW` + `Shell_NotifyIconW(NIM_ADD)` body is factored out as
  `notify_icon_data` (`tray.rs:516`) and `add_icon` (`tray.rs:536`); both
  `TrayIcon::attach` (`tray.rs:568`) and `TrayIcon::readd` (`tray.rs:642`)
  call it, so there is one description of the icon, not two.
  `Windowed::readd_tray_after_shell_restart` (`window.rs:1366`) drives it.
- **No second subclass, no second icon, no retry loop.** The subclass survives
  the shell restart and is deliberately *not* reinstalled. The previous icon
  went away with the previous taskbar, so `NIM_ADD` adds one icon. A refusal
  prints `rustamp: tray icon lost after a shell restart: ...` to stderr and the
  player keeps running, exactly like the existing attach failures.
- `Windowed::drain_tray` (`window.rs:1355`) is the one pump entry point
  (`window.rs:1639`, in `about_to_wait` where `drain_tray_menu` used to be
  called). It re-adds first, then drains the menu, then the left click, so a
  just-restored icon is live before this frame's clicks are considered.

## Gap 3 — the icon is the product's, not the stock glyph

### Provenance and the route chosen

The icon is **our own trivial art, committed as source** in
`crates/rustamp-platform/src/tray.rs`, module `art` (`tray.rs:152`): a palette
of four entries (`PALETTE`, `tray.rs:188`) plus sixteen 16-glyph ASCII rows
(`ROWS`, `tray.rs:197`) drawing a dark rounded plate with a green play
triangle. It was drawn for this crate in this lane. **No Winamp artwork, no
reference binary, no extracted resource and no third-party image** was used,
and it is deliberately *not* taken from `crates/rustamp-skin` — that is lane
J's, and skin data is not product branding.

Why this route and not the executable's own icon resource: the repository is
offline clean-room with no `.ico`, no `.rc`, no `build.rs` and neither `winres`
nor `embed-resource` anywhere. The resource route would need a new build
dependency plus manifest and lockfile edits, which this lane does not own. A
committed pixel map needs none of that, and unlike a binary `.ico` blob it is
reviewable in the diff.

### How it becomes an `HICON`

`art::scaled_bgra` (`tray.rs:246`) expands the rows to **straight-alpha BGRA**,
top-down, four bytes per pixel, nearest-neighbour scaled — so the plate's
straight edges stay crisp at 150 % and 200 % DPI rather than blurring.
`small_icon_size` (`tray.rs:441`) asks `GetSystemMetrics(SM_CXSMICON)` and
falls back to the art's own 16 px for a refused or absurd metric.
`build_own_icon` (`tray.rs:455`) creates a 32-bpp top-down `CreateDIBSection`,
copies the pixels in, pairs it with an explicitly zeroed monochrome
`CreateBitmap` mask (required by `ICONINFO`, ignored for a 32-bpp colour bitmap
with alpha, but never left undefined), and calls `CreateIconIndirect`. Both
bitmaps are released on every path by the `Bitmap` guard (`tray.rs:428`).
Every palette entry is either fully transparent or fully opaque, so the
straight and premultiplied forms of this art are byte-identical and the choice
cannot show up as a dark fringe.

All of these APIs are already enabled by this crate's existing `windows`
features (`Win32_Graphics_Gdi`, `Win32_UI_WindowsAndMessaging`): **no manifest
change was needed or made.**

### Lifetime

`OwnedIcon { handle, ours }` (`tray.rs:422`) records who owns the handle. The
guard takes ownership before any later fallible step in `attach`, so a refused
subclass or a refused `NIM_ADD` destroys the icon instead of leaking it.
`Drop` (`tray.rs:737`) removes the icon from the shell first and then calls
`DestroyIcon` **only when `ours` is true** — a stock `IDI_APPLICATION` handle
is shared process-wide and must never be destroyed. The handle stays in the
guard, which is what makes the gap-2 re-add possible.

### What a missing icon degrades to

The required chain, in order:

1. Our own icon cannot be built: warn on stderr (`rustamp: the tray icon
   artwork could not be built, falling back to the stock icon: ...`,
   `tray.rs:585`) and load the stock `IDI_APPLICATION`, marked `ours: false`.
2. The stock icon also fails: return `TrayError::IconLoad`. `attach_tray`
   (`window.rs`) warns and the player **runs with no tray icon at all**.
3. Startup and playback are **never** aborted for an icon, at any step.

## Model unchanged

`TrayMenuEntry`, `TRAY_MENU`, the labels, `command_id`, `commands`, the
one-based popup ids, `TPM_RETURNCMD | TPM_NONOTIFY`, `menu_entry_for_id` and
every command mapping are byte-for-byte as F5 left them. `TRAY_LEFT_CLICK` is
an alias for an **existing** entry, not a new one, so the tray still invents no
command and the set of commands the tray can produce is exactly what it was.
`HotkeyRegistry`, the hotkey vocabulary and `hotkeys.rs` are untouched. No
core/wire/config contract changed. `rustamp-app` keeps
`#![forbid(unsafe_code)]`; every Win32 call added here is in
`crates/rustamp-platform/src/tray.rs` with a `// SAFETY:` comment in the
existing style.

## Failure policy (additions to F5's)

- A refused shell-restart re-add warns and the player keeps running. No retry
  loop, no second subclass, no second icon.
- A failed `RegisterWindowMessageW` warns nothing and simply disables
  shell-restart recovery: the predicate matches nothing, so no `WM_NULL` can
  masquerade as a restart.
- The icon chain is the three steps above.
- The native callback still only queues: no App pointer, no Rust pointer in the
  HWND, no re-entry into command routing. Every created native menu and GDI
  object is released on every path. All three queues recover poisoned locks and
  are cleared at `WM_NCDESTROY` and in `Drop`.

## New tests (seven, all executed)

In `crates/rustamp-platform/src/tray.rs`:

- `:849` the left-click mapping resolves to the same model entry and the same
  command as the menu's show/hide row (`menu_entry_for_id(TRAY_MENU, 1)`).
- `:868` left and right stay distinct: popup id 0 (no selection) dispatches
  nothing, no other row's commands are reachable by a left click, and Quit in
  particular is not a left click away.
- `:885` the shell-restart predicate never matches when the registered id is 0
  — not even `WM_NULL` — and never matches `WM_NULL` for a valid id.
- `:902` the artwork has fixed dimensions (16 rows x 16 glyphs), uses only
  palette glyphs, every palette entry is actually drawn, and no colour repeats.
- `:932` transparent where intended (all four rounded corners), opaque where
  intended (the whole middle band), the accent lands where the triangle was
  drawn in BGRA order, and no transparent pixel carries colour.
- `:970` the scaled buffer is exactly `size * size * 4` bytes for nine sizes
  including 0 and 1; 16 px round-trips to the source; at 200 % the triangle
  and the cut corner both survive.

In `crates/rustamp-app/src/window.rs`:

- `:1964` a left click hides and then shows the main window through the **real
  App command channel** (`desktop_test_presenter` → `submit_platform_commands`
  → `App::submit`, checked via `app.main_visible()`), agrees with the menu's
  show/hide row, never quits, never moves the transport, and a right-click
  transport pick does not toggle the window. No event loop, window, hotkey
  registration, host spawn or audio device — `presenter.window.is_none()` is
  asserted.

All existing F3/F5 tray and hotkey tests pass unchanged; none was edited.

## Verification receipt — actually run, offline

| Command | Result |
| --- | --- |
| `cargo fmt --all -- --check` | exit 0 |
| `cargo clippy --offline --workspace --all-targets --all-features -- -D warnings` | exit 0 (4.41 s, final re-run) |
| `cargo build --offline --workspace` | exit 0 (1 m 08 s cold; 9.11 s on the final re-run) |
| `cargo test --offline -p rustamp-app -p rustamp-platform` | exit 0 — **247 passed, 0 failed, 11 ignored** across 14 suites |
| `git diff --check` | exit 0 |

Per-suite (passed/failed/ignored): `rustamp-app` lib 152/0/4 · `rustamp` bin
0/0/0 · `tests/config.rs` 6/0/0 · `tests/eq_window.rs` 0/0/1 ·
`tests/first_minute.rs` 9/0/0 · `tests/headless.rs` 9/0/1 ·
`tests/loopback.rs` 1/0/1 · `tests/main_menu.rs` 4/0/1 ·
`tests/playlist_window.rs` 0/0/1 · `tests/plughost_e2e.rs` 1/0/1 ·
`tests/real_playback.rs` 0/0/1 · `rustamp-platform` lib 65/0/0 ·
both doc-test suites 0/0/0.

The 11 ignored are the pre-existing `NO_GUI_ENV`-gated window, real-device and
host-spawn tests; this lane added none and un-ignored none. The seven new tests
account for the whole delta from the 240 that passed before them.

Two clippy findings from the first pass were corrected, not suppressed: the
`art` module mixed an outer `///` with inner `//!` docs
(`mixed_attributes_style`), and an alpha sweep used `chunks_exact(4)` where
`as_chunks::<4>()` is the current form.

The windowed player was **not** run. No window was opened, no audio device
touched, no VM used, no reference binary read. No commit, staging, push or git
stash operation was performed; the work is left in the working tree.

## Interactive-VM checklist — what only a real desktop can prove

Headless tests pin the mappings, the predicate and the pixel data. They cannot
pin OS behaviour. In the VM, with `--no-audio`:

1. **Icon pixels.** The tray shows *our* plate-and-triangle glyph, not the
   generic application icon, and the tooltip reads `Rustamp`. Only a real
   notification area can show that `CreateIconIndirect` produced a visible
   icon with correct transparency and no dark fringe.
2. **Real Explorer restart.** Kill and restart `explorer.exe`: exactly **one**
   icon returns, it is still ours, and both buttons still work afterwards —
   confirming the broadcast arrived, the subclass survived, and the re-add
   created no duplicate. Then force the re-add to fail (restart Explorer while
   blocking the add, if feasible) and confirm the warning plus continued
   playback.
3. **Click delivery and the split.** Left-click: the window hides, then shows,
   restored and focused, with **no popup**. Double-click: exactly **one**
   toggle, not two or four. Right-click: the popup opens and **nothing
   toggles**; cancel it; select every row. Repeat the right-click while the
   main window is hidden (foreground activation is best-effort there).
4. **DPI sizes.** At 100 %, 150 % and 200 % scaling the icon is sized from
   `SM_CXSMICON` and stays crisp; check a scale change during a run.
5. **No orphaned icon.** Quit via the Exit row and via the window close box:
   the icon disappears. Then kill the process abruptly: confirm whether a
   ghost icon remains until the next mouse-over (OS cleanup, not ours) and
   that no icon survives a subsequent clean launch/exit. Also confirm the
   stock-icon fallback path leaves nothing behind.
6. Re-run F5's own checklist items 1-5 above; this follow-up does not replace
   them.
