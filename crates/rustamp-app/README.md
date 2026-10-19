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

### Audio output mode

`audio.output_mode = "Shared"` (default) retains the automatic WASAPI/cpal
selection. `"Exclusive"` requests WASAPI exclusive access on Windows. A refusal
retries shared mode on the same endpoint; its actual `downgrade_reason()` is
reported through the app's notes (printed as `rustamp: ...` by the presenter).
If WASAPI cannot open either mode, cpal is tried with an explicit fallback note;
if that also fails, startup returns both errors. Outside Windows, exclusive is
unsupported: the app uses the normal cpal output and reports that limitation.
`--null` and `--no-audio` always override the mode without touching hardware.

The Preferences choice is saved immediately and applies at the next startup
or engine recreation, not to a currently open stream. The saved value is the
request, so a temporary device refusal never erases an exclusive preference.
Open diagnostics are also retained for later device reopens.

Selection/serialization tests are headless. The real app selection probe is
`output::tests::the_app_exclusive_path_opens_a_real_endpoint_or_reports_its_downgrade`:
it is ignored by default and **VM-only**, just like `rustamp-output`'s
`wasapi_exclusive` integration test. Neither is evidence of a real exclusive
open until explicitly run against the VM's endpoint.

### Audio output device

`audio.device = "Speakers (Realtek)"` selects the output device by name. The
name is matched the way the backends match it: trimmed, case-insensitive, and
*contained* in the device's friendly name — `"Speakers"` therefore matches
`"Speakers (Realtek)"`. Without the key the system default endpoint is used.

The app builds the plan with `rustamp_output::selection::plan` and resolves it
against the devices that exist (`rustamp_output::selection::resolve`):

* a name that matches exactly one device opens that device (WASAPI on Windows,
  cpal elsewhere);
* a name that matches **none** is reported through the app's notes — the note
  names the wanted device and the devices that do exist — and the player falls
  back to the default endpoint. The user sees it; the player never switches to
  another device silently;
* a name that matches **several** devices is reported the same way, because the
  player cannot know which one is meant.

The name is carried into the backend that opens the device, and the cpal
fallback behind WASAPI opens the *same* device. The device is honoured and
persisted: `audio.device` is written back to the configuration file on a clean
exit and is no longer reported as an unapplied setting.

Outside Windows the output crate exposes no enumeration, so the name goes to
the backend unchanged: the backend looks the device up with the same rule and
refuses to open a device that does not exist.

### Running the windowed player

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
`tests/main_menu.rs`, `tests/window_region.rs`, and the menu unit test in
`src/window.rs`):

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

## The open-media dialogs

The app's dialog model is headless: a command *requests* a media choice, and
the presenter decides how the OS asks for it. Two of the three faces of
`Command::OpenMedia` are native Windows dialogs, and both are opened **from
the tick** (`window::drain_media_pick`), never from a window procedure: they
are modal and run their own message loop.

| Face | Command id | Dialog |
|---|---|---|
| Open file(s) | 40029 (playlist: 1032) | the standard file-open dialog (`rustamp_platform::pick_media_files`) |
| Open directory | 40187 (playlist: 1036) | the shell's folder dialog (`rustamp_platform::pick_directory`) |
| Open location | 40185 (playlist: 1039) | a window of ours: a URL is typed, not picked |

A cancelled dialog adds nothing. The chosen files are appended to the playlist
with the requesting command's `play` flag untouched, so the main window's open
actions start the first added entry and the playlist's ADD only appends.

**A chosen directory contributes the media files of the tree below it.** That
depth is a Rustamp choice: S03 names the action and its shortcut, S04 covers
the menus that carry it, and S06 covers the playlist editor's ADD and LIST OPTS
dialogs — none of them says whether a directory contributes its own files or
the whole tree. Only the files the player's decoders claim are added; a
playlist file inside the directory is not imported (loading a playlist is the
LIST OPTS path). See `src/directory.rs` for the rules and their tests.

## The HTML playlist

`Ctrl+Alt+G` in the playlist editor (command id **40292**, S03 §2) and the MISC
menu's "Generate HTML playlist" row — the same id — write the current queue as
one HTML document. The document is a table of the entries (number, title,
length, location) under a title, next to the moment it was written and the
number of entries; it names no external file and runs no script.

The document is made by a **pure** generator (`src/html_playlist.rs`): the
entries and the date go in, the text comes out, and every text that comes
from the playlist is escaped, so a title with a `<` in it stays a title. The
app half collects the entries from the queue — the title the last playlist
load carried, else the file name, and the length it carried, if any — and
writes the document to the file the user chose.

The file is asked for the same way the playlist save asks for its file: the
app records the request, the presenter opens the native save dialog from the
tick (`window::drain_html_playlist`,
`rustamp_platform::save_html_playlist_file`, filter `*.html`, default name
`playlist.html`) and hands the answer back. A cancelled dialog writes nothing
and is noted.

**Own choices.** S03 §2 fixes the id and the chord and nothing else about the
document: its title ("Rustamp playlist"), its columns, the wording of the
summary line, the `--:--` cell for an entry without a known length and the
UTC date format are ours.

## The equalizer's automatic per-track presets (AUTO)

