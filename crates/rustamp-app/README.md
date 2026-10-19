# rustamp-app

The application layer: the crate that assembles the player. It owns the clock
and the OS, loads the config and the skin, builds the audio engine, opens the
main window and routes commands and events between the headless UI machine
(`rustamp-ui`) and the engine (`rustamp-audio`). See `src/lib.rs` for the map of
the modules and `docs/tasks/N1-app-wiring.md` for the task card.

The binary is `rustamp`:

```text
rustamp [<file>] [--null] [--no-audio] [--volume <0..=255>] [--skin <path>] [--seconds <n>]
```

## Headless mode — safe to run anywhere

```text
cargo run -p rustamp-app -- --null <file>
```

`--null` selects the deterministic null output and runs **headless**: the app
drives a virtual device instead of WASAPI/cpal and opens **no window**. This is
the mode the tests use (`tests/headless.rs` plays the repo's own 1.5 s FLAC
fixture and checks the exit code). `--seconds <n>` bounds a run; `--null` with
no file exits immediately.

## Windowed mode — headless test VM only

```text
cargo run -p rustamp-app -- <file>
```

Without `--null` the app opens the main window and plays on the best device for
the host. With `--no-audio` it opens the same window but plays silently on the
null output, so it needs **no audio endpoint** (a Hyper-V VM has none) — this
is the mode the window tests use.

> **GUI must never run on the host.**
>
> Everything graphical runs inside the disposable headless test VM. Run the
> windowed player and all tests only in the guest's interactive session.
> Compiling/checking without executing tests is allowed on the host.

### A spawned child must never create a window either

**A console window counts as a window on the host.** Any code that spawns a
child process — tests, tools, examples — must therefore pass
`CREATE_NO_WINDOW` (`std::os::windows::process::CommandExt::creation_flags(0x0800_0000)`);
a console-subsystem child (`rustamp.exe`) otherwise gets a console window of its
own when the parent has no console.

`tests/headless.rs` does this in one place, `no_window()`, which **every** spawn
site goes through, and `every_spawn_site_is_guarded` fails if a second spawn
site appears or the flag is dropped.

The windowed player is not needed to verify this crate: `cargo test -p
rustamp-app` covers the wiring headless, and the binary is only ever *built*,
never run windowed, by the default test suite.

## The GUI rail

`window::run` refuses to start when the environment variable `RUSTAMP_NO_GUI`
is set (`window::NO_GUI_ENV`), and `rustamp_app::run` checks the same rail
**before** it builds the audio engine, so a mis-run fails immediately instead of
opening a window. `tests/headless.rs` sets it on every binary it spawns, so a
test that accidentally loses `--null` fails loudly rather than starting a GUI.

The tests that open windows are `#[ignore]`d and VM-only
(`tests/headless.rs`, `tests/playlist_window.rs`, `tests/eq_window.rs`,
`tests/main_menu.rs`, and the menu unit test in `src/window.rs`):

```text
cargo test -p rustamp-app -- --ignored
```

Run them **inside the headless test VM**. The main-window and playlist-window
tests spawn the player with `--no-audio`, so they need no audio endpoint; the
in-process probes never touch audio either. Only `tests/real_playback.rs`
needs a real endpoint (a virtual audio device or the Hyper-V audio
redirection) — it exercises the production device path and fails loudly
without one. The spawned player is waited on for three seconds to check that
it is still running (the window opened and the event loop is live), then the
windows' OS geometry is read back — main 275 x 116, equalizer 275 x 116 at its
docked position, playlist 275 x 116 docked below the main window, menu popup
323 x 569 — and the player is killed. In-process probes build their event loop
off the main thread on purpose (`window::build_event_loop(true)`), because
tests run on harness worker threads; the product keeps the default.

## Windows of this crate

`rustamp-app` drives `winit`/`softbuffer` itself instead of reusing
`rustamp_ui::MainWindowPresenter`, because that presenter is a manual example
which drops the commands the UI machine emits and cannot route anything to the
engine. See the module docs in `src/window.rs`.

## Classic-window docking

`window.rs` applies `rustamp_ui::docking::DockedStack` positions to the main,
EQ and playlist windows. Main moves carry directly/transitively connected
neighbours. Shade/double-size changes reflow existing connections, not detached
windows. Title-bar drags move all three windows; dragging EQ/playlist moves
only that window. Those secondary moves snap to visible classic neighbours on
all four outside edges, with positive edge overlap and optional corner alignment.

`ui.snap_distance` is honoured and persisted (default 10 physical pixels,
inclusive; 0 disables snapping). Positions and visibility use the existing
`ui.windows` records. Exact coincident edges reconstruct connections at restart;
closed windows retain their geometry and follow their old connections. No core
schema change or separate docking flag is needed. The snap policy (including
programmatic moves), reconstruction and closed-window policy are Rustamp choices:
S05 §§3.4, 4.1 and 7 leave those behaviours open, not measured reference facts.
Screen/work-area snapping, monitor recovery and non-classic windows are not wired.

Geometry/configuration unit tests are headless. The ignored
`window::tests::docking_presenter_moves_snaps_detaches_reflows_and_restores`
opens the **production presenter** and checks actual OS positions after moves,
snapping, detaching, shade reflow and recreation from serialized configuration.
It does not simulate a pointer drag; tracking inside the native modal drag loop
still requires interactive VM acceptance. Run via the interactive-session runner:

```powershell
pwsh -File docs/notes/vm-run-tests.ps1 -Filter '--ignored:docking_presenter'
```

Never run the ignored test on the host; `require_test_vm()` refuses it there.
