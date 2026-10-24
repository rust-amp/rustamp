# Task card C1 — Skin loading and pixel compositor (Wave 1)

**Goal**
`rustamp-skin` loads a skin from a `.wsz`/`.zip` file or a folder, reads its
BMPs at every bit depth, `pledit.txt`, `viscolor.txt`, `region.txt` and the
cursors, exposes the sprite rectangles and falls back to the bundled default
skin when files are missing. `rustamp-gfx` provides the CPU compositor
(blit, integer scaling, bitmap fonts). When the card is done, a headless test
paints a real skin's sheet region into an RGBA buffer and compares pixels.

**Lane** C — skin and gfx (`crates/rustamp-skin/`, `crates/rustamp-gfx/`).

**Owned paths**
- `crates/rustamp-skin/**`
- `crates/rustamp-gfx/**`
- `THIRD_PARTY.md` (only this card touches it in this wave)
- Do **not** edit `Cargo.toml` at the repository root. The workspace dependency
  `zip` is already prepared; crate-local additions go into the crate's own
  `Cargo.toml`.

**Allowed sources**
- **webamp** (MIT) — `skinSprites` / `skinParser` and the skin-css tables, as
  the source for the sprite rectangles, for the classic-layout dimensions and
  for the file-name handling.
  Checkout: `D:\code\rustamp-free-sources\webamp`, commit
  `79117c66d79fdac690fc732a0ce8df543ccd97a0`. The two files that matter most are
  `packages\webamp\js\skinSprites.ts` and `packages\webamp\js\skinParser.js`;
  see `D:\code\rustamp-free-sources\NOTES.md` first.
- **Skin corpus** (test inputs, never committed): 20 real classic skins under
  `D:\code\rustamp-free-sources\skins-corpus\`. Use them for loader tests
  (must not panic, correct fallbacks). Do **not** copy any of them into the
  repository: `packages__webamp__assets__skins__base-2.91.wsz` is the original
  base skin and is handled separately through `rustamp-testdata`.
- The local `specs/` directory (`specs/S09-plugin-abi.md` only, informational).
- `docs/rfc/0001-core-contracts.md` — the frozen contract.
- `tests/golden/README.md` and the `rustamp-testkit` crate.
- Public BMP/ZIP format documentation.

**Forbidden sources**
- Winamp 5.x source, Winamp SDK headers or documentation, disassembly, the
  Winamp 2.95 binary, `D:\code\rustamp-research`. Nothing from them, not even
  briefly. Do not download anything yourself; ask if a source is missing.

**Spec dependencies**
- **S01 (geometry/sprite rects) and S17 (skin quirks) do not exist yet.** S01
  and S17 are the dirty room's job; you cannot write them and must not try.
- Therefore: webamp's tables are a **stopgap, not validated behaviour**. Mark
  every value taken from webamp in code comments as `// provisional: webamp,
  pending S01/S17` and collect them in a single module so they are easy to
  replace. Where a behaviour is unknown and matters for M1, file a spec request
  in the final report instead of inventing an answer.

**Interface contract**
- Consumes `rustamp_core` only. Any new public type you need in `rustamp-core`
  requires an RFC (`docs/rfc/`); prefer keeping the skin types local to
  `rustamp-skin` and note the proposal in `CONTRACT_NOTES.md` inside the crate.
- `rustamp-gfx` must stay independent of `rustamp-skin`: it takes RGBA buffers
  and sprite rectangles, not skin paths.

**What the skin crate must do**
- Load from `.wsz`, `.zip` and a plain folder; the loader is case-insensitive
  for entry names (skins ship with mixed case) and rejects path escapes.
- Decode BMP at 1, 4, 8 (palettized), 24 and 32 bpp, including the `BI_RLE8`
  case if it occurs in real skins. Honour the palette, not an ad-hoc guess.
- Keep the raw decompressed BMP bytes available as well as the decoded RGBA
  image: some classic sprites are tinted or region-checked from the raw data.
- Read `pledit.txt` (the colour keys and the font), `viscolor.txt` (24 colours
  with the documented defaults) and `region.txt` (window regions, at least the
  `Normal`/`WindowShade` sections and the `NumPoints`/`PointList` form).
- Expose named sprite rectangles in a table plus a lookup that returns the
  source sheet, so `rustamp-ui` can blit without knowing file names.
- Fall back **per file**: a missing `nums_ex.bmp` uses `numbers.bmp`, a missing
  file at all uses the bundled default skin. Never fail the whole load because
  one file is missing. Report what was substituted.
- `nums_ex.bmp` is the 11-digit variant; handle the extra digit.

**What the gfx crate must do**
- An RGBA canvas with: opaque blit, blit with a source rectangle, alpha blit,
  fill, and **integer scaling** (2×) that stays pixel-exact.
- Bitmap fonts: `text.bmp` (the 3×5 glyph grid, printable ASCII plus the
  documented glyph order) and the `numbers`/`nums_ex` time-display digits,
  including the "no digit" cell for positions shorter than the field.
- No unsafe, no panics on out-of-bounds rectangles: clip instead.

**Acceptance tests**
- [ ] `cargo fmt --all -- --check` clean.
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean.
- [ ] `cargo test --workspace --all-features` green, with new tests for:
  - [ ] a synthetic skin built in memory/temp dir, loaded from a folder, a
        `.zip` and a `.wsz`: identical result for all three;
  - [ ] case-insensitive entry names (`MAIN.BMP` vs `main.bmp`);
  - [ ] BMP decode per bit depth (1/4/8/24/32) against expected pixels;
  - [ ] missing-file fallbacks (`nums_ex` → `numbers`, unknown sprite → default
        skin) with the substitution reported;
  - [ ] `pledit.txt`, `viscolor.txt` and `region.txt` parsing, including the
        default values for keys and sections that are absent;
  - [ ] blit/clip/scale: out-of-bounds rectangles clip, 2× scaling is exact;
  - [ ] bitmap-font rendering of a known string matches a byte-exact expected
        RGBA buffer (no golden PNG needed for that; a small inline expectation
        is fine).
- [ ] No panic on a fuzzed/corrupt skin: a truncated zip entry, an empty BMP,
      a BMP with a bogus header and a zero-byte `pledit.txt` all return an error
      or a default, never a panic. `#[test]` with a property-style loop over
      mutations is fine.
- [ ] **Corpus test:** every file in `D:\code\rustamp-free-sources\skins-corpus\`
      loads without a panic and yields the sprites the main window needs, or
      reports exactly which file is missing and which default was substituted.
      The test skips (with a printed `SKIP`) when the corpus path does not
      exist, so it stays portable, and it reads the corpus through a
      `RUSTAMP_SKIN_CORPUS` environment variable that defaults to that path.
- [ ] `cargo test -p rustamp-plugin-abi --target i686-pc-windows-msvc` and
      `cargo build -p rustamp-plughost --target i686-pc-windows-msvc` still green.
- [ ] Report explicitly: which behaviours are webamp-derived and provisional,
      and which spec requests S01/S17 should answer.

**Provenance notes**
- Porting the sprite tables and the file-name handling from webamp (MIT)
  **requires** all of: the upstream project, the license, the upstream version
  or commit hash, the path inside this repository, and a `THIRD_PARTY.md` entry.
  Put the MIT notice at the top of the ported module.
- Write the port in Rust with our own structure and names. Do not transliterate
  JavaScript line by line.

**Report back**
Files changed, the provisional webamp-derived values, test results, the spec
requests for S01/S17, and any contract proposal for `rustamp-core`.
