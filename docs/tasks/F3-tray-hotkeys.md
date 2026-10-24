# Task card F3 — Tray icon and global hotkeys (M2)

**Goal**

Give the player its first real desktop presence beyond the main window: a
tray icon that shows the player is running and offers the expected actions
(show/hide the main window, play, pause/unpause, stop, next, previous, exit),
and system-wide hotkeys that map onto existing commands through the shortcut
action vocabulary rather than inventing one. Registering a taken combination
must fail loudly, and unregistering must be clean.

**Lane** F — platform (`rustamp-platform` plus the app start-up wiring).

**Owned paths**
- `crates/rustamp-platform/**` (new `tray.rs`, `hotkeys.rs`; `lib.rs`
  re-exports; two `windows` features added to `Cargo.toml`)
- `crates/rustamp-app/src/main.rs` (start-up wiring only)
- this card, `docs/tasks/F3-tray-hotkeys.md`

No edit outside those paths was made. In particular `app.rs` and `window.rs`
are untouched: the two `window.rs` seams below are a follow-up, not this
change.

**Allowed sources**
- `docs/PLAN.md` §2 (menus and tray via native popups) and §5 M2 (tray,
  global hotkeys, media keys and SMTC, single instance and CLI).
- `docs/tasks/BETA-readiness.md` BETA-11 (the gap this card narrows).
- `specs/S03-shortcuts.md` §§1, 4, 6, 7 (key-to-command ids; the global table;
  how the tables combine is provisional; system-wide hotkeys unmeasured).
