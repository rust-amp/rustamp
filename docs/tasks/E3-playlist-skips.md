# Task card E3 — Playlist skipped-entry reporting (wave 45)

**Scope**: `crates/rustamp-playlist/**`, `docs/tasks/E3-playlist-skips.md`.

## 1. Problem and Gap

Task card E2 audited `crates/rustamp-playlist` and recorded a documented degradation:
when a playlist file is well-formed overall, individual malformed or incomplete entries
(such as an empty `FileN=` in PLS, a `<track>` missing a `<location>` in XSPF, or an orphaned
`#EXTINF` directive in M3U) were silently dropped. The readers returned only the surviving
entries as a `Vec<LoadedEntry>` with no count and no indication of dropped entries, leaving
callers (and ultimately users) unaware of lost tracks.

## 2. Exactly what is dropped today, per format

| Format | Dropped Entry Condition | File and Line | Mechanism and Silent Failure |
|---|---|---|---|
| **M3U / M3U8** | Dangling `#EXTINF` directive | `crates/rustamp-playlist/src/m3u.rs:61-68, 85-88` | An `#EXTINF` sets `pending_title`/`pending_duration`. If followed by another `#EXTINF` or EOF without a media path, the metadata was overwritten or cleared silently without producing an entry. |
| **M3U / M3U8** | Unresolvable location | `crates/rustamp-playlist/src/m3u.rs:78-86` | If `base.resolve(line)` returned `None` (e.g. invalid `file://` URL), the line was ignored, pending metadata was cleared, and no entry was emitted. |
| **PLS** | Empty `FileN=` key | `crates/rustamp-playlist/src/pls.rs:89-91, 97-107` | `FileN=` with an empty value removed index `N` from `files`. The reader only iterated `files.keys()`, silently dropping the entry even if `TitleN` or `LengthN` was present. |
| **PLS** | Orphaned `TitleN`/`LengthN` | `crates/rustamp-playlist/src/pls.rs:97-107` | Any numbered index `N` present in `titles` or `lengths` without a matching `FileN` was silently omitted. |
| **PLS** | Unresolvable `FileN` location | `crates/rustamp-playlist/src/pls.rs:98-100` | If `base.resolve(file)` returned `None`, the loop did `continue`, dropping the track silently. |
| **XSPF** | `<track>` missing `<location>` | `crates/rustamp-playlist/src/xspf.rs:180-184` | In `push_track`, if `track.location` was `None` or empty, the function returned early without appending to `entries`. |
| **XSPF** | Unresolvable track `<location>` | `crates/rustamp-playlist/src/xspf.rs:180-184` | If `base.resolve(raw_location)` returned `None`, `push_track` returned early without appending to `entries`. |

Existing test pinning the silent degradation: `crates/rustamp-playlist/tests/audit.rs:251-270` (`bad_entries_in_a_good_file_are_skipped_and_the_rest_survive`).

## 3. Reporting Surface Design

### 3.1 Surface Types

We introduced two small, honest types in `crates/rustamp-playlist/src/entry.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedEntry {
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PlaylistReport {
    pub entries: Vec<LoadedEntry>,
    pub skips: Vec<SkippedEntry>,
}
```

Helper methods on `PlaylistReport`:
- `report.is_clean() -> bool`: returns `true` if `skips.is_empty()`.
- `report.skip_count() -> usize`: returns `skips.len()`.
- `report.into_entries() -> Vec<LoadedEntry>`: consumes the report and returns the entries.
- Implements `Deref<Target = [LoadedEntry]>` and `IntoIterator` for ergonomic access to entries.

### 3.2 Reporting Entry Points

Parallel reporting functions were added to `crates/rustamp-playlist/src/io.rs`:
- `read_playlist_report(bytes: &[u8], base: &Path) -> Result<PlaylistReport>`
- `read_playlist_report_with_hint(bytes: &[u8], base: &Path, path_hint: Option<&Path>) -> Result<PlaylistReport>`
- `read_playlist_report_path(bytes: &[u8], path: &Path) -> Result<PlaylistReport>`
- `load_playlist_file_report(path: &Path) -> Result<PlaylistReport>`
- `read_playlist_report_as(format: PlaylistFormat, bytes: &[u8], base: &Path) -> Result<PlaylistReport>`

Format modules provide corresponding `read_report` / `read_text_report` functions:
- `m3u::read_report`, `m3u::read_utf8_report`, `m3u::read_text_report`
- `pls::read_report`, `pls::read_text_report`
- `xspf::read_report`, `xspf::read_text_report`

### 3.3 Preserving the Existing API

The existing functions (`read_playlist`, `read_playlist_with_hint`, `read_playlist_path`,
`load_playlist_file`, `read_playlist_as`, and module `read`/`read_text` functions) continue to return
`Result<Vec<LoadedEntry>>` (or `Vec<LoadedEntry>`). They now delegate to the `*_report` functions
and call `.into_entries()`.
This keeps the existing API 100% intact and ensures zero breakage for existing callers.

### 3.4 Why this surface was chosen

1. **Honesty and Diagnostics**: A simple count (`usize`) informs the caller *that* entries were skipped,
   but gives neither the user nor the application any clue *which* entries failed or *why*. Collecting
   `SkippedEntry` with human-readable reasons (naming the orphaned directive, empty key, missing tag, or unresolvable location)
   enables actionable logging and error feedback.
2. **Backward Compatibility**: Returning a `PlaylistReport` from new entry points while delegating from
   existing ones allows callers that do not care to remain unchanged, while callers that do care can
   inspect `report.skips`.
3. **Small footprint**: No new dependencies, zero heap allocation for clean files beyond `Vec::new()`,
   and direct conversion via `into_entries()`.

## 4. Caller Audit and Recommendation for `rustamp-app`

### 4.1 Caller Audit
An audit across the entire workspace confirmed that no crate outside `rustamp-playlist`'s own test
suite currently invokes `read_playlist` or `load_playlist_file`.

### 4.2 Recommendation for `rustamp-app`
`crates/rustamp-app` owns the application wiring, UI dialogs, and user-facing notifications.
When connecting the `Load Playlist` dialog or command (S06 §4 LIST OPTS) to the playlist loader:
1. `rustamp-app` should call `rustamp_playlist::load_playlist_file_report(&path)` instead of the silent `load_playlist_file`.
2. The surviving tracks `report.entries` should be enqueued into the core playlist via `rustamp_playlist::append_loaded(&mut playlist, &report.entries)`.
3. If `!report.is_clean()`:
   - Log an informative message to stderr:
     ```rust
     eprintln!("rustamp: loaded {} tracks, skipped {} malformed entries from {}", report.len(), report.skip_count(), path.display());
     for skip in &report.skips {
         eprintln!("rustamp:   - skipped: {skip}");
     }
     ```
   - If UI notification is supported, surface a non-modal status bar note indicating `N tracks loaded (M skipped)`.

## 5. Verification

Tests added in `tests/audit.rs` and `tests/formats.rs`:
- Every format (M3U, M3U8, PLS, XSPF) was tested with a mixed file (1 good entry, 1 bad entry):
  each reports exactly 1 surviving entry and exactly 1 skip with the format-specific reason.
- Every format was tested with a clean file: each reports 0 skips and `report.is_clean() == true`.
- Disk-based loading via `load_playlist_file_report` was verified with a temporary file.
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`,
  and `cargo build --workspace` are clean.
