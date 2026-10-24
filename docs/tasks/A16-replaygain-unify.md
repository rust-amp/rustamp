# A16 — Unify the ReplayGain parser

**Date** 2026-10-17. **Lane** `rustamp-audio` + `rustamp-meta` (reader only).

**Status:** implemented; host compilation/linting only, VM test execution pending.

## 1. Goal

`rustamp-audio` (A15) mirrored the two ReplayGain key spellings and
`parse_gain_db` from `rustamp-meta` (E4) because it could not add a
dependency. This card removes that mirror: the audio crate now consumes
`rustamp-meta`'s constants and reader, so there is one owner of the format.
No behaviour changes: same keys, same parsing, same honest answers for
malformed input, same tests (now exercising the shared code).

## 2. Licences

- `rustamp-audio`: `GPL-3.0-or-later` (workspace licence).
- `rustamp-meta`: `GPL-3.0-or-later` (workspace licence).

Both crates share the workspace licence, so `rustamp-audio` depending on
`rustamp-meta` is an internal, same-licence edge. No external crate was
added, so the licence closure and the permissive-crate closures are
unaffected; `Cargo.lock` only gained the internal `rustamp-meta` entry for
`rustamp-audio`.

## 3. What was deleted and what now owns the format

Deleted from `crates/rustamp-audio/src/replaygain.rs`:

- the local `pub const TRACK_GAIN_KEY` / `ALBUM_GAIN_KEY` definitions;
- the local `pub fn parse_gain_db` implementation.

They are now re-exports of `rustamp_meta::replaygain::TRACK_GAIN_KEY`,
`rustamp_meta::replaygain::ALBUM_GAIN_KEY` and
`rustamp_meta::replaygain::parse_gain_db`. `select_gain_db` no longer
reads the `BTreeMap` by hand; it builds a `TrackMetadata` and delegates to
`rustamp_meta::replaygain::ReplayGainTags::from_metadata`, the metadata
crate's reader.

What now owns the format: `crates/rustamp-meta/src/replaygain.rs` — the
key constants (`TRACK_GAIN_KEY`, `ALBUM_GAIN_KEY`, …), `parse_gain_db`,
`ReplayGainTags::from_metadata` / `apply_to_metadata`, and the gain/peak
formatting. `rustamp-audio` only adds its own policy on top (mode,
fallback, preamp, linear conversion, application).

No cycle: `rustamp-meta` depends on `rustamp-core` and `lofty` only;
`rustamp-audio` depends on `rustamp-core` (and others) but not on
`rustamp-meta` previously, and nothing in `rustamp-meta` depends on
`rustamp-audio`. The dependency direction is acyclic. No layering rule is
violated, so no alternative was needed.

## 4. Tests

The audio unit tests (9) and integration tests (7) are unchanged in shape.
They now exercise the shared code:

- `gain_strings_parse_like_the_metadata_crate` calls `parse_gain_db`, which
  is now `rustamp_meta::replaygain::parse_gain_db`.
- `select_gain_db` (track/album preference, both fallbacks, malformed →
  absent) is implemented through `ReplayGainTags::from_metadata`.
- `effective_gain`, `gain_db_to_linear`, `apply_gain`, and the control
  round-trips are unchanged.

The audio side still gets the gain it expects from a tag, and a malformed
tag still plays unadjusted (gain `1.0`), because `from_metadata` treats
unparsable values as absent, identical to the previous mirror.

## 5. Files and line counts

- `crates/rustamp-audio/Cargo.toml`: +1 line (internal dep).
- `Cargo.lock`: +1 line (internal dep ordering).
- `crates/rustamp-audio/src/replaygain.rs`: 442 lines (was ~470; −28 net
  after re-exports and reader delegation).
- `crates/rustamp-meta/src/replaygain.rs`: 717 lines (unchanged).

Diff: 3 files, 25 insertions / 26 deletions.

## 6. Verification

- `cargo fmt --all` then `cargo fmt --all -- --check`: PASS (exit 0).
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`:
  PASS (exit 0, no warnings).
- `cargo build --workspace`: PASS (exit 0).
- `cargo test`: NOT RUN (Hyper-V test-VM rule; no test executed on the
  host).
- No window opened, no audio played, no download, no external dependency
  added, no commit or other git write.

Final `git status --short` (owned paths only):

```
 M Cargo.lock
 M crates/rustamp-audio/Cargo.toml
 M crates/rustamp-audio/src/replaygain.rs
?? docs/tasks/A16-replaygain-unify.md
```
