# RFC 0005: The missing app-level `Command`s (M1/M2)

Status: **Accepted in substance** on 2026-10-15 by the project owner (all
decision-relevant questions answered; the four dirty-room measurements and S15
may adjust details without re-approval). Was: Proposed.
Owner: Lane R (RFC planning). Written by: task card
[`docs/tasks/R5-rfc-app-commands.md`](../tasks/R5-rfc-app-commands.md).
Requires: RFC 0001 (core contracts), RFC 0004 (plugin-licensing split and the
`rustamp-proto` wire-type crate).
Specs used: S02 (audited), S03 (audited). S04 (menus) and S07 (equalizer) are
not started; where they would own a decision, this RFC says so instead of
guessing.
Deliberately out of scope: S04 menu trees, S05 docking, S14 title formatting
(named only as a dependency), any implementation, and the acceptance of this
RFC (that is the project owner's decision).

## 1. Why

Task D3 integrated the S03 shortcut tables into `rustamp-ui`
(`crates/rustamp-ui/src/shortcuts.rs`). Of the 55 distinct `ShortcutAction`s,
18 already map onto existing `Command`s, one is handled inside the UI machine
(`ToggleTimeRemaining`), and **36 come back as `ShortcutOutcome::Unhandled(action)`**:
the 30 distinct command ids of the S03 §1 main-window table that have no
`Command` yet (pinned by the test
`actions_without_a_command_are_unhandled_with_their_id`), plus the six
unconfirmed minibrowser commands that only the S03 §4 global table binds.
S03 §5 adds one more action with no `ShortcutAction` variant and no recorded
command id: *end of list* (Ctrl+click the next button). HANDOFF open point 18
calls for this RFC.

The `Command` enum (`crates/rustamp-proto/src/command.rs`, re-exported as
`rustamp_core::command::Command`) is the contract between UI, app, engine and
plugin host. S03 records the command ids as "the public `WM_COMMAND` ids that
plugins and external tools send", so every action reachable by such an id needs
a variant there — otherwise the plugin host has nothing to forward. This RFC
proposes, for each of the ~30 actions, a variant with exact name, payload and
semantics, or a reasoned verdict that no command is needed (UI-local,
app-local) or not yet possible (deferred), and states for each what it costs
the wire protocol and the IPC version.

Three problems are called out by the handoff and get their own sections:
`LoadSkin` needs a path (§6), `OpenPrefs` cannot carry a page (§7), and the
volume-key step is a placeholder (§8).

## 2. Facts

| # | Fact | Source |
|---|---|---|
| F1 | `Command` lives in `rustamp-proto` (permissive) and is re-exported by `rustamp-core` at `rustamp_core::command::Command`; `rustamp-ipc`/`rustamp-plughost` depend on proto, never on a GPL crate | RFC 0004 §3 |
| F2 | The current `Command` variants are exactly: `Play`, `Pause`, `TogglePause`, `Stop`, `StopWithFadeout`, `Next`, `Prev`, `Seek { to_ms }`, `SeekRelative { delta_ms }`, `SetVolume(u8)`, `SetBalance(i8)`, `ToggleShuffle`, `ToggleRepeat`, `SetShuffle(bool)`, `SetRepeat(bool)`, `AddItems { items, at, play }`, `Remove(Vec<EntryId>)`, `Clear`, `MoveEntries { ids, to }`, `Select(SelectOp)`, `PlayIndex(u32)`, `PlayEntry(EntryId)`, `Sort(SortKey)`, `Randomize`, `Reverse`, `SetBand { band, gain_db }`, `SetPreamp(f32)`, `SetEqEnabled(bool)`, `SetEqAuto(bool)`, `ToggleWindow(WindowId)`, `ToggleDoubleSize`, `ToggleWindowShade(WindowId)`, `ToggleAlwaysOnTop`, `LoadSkin(PathBuf)`, `Quit`, `OpenPrefs` | `crates/rustamp-proto/src/command.rs` |
| F3 | Units: ms are `u64` (`i64` for relative seeks); volume `u8` `0..=255`; balance `i8` `-127..=127`; indices `u32`; no `usize` in wire types | RFC 0001 |
| F4 | `WindowId` is `Main`, `Equalizer`, `Playlist`, `Vis`, `Gen(u32)`; there is no minibrowser or video window | `rustamp-proto/src/state.rs` |
| F5 | `PlayerState` carries transport, position, duration, volume, balance, shuffle, repeat, current index/id, playlist length, stream info and `eq`; it has no stop-after-current flag, no vis-plugin field, no easy-move flag and no bookmark list | `rustamp-proto/src/state.rs` |
| F6 | `Event` already has `SkinChanged(Option<PathBuf>)` (`None` = built-in default), `WindowVisibilityChanged { window, visible }`, `PlaylistEnded` and `Quit` | `rustamp-proto/src/event.rs` |
| F7 | The dispatch maps 18 actions to commands, returns `Handled` for `ToggleTimeRemaining` (40242) and `Unhandled(action)` for the rest; the 30 main-table unhandled ids are pinned by a test | `crates/rustamp-ui/src/shortcuts.rs` |
| F8 | `VOLUME_STEP: u8 = 10` in `crates/rustamp-ui/src/shortcuts.rs` is a placeholder ("provisional: step size not in S02/S03"); `SEEK_STEP_MS: i64 = 5000` by contrast is an S03 fact ("Rewind 5 seconds") | `crates/rustamp-ui/src/shortcuts.rs`, S03 §1 |
| F9 | The wire is postcard inside a `u32`-LE length prefix, `MAX_FRAME_LEN` = 16 MiB; `PROTOCOL_VERSION = 1`; peers compare it in `Hello`; the comparison is not implemented yet (both processes are stubs) | `crates/rustamp-ipc/src/{message,framing,transport}.rs` |
| F10 | postcard encodes enums by variant index (declaration order) and structs (and struct variants) positionally | postcard, via `framing.rs` |
| F11 | `Message` carries `CommandFromPlugin(Command)` host→main; there is no main→host command message | `crates/rustamp-ipc/src/message.rs` |
| F12 | RFC 0003 bumped `PROTOCOL_VERSION` 0→1 when adding `Message` variants, because "a peer that only speaks 0 would mis-decode a ring message; the bump makes the mismatch a clean `Hello` rejection instead of a decode error" | RFC 0003 §9 |
| F13 | S03 §1 marks these rows provisional: Alt+F (40043), Alt+M (40334), Ctrl+Alt+N (40305), the volume keys, F3 (40318), Ctrl+F4 (40224), F5 (40291), Alt+Enter (40337), Ctrl+Tab (40063). The minibrowser commands (40299–40303, 40309) are unconfirmed | S03 §1, §4, §6.2 |
| F14 | S03 §5 (mouse) binds start/end of list to Ctrl+click on the previous/next button, with no command id; the window-button clicks map to main menu / minimize / windowshade / close | S03 §5 |
| F15 | The playlist window (S03 §2) and equalizer window (S03 §3) do not exist; `ShortcutWindow` is `#[non_exhaustive]` with only `Main`, so their tables are not filled | `crates/rustamp-ui/src/shortcuts.rs` |

## 3. The rule: which actions get a `Command` variant

An S03 action gets a `Command` variant iff **its command id is public**
(recorded in S03, so plugins and external tools can send it) **and its effect
is something the player — engine or app — must perform**. Concretely, in
order:

1. **Engine state** (transport, playlist, EQ, volume, skin): variant, the
   engine applies it.
2. **App state** (dialogs, window management, the app's own stores): variant,
   the app applies it. The precedent is already in the enum — `OpenPrefs`,
   `Quit`, `LoadSkin`, `ToggleWindow`, `ToggleDoubleSize`,
   `ToggleAlwaysOnTop` and `ToggleWindowShade` are app-level intents, not
   engine mutations.
3. **UI-local**: the effect is confined to what the headless UI machine draws
   (no engine state, no app state, no persistence) → `ShortcutOutcome::Handled`,
   no variant (§9).
4. **App-local**: the effect is a process/OS action outside the player (spawn
   an instance) → no variant, the app/platform layer consumes the
   `Unhandled` action (§10).
5. **Deferred**: the target window, menu or plugin bridge does not exist yet
   → no variant, or a variant whose *handling* is deferred (§11).

Two corollaries:

- **The host-forward rule.** The plugin host must forward a variant iff S03
  records a public id for its action (the host maps id → `Command` in its
  `WM_COMMAND` table). A variant with no public id (`PlayLast`) is
  player-internal: the host never sees it. An action with a public id but no
  variant (`NewInstance`, everything deferred) cannot be forwarded; the host
  logs and drops it until the deferral lifts.
- **The append rule.** New variants are appended at the **end** of `Command`
  (after `OpenPrefs`), never inserted into the logical sections, so existing
  variant indices — and therefore existing postcard bytes — are unchanged
  (F10).

The enum's doc line ("every user or plugin intent that mutates the player")
is stretched by the dialog/window variants; the implementation lane should
reword it to "…that mutates the player or its windows and dialogs" (a doc
change in `rustamp-proto`, part of that lane, not of this RFC).

## 4. The master table

Every action of HANDOFF open point 18 appears exactly once, plus the six
global-table minibrowser commands (also `Unhandled` today) and the mouse-only
end-of-list. For context: the other 18 `ShortcutAction`s already map onto
existing commands and `ToggleTimeRemaining` stays UI-local (§9). Keys are the
main-window bindings; several actions have several.

| Action (S03 wording) | id | Verdict | Handled by | Plugin host |
|---|---|---|---|---|
| View file info | 40188 | new `OpenFileInfo` | app (dialog) | forwards 40188 |
| Toggle easy move | 40186 | new `ToggleEasyMove` | app/platform | forwards 40186 |
| Open the main menu | 40043 | new `OpenMainMenu` (handling deferred to S04) | app | forwards 40043 |
| Add current track as a bookmark | 40321 | new `AddBookmark` | app (bookmark store) | forwards 40321 |
| Edit bookmarks | 40320 | new `OpenBookmarkEditor` | app (dialog) | forwards 40320 |
| Jump to file | 40194 | new `OpenJump { target: JumpTarget::File }` | app (dialog → `PlayEntry`) | forwards 40194 |
| Jump to time | 40193 | new `OpenJump { target: JumpTarget::Time }` | app (dialog → `Seek`) | forwards 40193 |
| Select visualization plug-in | 40191 | new `OpenPrefsPage(PrefsPage::VisPlugin)` | app (prefs at page) | forwards 40191 |
| Configure visualization plug-in | 40221 | new `ConfigureVisPlugin` (handling deferred to M3) | app → host bridge | forwards 40221; player→host direction |
| Start/stop visualization plug-in | 40192 | new `ToggleVisPlugin` (handling deferred to M3) | app → host bridge | forwards 40192; player→host direction |
| Open file(s) | 40029 | new `OpenMedia { kind: Files, play: true }` | app (dialog → `AddItems`) | forwards 40029 |
| Open location | 40185 | new `OpenMedia { kind: Location, play: true }` | app (dialog → `AddItems`) | forwards 40185 |
| Open directory | 40187 | new `OpenMedia { kind: Directory, play: true }` | app (dialog → `AddItems`) | forwards 40187 |
| Minimize | 40334 | new `Minimize` | app/platform | forwards 40334 |
| Start a new instance | 40305 | **app-local, no variant** (§10) | platform (lane F / S13) | never sees it |
| Visualization options | 40317 | new `OpenVisOptions` | app (dialog/popup, S04) | forwards 40317 |
| Skin browser | 40219 | new `OpenSkinBrowser` | app (dialog → `LoadSkin`) | forwards 40219 |
| Toggle minibrowser | 40298 | **deferred** (§11.1) | — | — |
| Stop after current track | 40157 | new `ToggleStopAfterCurrent` | engine (transport mode) | forwards 40157 |
| Toggle video window | 40328 | new `WindowId::Video` + `ToggleWindow(WindowId::Video)` (decided §17) | app | forwards 40328 |
| About box | 40041 | new `OpenAbout` | app (dialog) | forwards 40041 |
| Playlist search (unconfirmed) | 40318 | **deferred** (§11.3) | — | — |
| Close the focused window | 40224 | new `CloseFocusedWindow` | app (window mgmt) | forwards 40224 |
| Refresh skin | 40291 | new `RefreshSkin` | app (skin pipeline) | forwards 40291 |
| 10 tracks back | 40197 | new `SkipBy { delta: -10 }` | engine | forwards 40197 |
| 10 tracks forward | 40195 | new `SkipBy { delta: 10 }` | engine | forwards 40195 |
| Toggle video fullscreen | 40337 | **deferred** (§11.2) | — | — |
| Switch to the next window | 40063 | new `FocusNextWindow` | app (window mgmt) | forwards 40063 |
| Show the main window | 40258 | new `ShowMainWindow` | app/platform | forwards 40258 |
| Start of list | 40154 | existing `PlayIndex(0)` (O1) | engine | forwards 40154 |
| Minibrowser command (unconfirmed) | 40299 | **deferred** (§11.1) | — | — |
| Minibrowser command (unconfirmed) | 40300 | **deferred** (§11.1) | — | — |
| Minibrowser command (unconfirmed) | 40301 | **deferred** (§11.1) | — | — |
| Minibrowser command (unconfirmed) | 40302 | **deferred** (§11.1; conflicts with 40185, S03 §6.1) | — | — |
| Minibrowser command (unconfirmed) | 40303 | **deferred** (§11.1) | — | — |
| Minibrowser command (unconfirmed) | 40309 | **deferred** (§11.1) | — | — |
| End of list (Ctrl+click next, S03 §5; no id recorded) | — | new `PlayLast` | engine | never (no public id, O15) |

Tally: 21 new variants, 2 actions reusing existing commands, 1 app-local,
1 UI-local (not in this table, §9), 9 deferred (7 minibrowser, video
fullscreen, playlist search), 1 mouse-only action covered by `PlayLast`.

## 5. The new variants

### 5.1 Engine (player-state mutations)

**`Command::SkipBy { delta: i32 }`** — for 40195 (Num3, +10) and 40197 (Num1,
−10).

- Semantics: move the current entry by `delta` positions in the same order
  `Next`/`Prev` follow, clamped to the list; `delta = 0` or an empty playlist
  is a no-op. Follows S02 §1: changing the entry while stopped does not start
  playback.
- Payload weighing: one variant with a signed step vs two unit variants
  (`TenTracksBack`/`TenTracksForward`). The step is fixed by the action (always
  10), but putting it on the wire keeps the engine generic (one code path) and
  keeps the constant in the two tables that already carry the ids (the UI's
  `MAIN_TABLE`, the host's id table) instead of a third place (engine code).
  The two-unit-variant alternative duplicates the engine path and scatters the
  step; it is rejected.
- UI prediction: no transport change (S02 §1); the machine tracks no current
  entry, so nothing else to predict.
- Host: 40195 → `SkipBy { delta: 10 }`, 40197 → `SkipBy { delta: -10 }`.
- Open: play order (shuffle) vs playlist order (O9, S15).

**`Command::PlayLast`** — for end of list (S03 §5 mouse; no id recorded).

- Semantics: play the last entry in play order; no-op on an empty playlist.
  Mirrors `PlayIndex(0)` for the other end of the list.
- Payload: none — the engine resolves the last entry. `PlayIndex(len - 1)`
  would force the caller to know `playlist_len`, which the UI machine does not
  hold. `PlayIndex(u32::MAX)` with clamping was considered and rejected as
  obscure.
- Host: never — no public id is recorded (O15).

**`Command::ToggleStopAfterCurrent`** — for 40157 (Ctrl+V).

- Semantics: toggle the stop-after-current mode: when the current track ends,
  the transport stops instead of advancing. Interacts with
  `Event::PlaylistEnded` and with S02 open question 5 (end-of-track behaviour,
  unmeasured).
- Why a toggle: S03 records one action and S02 shows no on/off indicator for
  it; the enum's convention for such single actions is `Toggle*`
  (`ToggleShuffle`, `ToggleRepeat`). The set-form (`SetStopAfterCurrent(bool)`)
  stays available if measurement shows a checked menu item.
- `PlayerState`: no field today (F5). **Owner decision O8 (§17): add a
  stop-after-current field** — a positional postcard struct change riding the
  same v2 bump. The mode is still engine-internal in behaviour; the field makes
  it observable.

### 5.2 App — dialogs

**`Command::OpenMedia { kind: OpenMediaKind, play: bool }`** — for 40029 (L,
Num0, Shift+X), 40185 (Ctrl+L, Ctrl+Num0, Ctrl+X), 40187 (Shift+L, Insert).
`OpenMediaKind { Files, Location, Directory }` is a new enum in
`rustamp-proto`.

- Semantics: open the player's open-file / open-location / open-directory
  dialog; on confirmation the app adds the chosen media
  (`Command::AddItems { items, at: None, play }`). `Location` produces
  `MediaLocation::Url`.
- Payload weighing: one variant with a kind payload vs three unit variants.
  The three actions share the dialog shape and the follow-up command; the
  payload enum keeps the variant count down and matches the existing style
  (`Select(SelectOp)`, `Sort(SortKey)`).
- The `play` flag: the main-window actions play (`true`). The playlist-window
  add commands (1032/1036/1039, deferred in §11.6) would add without playing;
  S03 does not state the difference, so the flag is carried from the start —
  adding a field later is a wire change. The `false` values are provisional
  (O5).
- Host: 40029 → `{ kind: Files, play: true }`, 40185 → `{ kind: Location,
  play: true }`, 40187 → `{ kind: Directory, play: true }`.

**`Command::OpenJump { target: JumpTarget }`** — for 40194 (J, Num.) and
40193 (Ctrl+J). `JumpTarget { File, Time }` is a new enum in `rustamp-proto`.

- Semantics: open the jump dialog. `File` lists playlist entries (confirm →
  `PlayEntry`); `Time` takes a position (confirm → `Seek { to_ms }`). The
  dialog is app UI; the follow-up commands already exist.
- Payload weighing: same as `OpenMedia` — two sibling dialogs, one variant with
  a payload.
- Host: 40194 → `{ target: File }`, 40193 → `{ target: Time }`.

**`Command::OpenFileInfo`** — for 40188 (Alt+3).

- Semantics: open the file-info box for the current track (S02 §2 clutterbar
  I, verified: with no track loaded, no window appears). No payload: the target
  is always the current entry, which the app resolves from state. Contrast: the
  playlist-window Alt+3 (40208, on the *selection*) is deferred (§11.6) and
  will need a payload or selection resolution then.

**`Command::OpenPrefsPage(PrefsPage)`** — for 40191 (Ctrl+K). See §7.

**`Command::OpenAbout`** — for 40041 (F1). Open the about box. No payload.
Named for symmetry with `OpenPrefs`.

**`Command::OpenSkinBrowser`** — for 40219 (Alt+S). See §6.

**`Command::RefreshSkin`** — for 40291 (F5, provisional). See §6.

**`Command::OpenBookmarkEditor`** — for 40320 (Ctrl+Alt+I). Open the
bookmark-management dialog. No payload.

**`Command::AddBookmark`** — for 40321 (Alt+I). Add the current track to the
bookmark list. No payload (current entry). The bookmark store does not exist
yet (O12); the variant is proposed so the tables are complete, and the handling
lands with the bookmark feature (M2-era), in-memory first.

**`Command::OpenVisOptions`** — for 40317 (Alt+O). Open the
visualization-options UI. No payload. S03 annotates 40191 as opening the
Preferences dialog but says nothing for 40317, so this RFC treats it as its own
UI with content per S04 (O7).

### 5.3 App — window management

**`Command::ToggleEasyMove`** — for 40186 (Ctrl+E). Toggle easy-move (a larger
drag target for moving the player's windows). No payload. Symmetric with the
existing `ToggleAlwaysOnTop`: a window-chrome toggle that is already a
`Command`. The platform layer applies it; S02 records no sprite for it, so the
UI machine has nothing to predict.

**`Command::Minimize`** — for 40334 (Alt+M, provisional). Minimize the player's
main window. No payload. Open (O4): S03 binds the same id in all three window
tables; whether it minimizes the main window or the focused window is not
stated (contrast 40224, which says "focused").

**`Command::FocusNextWindow`** — for 40063 (Ctrl+Tab, provisional). Move
keyboard focus to the next player window (main → equalizer → playlist → …, in
`WindowId` order). No payload.

**`Command::ShowMainWindow`** — for 40258 (Alt+W). Show (un-minimize, raise)
and focus the main window. No payload; deliberately one-way —
`ToggleWindow(WindowId::Main)` would *hide* it. This is also the tray action
for lane F.

**`Command::CloseFocusedWindow`** — for 40224 (Ctrl+F4, provisional). Close
whichever player window has focus: the equalizer/playlist windows hide (they are
toggle windows → `ToggleWindow`); the main window quits (`Quit`). The
main-window case is an assumption (O3): S03 says "close the focused window"
without stating what closing the main window does.

**`Command::OpenMainMenu`** — for 40043 (Alt+F, provisional). Open the main
menu popup (the same menu the menu button and the right-click open, S02 §2,
S03 §5). No payload. Handling deferred to S04 (§11.4).

### 5.4 App → host bridge (handling deferred to M3)

**`Command::ConfigureVisPlugin`** — for 40221 (Alt+K). Ask the loaded
visualization plug-in to show its configuration. No payload (the current vis
plugin is implied).

**`Command::ToggleVisPlugin`** — for 40192 (Ctrl+Shift+K). Start/stop the
visualization plug-in (whether vis data flows).

- Both are applied by the app *through the plugin host* (the plugin runs in
  `rustamp-plughost32.exe`), so they need a player→host message that does not
  exist yet (F11). This RFC proposes `Message::CommandToHost(Command)`
  mirroring `CommandFromPlugin` (§12); the overlap with the `WinampIpc`
  placeholder is S10's to resolve (O13).
- The variants are proposed now so the UI tables and the host's id mapping are
  complete; the handling lands with M3. Until then the app logs and no-ops.
- Host forwarding: a plugin can send 40221/40192 (e.g. a hotkey plugin), so
  the host forwards them like any other id — but note the direction: these
  commands *originate* in the player and travel to the host.

### 5.5 Reused existing commands (no new variant)

- **StartOfList (40154, Ctrl+Z) → `Command::PlayIndex(0)`.** `PlayIndex` means
  "play the entry at an index"; the first entry is index 0 and the caller
  needs no state. Open (O1): if measurement shows start-of-list only *moves*
  the current entry (as `Prev`/`Next` do while stopped, S02 §1), the mapping
  changes to a move-current variant; this RFC adopts "play" as the default
  reading. Host: forwards 40154 → `PlayIndex(0)`.
- **ToggleVideoWindow (40328, Alt+V) → `Command::ToggleWindow(WindowId::Video)`.**
  `WindowId` gains a `Video` variant (owner decision O2, §17 — the 2.95 video
  window is not the visualization window). Additive enum addition with the same
  version story. Host: forwards 40328.

## 6. The `LoadSkin`-path problem

`Command::LoadSkin(PathBuf)` requires the caller to name a skin. The S03 skin
actions never carry a path at dispatch time: the skin browser (40219) is a
*browser* (the path comes from the user's pick, inside the app), and refresh
skin (40291) re-applies the *current* skin — the UI machine does not know the
current skin path (`MainWindowState` has no skin field). The division of labour
this RFC proposes:

- `LoadSkin(PathBuf)` — **unchanged**. Explicit loads only: the skin browser's
  result, the preferences skin page, a plugin, the CLI. Callers that have a
  path.
- `RefreshSkin` — **new, no payload** (40291). The app re-loads the skin it
  last reported through `Event::SkinChanged(Option<PathBuf>)`; `None` means the
  built-in default, so refresh re-applies the default. No new state: the app
  must already track the current skin to render it.
- `OpenSkinBrowser` — **new, no payload** (40219). The app shows the browser;
  the pick becomes `LoadSkin(path)` (the existing variant).

So the UI machine never constructs a `LoadSkin` — it has no path — and the
"needs a path" problem is solved by giving the two pathless intents their own
variants instead of stretching `LoadSkin`. Alternatives rejected:

- `LoadSkin(Option<PathBuf>)` with `None` = refresh: changes the shape of an
  existing variant (old peers mis-decode) and conflates two actions.
- Tracking the current skin path in `PlayerState`: a positional struct change
  on the wire (F10) plus a state field for a UI concern.

A side benefit: `RefreshSkin` carries no path, so it sidesteps the RFC 0001
known limitation (non-UTF-8 `PathBuf` over IPC) for the common case. Note that
F5 (40291) is a provisional row and the skin browser is an M2 feature (PLAN:
"Skin browser, preferences").

## 7. The `OpenPrefs`-page problem

`Command::OpenPrefs` is a unit variant; it cannot say which page to open. S03
§1 records that Ctrl+K (40191, "Select visualization plug-in") is a *command*
that "opens the Preferences dialog" — i.e. the preferences dialog at the
visualization-plug-in page, not the dialog in general. The proposal:

- Keep `OpenPrefs` **exactly as it is**. Ctrl+P (40012) opens the dialog at its
  remembered page.
- Add **`Command::OpenPrefsPage(PrefsPage)`**; 40191 →
  `OpenPrefsPage(PrefsPage::VisPlugin)`.
- `PrefsPage` is a **new enum in `rustamp-proto`**, provisional, with only
  `VisPlugin` defined now. The full page list is S04's (not started); growing
  the enum later follows the same append rule.

Rejected: changing `OpenPrefs` to `OpenPrefs { page: Option<PrefsPage> }` —
that changes an existing variant's wire shape (a unit variant gains a payload;
old peers mis-decode, F10) and touches every existing caller for a change only
one action needs. The host maps 40012 → `OpenPrefs` and 40191 →
`OpenPrefsPage(PrefsPage::VisPlugin)`.

Open (O6/O7): which page 40191 opens exactly, whether 40317 (Alt+O,
"Visualization options") is also a preferences page or its own dialog — S03
annotates 40191 as opening the Preferences dialog but says nothing for 40317,
so this RFC treats 40317 as its own UI (`OpenVisOptions`).

## 8. The volume-step placeholder

`VOLUME_STEP: u8 = 10` (core units, `0..=255`) in
`crates/rustamp-ui/src/shortcuts.rs` is a placeholder; neither S02 nor S03
states the step (F8), and HANDOFF open point 18 says the dirty room must
measure it. **This RFC does not invent a value**: the constant stays named,
provisional, in the UI crate, and the measurement is a separate dirty-room
task (O10).

The design around it is ratified: the step is applied by the UI machine
(`volume_command`), which converts the machine's percent volume to core units
(`core_volume`), adds or subtracts `VOLUME_STEP` with saturation, and emits the
absolute `SetVolume(u8)`. The wire carries absolute values only; the step never
crosses IPC and never enters `rustamp-proto`. Contrast `SEEK_STEP_MS = 5000`,
which S03 states ("Rewind 5 seconds") — a spec constant, not a placeholder.

The step covers Up/Down/Num8/Num2 (all provisional rows, F13); key repeat
delivers the action again (an implementation choice, not an S03 fact). No
balance keys exist in S03 §1 (pan is an equalizer-window action, deferred); when
S02/S07 answer the balance law (HANDOFF open point 7), the same pattern applies.

## 9. What stays UI-local

Ratified: **`ToggleTimeRemaining` (40242) stays `ShortcutOutcome::Handled`.**
The machine flips `MainWindowState::time_remaining`; S02 §4 verified that only
the minus-sign region changes. No variant, no IPC — the host cannot forward
40242 today; that is an accepted pre-S10 gap, to be revisited when S10 defines
the IPC surface for it.

The rule for future UI-local actions: the effect is confined to what the
headless UI machine draws — no engine state, no app state, no window
management, no persistence. The playlist-window scroll keys (40256/40257) and
font-size keys (40335/40336) are the same kind when those windows exist (§11.6).

Caveat: the real player persists the elapsed/remaining preference; RFC 0001
defers config file I/O. When config lands, this toggle needs a config write (or
becomes a `Command`) — O14.

## 10. What stays app-local (no variant)

**`NewInstance` (40305, Ctrl+Alt+N, provisional): no variant.** The effect is
a new process, entirely outside the player; S13 (command line) and the
single-instance design (lane F) own it, and no player state changes. The
app/platform layer consumes `Unhandled(NewInstance)` when it exists. The host
never sees it: there is no variant to forward, and a plugin asking for a new
instance is not a player intent. Rejected alternative: a `Command::NewInstance`
variant for id coverage — it would put platform policy into the wire contract
before S13 defines single-instance semantics. Revisit with S13/lane F (O16).

## 11. Deferred actions

### 11.1 Minibrowser (7 actions)

`ToggleMinibrowser` (40298) and the six unconfirmed commands (40299, 40300,
40301, 40302, 40303, 40309): no spec owns the minibrowser, `WindowId` has no
variant for it (F4), and the commands are unconfirmed (S03 §6.2). No variants.
When a minibrowser spec exists: either `WindowId::Minibrowser` +
`ToggleWindow`/`ToggleWindowShade`, or dedicated variants. The Ctrl+L conflict
(40302 in the global table vs 40185 in the main table) stays provisional
(S03 §6.1); the window table wins in the current `lookup`.

### 11.2 Video fullscreen

`ToggleVideoFullscreen` (40337, Alt+Enter, provisional): the video window is
unspecified (O2) and fullscreen is a mode of a window that does not exist. No
variant until the video window is specified.

### 11.3 Playlist search

`PlaylistSearch` (40318, F3, unconfirmed): the search UI belongs to the
playlist editor (S03 §2 binds F3 there too). Deferred until the playlist window
exists; future name `OpenPlaylistSearch` (app dialog; confirm →
selection/`PlayEntry`).

### 11.4 Menus

`OpenMainMenu` (40043): variant proposed now (§5.3), handling deferred to S04.

### 11.5 Plugin bridge

`ConfigureVisPlugin`/`ToggleVisPlugin`: variants proposed now (§5.4),
handling deferred to M3.

### 11.6 Playlist-window table (S03 §2) — unfillable until the window exists

| Action | id | Verdict |
|---|---|---|
| Sort by title / filename / path | 40209/40210/40211 | existing `Sort(SortKey::{Title, FileName, PathAndFileName})` |
| Select all / invert | 40205/40171 | existing `Select(SelectOp::{All, Invert})` |
| Clear | 40214 | existing `Clear` |
| Reverse / randomize | 40213/40212 | existing `Reverse` / `Randomize` |
| Remove selected | 1034 | existing `Remove(Vec<EntryId>)` (app resolves the selection) |
| Crop to selected | 1035 | **no command today** — remove-all-but-selected; expressible as `Remove(complement)` once the app can enumerate ids, or a future variant (playlist lane) |
| Move selected entry up/down | 40215/40216 | existing `MoveEntries { ids, to }` |
| Move selection up/down | 40289/40290 (provisional) | `Select(SelectOp::Set(..))` computed by the app, or UI-local to the playlist machine — decide with the window |
| Jump to first/last entry | 40264/40265 (provisional) | decide with StartOfList/EndOfList (O1) |
| Scroll page up/down | 40256/40257 (provisional) | UI-local to the playlist window (same rule as §9) |
| Play selected | 40184 | existing `PlayEntry(EntryId)` (app resolves the selection) |
| Add file/location/directory | 1032/1036/1039 | `OpenMedia { kind, play: false }` (play flag provisional, O5) |
| Open / save playlist | 40202/40204 | **no commands today** — `rustamp-playlist` has the I/O but there is no `LoadPlaylistFile`/`SavePlaylistFile` command (playlist-lane gap) |
| Edit entry / extended info / generate HTML / remove dead files / bookmark selected / file info on selection | 40255/40293/40292/40222/40319/40208 | app dialogs/actions with selection payloads; deferred; names decided with the window |
| Font size | 40335/40336 (provisional) | playlist-window view setting; likely UI-local to that window's machine (§9 rule) or a small command — decide with the window |
| all common rows | — | the main-window decisions of §4 |

### 11.7 Equalizer-window table (S03 §3) — unfillable until the window exists

| Action | id | Verdict |
|---|---|---|
| Band up/down one step | 40267–40276, 40279–40288 | existing `SetBand { band, gain_db }`; the app computes the absolute target from `PlayerState.eq.bands[band] ± step`; the step is S07's (not started → provisional constant, same treatment as `VOLUME_STEP`) |
| Preamp up/down | 40277/40278 (provisional; direction unresolved, S03 §6.7) | existing `SetPreamp(f32)` with the same pattern; direction open |
| Toggle EQ enabled / autoload | 40244/40245 | existing `SetEqEnabled(!current)` / `SetEqAuto(!current)` (no `Toggle*` variants exist for these; the app computes from `PlayerState.eq`) |
| Load preset / presets menu | 40172/40246 | **no preset command today** — a preset could be a sequence of `SetBand`/`SetPreamp`, but a named-preset command belongs to the EQ lane (gap) |
| Pan left/right | 40313/40314 (provisional) | existing `SetBalance` with a step; the balance law is an open S02/S07 request (HANDOFF open point 7) → step provisional |
| all common rows | — | the main-window decisions of §4 |

## 12. Wire and IPC impact

**What changes on the wire:** only `Command` grows — 21 new variants appended
after `OpenPrefs`, plus three new enums (`OpenMediaKind`, `JumpTarget`,
`PrefsPage`). No existing variant changes shape or index. One `Message`-level
addition is proposed for M3: `Message::CommandToHost(Command)`, mirroring
`CommandFromPlugin`, for the player→host direction of §5.4.

**Recommendation: bump `PROTOCOL_VERSION` 1 → 2 when the new variants land.**

- The command surface is part of the protocol: a v1-only peer cannot decode a
  `CommandFromPlugin` carrying a new variant (postcard decode error →
  `IpcError::Codec`). The bump turns a mixed-version pair into a clean `Hello`
  rejection — RFC 0003 §9's exact reasoning for the 0→1 bump (F12).
- The proposed `Message::CommandToHost` is a `Message`-level addition, which
  by the `message.rs` rule ("Bumped on any incompatible change to `Message`")
  and the RFC 0003 precedent forces the bump on its own.
- It is cheap now: M3 has not started, there are no released peers, and host
  and player ship together. After M3 it is expensive.
- If the owner splits the work (variants now, `CommandToHost` with M3), the
  bump should still land with the variants, because new→old sends become
  possible the moment the host can map a new id.
- Appending (F10) keeps old→new decoding intact: a v2 player decodes every v1
  command unchanged; the handshake is what prevents new→old sends. The version
  history in `message.rs` gains: `2`: adds the RFC 0005 app-level `Command`
  variants and, with M3, `Message::CommandToHost`.

**Framing: unchanged.** `u32`-LE length prefix + postcard, 16 MiB cap. The new
commands are a variant index plus at most a small enum and a bool — a few bytes
per frame, far under the cap.

Per-variant forwarding is in the master table (§4). The host's id→`Command`
table lives in `rustamp-plughost`; the UI's action→`Command` mapping lives in
`rustamp-ui`; the public ids are the shared fact that keeps the two in
agreement.

## 13. The permissive-crate invariant

- All new wire types go into `rustamp-proto` (the `Command` variants,
  `OpenMediaKind`, `JumpTarget`, `PrefsPage`), re-exported by `rustamp-core` at
  the existing paths; the proposed `Message::CommandToHost(Command)` goes into
  `rustamp-ipc`. Both crates are `MIT OR Apache-2.0` (RFC 0004); no GPL crate
  carries a wire type.
- The host's id table maps public S03 ids (interoperability facts) to proto
  commands — safe in the permissive crate.
- `ShortcutAction`, its command ids, `VOLUME_STEP` and `SEEK_STEP_MS` stay in
  `rustamp-ui` (GPL): they are UI facts, not wire facts.
- Drift risk: the UI table and the host table both encode S03's ids.
  Mitigation for the implementation lane: one test that walks the S03 id list
  once (the UI already exposes `ShortcutAction::command_id()`; the host table
  should be checked against the same list, e.g. a shared fixture or a generated
  table).
- RFC 0004 acceptance 4 (no GPL crate in the permissive set's dependency graph)
  must keep holding.

## 14. Backwards compatibility

- **In-process:** `rustamp_core::command::Command` keeps resolving (re-export).
  The variants are append-only, so pre-0005 encodings are byte-identical and
  the `cargo xtask golden` vectors for commands stay green. Exhaustive matches
  downstream (engine applier, UI machine) need new arms or a catch-all; the
  implementation lane should consider `#[non_exhaustive]` on `Command` so
  future additive changes stop breaking matches — a decision for that lane,
  noted here, not made here.
- **Wire:** `PROTOCOL_VERSION` 1 → 2; mixed pairs are rejected at `Hello` once
  the comparison lands with M3. v1→v2 decoding of old commands is unaffected
  (append-only); v2→v1 is prevented by the handshake.
- **Untouched:** `PlayerState`, `Event`, config, framing; `LoadSkin(PathBuf)`
  and `OpenPrefs` keep their exact shapes; no `Message` change before M3 except
  the proposed `CommandToHost`.
- **UI behaviour:** no S02-visible change; `VOLUME_STEP` stays a provisional
  placeholder.

## 15. Acceptance criteria for the implementation lane

1. `dispatch_shortcut` maps every main-table action to a `Command` (existing
   or new) or a documented verdict; `Unhandled` remains only for the deferred
   actions of §11 (7 minibrowser, video fullscreen, playlist search) and the
   app-local `NewInstance`, which the app consumes.
2. Every action of HANDOFF open point 18 appears exactly once in the
   implementation's mapping, with the variant names of §5 or the
   reused/deferred verdicts of §4.
3. The new variants are appended after `OpenPrefs`; a wire test shows every
   pre-0005 `Command` encodes to the same bytes as before (golden vectors
   green).
4. `PROTOCOL_VERSION = 2` with the version history updated; the `Hello`
   comparison (M3) rejects mixed pairs.
5. The host's id table covers every public id in §4 that has a variant; ids
   without variants are logged and dropped.
6. `VOLUME_STEP` is still a named, provisional constant; no measured value is
   invented; the dirty-room measurement is scheduled as its own task.
7. Each new variant has a dispatch test in `shortcuts.rs` in the existing
   style (action → expected `Command`), and the UI machine's prediction table
   gains entries for the commands with an S02-visible effect
   (`PlayIndex`/`PlayEntry`/`PlayLast` → `Playing`; `SkipBy` → no transport
   change, matching S02 §1).
8. No GPL crate enters the permissive set's dependency graph (RFC 0004
   acceptance 4).
9. `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets
   --all-features -- -D warnings` and `cargo test --workspace --all-features`
   green (plus the i686 checks for the permissive crates per HANDOFF §7).

## 16. Open questions

| ID | Question |
|---|---|
| O1 | Do start/end of list *play* or only move the current entry? (S02 §1 for prev/next suggests move-only; affects `PlayIndex(0)`/`PlayLast`.) |
| O2 | Is the 2.95 video window the same window as `WindowId::Vis`? (40328.) |
| O3 | What does "close the focused window" do to the main window — quit or hide? (40224.) |
| O4 | Does Alt+M minimize the main window or the focused window? (40334.) |
| O5 | Do the playlist-window add commands (1032/1036/1039) play the added media? (The `play: false` values are unmeasured.) |
| O6 | Which preferences page does 40191 open exactly; the full `PrefsPage` list waits for S04. |
| O7 | What is the 40317 visualization-options UI — a preferences page or its own dialog? |
| O8 | Stop-after-current: toggle or one-shot? Any UI indicator? Does `PlayerState` need a field (a wire change)? |
| O9 | Does `SkipBy` follow the play order (shuffle) or the playlist order? (S15.) |
| O10 | `VOLUME_STEP`: dirty-room measurement; the placeholder stays until then. |
| O11 | EQ band/preamp step (S07) and balance pan step (S02/S07): same provisional-constant treatment when those windows land. |
| O12 | Bookmarks: store location and persistence format — no spec owns bookmarks. |
| O13 | The player→host command message: new `Message::CommandToHost(Command)` vs routing through the `WinampIpc` placeholder (S10) — resolve the overlap with S10. |
| O14 | `ToggleTimeRemaining` persistence when config file I/O lands. |
| O15 | End-of-list has no recorded command id — confirm whether 2.95 has one so the host can forward it. |
| O16 | `NewInstance`: revisit a variant once S13/lane F define single-instance semantics. |
| O17 | Minibrowser: in scope at all? (Unconfirmed commands; no owning spec.) |
| O18 | Non-UTF-8 skin paths over IPC (RFC 0001 known limitation) — `RefreshSkin` avoids it for the common case. |

## 17. Owner decisions (2026-10-15)

The project owner decided all decision-relevant questions. Status is now:
**accepted in substance** — the four measurements below and S15 (O9) may adjust
details without re-approval.

| ID | Decision | Consequence for the implementation lane |
|---|---|---|
| O1 | Start/end of list **play** (default adopted) | `PlayIndex(0)` / `PlayLast` as proposed |
| O2 | The video window is **not** `WindowId::Vis` — add **`WindowId::Video`** | Additive enum addition with the same version story (§5.5); host maps 40328 to `ToggleWindow(WindowId::Video)`; fullscreen (deferred) targets it |
| O3 | Closing the main window **quits** | `CloseFocusedWindow` on main → `Quit` |
| O4 | Alt+M minimizes the **main** window | `Minimize` targets main |
| O5 | **Measure in 2.95** whether the playlist add commands play | Dirty-room task; `play: false` stands until measured |
| O6/O7 | **Verify in 2.95** which page 40191 opens and what 40317 opens | Dirty-room task; `OpenPrefsPage(VisPlugin)` + `OpenVisOptions` stand until verified |
| O8 | Stop-after-current is a toggle **with a `PlayerState` field** | Wire change riding the same v2 bump (§5.1 deliberately did not propose it; the owner overruled — add the field) |
| O9 | Deferred to **S15** | `SkipBy` order follows the shuffle-sequence decision |
| O10 | Dirty-room measurement stands | `VOLUME_STEP` stays placeholder |
| O11 | Provisional constants (default adopted) | As proposed |
| O12 | Bookmark handling lands with the feature (default adopted) | As proposed |
| O13 | **`Message::CommandToHost(Command)`** (default adopted) | As proposed; overlap with `WinampIpc` resolved with S10 |
| O14 | Persist the elapsed/remaining preference when config I/O lands | Noted for the config lane |
| O15 | **Measure** whether 2.95 has an end-of-list id | Dirty-room task; `PlayLast` stays id-less until measured |
| O16 | `NewInstance` stays app-local (default adopted) | As proposed; revisit with S13 |
| O17 | Minibrowser stays deferred (default adopted) | No spec task |
| O18 | Noted (default adopted) | As proposed |
