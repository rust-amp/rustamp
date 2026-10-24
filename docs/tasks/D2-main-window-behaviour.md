# Task card D2 — Main window behaviour from S02 (M1)

**Goal**
Give the main window its behaviour: a headless state machine driven by
`rustamp_core::Command`/`Event`, mouse dispatch through the existing `hit_test`,
the marquee and time-display logic, the hint strings, and the **pressed window
buttons** that the S01/S02 reconciliation turned up. S02 is audited — implement
it, do not re-derive it.

**Lane** D — windows/UI (`rustamp-ui`).

**Owned paths**
- `crates/rustamp-ui/**`
- `tests/golden/**` if you add a golden (same rule as D1: prefer structural
  assertions).
- Do **not** edit `Cargo.toml` at the repository root, `rustamp-core`,
  `rustamp-skin`, `rustamp-gfx`, `rustamp-audio`, `xtask`, `docs/**` or
  `THIRD_PARTY.md`. If you believe something outside these paths must change,
  report it instead of doing it.

**Allowed sources**
- **`specs/S02-main-window-behaviour.md` (audited)** — your contract for
  behaviour. Every rule you implement must be traceable to a row in it.
- **`specs/S01-window-geometry.md` (audited)** — geometry.
- `specs/S17-skin-loading-quirks.md` (audited) — fallbacks.
- `specs/S03-shortcuts.md` (audited) — read it, but **do not implement
  keyboard shortcuts in this card** (see scope).
- The repository, `tests/golden/README.md`, and public `winit` documentation.

**Forbidden sources**
- Winamp 5.x source, Winamp SDK headers or documentation, disassembly, the
  Winamp 2.95 binary, `D:\code\rustamp-research`. Nothing from them.
- Do not download anything. Do not run a reference binary.
- **No input to the user's desktop.** Tests must not open a window or send
  input. If you verify the example interactively, you may post mouse messages
  **to your own example window only**, and only with the guarded pre-flight
  (resolve the HWND, check the point resolves to your window, clamp to the
  client rect, always release the button). Never move the user's cursor.

**Spec dependencies and scope**
- **In scope (S02):** the transport state machine and the states it produces;
  the clutterbar's five actions; the marquee (rate, separator, autoscroll
  toggle); the time display (elapsed/remaining, the pause blink, the leading
  "no digit" rule, what stopped shows); kbps/kHz and mono/stereo from stream
  info; the button press/release/drag behaviour of the transport buttons, the
  position slider, the volume and balance sliders, and the shuffle/repeat/EQ/PL
  toggles; the hint strings and where they appear; momentary lit-while-held.
- **Out of scope:** keyboard shortcuts (S03 — a later card), menus (S04 does not
  exist), docking and snapping (S05), the playlist and equalizer windows, and
  anything S02 marks **provisional** or lists as a gap — implement those as
  clearly marked provisional or leave them out, and say which.
- **The S01/S02 reconciliation:** S01 said the small window-button cells in the
  title-bar sheet are unused; S02's measured pressed state **does** draw them.
  Fix the renderer so the pressed window buttons come from those cells, keep the
  normal state as it is, and state in your report which spec row you followed
  for each of the two states.

**Interface contract**
- `rustamp-ui` consumes `rustamp_core` types (`Command`, `Event`,
  `PlayerState`, `StreamInfo`, volume/balance/position units). Do not invent a
  parallel state type where a core type exists, and do not change any core type
  — that needs an RFC.
- The **state machine must be headless**: no `winit`, no `softbuffer` and no
  wall-clock calls in the module that computes state. Take time as a parameter
  (milliseconds) so tests are deterministic; the presenter supplies it.
- Emitted intent comes out as `Command`s; do not call the audio engine
  directly, and do not add a dependency on `rustamp-audio`.

**Acceptance tests**
- [ ] `cargo fmt --all -- --check` clean.
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean.
- [ ] `cargo test --workspace --all-features` green, with new tests for:
  - [ ] the transport state machine: every transition S02 documents, driven by
        `Command`s and `Event`s, including the states that must not change;
  - [ ] mouse dispatch: press/release/drag on every interactive widget emits the
        right `Command` and nothing else; a press outside any widget is inert;
        a drag that leaves the slider is clamped, not lost;
  - [ ] the five clutterbar actions each produce their documented effect;
  - [ ] the time display: elapsed and remaining, the leading "no digit" rule at
        the documented boundaries, the pause blink with injected time, and what
        stopped shows;
  - [ ] the marquee: scroll rate and separator from injected time, the autoscroll
        toggle, and the title source;
  - [ ] kbps/kHz and mono/stereo come from `StreamInfo` and are absent when it is
        unknown;
  - [ ] the hint strings are the exact strings S02 records, on exactly the widgets
        S02 says (and **not** on the transport/shuffle/repeat/EQ/playlist
        controls — S02 corrected that);
  - [ ] the pressed window buttons draw from the small sheet cells, and the
        normal state is unchanged (pixel assertions);
  - [ ] the 20-skin corpus still renders every state without panicking.
- [ ] `cargo test -p rustamp-skin` still green (the renderer's sprite use is the
      only link).
- [ ] `cargo test -p rustamp-ipc --target i686-pc-windows-msvc`,
      `cargo test -p rustamp-plugin-abi --target i686-pc-windows-msvc` and
      `cargo build -p rustamp-plughost --target i686-pc-windows-msvc` still green.
- [ ] `cargo xtask golden` reports no mismatch.
- [ ] The example still runs; state in the report how you verified the
      interactive path (tests, or a guarded message to your own window).

**Provenance notes**
- webamp stays attributed; if you port more of it, extend `THIRD_PARTY.md` —
  but note that S02 is the authority and webamp is a cross-check only.
- Specs are facts, not code: no code may be copied from them.

**Report back**
Files changed, the S02 rows you implemented and the ones you left provisional or
out of scope (with the reason), how you resolved the window-button
contradiction, how the interactive path was verified, the new test list and
results, anything you could not verify, and the questions the behaviour exposed
that belong to S03/S04/S05.
