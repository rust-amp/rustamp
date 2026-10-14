# Task card D1 — Main window: geometry, widgets and rendering (M1)

**Goal**
`rustamp-ui` renders the classic main window: the background and title bar, the
transport buttons, the position, volume and balance sliders, the clutterbar, the
time display, the kbps/kHz and mono/stereo indicators, the play/pause indicator
and the title marquee — all placed from the **audited** spec S01 and painted
through `rustamp-gfx` onto a `softbuffer` surface in a `winit` window. The
sprite table in `rustamp-skin` is reconciled with S01 in the same pass.

**Lane** D — windows/UI (`rustamp-ui`, plus the sprite-table correction in
`rustamp-skin`).

**Owned paths**
- `crates/rustamp-ui/**`
- `crates/rustamp-skin/src/webamp.rs` and its tests — **only** the sprite-table
  reconciliation with S01 (part 1 below). Do not restructure the loader.
- `tests/golden/**` if you add a golden case (see the note on goldens).
- Do **not** edit `Cargo.toml` at the repository root, `rustamp-core`,
  `rustamp-gfx`, `xtask` or `docs/**`. `winit` and `softbuffer` are already
  wired into `crates/rustamp-ui/Cargo.toml`; `rustamp-skin` and `rustamp-gfx`
  are available to you.

**Allowed sources**
- **`specs/S01-window-geometry.md` (audited)** — the authority for every
  rectangle, size and position in this card.
- **`specs/S17-skin-loading-quirks.md` (audited)** — fallbacks and encodings.
- `specs/S09-plugin-abi.md` — not needed here, informational only.
- `rustamp-skin`, `rustamp-gfx`, `rustamp-core` in this repository, and
  `tests/golden/README.md`.
- **webamp** (MIT, `D:\code\rustamp-free-sources\webamp`) — read-only
  cross-check for structure and naming. It is already attributed in
  `THIRD_PARTY.md`. Where webamp and S01 disagree, **S01 wins**; S01's
  "webamp deltas" section lists exactly where.
- Public `winit`/`softbuffer` documentation (local crate sources).

**Forbidden sources**
- Winamp 5.x source, Winamp SDK headers or documentation, disassembly, the
  Winamp 2.95 binary, `D:\code\rustamp-research`. Nothing from them.
- Do not download anything (no `cargo add`, no git clone, no web fetching). No
  reference binary may be run.
- Do not open a window during tests, and do not touch the user's screen.

**Spec dependencies**
- **S01 is audited and is your contract.** It marks rows **verified** and
  **provisional**; carry that distinction into the code (see below).
- **S02 (main-window behaviour) is only a draft and must not be implemented**:
  no state machine, no click handling, no marquee timing, no shortcut handling.
  You render *states*; who changes them comes with S02. S03/S04/S05/S06 are out
  of scope entirely.
- Where S01 says a value is provisional, keep it provisional in the code and in
  your report. Do not promote a guess to a fact.

**Part 1 — reconcile the sprite table with S01**
- `crates/rustamp-skin/src/webamp.rs` currently carries webamp's rectangles
  marked `// provisional: webamp, pending S01/S17`. Replace the values S01
  measured with S01's values and reference the spec (e.g. `// S01 §2.3`), and
  keep a marker where S01 is still provisional.
- At minimum apply S01's delta list: transport buttons **22 px wide at a 23 px
  pitch**; the working/idle decoder indicator sprites are **swapped** relative
  to webamp; the clutterbar letter sprites are **4×6** (not 8×7 with transparent
  padding); the position-slider thumb starts at the measured x, not webamp's
  (S01 records the residual as unresolved — keep it provisional); kbps/kHz are
  drawn with **`text.bmp`**, not `numbers.bmp`.
- Keep the module's MIT notice and the `THIRD_PARTY.md` entry intact. The corpus
  test (20 real skins) must keep passing.

**Part 2 — geometry and widgets in `rustamp-ui`**
- A `MainWindowLayout` (or equivalent) holding every rectangle from S01 as named
  constants, in one place, each annotated with its S01 reference.
