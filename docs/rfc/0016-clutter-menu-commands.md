# RFC 0016: Clutterbar menu commands - `OpenOptionsMenu` and `OpenVisualizationMenu`

Status: **Accepted 2026-10-19.**
Scope: `rustamp-proto` (two `Command` variants), `rustamp-ui` (the machine,
two popup models), `rustamp-app` (pending-popup state, routing, presenter).
No change to the engine, the rings or the decoder protocol.

## 1. Why

Clutterbar letters O and V (S02 §2) are drawn, lit while held and show their
marquee hints, but released over the letter they emitted nothing. S02 §2 says
O "opens the Options popup menu" and V "opens the visualization menu"; S04 §1
settles both as `#32768` popups opened by a click at client (14,25) and
(14,56). Letters A, D and I already emit commands; O and V were the gap.

## 2. Evidence (all from `specs/`)

- S02 §2: behaviour, momentary lit state, hints `OPTIONS MENU` and
  `VISUALIZATION MENU` (§2.1).
- S04 §1: click points, window class, sizes (371 x 348, 399 x 151).
- S04 §4.1 / §4.2: Options rows, separators, hints, and the tick that moves
  between `Time elapsed` and `Time remaining`.
- S04 §5: visualization menu, four rows, one separator after the first.

## 3. Contract change

Two unit variants appended to `Command` (postcard indices 0x39 and 0x3a,
after `OpenPlaylist`):

| Variant | Emitted by | Owner |
|---|---|---|
| `OpenOptionsMenu` | release inside letter O | app |
| `OpenVisualizationMenu` | release inside letter V | app |

Both are app-owned in `routing::app_owns`. Appending keeps every existing
index; an older peer that does not know them rejects the frame like any
unknown variant.

## 4. Wiring

1. `MainWindow::release_command` emits the command only when the pointer is
   released inside the held letter; a release outside emits nothing and clears
   the lit state.
2. `App` stores a pending `ClutterMenu` (`Options` / `Visualization`); a second
   request replaces the first. `App::take_clutter_menu` hands it to the
   presenter.
3. `Windowed::drain_clutter_menu` runs from the tick next to
   `drain_main_menu` and shows the system popup (`show_popup_menu`) at the
   letter's click point, then routes the chosen entry through
   `App::activate_menu_choice`.
4. `rustamp_ui::menu::native_options_menu` / `native_visualization_menu` build
   the row trees. Checks resolve from live state.
5. New `MenuChoice::SetTimeRemaining(bool)`: the two time rows *set* the
   reading rather than toggle it, because S04 §4.2 shows the tick moving
   between the rows, never clearing. The app flips the display only when the
   reading differs.

## 5. Unmeasured choices (ours)

- Popup screen position: S04 §9 measured none; the popup's top-left anchors at
  the letter's click point.
- The `Skins` submenu reuses the main menu's provisional entries (S04 §9).
- `R` / `S` hints on `Repeat` / `Shuffle` are provisional (S04 §4.3). Their
  ticks follow live state (the capture shows them unchecked in the default
  state).
- Visualization row 1 has no hint (S04 §5, icon pixels unidentified).
- No plug-in list is appended below `Select plug-in...` (open in S04 §5).
- `Start/Stop plug-in` and `Configure plug-in...` produce the existing
  "needs the plugin host" notes until the plugin host lands.
- Click-through of these popups was not measured (S04 §9); the actions are the
  S03 ids already bound to the same shortcuts.

## 6. Compatibility and tests

- Wire: `rustamp-ipc/tests/command_wire.rs` pins both encodings.
- UI: release inside / outside letter tests, popup rows, separators, hints and
  live checks, time-row setter semantics.
- App: commands queue the right popup once; the time rows never clear the
  reading.
- Not covered: the live popup display (needs the interactive session).
