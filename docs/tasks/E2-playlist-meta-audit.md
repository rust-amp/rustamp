# E2 — playlist and metadata audit (wave 40)

Scope: `crates/rustamp-playlist`, `crates/rustamp-meta`. Spec basis: `specs/S06-playlist.md`
settles window behaviour only (title bar, buttons, selection, scrolling, REM, dialogs). It says
nothing about playlist *files*, tags or title formatting, so no spec line is contradicted by the
code, and no behaviour below was taken from the spec. The choices are the loudest safe default.

## Findings (severity order) and what changed

| # | Finding | Where (before) | Now |
|---|---|---|---|
| 1 | Binary data named `.m3u`/`.m3u8` was parsed as M3U: every line became an entry | `format.rs` detect, early return for the M3U hint | rejected (`UnsupportedFormat`) |
| 2 | HTML/other XML named `.xspf`/`.pls`/`.m3u` gave an empty list or junk entries, no error | same | rejected |
| 3 | Unknown extension (`.txt`, `.mp3`) with marker-less text parsed as an M3U list | detect fallback | rejected; no extension / no path keeps the bare-list fallback |
| 4 | Truncated XSPF returned the tracks seen so far, no error | `xspf::read_text` by design | `read_playlist_as` runs `xspf::validate` first; module reader unchanged |
| 5 | `read_playlist_as(fmt, bytes)` believed the caller: PLS bytes as M3U gave junk entries, M3U as XSPF gave `[]` | `io.rs` | content-vs-format check, M3U/M3U8 interchangeable |
| 6 | Writers silently changed an entry: line break in a location became a space (a different file), a leading `#` made an M3U entry a comment, empty or padded locations vanished or were trimmed | `m3u.rs` `sanitize_line`, `pls.rs` | `PlaylistError::InvalidEntry` (maps to core `InvalidArgument`) from `write_playlist`; titles still sanitized (documented loss) |
| 7 | `write_tags` wrapped `year as u16` (70000 became 4464) | `rustamp-meta/src/write.rs` | `year > 9999` is `InvalidArgument`, checked before the file is opened |
| 8 | Crate descriptions claimed B4S I/O (playlist) and S14 title formatting (meta) | both `Cargo.toml` | corrected |

## Left alone on purpose (documented in the CONTRACT_NOTES files)

- Per-entry leniency inside a well-formed file (empty `FileN=`, track without location, dangling
  `#EXTINF`): pinned by `tests/audit.rs`; the readers return only survivors, no skipped count.
- Marker-less text named `.pls` reads as an M3U list (existing tested rule).
- Round-trip losses (trimmed titles, line breaks, artist/album): table in
  `rustamp-playlist/CONTRACT_NOTES.md` section 4. Needs S14 and a core RFC.
- Meta: `extra` keys without a target field are skipped, `DISCTOTAL` is written only where lofty
  has a key, unparsable dates/numbers become `None`, `BestAttempt` parsing drops damaged items, a
  media-looking file with a damaged body may read as empty. Making these errors needs a strict
  parsing decision and a skipped-items API; no spec asks for it.
- B4S: still unsupported (no public format description); recognized by content and extension and
  rejected on every read and write path, tested.

## Missing spec decisions

S14 (raw vs formatted title; artist/album storage), S15 (shuffle sequence), a spec for playlist
file handling (unknown extension policy, skipped-entry reporting), and a B4S format description.

## Verification

`cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`
and `cargo build --workspace` only. The new tests (`rustamp-playlist/tests/audit.rs`, the audit
block at the end of `rustamp-meta/tests/tags.rs`) were **not run** on this host; they must run in
the test VM.
