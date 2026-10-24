# W11 docs sweep — correcting documentation left stale by the Muse-wave batch

**Role:** documentation-only correction pass following the W10 batch review
([`W10-batch-review.md`](W10-batch-review.md)). **Docs only — no code was changed, nothing was
built or tested, nothing was committed.**

**Lane worktree:** `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave58-docs-sweep`
(the only tree read or written). `git status --short` there contains only the six `docs/` files
listed in §5 plus this new card.

**Scope mapping.** The request named *F11* (docs claiming the Rust theme is generated/embedded
"at build time") and *F12* (docs describing the built-in default skin as the look-alike), plus
"F10's second half" (any other doc claim the review names as stale). Those two items are exactly
**W10 §2 finding F10, bullets 2 and 3**; W10's own F11/F12 are unrelated (a doubly printed config
error and the flush-test comment). This card therefore cites the **W10 F10** labels, and records
one further stale doc claim the review names (F5) that the sweep also fixes.

**Method.** Each correction was checked against the code before the prose was touched; every
corrected line carries a `file:line` citation. Where a claim could not be settled from code in the
tree, the text was **left unchanged** and the item is listed in §4 as unverified. Historical task
cards are kept honest with a dated "(as of …)" / superseded-note framing rather than by erasing what
they recorded when they landed.

---

## 1. Corrected claims

| # | File:line | Was | Now | Code evidence | Source |
|---|---|---|---|---|---|
| C1 | `docs/tasks/J2-rust-theme-skin.md:100` | "`verify_static_dimensions` verifies … **at build time**, failing loudly" | "… **when the generator is run by hand** …; it does **not** run at build time" | `generate_skin.py:1450` calls it from `main()`; `:1453` `if __name__ == "__main__"`; no `build.rs` under `crates/rustamp-skin/`; assets are embedded at compile time by `rust_theme.rs:14-50` (`include_bytes!`/`include_str!`) | W10 F10 bullet 2 (requester "F11") |
| C2 | `docs/PLAN.md:9` | "The bundled default is a **look-alike skin**" | built-in default is the newly drawn **Rust-themed** skin (task J2); the **look-alike** (task J1) is still shipped and selectable (`--skin lookalike` / `builtin:lookalike`) | `BuiltinDefaultSkin` → `rust_theme::sheet_image` (`skin.rs:105-116`); `builtin()` loads the Rust theme (`app/src/skin.rs:85-100`); `choose(None,None)==Builtin` (`app/src/skin.rs:30-36`); look-alike path (`:38-46`) | W10 F10 bullet 3 (requester "F12") |
| C3 | `docs/PLAN.md:11` | "The look-alike skin is used to check rendering and layout of **what we actually ship**" | original base skin is `classic-base` (external `rustamp-testdata`); the committed goldens are the `lookalike` namespace; the built-in default (Rust theme) has **no golden namespace yet** | `golden.rs:7-10` defines only `SKIN_LOOKALIKE` and `SKIN_CLASSIC_BASE`; only `tests/golden/lookalike/` is committed | W10 F10 bullet 3 |
| C4 | `docs/PLAN.md:107` | "`rustamp-skin-default` (the look-alike default skin …)" | "`rustamp-skin-default` (skin packaging: the bundled default is the Rust theme, task J2, and the look-alike skin, task J1, stays selectable)" | same as C2 | W10 F10 bullet 3 |
| C5 | `docs/PLAN.md:151` | J-lane row: "look-alike default skin (new art …)" | "built-in default skin (Rust theme, task J2) and selectable look-alike skin (task J1) …" | same as C2 | W10 F10 bullet 3 |
| C6 | `docs/PLAN.md:202` | "*Look-alike default skin:* Rustamp's own approved snapshots …" | "*Look-alike skin:* …" + "(The built-in default is the Rust theme, task card J2; it has no golden namespace yet.)" | `golden.rs:7-10`; only `tests/golden/lookalike/` committed | W10 F10 bullet 3 |
| C7 | `docs/tasks/J1-lookalike-skin.md:52` (§4) | Wiring bullets stated as current ("**Builtin Skin Implementation**: `BuiltinDefaultSkin::sheet_image` … delegates to `lookalike::sheet_image`"; "`skin::builtin()` loads … the lookalike skin on first run") | superseded note + bullets retitled "(as of J1)" with a **Now** clause: `BuiltinDefaultSkin` delegates to `rust_theme::sheet_image`; `builtin()` loads the Rust theme; `BuiltinLookalikeSkin` supplies the look-alike sheets | `skin.rs:105-116` (default → rust), `skin.rs:132-143` (look-alike fallback), `app/src/skin.rs:85-100`, `app/src/skin.rs:30-36` | W10 F10 bullet 3 |
| C8 | `docs/tasks/J3-goldens-2.md:5` | "Extend the committed golden-image suite for the **shipped look-alike default skin**" | "… for the **look-alike skin**" + dated note ("It was the shipped default when this card was written; the built-in default is now the Rust theme, task card J2") | `golden.rs:7`; only `tests/golden/lookalike/` committed | W10 F10 bullet 3 |
| C9 | `docs/tasks/D1-main-window.md:99` | "`tests/golden/lookalike/` is reserved for **our shipped default skin**" | "… for the **look-alike skin** (the built-in default is now the Rust theme, task card J2)" | `golden.rs:7`; `skin.rs:105-116` | W10 F10 bullet 3 |
| C10 | `docs/tasks/N6-config.md:36` | "`ui.easy_move` (**honoured in drag handling**; persisted on clean exit)" | "… (persisted on clean exit and read for the menu check; **not** honoured in drag handling …)" | no drag call site: only the menu check `app.rs:520-521` / `menu.rs:68`, the toggle `app.rs:1272`, and the write `config.rs:287-288`; `window.rs` has no `drag_window()` | W10 F5 / N6-2 (additional stale claim) |