- A `MainWindowState` that describes what to draw: transport state
  (stopped/playing/paused), clutterbar flags, shuffle/repeat, windowshade,
  double-size, elapsed/remaining time, a title string, kbps/kHz text, mono/stereo,
  and the current position/volume/balance values. It is a plain data struct —
  **no** transitions, no timers.
- A renderer that produces one RGBA frame for a given `MainWindowState` and a
  loaded `Skin`, using `rustamp-gfx` for every blit. Missing sprites fall back
  per S17; the renderer must never panic on a skin that lacks a sheet.
- Widgets to draw: title bar (focused and unfocused), transport buttons
  (previous/play/pause/stop/next/eject, both states), position slider with thumb,
  volume and balance sliders with thumbs, clutterbar letters and its background,
  time display via `numbers`/`nums_ex` (including the "no digit" cell and the
  minus sign), kbps/kHz via `text.bmp`, mono/stereo, the play/pause indicator,
  and the marquee text via `text.bmp`.
- `hit_test(x, y) -> Option<Widget>` over the same rectangles, so a later card
  can wire clicks without re-deriving geometry.
- Double-size is integer 2× scaling of the same frame; windowshade uses S01's
  shade geometry.

**Part 3 — presentation**
- A `winit` window + `softbuffer` presenter that paints the frame, with integer
  scaling for double-size. Keep it small and behind a thin API so the app lane
  can drive it later.
- A runnable example (`cargo run -p rustamp-ui --example main_window`) that opens
  the window, paints one state, and stays open until closed. It must never open
  during `cargo test`.

**A note on goldens**
`tests/golden/lookalike/` is reserved for our shipped default skin, and that art
(Lane J) does not exist yet. So: **prefer structural assertions** (exact
rectangles, hit-test round trips, spot pixel checks against a skin you build in
the test). If you do add a golden PNG, it must use a deterministic skin built in
the test, be documented in `tests/golden/README.md` as the harness's synthetic
skin rather than the shipped one, and `cargo xtask golden` must pass. If in doubt,
skip the golden and say so — a documented gap beats a misleading baseline.

**Acceptance tests**
- [ ] `cargo fmt --all -- --check` clean.
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean.
- [ ] `cargo test --workspace --all-features` green, with new tests for:
  - [ ] every layout rectangle equals its S01 value (table-driven, so a drift in
        one number fails one named case);
  - [ ] `hit_test` returns the widget whose rectangle contains the point, for
        every widget, at its corners and centre, and `None` outside the window;
  - [ ] rendering a frame with a synthetic skin puts the expected sprite pixel
        at known coordinates for: a transport button, the time digits, a
        clutterbar letter, the position thumb and a marquee glyph;
  - [ ] double-size is exactly 2× in both axes, pixel for pixel;
  - [ ] windowshade height and content match S01's shade geometry;
  - [ ] a skin missing a sheet still renders (fallback path) and does not panic;
  - [ ] the corpus of 20 real skins renders a frame without panicking.
- [ ] `cargo test -p rustamp-skin` green (the sprite-table change must not break
      the loader or the corpus test).
- [ ] `cargo test -p rustamp-ipc --target i686-pc-windows-msvc`,
      `cargo test -p rustamp-plugin-abi --target i686-pc-windows-msvc` and
      `cargo build -p rustamp-plughost --target i686-pc-windows-msvc` still green.
- [ ] `cargo xtask golden` reports no mismatch.
- [ ] Report the example command and confirm you ran it once (state what you saw;
      do not leave a window open).

**Provenance notes**
- webamp stays attributed in `THIRD_PARTY.md`; if you port any further webamp
  code (beyond the already-recorded tables), extend that entry.
- S01 is a spec, not code: no code may be copied from it, and its numbers are
  facts you may hard-code.

**Report back**
Files changed, which S01 rows you applied and which you had to leave
provisional (and why), whether you added a golden and what it contains, the
example command and its result, anything you could not verify, and the S02
questions the rendering exposed.
