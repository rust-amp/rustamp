# Task card F5 — Apply the F3 tray and hotkey seams

Date: 2026-10-18. Status: implemented and compile-checked; test execution and
interactive-desktop acceptance are pending with the orchestrator.

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
