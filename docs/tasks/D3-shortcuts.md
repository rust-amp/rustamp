# Task card D3 — Main-window keyboard shortcuts from S03 (M1)

**Goal**
Give the main window its keyboard: a headless mapping from a key press (key plus
modifiers) to the action S03 binds to it, and the dispatch that turns the action into
`rustamp_core::Command`s or an explicit "not expressible yet" result. S03 is audited —
implement it, do not re-derive it.

**Lane** D — windows/UI (`rustamp-ui`).

**Owned paths**
- `crates/rustamp-ui/**`
- Do **not** edit the root `Cargo.toml`, `rustamp-core`, `rustamp-proto`,
  `rustamp-skin`, `rustamp-gfx`, `rustamp-audio`, `xtask`, `docs/**` or
  `THIRD_PARTY.md`. If something outside these paths must change, report it as a
  finding instead of editing it. In particular **do not add a `Command` variant**:
  that is a contract change and needs an RFC.

**Allowed sources**
- **`specs/S03-shortcuts.md` (audited)**: your contract. Every binding you implement
  must be traceable to a row in it. Use its §1 (main window), the main-window-relevant
  rows of §4 (global) and §6 (open questions).
- `specs/S02-main-window-behaviour.md` (audited): the actions' effects.
- `specs/S01-window-geometry.md`, `specs/S17-skin-loading-quirks.md` (audited).
- The repository, `docs/tasks/D2-main-window-behaviour.md` (shape and conventions) and
  public `winit` documentation.

**Forbidden sources**
- Winamp 5.x source, Winamp SDK headers or documentation, disassembly, the Winamp 2.95
  binary, `D:\code\rustamp-research`, `D:\code\rustamp-refbin`. Nothing from them.
- Do not download anything. Do not run a reference binary.
- **No input to the user's desktop.** Tests must not open a window or send input.

**Scope**
- **In scope:**
  - A new headless module in `rustamp-ui` (suggested: `shortcuts.rs`) with:
    - a `Key` type (letters, digits, function keys, arrows, numpad keys, Insert, Enter,
      Tab, grave, and so on) and a `Modifiers` set (Ctrl, Alt, Shift), independent of
      `winit`;
    - a `ShortcutAction` enum with one variant per distinct S03 §1 action, carrying the
      public command id as a documented constant (the ids are interoperability facts and
      are in the spec);
    - the main-window table as data, plus a lookup `Key + Modifiers -> Option<ShortcutAction>`.
  - Dispatch on `MainWindow` (or a thin wrapper) that returns a result type with three
    cases: `Commands(Vec<Command>)` for actions that map onto existing `Command`s
    (play, pause, stop, next, previous, toggle shuffle/repeat, `SeekRelative` ±5000 ms,
    toggle double size, toggle always on top, toggle equalizer/playlist windows,
    toggle windowshade, stop with fadeout, quit if S03 binds it); `Handled` for effects
    that stay inside the machine (toggle elapsed/remaining time, anything S02 models
    locally); and `Unhandled(ShortcutAction)` for actions with no `Command` yet (dialogs,
    menus, preferences, skin browser, jump to file/time, open file/location/directory,
    "10 tracks back/forward", start/end of list, stop after current track, bookmarks,
    visualization plug-in actions, minibrowser, video, minimize, new instance, easy
    move, file info). The app layer will handle `Unhandled` later.
  - Volume up/down (Up, Down, Num8, Num2): emit `SetVolume` computed from the current
    volume. **S03 and S02 do not state the step size.** Use one named constant, mark it
    `provisional: step size not in S02/S03`, and report it.
  - Wire the example presenter: `winit` keyboard events map into `Key`/`Modifiers`, are
    dispatched, and the resulting `Command`s are printed or applied the same way mouse
    commands are today. Keep the `winit` mapping in `presenter.rs`; the shortcut module
    stays free of `winit`.
