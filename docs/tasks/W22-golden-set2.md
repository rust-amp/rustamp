# Task card W22 — "Golden cases, second set": already satisfied at main

## Verdict (read this first)

The ready-queue row this lane was started from is **already satisfied at `main`
(`b9f92e3`)** and the row is **stale**: it was written *before* the work landed
and was never removed from `docs/tasks/LANE-QUEUE.md`.

Every case the row names already exists, for **both** committed skin
namespaces, with its expected image committed and tracked by git:

- a double-size main window — `main_window_double_size`,
- the equaliser with a raised band — `equalizer_window_single_raised_band`,
- a playing main window showing a time and a title — `main_window_playing`.

**No golden needs blessing.** There is no image to create and no test to add;
adding one would duplicate an existing case and would ask the orchestrator to
bless an image for a state that is already pinned. This lane therefore changed
**no source and no test** and wrote only this card. Inventing a duplicate case
here would have been worse than doing nothing (see `docs/HANDOFF.md`, the
playlist lane that was praised for reporting "no change needed" rather than
manufacturing work).

The rest of this card records the evidence, the exact cases and images, the
(empty) bless list, and what only the VM can prove — so the row can be closed
without re-deriving any of it. **To close it: delete the `#5` row from
`docs/tasks/LANE-QUEUE.md`** (the list's own rule is to remove what is done).

## The queue row, and why it is stale

| Fact | Value |
|---|---|
| Row | `docs/tasks/LANE-QUEUE.md` line 20, table `#5`: *"Golden cases, second set — double-size main window, the EQ with a raised band, a playing main window with a time and a title"*, owns `rustamp-ui` tests, source `tests/golden/README.md` |
| Row introduced by | `9a5fa12` "Make refilling the lanes automatic and keep a ready queue for it (owner rule)" (2026-10-17) |
| The row's three cases landed | `529ab9d` "Pin four more rendered states as golden images" (2026-10-17) and `68b7941` "Bring the equaliser renderer up to date" (2026-10-17) |
| Ancestry proof that the row is stale | `git merge-base --is-ancestor 9a5fa12 529ab9d` exits **0** (the queue commit is an *ancestor* of the commit that delivered the work) and `git merge-base --is-ancestor 529ab9d 9a5fa12` exits **1**. The row therefore described work that landed **later the same day** and was never struck off |
| Cases moved into the shared helper, `rust-theme` namespace added | `991a50d` "Give each built-in skin its own golden namespace" |

The queue's own rule ("Keep this list current: add what the latest audits found,
**remove what is done**") was simply not applied to this row.

## The crate and the existing mechanism

The goldens are owned by **`crates/rustamp-ui`** — this is the expected crate,
not a neighbouring one.

- Case bodies and the shared helper: `crates/rustamp-ui/tests/golden_cases/mod.rs`
- Per-skin test binaries that register the cases:
  `crates/rustamp-ui/tests/golden_lookalike.rs` (namespace `lookalike`) and
  `crates/rustamp-ui/tests/golden_rust_theme.rs` (namespace `rust-theme`)
- Comparison, bless and loud failure: `crates/rustamp-testkit/src/golden.rs`
  (`assert_golden`, tolerance 0, `RUSTAMP_BLESS=1` bless mode)
- Expected images: `tests/golden/<namespace>/<name>.png`
- Runner: `cargo xtask golden [--skin <namespace>] [--bless]`
- Contract/documented behaviour: `tests/golden/README.md`

The mechanism is a per-skin `register_golden_tests!` invocation that expands the
shared case functions into the `golden_lookalike_*` / `golden_rust_theme_*`
tests. Both skins run the *same* case list, so the two namespaces are directly
comparable. Nothing in this lane needed a second mechanism, because no case was
missing.

**A missing image already fails loudly**, which is the property this task was
meant to guarantee: `assert_golden` returns `GoldenOutcome::Missing` and panics
with

```
golden missing: <path>
hint: run `cargo xtask golden --bless` (or set RUSTAMP_BLESS=1) to create it
```

(`crates/rustamp-testkit/src/golden.rs:278`). There is no silent-skip path for
the two committed namespaces; only `classic-base` skips, and only when
`RUSTAMP_TESTDATA` is unset.

## The three cases, as they exist at main

| Queue wording | Test fn (both skins) | Case fn (`golden_cases/mod.rs`) | State built (public API only) | Expected image | Size |
|---|---|---|---|---|---|
| a playing main window showing a time and a title | `golden_lookalike_main_window_playing` / `golden_rust_theme_main_window_playing` | `case_main_window_playing` (line 116) | `playing_state()`: `TransportState::Playing`, `time_seconds: 222` (3:42, so the leading tens-of-minutes cell stays blank), `title: "1. Rustamp - the look-alike default skin (3:42)"`, `marquee_offset: 10` (scrolled), `kbps: Some(192)`, `khz: Some(44)`, `channels: MonoStereo::Stereo`, `position: 37`, `volume: 80` | `tests/golden/{lookalike,rust-theme}/main_window_playing.png` | 275 × 116 |
| a double-size main window | `golden_lookalike_main_window_double_size` / `golden_rust_theme_main_window_double_size` | `case_main_window_double_size` (line 122) | `MainWindowState { double_size: true, ..Default::default() }` — the same default state as `main_window`, doubled; the case asserts `frame_size(&state) == (550, 232)` (S05 §2.1) | `tests/golden/{lookalike,rust-theme}/main_window_double_size.png` | 550 × 232 |
| the equaliser with a raised band | `golden_lookalike_equalizer_window_single_raised_band` / `golden_rust_theme_equalizer_window_single_raised_band` | `case_equalizer_window_single_raised_band` (line 165), through `equalizer_golden` | `EqualizerState::default()` with `bands[4] = 0` (maximum saved value); every other band flat at 31. `equalizer_golden` asserts `eq_frame_size ... == (275, 116)` | `tests/golden/{lookalike,rust-theme}/equalizer_window_single_raised_band.png` | 275 × 116 |

What each one pins down:

- **`main_window_playing`** — the marquee text row and its scroll offset
  (S02 §3, 5 px per cell), the four time digits with the blank leading cell
  (S01 §2.3 / S02 §4), the `kbps`/`kHz` numeral fields (S02 §5), the lit stereo
  label (S02 §6), the playing and decoder-active indicators (S02 §1), and the
  moved position/volume thumbs (S01 §2.1).
- **`main_window_double_size`** — the whole-frame exact integer 2x of S05 §2.1.
  The committed PNG is **exactly** the 2x of `main_window.png`: verified offline
  for both namespaces by decoding the two PNGs and comparing every 2x2 block
  (127600 samples, **0** differing pixels, max channel delta 0). The invariant is
  independently asserted in code by `crates/rustamp-ui/tests/render.rs:398`
  (`double_size_is_exactly_twice_in_both_axes`:
  `doubled == single.scaled(2)`), which is why the golden case does not repeat
  it.
- **`equalizer_window_single_raised_band`** — one band off the flat row: the
  thumb-y mapping for saved values 0 → row 38, 31 → 63, 63 → 89 (S01 §3.1) and
  the graph background the thumb sits over (S01 §3). Saved units are `0..=63`
  with 31 flat.

All three are built exclusively from public fields/methods of `rustamp-ui`. No
private field had to be poked, and no renderer had to change.

## Bless list: empty

There is **no image for the orchestrator to bless**. All six images that the
three named cases produce (three cases × two committed namespaces) already exist
and are tracked by git:

| Image | Namespace | Size | `git ls-files` |
|---|---|---|---|
| `main_window_playing.png` | `lookalike` | 275 × 116 | tracked |
| `main_window_double_size.png` | `lookalike` | 550 × 232 | tracked |
| `equalizer_window_single_raised_band.png` | `lookalike` | 275 × 116 | tracked |
| `main_window_playing.png` | `rust-theme` | 275 × 116 | tracked |
| `main_window_double_size.png` | `rust-theme` | 550 × 232 | tracked |
| `equalizer_window_single_raised_band.png` | `rust-theme` | 275 × 116 | tracked |

A full inventory of the two committed namespaces confirms there is nothing
outstanding anywhere: **37 expected images, 37 present, 0 missing, 0 extra** in
`lookalike`, and the same in `rust-theme`. (Expected set = every case function
in `golden_cases/mod.rs`, including the ten `vis_*` states from
`case_visualization`.)

Re-blessing would therefore be at best a no-op and at worst churn: it would
overwrite committed artwork whose pixels are already the render's pixels, and it
is exactly the "bless from a process that cannot see the real render"
fabrication this lane was told to avoid. **Do not run a bless for this row.**

If a bless is ever wanted anyway (it is not, for this row), the command and order
would be, per namespace, in this order — using the shape the README already
documents:

```
cargo xtask golden --skin lookalike  --bless   # then rust-theme
```

and within a namespace the cases run in registration order, so
`main_window_playing`, then `main_window_double_size`, then
`equalizer_window_single_raised_band`. Blessing is the orchestrator's call in
the guest VM, never a lane's.

## Why this lane added nothing

Concretely, every candidate addition was already covered:

| Candidate | Already covered by |
|---|---|
| `main_window_playing` case + image | `case_main_window_playing`, both namespaces, images committed |
| `main_window_double_size` case + image | `case_main_window_double_size`, both namespaces, images committed |
| `equalizer_window_single_raised_band` case + image | `case_equalizer_window_single_raised_band`, both namespaces, images committed |
| in-code assertion that double-size is exactly 2x | `crates/rustamp-ui/tests/render.rs:398`, and by construction in `render_main_window` (`render_1x(...).scaled(2)`, `crates/rustamp-ui/src/render.rs:55-66`) |
| "fail loudly when the image is absent" | `assert_golden`'s `GoldenOutcome::Missing` panic, `crates/rustamp-testkit/src/golden.rs:278` |

No production-code change was required, and none was made. No case was
inexpressible through the public API.

## The one adjacent state deliberately not added

For completeness, the only neighbouring state we could find that has no golden of
its own is a **double-size playing main window** (`MainWindowState { double_size:
true, transport: Playing, time_seconds: .., title: .., .. }`): `main_window_double_size`
doubles the *default/stopped* state and `main_window_playing` exists only at 1x.

It was **not** added, for two reasons: the queue row does not ask for it, and the
incremental value is small because `case_visualization` already emits
`vis_spectrum_double_size` — a doubled frame whose transport state is `Playing`.
Adding it would also need a row in `tests/golden/README.md`, which this lane does
not own. If the orchestrator wants it, it is a small, well-specified follow-up
(one case fn + one registration + the README row + a guest bless of
`main_window_playing_double_size.png`, 550 × 232, in both namespaces) — a lane
decision, not one taken here.

## Boundary and provenance

- Work happened only inside the wave-103 worktree
  `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave103-golden`;
  `D:\code\Rustamp` was read only, and nothing outside the worktree was written
  (proved below with `git status --short` in both trees).
- The files owned by other lanes (`rustamp-app`, `rustamp-audio`, `rustamp-ipc`,
  `rustamp-mediahost`, `rustamp-plughost`, `xtask`) were not touched.
- No downloads, no new dependencies, no git writes, no commit.
- No `cargo test`, no golden suite, no VM and no VM broker: the suite belongs to
  the orchestrator's guest. The only commands run were the three permitted gates.
- No expected image was blessed, created, deleted or modified by this lane.

## Verification (host, no tests)

| Command | Exit code |
|---|---|
| `cargo fmt --all -- --check` | **0** |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | **0** (no warnings) |
| `cargo build --workspace` | **0** |

Offline evidence gathered with `es.exe` (the project's file search), `git`, and a
read-only PNG decode via `System.Drawing` (no repository tooling, no test run):

- `git ls-files` shows all six images tracked.
- PNG dimension check: the six images have the sizes in the tables above.
- 2x check: `lookalike/main_window_double_size.png` and
  `rust-theme/main_window_double_size.png` are each an exact 2x of their
  namespace's `main_window.png` (0 of 127600 sampled pixels differ).
- Image inventory: 37/37 present, 0 missing in each namespace.

## What only the virtual machine can prove

Nothing here can substitute for the guest run. Only the orchestrator's VM can:

1. **Run the golden suite green** (`cargo xtask golden`, or
   `--skin lookalike` / `--skin rust-theme`) — i.e. that the committed images
   still equal what the renderers produce at `main`. This goldens suite is
   headless and needs no desktop, but it is still the orchestrator's run, not a
   lane's.
2. **Confirm the absence-of-image path is loud** if the orchestrator ever wants
   it demonstrated (e.g. temporarily moving one PNG aside) — the panic message in
   `golden.rs` is the mechanism, but only a real run shows it firing.
3. Everything the interactive session owns generally (any window, dialog, menu
   or plugin host) — irrelevant to these three cases, which render in-process
   with no window, no audio and no clock, but recorded here so the boundary is
   explicit.

No image needs blessing for this row, so the expected outcome of the guest run
for these three cases is **green with no diff artefacts and no `BLESSED` line**.