- `specs/S02-main-window-behaviour.md` §1 (a file on the command line starts
  playing — reused as the hotkey/tray mapping precedent: reuse, don't invent).
- The existing `crates/rustamp-platform` (F1 DPI, F2 single instance) and the
  `windows` crate 0.62 API (verified against the vendored sources in the local
  Cargo registry; no Winamp material read).
- `docs/tasks/F2-single-instance.md` (the queue-and-drain pattern this card
  copies for hotkey events).

S04 (menu trees) and S13 (single instance/switches) do not exist as specs.
The behaviours they would have fixed are therefore **own choices**, stated as
such below; none is promoted from a guess to a fact.

**Forbidden sources**
- Winamp 5.x source, Winamp SDK headers or documentation, disassembly, the
  Winamp 2.95 binary, `D:\code\rustamp-research`, `D:\code\rustamp-refbin`.
- No downloads, no `cargo add`, no new dependencies, nothing outside
  `Cargo.lock`. (Only `windows` **features** were added —
  `Win32_UI_Input_KeyboardAndMouse` for `RegisterHotKey` and `Win32_UI_Shell`
  for `Shell_NotifyIconW` — still nothing new is downloaded, because `windows`
  is one crate with feature-gated modules; same precedent as F2.)
- No reference binary run, no window, no audio.

**Scope**
- **In scope**
  - A `rustamp-platform` tray module: the menu model (`TrayMenuEntry`,
    `TRAY_MENU`), each entry's label, its S03 id where one exists, and its
    mapping onto existing `Command`s; plus the Win32 icon attach guard
    (`TrayIcon::attach` / `Drop` removes the icon).
  - A `rustamp-platform` hotkeys module: the headless chord/action model, the
    default `Ctrl+Alt` table, the `HotkeyRegistry` bookkeeping over a
    `HotkeyBackend` trait, the Win32 backend (`RegisterHotKey` /
    `UnregisterHotKey` + `WM_HOTKEY` pump), the process-global pending queue,
    and the session thread that holds the defaults for the whole run.
  - Start-up wiring in `crates/rustamp-app/src/main.rs`: the first (windowed)
    instance starts the hotkey session and keeps it alive; a refused chord is
    a stderr warning, never a startup failure; `--null` registers nothing.
- **Out of scope:** media keys/SMTC, taskbar thumb buttons, jump lists, file
  associations, the external-control window, drag-and-drop, the S04 menu
  contents, any skin artwork (lane J), and the two `window.rs` follow-up seams
  below (a lane that owns `window.rs` wires them; this lane was forbidden to
  touch that file).

**Interface contract**
- `rustamp-platform` keeps depending only on `rustamp-core`, `thiserror` and
  `windows` (Win32). It does **not** depend on `winit`, `softbuffer` or
  `rustamp-ui`, and it does not call `App` (F2 contract, kept).
- No `rustamp-core` contract changes: every tray entry and every hotkey maps
  onto an existing `Command` variant (`Play`, `TogglePause`, `Stop`, `Next`,
  `Prev`, `ToggleWindow(Main)`, `Quit`). The shortcut reuse is by id and
  documented name (`HotkeyAction::shortcut_action` /
  `HotkeyAction::command_id`), not by type, because the platform crate may not
  name `rustamp-ui::ShortcutAction`.
- Fired hotkeys queue process-globally; the UI thread drains them with
  `take_pending_hotkeys()` and turns each into `action.commands()` through the
  normal command path (F2's `take_pending` shape, same few lines).
- Tray selections call `entry.commands()` at the popup site (`window.rs`
  follow-up); the model itself is pure.

**Design (and why)**

- **Tray menu = data, not code.** `TRAY_MENU` lists seven entries in display
  order; `label()`, `command_id()` and `commands()` are total functions over
  it. The popup itself is presenter detail (built in `window.rs` on
  `TRAY_CALLBACK_MESSAGE`), so the model carries no separators or HWNDs.
- **One show/hide entry, one toggle command.** `ToggleMainWindow` maps onto
  `Command::ToggleWindow(WindowId::Main)` — no visibility state is needed
  headless, and the presenter resolves the toggle. It borrows no S03 id: S03
  records no toggle-main-window row (`Alt+W` 40258 is a one-way *show*), and
  borrowing that id for a hide would lie about what it means.
- **Pause means `TogglePause`.** The entry and the hotkey reuse the S03 C-key
  "Pause / unpause" action (40046), not a new pause command: one chord, one
  entry, works from either transport state.
- **Defaults are `Ctrl+Alt` chords.** S03 uses `Ctrl+Alt` for only three
  in-app rows (`I`, `N`, `R`), so the shape stays out of the way; the letters
  echo the main table's mnemonics (`X` play, `C` pause, `V` stop, `B` next,
  `Z` previous) and `H` (unbound in every S03 table) shows/hides.
  `Ctrl+Alt+N` is deliberately avoided — it is S03's provisional "new
  instance" (40305).
- **Registry is the single registrar.** Every OS registration in this process
  goes through `HotkeyRegistry`, so an in-process double registration is
  caught before the OS sees it (`AlreadyBound`, backend untouched). An OS
  refusal (`BackendTaken`) records nothing, so a retry never wedges;
  `unregister` removes the binding first and unregisters exactly once
  (`UnknownId` touches no backend; a backend failure still leaves the chord
  reusable by construction).
- **`RegisterHotKey` with a null window.** `WM_HOTKEY` then lands in the
  calling thread's queue, so the session thread registers on itself and pumps
  only `WM_HOTKEY` (`PeekMessageW` filtered to 786 exactly — it never steals
  another thread's messages). Modifier bits match the Win32 `MOD_*` values by
  construction, so the backend passes them through unchanged.
- **Session lifetime = process lifetime.** `main.rs` holds the
  `HotkeySession` guard next to the F2 `Instance` guard; `Drop` stops the
  thread and joins it when it has already stopped (a still-pumping thread
  detaches instead of hanging shutdown; its hotkeys die with the process,
  which exits right after). `--null` starts no session, like F2 starts no
  lock.
- **Tray attach needs the main `HWND`.** That handle exists only after
  `window.rs` builds the OS window, so `TrayIcon::attach` is provided compiled
  (stock `IDI_APPLICATION` icon — no invented artwork) but called from the
  `window.rs` follow-up, not from `main.rs`. `Drop` sends `NIM_DELETE`: an
  icon can never outlive its owner.
- **Degrade, never wedge** (F2 principle): a refused chord is a warning, a
  thread that never reports leaves warnings empty rather than blocking
  startup, and an icon refusal fails the attach call without taking the
  player down.

**Own choice vs spec-settled**

| Item | Status |
|---|---|
| Transport/window target ids (Play 40045, Pause/unpause 40046, Stop 40047, Next 40048, Previous 40044) and the `Command` variants | Settled (S03; the mapping reuses them) |
| Tray membership, order, labels; tooltip text; callback message `WM_APP + 1` | Own choice (S04 not started) |
| Show/hide as one toggle entry onto `ToggleWindow(Main)` with no borrowed id | Own choice, stated in code |
| Default `Ctrl+Alt` chords (X/C/V/B/Z/H) and their letters | Own choice (S03 §7 leaves system-wide hotkeys unmeasured) |
| Exit entry onto `Quit` with no id (S03 has no quit accelerator row) | Own choice, stated in code |
| Wire/threading shape (null-window hotkeys, session thread, global queue) | Own choice (follows the F2 precedent) |

**Acceptance tests**
- [x] `cargo fmt --all -- --check` clean.
- [x] `cargo clippy --workspace --all-targets --all-features -- -D warnings`
      clean.
- [x] `cargo build --workspace` (offline) green.
- [x] Headless unit tests against the real code (compiled by `clippy
      --all-targets` here; **not run on this host** — tests run in the Hyper-V
      VM only): tray menu order/membership, every entry's exact `Command`
      mapping, S03 id pinning, label uniqueness; defaults distinct and all
      `Ctrl+Alt`; every default's exact `Command` mapping; taken chord fails
      loudly with exactly one backend call; unregister is clean (log shows
      `register 1 / unregister 1 / register 2`, fresh id); unknown id fails
      with an untouched backend; backend refusal records nothing and the retry
      wins; the pending queue drains in order; `describe` output.
- [ ] `cargo test --workspace --all-features` green in the test VM (the new
      tests were compiled but not run here).
- [ ] **Needs the VM / a real desktop** (nothing below can be exercised
      headless; opening a window or registering a hotkey on this host is
      forbidden):
  1. `RegisterHotKey` round-trip: defaults register on a clean VM logon
      session, a chord taken by another app warns (not wedges), firing each
      default delivers the mapped `Command` (play/pause/stop/next/previous
      audible or state-visible; show/hide flips the OS window).
  2. Conflict path: registering a default while another app holds it yields
      `BackendTaken` + the stderr warning, and the player keeps the chords it
      got.
  3. `TrayIcon::attach` from the `window.rs` follow-up: the icon appears with
      the tooltip, each menu row fires its command, `Drop`/exit removes the
      icon (no orphan after quit or crash-kill where the OS cleans it).
  4. The `window.rs` follow-up seams themselves (a lane owning that file):
      `about_to_wait` drains `take_pending_hotkeys()` into `submit` (four
      lines, F2 shape); the main window's raw `HWND` feeds
      `TrayIcon::attach`, and `TRAY_CALLBACK_MESSAGE` pops the menu built from
      `TRAY_MENU`.
  5. A genuine two-process check: hotkeys fire while another app has focus
      (the property S03 §7 leaves unmeasured).

**Provenance notes**
- No free-source code was taken. The Win32 calls are used through the
  vendored `windows` crate; item names/signatures were verified against the
  local registry sources (API facts, not copied code). No Winamp material was
  read.

**Report back**

Files and line counts: `crates/rustamp-platform/src/tray.rs` (315 lines:
menu model + tests + Win32 attach), `crates/rustamp-platform/src/hotkeys.rs`
(792 lines: model + registry + fake-backend tests + Win32
backend/session), `crates/rustamp-platform/src/lib.rs` (34 lines:
re-exports), `crates/rustamp-app/src/main.rs` (151 lines: session start-up
wiring only), `crates/rustamp-platform/Cargo.toml` (28 lines: two `windows`
features added), this card (205 lines).
The design and why: see above. Own choice vs spec: see the table (defaults,
tray entries/labels and the toggle mappings are ours; target ids and
`Command`s are S03's). Tests: headless mapping/bookkeeping tests listed above
(compiled, VM run pending); the VM checklist is §Acceptance items 1–5.
`fmt`/`clippy`/`build` results: all green (see evidence). No `cargo test`,
no test binary, no player run, no window, no audio and no commit were made on
this host.
