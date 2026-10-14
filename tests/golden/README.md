# Golden images

Pixel-exact reference images for headless-rendering tests. Helpers live in the
`rustamp-testkit` crate (add it as a `dev-dependency`).

## Skin namespaces

| Namespace | Meaning | Where goldens live | Committed? |
|---|---|---|---|
| `lookalike` | Our shipped default skin | `tests/golden/lookalike/<name>.png` | **Yes** |
| `classic-base` | The original base skin (separate download) | `$RUSTAMP_TESTDATA/golden/classic-base/<name>.png` | **Never** |

`RUSTAMP_TESTDATA` points at a checkout of the external `rustamp-testdata`
repository (reference captures from real Winamp 2.95, audio fixtures). When it
is unset, `classic-base` tests print `SKIP golden classic-base/<name>: ...` and
pass. Images from the original skin or captured from the real program must
never be committed to this repository; `tests/golden/classic-base/` should be
git-ignored as a safeguard.

## Writing a golden test

```rust
use rustamp_testkit::{assert_golden, Snapshot, SKIN_LOOKALIKE};

#[test]
fn golden_lookalike_main_window_stopped() {
    let window = build_main_window(/* ... */);       // implements Snapshot
    assert_golden("main_window_stopped", SKIN_LOOKALIKE, &window.render_image());
}
```

Conventions:

* Test fn names start with `golden_`, then the skin namespace with `-` as `_`
  (`golden_lookalike_*`, `golden_classic_base_*`). `cargo xtask golden` filters
  on this prefix; `--skin classic-base` narrows it to `golden_classic_base`.
* Case names are `snake_case`, e.g. `main_window_playing_clutterbar_o`. They
  may contain `/` for subfolders.
* PNG, 8-bit RGBA, comparison is exact (tolerance 0).

## Workflow

```
cargo xtask golden                      # run all golden_* tests
cargo xtask golden --skin lookalike     # only one namespace
cargo xtask golden --bless              # write/overwrite goldens (RUSTAMP_BLESS=1)
```

* Missing golden: the test fails with a hint to bless.
* Mismatch: the test writes `target/golden-out/<skin>/<name>.actual.png` and
  `<name>.diff.png` (red pixels over a dimmed copy of the expected image) and
  panics with the mismatch count, bounding box and max channel delta.
  `cargo xtask golden` lists those files afterwards.
* After an intended change, bless, review the PNG diff in your PR, and commit
  only `lookalike` goldens.

## Capturing references from a running program

`cargo xtask capture --list` and
`cargo xtask capture --title <substr> [--class <cls>] [--client-only] --out file.png`
(Windows only) produce reference PNGs of any window. Store results for the
original skin in `rustamp-testdata`, not here.
