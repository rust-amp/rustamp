# RFC 0010: a core command for the playlist window's Load Playlist dialog

## Goal

The playlist window's **LIST OPTS** button opens a dialog the measurement
titles **`Load Playlist`** (S06 §4), but the core command set has no command
that opens that dialog: `PlaylistDialog::LoadPlaylist.opening_command()`
returns `None`. This RFC adds one additive app-level `Command` variant for it
and wires the UI model and the app's playlist path to it, without inventing
any dialog behaviour the spec does not fix.

## Evidence

- S06 §4 (settled, run `20261016-081527`, steps 7–8, 19–20, 23–24): a second
  real click on **LIST OPTS at (242,92)** opens a dialog titled
  **`Load Playlist`**, class `#32770`, client size 1503 x 778 (title and size
  from window metadata, not from pixels).
- The same section settles the sibling: ADD (second real click at (23,92))
  opens `Add file(s) to playlist` (`#32770`, same size).
- The UI model (`crates/rustamp-ui/src/playlist.rs`) carries both titles and
  maps ADD to `Command::OpenMedia { kind: Files, play: false }`, but
  `PlaylistDialog::LoadPlaylist.opening_command()` returns `None`, with a
  comment saying that adding a command needs an RFC. That comment is this
  RFC's gap (finding F1 of the playlist-behaviour lane).
- RFC 0005 §11.6 already names the hole from the other side: "Open / save
  playlist | 40202/40204 | **no commands today**".

## Decision

Append one unit variant at the end of `Command` (after `ToggleVisPlugin`, per
the RFC 0005 §3 append rule):

```rust
/// Open the load-playlist dialog (S06 §4, LIST OPTS button); on confirmation
/// the app loads the chosen playlist file. No payload: the path comes from
/// the user's pick, inside the app.
OpenPlaylist,
```

And:

- `PlaylistDialog::LoadPlaylist.opening_command()` returns
  `Some(Command::OpenPlaylist)`.
- `routing::app_owns` returns `true` for it (explicit arm; that match is
  exhaustive on purpose).
- `App::apply_app` handles it through the existing dialog-stub path (a note,
  no window): the spec fixes the dialog's title only, so there is deliberately
  nothing else to apply yet.
- The engine ignores it via its existing catch-all (it is not an engine
  mutation); the main-window behaviour machine predicts nothing for it (a
  dialog has no S02-visible effect); the plugin host maps no id to it (S06
  records no public `WM_COMMAND` id for LIST OPTS — message exposure beyond
  one `WM_USER` probe is open per S06 §8 Q10).

## Why an app-level command, not a UI-local action

RFC 0005 §3 gives the rule: an action gets a `Command` variant when its effect
is something the player — engine or app — must perform, and dialog management
is app state. The precedent is already in the enum: `OpenPrefs`,
`OpenMedia`, `OpenFileInfo`, `OpenAbout`, `OpenSkinBrowser` and the rest are
app-level intents, not engine mutations. The headless UI machine never opens a
dialog itself; it returns dialog requests (titles only) and `Command`s, and
the app opens the OS window. LIST OPTS is the same shape as ADD — a bottom-bar
button whose only settled effect is opening a titled dialog — and ADD already
maps to a core command (`OpenMedia`). Leaving LIST OPTS without one would keep
two sibling buttons on two different contracts for no behavioural reason.

Reusing `OpenMedia` was considered and rejected: `OpenMedia` promises the
open-file/location/directory dialog whose confirmation becomes `AddItems`
(append semantics), while S06 titles this dialog `Load Playlist` — a distinct
observed intent whose confirmation semantics the spec does not fix (see below).
Conflating the two would change what `OpenMedia` means.

## Backwards compatibility

The variant is appended after `ToggleVisPlugin`, so every existing variant
keeps its declaration index and every pre-existing postcard byte string is
unchanged (the RFC 0005 §12 wire proof still holds untouched). An older
consumer that receives the new variant fails the postcard decode with an
unknown variant index instead of mis-decoding it — there is no silent
misinterpretation. No current wire path emits it (the host forwards no id to
it and the UI's shortcut tables do not produce it), so `PROTOCOL_VERSION`
stays at 3; the next wire-affecting change takes the bump per the RFC
0005/0006 precedent.

## What stays open

- **Whether loading from that dialog differs from the existing media-opening
  command.** The spec fixes the dialog's title only: it says nothing about
  the file-type filter, whether the loaded playlist replaces or appends to the
  list, what happens to the selection or the current entry, or how dead
  entries are treated. Until a spec settles those, the app records the dialog
  request and implements nothing beyond it — in particular, confirmation is
  *not* mapped to `AddItems` here, because that would assert append semantics
  the measurement does not contain.
- The `WM_COMMAND` id (if any) behind LIST OPTS, and therefore the host's
  forwarding entry, waits for S06 Q10 (message exposure).
- Save-playlist (RFC 0005 §11.6's `40204` side) is untouched by this RFC.

## RFC acceptance

- The RFC exists in the house style (`docs/rfc/0010-load-playlist-command.md`).
- The variant is appended after `ToggleVisPlugin` with a doc comment naming
  S06 §4; no existing variant changes meaning or position.
- `PlaylistDialog::LoadPlaylist.opening_command()` returns the new command;
  the model test pins the mapping.
- `routing::app_owns` names the variant in an explicit arm (no new
  catch-all); the app handles it through the dialog-stub path with a test.
- `cargo fmt --all -- --check` and
  `cargo clippy --workspace --all-targets --all-features -- -D warnings` are
  clean.
- No test suite is run (the orchestrator runs the suite in the Hyper-V test
  VM).
- No window is opened.
- No git write is performed.
