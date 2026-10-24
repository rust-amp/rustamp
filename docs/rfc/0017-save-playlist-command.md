# RFC 0017: a core command for saving the playlist

Status: **Proposed** (2026-10-19). Acceptance belongs to the integration lead.

## Goal

Users can save the current playlist to a file in M3U, PLS or XSPF format. The
I/O already exists (`rustamp_playlist::write_playlist`), and RFC 0005 §11.6
records the gap: "Open / save playlist | 40202/40204 | **no commands today**".
This RFC adds the one command the save path needs and nothing about its UI.

## Evidence

- `specs/S03-shortcuts.md` records command id **40204** for "Save playlist"
  (menu), next to 40202 for "Open playlist".
- `rustamp-playlist` writes M3U, M3U8, PLS and XSPF (`write_playlist`). B4S is
  recognized but not implemented.
- RFC 0010 left save-playlist untouched on purpose ("Save-playlist (RFC 0005
  §11.6's `40204` side) is untouched by this RFC").

## Decision

Append one variant at the end of `Command` (after `OpenVisualizationMenu`, per
the RFC 0005 §3 append rule):

```rust
/// Write the current playlist to the given file. The format follows the
/// extension (`.m3u`, `.m3u8`, `.pls`, `.xspf`); any other extension is
/// refused. S03 records command id 40204 for it. RFC 0017.
SavePlaylist(PathBuf),
```

Wire index: **59 (0x3b)**. Every existing variant keeps its index.

And:

- `routing::app_owns` returns `true` for it (explicit arm).
- `App::apply_app` calls `App::save_playlist_file(path)`.
- `save_playlist_file` picks the format from the extension, writes the queue
  in queue order (`TrackSource::Path` → `MediaLocation::File`,
  `TrackSource::Url` → `MediaLocation::Url`) and writes the file.
- An unknown extension, B4S or an I/O failure records an error the same way a
  failed load does (a note, `last_error`, the published error). No file is
  written for an unknown extension or B4S.
- The engine ignores the command (its catch-all); the main-window machine
  predicts nothing for it.

## Why a payload, and why no dialog here

`SavePlaylist` carries the target path, so it is the *action*, not the
*request for a dialog*. Opening a save dialog needs its own intent (a
`DialogKind` plus an open command, as `LoadPlaylist` has through
`OpenPlaylist`). That is a separate decision with its own UI placement (File
menu, playlist LIST OPTS, or Ctrl+S) and is **not** part of this RFC. Until
it lands, the command is reachable through `submit` and tests only.

## Backwards compatibility

The variant is appended, so every existing postcard byte string is unchanged.
An older consumer that receives index 59 fails the decode with an unknown
variant index. No current wire path emits it, so `PROTOCOL_VERSION` stays as
it is, with the same reasoning as RFC 0010 and RFC 0016.

## What stays open

- **Where the user reaches it** (File menu, LIST OPTS, Ctrl+S). The spec fixes
  the id but not the placement of the save entry.
- **Titles.** Saved entries carry no title yet; the queue has no per-entry
  title source at this layer.
- **Overwrite confirmation.** The save overwrites an existing file without
  asking.
- **Relative paths.** Entries are written as absolute paths.

## RFC acceptance

- The variant is appended after `OpenVisualizationMenu` with a doc comment.
- `routing::app_owns` names the variant in an explicit arm.
- Unit tests pin the wire index (0x3b) and round-trip; app tests write M3U,
  M3U8, PLS and XSPF files and read them back in queue order, and refuse an
  unknown extension and B4S without writing.
- `cargo fmt --all -- --check` and
  `cargo clippy --workspace --all-targets --all-features -- -D warnings` are
  clean.
- The test suite runs in the Hyper-V test VM, not on the host.