- **Out of scope:**
  - the playlist-editor and equalizer tables (S03 §2 and §3): those windows do not
    exist yet. Structure the table so a window kind selects it, but only fill the main
    table;
  - menus (S04 does not exist), docking (S05), key repeat policy beyond "a repeat
    delivers the action again" (state your choice);
  - the minibrowser keys and the video keys: map them to `Unhandled`.
- **Provisional rules (S03 §4 and §6):**
  - How the four tables combine is provisional. Check the main table first, then the
    global table. On a conflict (`Ctrl+L` is "open location" in the main table and a
    minibrowser command globally) the main table wins. Mark this `provisional` in code
    and in the report.
  - Rows S03 marks `provisional` (F3, Ctrl+F4, F5, Alt+M, Ctrl+Alt+N, Alt+Enter,
    Ctrl+Tab, Ctrl+W, volume keys, Alt+F) may be implemented, but each carries a
    provisional marker, and the report lists them.
  - Duplicate bindings (for example `Num.`, `Insert`, `X`, `Z`, `Num4` to `Num6`) are
    separate table rows mapping to the same action.

**Interface contract**
- `rustamp-ui` consumes `rustamp_core::Command`; it must not call the audio engine and
  must not depend on `rustamp-audio`.
- The shortcut module is headless: no `winit`, no `softbuffer`, no clock.
- Public items get doc comments citing the S03 section.
- Reuse the existing helpers (`volume_percent`, `core_volume`, `transport_transition`)
  instead of duplicating them.

**Acceptance tests**
- [ ] `cargo fmt --all -- --check` clean.
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean.
- [ ] `cargo test --workspace --all-features` green, with new tests for:
  - [ ] **table coverage**: every key row in S03 §1 has a lookup test, and the table has
        exactly the S03 §1 row count (write the count down; a missed row fails);
  - [ ] **no stray bindings**: a key not in the table maps to nothing;
  - [ ] **each mapped action**: transport keys emit the right `Command`, `Left`/`Right`/
        `Num7`/`Num9` emit `SeekRelative` ∓/± 5000, the toggles emit theirs, and the
        window toggles use `WindowId::Equalizer`/`Playlist`;
  - [ ] **`Handled`**: `Ctrl+T` toggles elapsed/remaining in `MainWindow` state;
  - [ ] **`Unhandled`**: every action without a `Command` yields `Unhandled(action)`
        carrying its command id;
  - [ ] **volume keys** clamp at 0 and 255 and use the named provisional step;
  - [ ] **modifiers**: `L`, `Shift+L`, `Ctrl+L`, `Alt+L`-like variants resolve to
        different actions or none; `Ctrl+K` and `Ctrl+Shift+K` differ;
  - [ ] **table combination**: the `Ctrl+L` conflict resolves to the main-table meaning,
        and the test is marked provisional.
- [ ] `cargo test -p rustamp-skin` still green.
- [ ] `cargo test -p rustamp-ipc --target i686-pc-windows-msvc`,
      `cargo test -p rustamp-plugin-abi --target i686-pc-windows-msvc` and
      `cargo build -p rustamp-plughost --target i686-pc-windows-msvc` still green.
- [ ] `cargo xtask golden` reports no mismatch; `cargo xtask ring-cross` still passes.
- [ ] The example still builds. State in the report how you verified the keyboard path
      (tests only is acceptable; do not send input to the user's desktop).

**Provenance notes**
- Specs are facts, not code: do not copy anything out of them. The command ids appear
  as documented constants because S03 records them as public interoperability ids.
- If you take anything from webamp, extend `THIRD_PARTY.md` as a finding (you may not
  edit it); S03 is the authority.

**Report back**
Files changed; the S03 §1 rows implemented, the number of rows, and which are
provisional; every action that needs a new `Command` (with the id and a proposed
variant name) so the lead can write an RFC; the volume-step finding; how you resolved
the table-combination question; how the keyboard path was verified; the new test list
and results; anything you could not verify; and questions that belong to S04/S05/S06/S07.
