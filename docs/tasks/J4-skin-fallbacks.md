# Task card J4 — Skin cursor fallback, lookalike dimensions, generator README

Closes three W10-review findings in the skin crate (all proven, all in
`crates/rustamp-skin`): **F9** (cursors get no fallback), **F6** (the lookalike
lost its only dimension test), **F10** (the generator README is stale).

Lane: skin fallbacks. Owned paths: `crates/rustamp-skin/**` (source, tests,
generator directory) and this card. `crates/rustamp-ui`, `crates/rustamp-app`,
`tests/golden/**` and every other crate were not touched; no PNG was touched.

## 1. F9 — missing cursors now follow the sheet rule

**Before:** `skin.rs` (`from_source`, old lines 251–266) pushed a cursor only
when the container carried it — no `SkinFallback` call, no `Substitution`.
A skin without a cursor left the loader with nothing, while a missing sheet
fell back to the built-in artwork (`resolve_default_fallbacks`). 13 of the 24
names in `CURSOR_NAMES` (`webamp.rs:526–551`) are absent from both built-in
skins, so 13 loader-requested names silently resolved to nothing.

**Decision (the honest choice per the finding):** a missing cursor falls back
to the built-in skin's own cursor of the same name, exactly like a missing
sheet — and a name the built-in skin does not carry either stays absent with
no substitution, exactly like a sheet the fallback has no size for. What a
skin author gets today: ship any of the 24 names and it wins; omit one the
built-in Rust theme carries (11 names) and the player uses the built-in
cursor and records a `DefaultSkin` substitution; omit one of the other 13
(`eqclose`, `eqnormal`, `mainmenu`, `mmenu`, `min`, `pclose`, `pwinbut`,
`pwsnorm`, `pwssize`, `songname`, `winbut`, `wsnormal`, `wsposbar`) and
`Skin::cursor` returns `None` — no silent wrong cursor is ever invented.

**Changes** (`crates/rustamp-skin/src/skin.rs`):
- `SkinFallback` gains a defaulted `cursor(&self, name) -> Option<CursorFile>`
  (skin.rs:94); the default is `None`, so a sheets-only fallback (and every
  existing implementor) keeps the old behaviour with no code change.
- `BuiltinDefaultSkin::cursor` (skin.rs:128) serves the Rust theme's own
  cursor bytes (`rust_theme::cursor_bytes`, case-insensitive) with the same
  `Cur`/`Ani` RIFF sniff the loader uses. `BuiltinLookalikeSkin` ships no
  cursors, so it keeps the `None` default (documented on the type).
- `from_source` calls the new `resolve_cursor_fallbacks` (skin.rs:468) after
  the cursor loop: per `CURSOR_NAMES` slot, keep a shipped cursor (a shipped
  `foo.ani` counts as carrying the `foo.cur` slot via `cursor_stem`,
  skin.rs:450, so the fallback never shadows it), otherwise take the
  fallback's cursor and push a `Substitute::DefaultSkin` substitution.
- Doc lines updated to state the rule: `Skin::cursors` (skin.rs:358),
  crate docs (`lib.rs`).

**Effect on existing tests:** `rust_theme::builtin_skin` still loads with zero
substitutions (the 11 shipped cursors cover their slots; the other 13 get no
fallback either). The synthetic skins in `tests/loader.rs` carry no cursors,
so they now load 11 fallback cursors plus 11 `DefaultSkin` substitutions —
no existing assertion pins counts there (all use `.any(...)`), and
`corpus.rs` only prints substitutions.

## 2. F6 — the lookalike's dimensions are pinned again

**Before:** `tests/lookalike.rs` (lines 5, 10, 28) imported and exercised
`BuiltinDefaultSkin`, which J2 re-pointed to the Rust theme — so the J1
dimension/sprite tests silently covered the Rust theme and
`BuiltinLookalikeSkin` had no dimension test.

**Changes** (`crates/rustamp-skin/tests/lookalike.rs`, rewritten):
- `LOOKALIKE_SHEET_SIZES` (line 26) pins the lookalike's own 16 sheet
  dimensions, decoded from the committed assets; the five non-spec sizes are
  called out in the comment (titlebar 350 wide / spec 344, balance 47 wide /
  spec 68, monoster 56 wide / spec 58, eq_ex 275×56 / spec 275×82, gen
  178×86 / spec 194×109).
- `lookalike_sheets_decode_at_their_own_dimensions` (line 46): the 16 sheets
  decode at those sizes and `lookalike::sheet_bytes` is `Some` for exactly
  those 16 (`None` for `GenEx`/`Mb`/`Video`).
- `lookalike_source_loads_with_its_own_bytes` (line 81): the 16 sheets keep
  their embedded bytes (non-empty `sheet_raw`); the 3 newer sheets come from
  the fallback fill at the spec extents (empty `sheet_raw`).
