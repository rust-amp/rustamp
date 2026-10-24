# Task card N1 — Wire the application: a player that actually plays (M1)

**Goal**
Make `rustamp-app` a real application. Today every piece exists and is tested —
engine, output, decoders, skin, gfx, UI (layout, renderer, behaviour machine,
shortcuts), playlist I/O, tags — and **nothing assembles them**. After this card:
`cargo run -p rustamp-app -- <file>` starts the engine, shows the main window
with the audited layout, plays the file, and the window follows the player state.

**Lane** N — application.

**Owned paths**
- `crates/rustamp-app/**`
- `crates/rustamp-prefs/**` (only if a preferences/config surface is needed; it is a stub today)
- The root `Cargo.toml` **only** for `[workspace.dependencies]` entries you genuinely need, and only if nothing else is adding them.
- Do **not** edit `rustamp-core`, `rustamp-proto`, `rustamp-ipc`, `rustamp-ui`, `rustamp-audio`, `rustamp-skin`, `rustamp-gfx`, `rustamp-playlist`, `rustamp-meta`, `rustamp-decoders`, `xtask`, `docs/**` or `THIRD_PARTY.md`. A contract change needs an RFC; anything else you need outside your paths is a **finding**, not an edit.

**Allowed sources**
- `docs/rfc/0001-core-contracts.md` (the contracts you wire: `Command`, `Event`, `PlayerState`, the bus, `Decoder`/`DecoderRegistry`, `OutputDevice`, `Config`).
- `specs/S01-window-geometry.md`, `S02-main-window-behaviour.md`, `S03-shortcuts.md` (audited) — the UI's behaviour and its `Command` mapping.
- The crates themselves, `crates/rustamp-ui/examples/main_window.rs` (the existing presenter example), `docs/HANDOFF.md` §4 for what each crate does.

**Forbidden sources**
- Winamp 5.x source, Winamp SDK headers or documentation, disassembly, the Winamp 2.95 binary, `D:\code\rustamp-research`, `D:\code\rustamp-refbin`. Nothing from them. Do not download anything.

**Scope**
- **In scope:** the app's structure and lifecycle: config load, skin load (with the repo's look-alike asset and a clear fallback), decoder registry, output device selection (`--null` for tests, WASAPI/cpal default otherwise), engine start, the main window, **command routing** (UI outcome → `Command` → engine/app), **event routing** (`Event` → UI state), the `Unhandled` shortcut actions logged with their command ids, and a CLI (`<file>`, `--null`, `--volume`, `--skin`).
- **Out of scope:** menus (S04 does not exist), docking (S05), the equalizer and playlist windows, the plugin host (M3), preferences UI, tray/single-instance (lane F). Do not invent behaviour the specs do not state; where S02 marks something provisional, keep it provisional.
- **No window in tests.** The wiring must be testable headless: the command/event routing, the config and CLI parsing, and the state application must have unit tests that open nothing. You may **build** the binary; do not **run** the windowed one — another agent may be driving a GUI on this machine.

**Interface contract**
- Consume the frozen contracts; do not change them. `rustamp-ui`'s `MainWindow` machine and `shortcuts` produce `Command`s and a `Handled`/`Unhandled` outcome — the app applies them.
- Keep the boundary honest: the app owns the clock and the OS, the UI machine stays headless.

**Acceptance tests**
- [ ] `cargo fmt --all -- --check` clean.
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean.
- [ ] `cargo test --workspace --all-features` green, with new tests for: CLI parsing (`<file>`, `--null`, `--volume`, `--skin`), config load with defaults, command routing (a UI outcome becomes the right `Command`), event routing (`PlayerState`/`Event` updates the UI state), and the `Unhandled` path carrying the command id.
- [ ] `cargo build -p rustamp-app` green; the binary starts and **exits cleanly** with `--null` on a reference file in a headless test if that can be done without a window (if not, say so).
- [ ] `cargo test -p rustamp-skin`, the i686 `rustamp-ipc`/`rustamp-plugin-abi` tests and the `rustamp-plughost` i686 build still green; `cargo xtask golden` and `cargo xtask ring-cross` still pass.

**Provenance notes**
- Specs are facts, not code. Nothing is copied out of them.

**Report back**
Files changed; what the app now does end to end; which contract gaps you hit (findings, not edits); how the headless tests cover the wiring; what a human should do to hear sound; anything you could not verify.