All ten edits are in `docs/` only. C1–C9 are W10 F10 (the requested scope); C10 is the further
stale doc claim the review names in F5, folded in because it is the same class of defect
(a card documenting behaviour the code does not have).

---

## 2. What was verified, and how

- **No `build.rs`** anywhere in the repo (`git ls-files` returns nothing matching), so C1's
  "does not run at build time" is exact, not an inference from the Python entry point.
- **Embedding is compile-time**, not generated at build: `rust_theme.rs:14-50` lists every sheet
  and cursor as `include_bytes!`/`include_str!` over committed files under
  `crates/rustamp-skin/assets/rust-theme/`.
- **The default is the Rust theme**: `BuiltinDefaultSkin::sheet_image` (`skin.rs:105-116`) returns
  `rust_theme::sheet_image` and only falls back to a neutral fill; `SkinFallback` for
  `BuiltinLookalikeSkin` (`skin.rs:132-143`) is the only look-alike path; `builtin()` builds from
  `BuiltinDefaultSkin::source()` (`app/src/skin.rs:85-100`); `choose(None, None)` is `Builtin`
  (`app/src/skin.rs:34`).
- **The look-alike is still reachable**: `resolve_choice` maps `lookalike` / `builtin:lookalike`
  to `SkinChoice::Lookalike` (`app/src/skin.rs:38-46`), so C2/C4/C5 stay true about it.
- **Golden namespaces**: only `SKIN_LOOKALIKE` and `SKIN_CLASSIC_BASE` exist
  (`golden.rs:7-10`), so C3/C6/C8/C9 correctly stop implying a shipped-golden namespace for the
  Rust theme.
- **`easy_move`**: read only for the menu check (`app.rs:520-521`, `menu.rs:68`), toggled at
  `app.rs:1272`, written at `config.rs:287-288`; no `WindowEvent::Moved` arm starts a drag.

---

## 3. Files changed (all under `docs/`)

| File | Nature |
|---|---|
| `docs/PLAN.md` | 5 lines (9, 11, 107, 151, 202) |
| `docs/tasks/J1-lookalike-skin.md` | §4 wiring, superseded framing |
| `docs/tasks/J2-rust-theme-skin.md` | line 100 |
| `docs/tasks/J3-goldens-2.md` | line 5 + dated note |
| `docs/tasks/D1-main-window.md` | line 99 |
| `docs/tasks/N6-config.md` | line 36 |
| `docs/tasks/W11-docs-sweep.md` | this card (new) |

---

## 4. Left alone, and why

These are the W10-named or review-adjacent claims the sweep deliberately did **not** touch. They
fall outside "stale documentation" (measurement, judgment or process claims), are not verifiable
from the tree, or were not owned by this pass.