- `every_sprite_fits_within_the_lookalike_sheets` (line 121) restores the
  sprite-rectangle fit check against the lookalike's own (partly non-spec)
  sheets; the magenta and text-font tests are kept, retargeted to
  `BuiltinLookalikeSkin`.
- `builtin_default_skin_is_still_the_rust_theme` (line 257) guards the F6
  confusion directly: the default is the 19-sheet / 11-cursor Rust theme with
  zero substitutions. The Rust theme's own suite (`tests/rust_theme.rs`) is
  untouched.

## 3. F10 — the generator README describes the generator again

**Stale claim:** `generator/README.md` still described the J1 lookalike
generator (16 BMPs into `assets/lookalike/`, five non-spec sizes), while
`main()` writes only the Rust theme.

**Fix** (README rewritten): documents the 33 `RUST_THEME_GENERATORS` outputs
into `assets/rust-theme/` (19 BMPs incl. `nums_ex.bmp`, 11 CURs, 3 TXTs) with
a file table; documents `verify_static_dimensions` and explicitly corrects
the J2 card's "at build time" claim (there is no `build.rs` — the check runs
when the script is executed manually); marks `assets/lookalike/` as frozen
J1 outputs and `LOOKALIKE_DIR` as dead. The other two F10 items (the J2 card's
build-time claim at `J2-rust-theme-skin.md:100` and the `rustamp-app` skin-doc
comments) live outside this lane's owned paths, so they are noted here, not
edited.

## 4. Tests

New/rewritten, all headless (in-memory `MemorySource` or `tempfile` folders;
no `.wsz` fixture needed, so nothing is left to the VM):
- `tests/loader.rs`: `missing_cursors_fall_back_to_the_builtin_skin` (a
  cursor-less skin loads, `close.cur` is usable and byte-identical to the
  built-in cursor, substitution recorded), plus the sheet-fallback pin that
  already existed (`missing_main_sheet_uses_default_skin`);
  `cursor_missing_from_both_skin_and_fallback_stays_absent` (the 13-name rule:
  `eqclose.cur` absent, no substitution);
  `shipped_cursor_is_never_shadowed_by_the_fallback`;
  `ani_variant_carries_the_cur_slot`.
- `tests/lookalike.rs`: as in §2.
- If a case needs a real `.wsz` fixture only the VM has: none does. The
  `RUSTAMP_TESTSKINS` fixture comparison (`tests/rust_theme.rs`) is untouched
  and still skips without the variable.

## 5. Own choices

1. Cursor fallback = sheet fallback (F9's "honest choice"), including the
   absent-on-both-sides stays-absent rule, rather than a placeholder cursor.
2. `SkinFallback::cursor` defaults to `None` (backwards compatible: no other
   implementor exists in-tree, and the default keeps sheets-only fallbacks
   total on cursors).
3. Stem matching ignores `.cur`/`.ani` suffixes case-insensitively, so a
   shipped `.ani` is never shadowed by a fallback `.cur`.
4. The lookalike dimension table pins the five non-spec sizes as-is instead
   of "fixing" the art (renderer-owned coupling, flagged for its lane).

## 6. Verification (host; `cargo test` not run per lane constraint)

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | clean (exit 0; one `cargo fmt --all` pass applied first) |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | clean, no warnings |
| `cargo build --workspace` | success (dev profile) |
| `cargo test` | **not run** (suite runs in the Hyper-V test VM; orchestrator runs it) |

No test was run, no window opened, no audio played, no commit made, no
download, no `cargo add`, no new dependency. `git status --short` in the
worktree shows only the five owned files below; no PNG touched.

## 7. Files and line counts

| File | Change |
|---|---|
| `crates/rustamp-skin/src/skin.rs` | +fallback `cursor` on the trait/default skin, `cursor_stem`, `resolve_cursor_fallbacks`, doc updates (~+60) |
| `crates/rustamp-skin/src/lib.rs` | fallback-rule doc line (+2/−1) |
| `crates/rustamp-skin/tests/loader.rs` | 4 new cursor-fallback tests (~+90) |
| `crates/rustamp-skin/tests/lookalike.rs` | rewritten for `BuiltinLookalikeSkin` + routing guard (~+280/−156) |
| `crates/rustamp-skin/generator/README.md` | rewritten for the Rust-theme generator |
| `docs/tasks/J4-skin-fallbacks.md` | this card |

## 8. Provenance

Clean-room: no Winamp source, SDK headers or disassembly read; no
`D:\code\rustamp-research` / `D:\code\rustamp-refbin` read or written.
Sources used: `specs/S01-window-geometry.md` (dimensions),
`specs/S17-skin-loading-quirks.md` (fallback behaviour), `specs/S02-*`
(sprite layout, unchanged), the in-tree `webamp.rs` sprite tables (MIT,
attributed in `THIRD_PARTY.md`), and the W10 finding text itself. No third-
party code copied; no new dependency.