The equalizer window's AUTO button (`A`, command 40245) switches the engine's
`autoload_eq` flag — the saved-state key S07 §12 item 13 settles — and while
that flag is set, the app picks a preset for every track that starts. The
rules live in `eq-autoload.toml`, beside the configuration file and next to
the user's `presets.eqf` library; the file is written by hand and read once at
start-up.

```toml
# The preset for a track that matches no rule. Without it, such a track keeps
# the curve it has.
default = "Flat"

# The rules, in the order they are tried. The first match decides.
[[rule]]
tag = "genre"          # genre, artist or album
value = "Rock"
preset = "Rock"

[[rule]]
tag = "artist"
value = "Miles Davis"
preset = "Jazz"
```

`preset` names a built-in preset or one of the user's saved presets. A value
matches ASCII-case-insensitively, with surrounding space ignored.

**What the specification settles and what is Rustamp's own choice.** S07 §12
item 13 settles the flag behind the AUTO button and nothing else: the
specification names the flag, not the behaviour behind it. The trigger (the
track start), the key (genre, artist or album), the order (the file's), the
fallback (the file's `default`, otherwise the current curve) and the
precedence (the auto-load runs at the track start only, so a manual change
during the track is never overwritten; the next track start decides again) are
Rustamp's design. See `src/eq_autoload.rs`.

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

## The skin's window shape (`region.txt`)

A skin's `region.txt` describes the **visible** area of a window (S17 §7):
everything outside the polygons is transparent. `rustamp-skin` turns those
polygons into a per-pixel mask (`rustamp_skin::Region::mask`); the app turns
the mask into rectangles and hands them to the operating system, which clips
the window to them (`SetWindowRgn`).

| Window | Section of `region.txt` |
|---|---|
| main window | `[Normal]`, `[WindowShade]` while it is shaded |
| equalizer | `[Equalizer]` |
| playlist editor | none: it keeps its rectangle (own choice) |

The rules the app follows (`src/region.rs`):

* **No mask, no shape.** A skin without a `region.txt` — or with a section
  that has no polygon, which is the base skin's case (S17 §7) — leaves the
  window a plain rectangle.
* **A mask that covers no pixel is not a shape.** The window stays a
  rectangle; an empty region would make the window invisible.
* **The shape is scaled with the skin.** The polygons are drawn in the skin's
  own pixels, so the mask is built for the layout size and scaled to the
  window's physical pixels, exactly like the skin's bitmaps in the
  double-size mode.

The shape is applied again whenever one of its inputs changes: the window is
resized (which the shade and the double-size mode both do), another skin is
loaded, or the window is shown again.

The FFI lives in `rustamp-platform` (`window_region`), because the app crate
is `deny(unsafe_code)`. The handle belongs to the **system** after a
successful `SetWindowRgn` — the app must not delete it then; a call that
fails leaves the handle with the caller, which deletes it.

The rectangles and the section choice are covered headless, in
`src/region.rs`. The operating system's window region itself is covered by
the ignored `tests/window_region.rs`, which cuts a real window to a skin's
shape, reads the shape back and clears it again: **VM-only**.

## The playlist editor's windowshade

The playlist editor's skinned **Shade** button (the 9 x 9 cell at (254,3), the
same place the main window's shade button sits) collapses the editor to its
14 px title strip and expands it again. The click is routed through
`App::playlist_mouse_down` → `App::playlist_title_button`, which sends
`Command::ToggleWindowShade(WindowId::Playlist)`; the app applies it to the
playlist model, the presenter resizes the OS window to the new client area
(275 x 116 at rest, 275 x 14 shaded) and the docked stack re-flows with it.

A shaded editor is a title strip: it has no list and no bottom bar, so only
its title-bar buttons answer a click. Its shade state is saved with the other
windows (`ui.windows`), so a shaded editor comes back shaded.

The button's own rect and the shade band's drawing are **provisional**
(S01 §4; S05 §1.5 leaves the editor's shade unmeasured). The 14 px band
height is S01 §1. The headless tests cover the click, the state flip, the
new height, the docked reflow and the config round-trip; the OS window's
resize is checked by the ignored
`window::tests::the_playlist_window_shade_resizes_the_real_window` in the
test VM.

## Double-clicking a title bar

S03 §5: "Double-click the title bar: toggle windowshade" (verified on the
2.95). The gesture works on the **main**, **equalizer** and **playlist**
windows, and it sends `Command::ToggleWindowShade` for that window.

What counts as a double click is the rule the playlist list already used —
the second press within **500 ms** and within **4 px** of the first, on each
axis. It lives in one place now (`src/double_click.rs`), and the playlist
list uses it too, so there is a single threshold in the player.

The press is only a title-bar press when it lands in the title bar and not
on one of its controls:

- the main window's four window buttons (the hit test answers them before
  the band they sit in) and, in the shaded form, the position slider inside
  the band;
- the playlist editor's close and shade buttons;
- the equalizer has no title-bar button at all (S05 §1.5).

A press on such a control **wins**: the button's own action runs and the
double click does not fire. It also ends the sequence, so the next title-bar
press starts a new one.

The gesture is headless (`App::title_bar_double_click`, the entry point the
presenter asks about on every press); the tests are in
`tests/title_bar_double_click.rs`. What only the test VM can show is the
gesture against the real windows: the first press of a double click starts
the operating system's title-bar drag (it moves nothing when the mouse stays
put), and the second press is the one that toggles the shade.
