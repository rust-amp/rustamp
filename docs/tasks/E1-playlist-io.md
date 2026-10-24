# Task card E1 — Playlist file I/O and reordering (Wave 2)

**Goal**
`rustamp-playlist` reads and writes the classic playlist formats on top of the
frozen `rustamp_core::Playlist` model, resolves relative paths the way a media
player must, and provides the one-shot reordering operations the UI needs.
When the card is done, opening a real `.m3u`, `.m3u8`, `.pls` or `.xspf` gives
the right entries with the right titles and durations, and writing them back
round-trips.

**Lane** E — playlist (`crates/rustamp-playlist/`).

**Owned paths**
- `crates/rustamp-playlist/**`
- Do **not** edit `Cargo.toml` at the repository root and do not touch any
  other crate. `rustamp-core`'s `Playlist` is frozen; if you need a change
  there, write a contract proposal in `crates/rustamp-playlist/CONTRACT_NOTES.md`
  and report it instead of editing `rustamp-core`.

**Allowed sources**
- `rustamp_core` (the frozen model) and `docs/rfc/0001-core-contracts.md`.
- Public, published format descriptions for M3U/M3U8, PLS and XSPF, plus
  general knowledge of how these formats are used in practice.
- `docs/PLAN.md` (section 2 and the milestone list) for scope.

**Forbidden sources**
- Winamp 5.x source, Winamp SDK headers or documentation, disassembly, the
  Winamp 2.95 binary, `D:\code\rustamp-research`. Nothing from them.
- Do not download anything (no `cargo add`, no git clone, no web fetching). If
  you need a free reference implementation (DeaDBeeF is copy-allowed), say so
  in your report and the orchestrator will provide it.

**Spec dependencies**
- **S06** (playlist editor behaviour), **S14** (title formatting) and **S15**
  (shuffle algorithm) do not exist yet.
  - Do **not** implement the shuffle *sequence* algorithm: that is S15. The
    one-shot `randomize` (a single random permutation of the current order) is
    in scope, the "shuffle" toggle behaviour is not.
  - Do **not** implement title formatting (`%1`–`%7`): that is S14. Your
    readers may fill `title`/`duration_ms` from what the file itself contains
    (`#EXTINF`, `TitleN`, `LengthN`, XSPF `title`/`duration`).
  - The playlist *editor* (selection, drag-reorder, scrolling, menus) is S06 and
    belongs to another lane. Model-level helpers only.
- **B4S** is a Winamp-specific XML format with no spec and no free reference
  available in this repo. Implement it **only** if you can point at a public
  format description you actually used; otherwise leave a clearly marked
  `Unsupported` arm plus a spec request in your report.

**Interface contract**
- A `PlaylistFormat` enum (`M3u`, `M3u8`, `Pls`, `Xspf`, ...), detection by
  extension **and** by content sniffing, `read_playlist(bytes, base) ->
  Result<Vec<LoadedEntry>>` and `write_playlist(format, entries) -> Result<Vec<u8>>`,
  where `LoadedEntry` carries a `MediaLocation` plus optional title/duration.
  Loading into a `Playlist` must go through the existing `insert_many`/`push`
  API so entry ids stay stable and never reused.
- Additions to `rustamp-ipc` or `rustamp-core` are **not** part of this card.

**What must work**
- **M3U/M3U8.** `#EXTM3U`, `#EXTINF:<seconds>,<title>`, `#EXTALBUM`,
  `#EXTART`, plain comment lines, blank lines, CRLF and LF, and a file with no
  header at all. `.m3u8` is UTF-8; `.m3u` is frequently Windows-1252 or
  CP-1251 — decode UTF-8 first and fall back to a **hand-written
  Windows-1252 table** (256 entries, no new dependency). Never panic on invalid
  bytes; use replacement characters.
- **Relative path resolution.** A relative entry resolves against the playlist
  file's directory, not the process working directory. Windows separators and
  forward slashes both work. `file://` URLs become `MediaLocation::File`, and
  `http(s)://` entries become `MediaLocation::Url` and are left untouched.
- **PLS.** `[playlist]`, `FileN`, `TitleN`, `LengthN`, `NumberOfEntries`,
  case-insensitive keys, missing `N` entries tolerated, out-of-order or
  duplicate indices handled deterministically (document the rule).
- **XSPF.** The `track` list with `location`, `title`, `duration` (ms),
  `xml:base` and relative resolution; entity unescaping; unknown elements
  ignored. A minimal XML reader written in-crate is fine — no new dependency.
- **Writing.** M3U (with `#EXTM3U`/`#EXTINF` when the entry has a title or
  duration), PLS and XSPF. UTF-8 output with `\r\n` for M3U, matching what the
  formats conventionally use.
- **Reordering.** `randomize` (one random permutation, using a small in-crate
  PRNG you implement, seeded so tests are deterministic) built on
  `Playlist::apply_order`. `sort` and `reverse` already exist in core: do not
  duplicate them.
- Nothing in this card allocates on an audio path, but keep the readers
  streaming-friendly: a 50 000-entry file must parse without pathological
  memory or quadratic behaviour.

**Acceptance tests**
- [ ] `cargo fmt --all -- --check` clean.
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean.
- [ ] `cargo test --workspace --all-features` green, with new tests for:
  - [ ] each format: a small hand-written sample parses to the expected
        locations, titles and durations;
  - [ ] round-trip per format: entries → bytes → entries keeps order,
        locations, titles and durations;
  - [ ] relative-path resolution from a playlist in a subdirectory (the test
        must prove the working directory is irrelevant);
  - [ ] `file://` and `http://` handling;
  - [ ] Windows-1252 fallback for `.m3u` with a non-UTF-8 byte;
  - [ ] malformed input never panics: truncated `#EXTINF`, `PLS` with a
        non-numeric index, XSPF with unclosed tags, an empty file, a binary
        blob;
  - [ ] a **property test** (deterministic seed, no new dependency): random
        lists of entries survive write → read unchanged, and `randomize`
        produces a permutation of exactly the same ids;
  - [ ] a 50 000-entry M3U parses in well under a second (state the measured
        time in your report, keep the threshold generous enough for CI).
- [ ] `cargo test -p rustamp-plugin-abi --target i686-pc-windows-msvc` and
      `cargo build -p rustamp-plughost --target i686-pc-windows-msvc` still green.
- [ ] Report which formats you did **not** implement and why (B4S, shuffle).

**Provenance notes**
- No third-party code is expected here. If you port code from a free project,
  stop and report it: it needs a `THIRD_PARTY.md` entry and a kept license
  header.

**Report back**
Files changed, the formats implemented, the deterministic rules you chose for
ambiguous input, measured timings, the spec requests for S06/S14/S15/B4S, and
any contract proposal for `rustamp-core`.