- **G3 mechanism and the ~7.8 ms figure** — `docs/tasks/G3-wake-timeout.md:82-89` (the "kernel timer
  interrupt fires at the next tick boundary (~7–8 ms later)" explanation) and `:80` (the one place
  the number appears). W10 F7 / G3-4 / G3-5 said the mechanism was self-inconsistent and the figure
  had no artefact in the tree. **Settled in task card G5 (`0f12340`)**: `G3-wake-timeout.md` was rewritten
  to withdraw the self-inconsistent tick explanation, mark the ~7.8 ms figure as an unevidenced single
  observation without in-tree run log or capture, extract and deterministically test `timeout_ms`
  (`crates/rustamp-ipc/src/shm/wake.rs:145,507`), document honest bounds in test doc comments
  (`crates/rustamp-ipc/src/shm/ring.rs:1837,1858,1892`), and commit a reproducible host probe
  (`crates/rustamp-ipc/examples/wake_timeout_probe.rs`).
- **P4 §4 framing** — `docs/tasks/P4-dll-loader.md:70-88` ("What the VM Run Must Show"). W10 F8
  says §4 reads as a safety barrier while the loader is an ABI loader, not a sandbox. This is a
  framing/security-posture judgment about a lane card, not a stale-by-the-batch sentence with a
  code check; it is left as a review finding for the lane owner.
- **A12 flush scope / cpal gap** — `docs/tasks/A12-output-flush.md:78` and the paired
  `docs/HANDOFF.md:231`. W10 F3 says the claim holds on WASAPI and the null double but not on the
  cpal fallback (whose `flush` is a documented no-op, `cpal_out.rs:365-367`). The card's own text
  frames it as a device question ("what only a real device can prove"), and the reviewer's remedy
  is an either/or judgment (make the fix backend-complete, or narrow the claim). `docs/HANDOFF.md`
  is orchestrator-owned and off-limits to this pass, so the pair is left for that owner.
- **B3 / RFC 0013 status** — `docs/rfc/0013-decoder-selection.md:3` still reads "Status:
  **Proposed**" while core comments cite it as binding (W10 B3-6). This is a **process** matter
  (RFC approval), not documentation left stale by the batch; advancing an RFC's status is an
  owner decision, not a docs edit.
- **`docs/PLAN.md:175` ("Skin browser, preferences, original default skin")** — ambiguous: it reads
  as the *feature* of offering the original Winamp default skin (`base-2.91`, the optional
  separate download), which is the shipped behaviour, rather than a claim that the original is
  Rustamp's default. Left as-is to avoid misreading the intent.
- **`docs/tasks/S07r-eq-renderer.md:88` ("The shipped lookalike's track atlas …")** — the look-alike
  is still shipped (selectable), so "the shipped lookalike" is not false; it never claimed to be
  the default. It is also a historical audit note. Left as-is.
- **`docs/tasks/J3-goldens-2.md:33`** — the blessed-golden case embeds the literal marquee title
  `"1. Rustamp - the look-alike default skin (3:42)"`. Editing the card's prose does not change the
  committed PNG, and changing the string would require re-blessing the golden (cannot run tests
  here). Left as-is.

## 5. Out of scope (not owned by this pass)

- **`crates/rustamp-skin/generator/README.md:1-34`** (W10 F10 bullet 1) still describes the
  look-alike generator, its `assets/lookalike/` output and five non-spec sizes, while `main()`
  writes `RUST_THEME_DIR` and `LOOKALIKE_DIR` (`generate_skin.py:18`) is dead. This file is a crate
  doc, **outside the owned `docs/**` paths**, so it is left for its owner.
- **Rust doc comments** — `crates/rustamp-app/src/skin.rs:4-7`, `crates/rustamp-app/src/lib.rs:12`,
  `crates/rustamp-skin/src/skin.rs:76-79`, `crates/rustamp-testkit/src/golden.rs:5-6` (W10 F10
  bullet 3's code half). `.rs` files are off-limits to a docs pass; listed for the code owner.
- **`tests/golden/README.md:10`** ("`lookalike` … Our shipped default skin") — outside `docs/`,
  off-limits here.
- **`docs/HANDOFF.md:15`** ("installer bundles a look-alike default skin") — orchestrator-owned; do
  not edit. (`docs/HANDOFF.md:231` on the flush claim is likewise that owner's.)

---

## 6. Hygiene

- Only files under `docs/` were written. **No `.rs` file, test, golden image, `docs/HANDOFF.md` or
  review card was edited**; the only new file is this card.
- **No `cargo`, test, build, generator or reference-binary command was run.** No downloads. No
  window.
- **Nothing was committed.** All changes are uncommitted in the lane worktree; see
  `git status --short` below.
- Bound to `docs/**` except `docs/HANDOFF.md`, `docs/tasks/W10-batch-review.md` and the M14 card.
