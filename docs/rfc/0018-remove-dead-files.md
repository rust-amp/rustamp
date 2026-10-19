# RFC 0018: a core command for "Remove all dead files"

Status: **Proposed** (2026-10-19). Acceptance belongs to the integration lead.

## Goal

`Alt+Delete` in the playlist editor removes every entry whose local file no
longer exists on disk ("Remove all dead files", S03 §2, command id **40222**).
The playlist model and the app already remove entries by id
(`Command::Remove`), but the *scan* — which entries are dead — is app state
that a plugin or an external tool can request through the public id, so the
action needs its own command under RFC 0005 §3 rule 2 ("App state … variant,
the app applies it").

The sibling row of this task, **Clear the playlist** (`40214`), needs no new
variant: `Command::Clear` already exists and is routed to the app, so this RFC
only adds the chord wiring for it (S03 §2 binds both `Ctrl+Shift+Delete` and
`Ctrl+N` to that one id).

## Evidence

- `specs/S03-shortcuts.md` §2 records, verbatim:
  - `Alt+Delete | 40222 | Remove all dead files | menu`
  - `Ctrl+Shift+Delete | 40214 | Clear the playlist | menu`
  - `Ctrl+N | 40214 | Clear the playlist | menu`
- `specs/S06-playlist.md` settles entry *removal* as a list-band operation
  (§5.1) and fixes one no-op rule: without anything staged, a removal changes
  nothing (§5.3). The spec says nothing about dead files themselves — which is
  exactly the gap this RFC closes.
- `docs/rfc/0005-app-commands.md` §11.6 lists "remove dead files" (`40222`)
  among the deferred app actions, with the note that the names are decided
  together with the playlist window. That window now exists.
- `Command::Clear` (existing) and `Command::Remove(Vec<EntryId>)` (existing)
  cover the effect; the new variant only carries the *request* to compute the
  dead set.

## Decision

Append one variant at the end of `Command`, after `SavePlaylist` (RFC 0017;
appending never inserts into the logical sections, RFC 0005 §3):

```rust
/// Remove every entry whose local file no longer exists on disk ("Remove
/// all dead files", Alt+Delete, S03 §2, command id 40222). Network streams
/// ([`MediaLocation::Url`](crate::playlist::MediaLocation::Url)) are never
/// checked and always survive; the spec fixes the label only, so the
/// stream rule is an own choice (RFC 0018). No payload: the app owns the
/// player's state and resolves the dead entries itself. RFC 0018.
RemoveDeadFiles,
```

**Wire index: 60 (0x3c)**, directly after `SavePlaylist` (59, 0x3b). Every
existing variant keeps its index.

And:

- `routing::app_owns` returns `true` for it (explicit arm).
- `App::apply_app` calls `App::remove_dead_files`.
- `remove_dead_files` pairs the app's id table with the engine queue and keeps
  the ids of entries whose source is dead, then calls the ordinary
  `App::remove_entries(&dead)` — no second removal path. That means the
  playing entry stops the engine and clears Current, surviving entries keep
  their stable ids, and the playlist model is re-synced, exactly as after
  `Delete` or `REM`.
- Dead means: a `TrackSource::Path` for which `Path::exists()` is `false`.
- A `TrackSource::Url` is a network stream, not a file, so it is **never
  probed and always survives**. S03 settles the menu label only, so this rule
  — like keeping a path that now names an existing directory — is an **own
  choice**.
- An empty dead set is a silent no-op (no note, no event), mirroring S06 §5.3.
- The engine ignores the command (its catch-all); the UI machine predicts
  nothing for it.

### Keyboard

- `PlaylistKeyAction` gains `RemoveDead` (`Alt+Delete`) and `ClearPlaylist`
  (`Ctrl+Shift+Delete`, `Ctrl+N`), and `playlist_key_action` maps them.
  `Ctrl+Shift` is checked as a modifier *set* before the single-modifier arms.
- `App::playlist_key_down` submits `RemoveDeadFiles` or `Clear` and re-syncs
  the playlist model, the same shape as the existing Delete and Crop rows.
- Neither action needs a selection, so both work on an unstaged list — unlike
  `Delete` (S06 §5.3) and `Crop` (own choice).

## Why no payload

The set of dead entries depends on the filesystem at the moment of the
request, so a caller that could send the ids would have to do the same scan the
app does. A payload would therefore duplicate the only correct computation.
`Remove(Vec<EntryId>)` remains the way to remove a known set.

## Backwards compatibility

The variant is appended, so every existing postcard byte string is unchanged.
An older consumer that receives the new index fails the decode with an unknown
variant index. No current wire path emits it (the plugin host has no
`WM_COMMAND` id table yet), so `PROTOCOL_VERSION` stays as it is, with the same
reasoning as RFC 0010, RFC 0016 and RFC 0017.

## What stays open

- **The `40222` menu label's exact rows.** S03 records the label and the id
  only; whether 2.95 also lists it in the MISC popup is a visual reading
  (S06 §4) and is not claimed here.
- **Directories.** A queued path that now names a directory is kept, because
  it *exists*; the spec does not settle the case.
- **Network shares and permission errors.** `Path::exists()` returns `false`
  for an unreachable share, so such an entry counts as dead. A stricter check
  (open the file) is not specified and would block on I/O.
- **Undo.** There is none, matching every other removal.
- **Forwarding.** The plugin host still has no id → command table; when it
  gains one, `40222` maps to `RemoveDeadFiles`.

## RFC acceptance

- The variant is appended with a doc comment; no existing variant moves.
- `routing::app_owns` names the variant in an explicit arm.
- A wire test pins the index (**0x3c**, after `SavePlaylist`'s 0x3b) and
  round-trips the variant.
- App tests: a mixed playlist of an existing file, a missing file and a URL
  loses only the missing file and keeps stable ids; a second run is a silent
  no-op; a dead current entry is cleared.
- Playlist-window tests: `Alt+Delete`, `Ctrl+Shift+Delete` and `Ctrl+N`
  produce `RemoveDeadFiles` / `Clear` and update model and queue.
- The key map covers the new rows and leaves neighbouring chords unmapped.
- `cargo fmt --all -- --check` and
  `cargo clippy --workspace --all-targets --all-features -- -D warnings` are
  clean; `cargo test --workspace --all-features` passes in the test VM.
