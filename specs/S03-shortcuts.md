---
Spec ID: S-03
Title: Keyboard and mouse shortcut table, per window
Sources consulted (category only): dirty room: binary resource data (accelerator and menu tables); dirty room: binary behaviour capture; free: webamp (cross-check only)
Author agent: R-agent (dirty room, capture session 2026-10-14)
Auditor: Spec Auditor (dirty room)
Audit date: 2026-10-14
Winamp version validated against: 2.95 (winamp.exe SHA-256 bca7a9fe1626b487d1b47bb5210670cf572d1a4cce23062076641d583b268561, extracted from winamp295.exe SHA-256 ef1423999cbefd4f2a3a98acee2b194a4260268cb4517abda9f12963e866b7e5)
Status: audited
---

# S03 — Keyboard and mouse shortcuts, per window

Scope: which key or mouse gesture triggers which action, for the main window, the
playlist editor, the equalizer and the global set. Menu trees and labels are S04; the
actions themselves are described in the spec that owns them (S02 for the main window,
S06 for the playlist, S07 for the equalizer).

The key-to-command mapping is the accelerator table the 2.95 executable carries in its
own resources (four tables). The action text is taken from the program's own menu
resources, which label almost every command. The command ids are the public
`WM_COMMAND` ids that plugins and external tools send, so they belong in the spec.

Column **confirmation**:

- **command** — the command's effect was observed on the running 2.95 in this audit by
  sending the `WM_COMMAND` id and reading the resulting state back;
- **menu** — the action text comes from the program's own menus and the command is in
  its accelerator table; the command's effect was not separately exercised in this
  audit;
- **provisional** — the command has no menu label; the action is inferred and
  unconfirmed.

Provenance note: an earlier session of this spec recorded some rows as observed after
synthesized desktop clicks. This audit re-checked the click-driven rows with window
messages (and, for pressed and dragged states, with guarded real input inside the
sandbox window); the rows that could not be reproduced are withdrawn or marked
provisional below.

The four tables are the main window, the playlist editor, the equalizer and a global
set. Where a key appears in more than one table it maps to the same command, except for
a small number of conflicts noted below. How the tables combine when a non-main window
has focus (whether the global table is always active, and which table wins on a
conflict) is **provisional**.

---

## 1. Main window

| Key | Command | Action | Confirmation |
|---|---|---|---|
| Alt+3 | 40188 | View file info | menu |
| Ctrl+A | 40019 | Toggle always on top | command |
| B | 40048 | Next track | command |
| C | 40046 | Pause / unpause | command |
| Ctrl+D | 40165 | Toggle double size | command |
| Ctrl+E | 40186 | Toggle easy move | menu |
| Alt+E | 40040 | Toggle playlist editor | command |
| Alt+F | 40043 | Open the main menu | provisional |
| Alt+G | 40036 | Toggle equalizer | command |
| Alt+I | 40321 | Add current track as a bookmark | menu |
| Ctrl+Alt+I | 40320 | Edit bookmarks | menu |
| J | 40194 | Jump to file | menu |
| Ctrl+J | 40193 | Jump to time | menu |
| Ctrl+K | 40191 | Select visualization plug-in | command (opens the Preferences dialog) |
| Alt+K | 40221 | Configure visualization plug-in | menu |
| Ctrl+Shift+K | 40192 | Start/stop visualization plug-in | menu |
| L | 40029 | Open file(s) | menu |
| Ctrl+L | 40185 | Open location | menu |
| Shift+L | 40187 | Open directory | menu |
| Alt+M | 40334 | Minimize | provisional |
| Ctrl+Alt+N | 40305 | Start a new instance | provisional |
| Alt+O | 40317 | Visualization options | menu |
| Ctrl+P | 40012 | Preferences | menu |
| R | 40022 | Toggle repeat | command |
| S | 40023 | Toggle shuffle | command |
| Alt+S | 40219 | Skin browser | menu |
| Ctrl+T | 40242 | Toggle elapsed/remaining time | command |
| Alt+T | 40298 | Toggle minibrowser | menu |
| V | 40047 | Stop | command |
| Ctrl+V | 40157 | Stop after current track | menu |
| Alt+V | 40328 | Toggle video window | menu |
| Shift+V | 40147 | Stop with fadeout | menu |
| Num. | 40194 | Jump to file | menu |
| Down | 40059 | Volume down | provisional |
| F1 | 40041 | About box | menu |
| F3 | 40318 | Playlist search (unconfirmed) | provisional |
| Ctrl+F4 | 40224 | Close the focused window | provisional |
| F5 | 40291 | Refresh skin | provisional |
| Insert | 40187 | Open directory | menu |
| Left | 40061 | Rewind 5 seconds | menu |
| Num0 | 40029 | Open file(s) | menu |
| Ctrl+Num0 | 40185 | Open location | menu |
| Num1 | 40197 | 10 tracks back | menu |
| Num2 | 40059 | Volume down | provisional |
| Num3 | 40195 | 10 tracks forward | menu |
| Num4 | 40044 | Previous track | command |
| Num5 | 40045 | Play | command |
| Num6 | 40048 | Next track | command |
| Num7 | 40061 | Rewind 5 seconds | menu |
| Num8 | 40058 | Volume up | provisional |
| Num9 | 40060 | Forward 5 seconds | menu |
| Alt+Enter | 40337 | Toggle video fullscreen | provisional |
| Right | 40060 | Forward 5 seconds | menu |
| Ctrl+Tab | 40063 | Switch to the next window | provisional |
| Up | 40058 | Volume up | provisional |
| Ctrl+W | 40064 | Toggle windowshade | command |
| Alt+W | 40258 | Show the main window | menu |
| X | 40045 | Play | command |
| Ctrl+X | 40185 | Open location | menu |
| Shift+X | 40029 | Open file(s) | menu |
| Z | 40044 | Previous track | command |
| Ctrl+Z | 40154 | Start of list | menu |

---

## 2. Playlist editor

| Key | Command | Action | Confirmation |
|---|---|---|---|
| Ctrl+Shift+1 | 40209 | Sort list by title | menu |
| Ctrl+Shift+2 | 40210 | Sort list by filename | menu |
| Ctrl+Shift+3 | 40211 | Sort list by path and filename | menu |
| Alt+3 | 40208 | File info | menu |
| Ctrl+A | 40205 | Select all | menu |
| Ctrl+Alt+A | 40019 | Toggle always on top | command |
| B | 40048 | Next track | command |
| C | 40046 | Pause / unpause | command |
| Ctrl+D | 40165 | Toggle double size | command |
| Ctrl+E | 40255 | Edit the playlist entry | menu |
| Alt+E | 40040 | Toggle playlist editor | command |
| Ctrl+Alt+E | 40293 | Read extended info on selection | menu |
| Alt+F | 40043 | Open the main menu | provisional |
| Alt+G | 40036 | Toggle equalizer | command |
| Ctrl+Alt+G | 40292 | Generate HTML playlist | menu |
| Ctrl+I | 40171 | Invert selection | menu |
| Alt+I | 40319 | Bookmark the selected item(s) | menu |
| J | 40194 | Jump to file | menu |
| Ctrl+J | 40193 | Jump to time | menu |
| Ctrl+K | 40191 | Select visualization plug-in | command |
| Alt+K | 40221 | Configure visualization plug-in | menu |
| Ctrl+Shift+K | 40192 | Start/stop visualization plug-in | menu |
| L | 1032 | Add file(s) | menu |
| Ctrl+L | 1039 | Add location | menu |
| Shift+L | 1036 | Add directory | menu |
| Alt+M | 40334 | Minimize | provisional |
| Ctrl+N | 40214 | Clear the playlist | menu |
| Ctrl+Alt+N | 40305 | Start a new instance | provisional |
| Ctrl+O | 40202 | Open playlist | menu |
| Alt+O | 40317 | Visualization options | menu |
| Ctrl+P | 40012 | Preferences | menu |
| R | 40022 | Toggle repeat | command |
| Ctrl+R | 40213 | Reverse the list | menu |
| Ctrl+Shift+R | 40212 | Randomize the list | menu |
| S | 40023 | Toggle shuffle | command |
| Ctrl+S | 40204 | Save playlist | menu |
| Alt+S | 40219 | Skin browser | menu |
| Ctrl+T | 40242 | Toggle elapsed/remaining time | command |
| Alt+T | 40298 | Toggle minibrowser | menu |
| V | 40047 | Stop | command |
| Ctrl+V | 40157 | Stop after current track | menu |
| Alt+V | 40328 | Toggle video window | menu |
| Shift+V | 40147 | Stop with fadeout | menu |
| Ctrl+Num+ | 40335 | Increase the playlist font size | provisional |
| Ctrl+Num- | 40336 | Decrease the playlist font size | provisional |
| Num. | 40194 | Jump to file | menu |
| Delete | 1034 | Remove the selected item(s) | menu |
| Ctrl+Delete | 1035 | Crop to the selected item(s) | menu |
| Alt+Delete | 40222 | Remove all dead files | menu |
| Ctrl+Shift+Delete | 40214 | Clear the playlist | menu |
| Down | 40290 | Move the selection down | provisional |
| Alt+Down | 40216 | Move the selected entry down | provisional |
| Up | 40289 | Move the selection up | provisional |
| Alt+Up | 40215 | Move the selected entry up | provisional |
| Home | 40264 | Jump to the first entry | provisional |
| End | 40265 | Jump to the last entry | provisional |
| PgUp | 40256 | Scroll up one page | provisional |
| PgDn | 40257 | Scroll down one page | provisional |
| Enter | 40184 | Play the selected item(s) | menu |
| Insert | 1036 | Add directory | menu |
| Ctrl+Insert | 1032 | Add file(s) | menu |
| Shift+Insert | 1039 | Add location | menu |
| F1 | 40041 | About box | menu |
| F3 | 40318 | Playlist search (unconfirmed) | provisional |
| Ctrl+F4 | 40224 | Close the playlist editor | provisional |
| F5 | 40291 | Refresh skin | provisional |
| Left | 40061 | Rewind 5 seconds | menu |
| Right | 40060 | Forward 5 seconds | menu |
| Num0 | 1032 | Add file(s) | menu |
| Ctrl+Num0 | 1039 | Add location | menu |
| Num1 | 40197 | 10 tracks back | menu |
| Num2 | 40059 | Volume down | provisional |
| Num3 | 40195 | 10 tracks forward | menu |
| Num4 | 40044 | Previous track | command |
| Num5 | 40045 | Play | command |
| Num6 | 40048 | Next track | command |
| Num7 | 40061 | Rewind 5 seconds | menu |
| Num8 | 40058 | Volume up | provisional |
| Num9 | 40060 | Forward 5 seconds | menu |
| Alt+Enter | 40337 | Toggle video fullscreen | provisional |
| Ctrl+Tab | 40063 | Switch to the next window | provisional |
| Ctrl+W | 40064 | Toggle windowshade | command |
| Alt+W | 40258 | Show the main window | menu |
| X | 40045 | Play | command |
| Ctrl+X | 40185 | Open location | menu |
| Shift+X | 40029 | Open file(s) | menu |
| Z | 40044 | Previous track | command |
| Ctrl+Z | 40154 | Start of list | menu |

Note: the playlist add/remove commands (`1032` add file, `1034` remove, `1035` crop,
`1036` add directory, `1039` add location) are also bound to `.` and `-` with the
Ctrl/Alt combinations listed above; they are in the same table.

---

## 3. Equalizer

| Key | Command | Action | Confirmation |
|---|---|---|---|
| 1…9, 0 | 40267…40275, 40276 | Raise one band by one step (1 = first band, 0 = last band) | command |
| Grave (backtick) | 40277 | Raise the preamp by one step | provisional |
| Q W E R T Y U I O P | 40279…40288 | Lower one band by one step (Q = first band … P = last band) | command |
| Tab | 40278 | Lower the preamp by one step | provisional |
| A | 40245 | Toggle EQ autoloading | menu |
| N | 40244 | Toggle EQ enabled | menu |
| S | 40246 | Open the EQ presets menu | provisional |
| Ctrl+S | 40172 | Load an EQ preset | menu |
| Left | 40313 | Pan left | provisional |
| Right | 40314 | Pan right | provisional |
| Ctrl+A | 40019 | Toggle always on top | command |
| B | 40048 | Next track | command |
| C | 40046 | Pause / unpause | command |
| Ctrl+D | 40165 | Toggle double size | command |
| Ctrl+E | 40186 | Toggle easy move | menu |
| Alt+E | 40040 | Toggle playlist editor | command |
| Alt+F | 40043 | Open the main menu | provisional |
| Alt+G | 40036 | Toggle equalizer | command |
| J | 40194 | Jump to file | menu |
| Ctrl+J | 40193 | Jump to time | menu |
| Ctrl+K | 40191 | Select visualization plug-in | command |
| Alt+K | 40221 | Configure visualization plug-in | menu |
| Ctrl+Shift+K | 40192 | Start/stop visualization plug-in | menu |
| Alt+M | 40334 | Minimize | provisional |
| Ctrl+Alt+N | 40305 | Start a new instance | provisional |
| Ctrl+P | 40012 | Preferences | menu |
| Alt+S | 40219 | Skin browser | menu |
| Ctrl+T | 40242 | Toggle elapsed/remaining time | command |
| Alt+T | 40298 | Toggle minibrowser | menu |
| V | 40047 | Stop | command |
| Ctrl+V | 40157 | Stop after current track | menu |
| Alt+V | 40328 | Toggle video window | menu |
| Shift+V | 40147 | Stop with fadeout | menu |
| Num. | 40194 | Jump to file | menu |
| Down | 40059 | Volume down | provisional |
| Up | 40058 | Volume up | provisional |
| F1 | 40041 | About box | menu |
| F3 | 40318 | Playlist search (unconfirmed) | provisional |
| Ctrl+F4 | 40224 | Close the equalizer | provisional |
| F5 | 40291 | Refresh skin | provisional |
| Alt+Enter | 40337 | Toggle video fullscreen | provisional |
| Ctrl+Tab | 40063 | Switch to the next window | provisional |
| Ctrl+W | 40064 | Toggle windowshade | command |
| Alt+W | 40258 | Show the main window | menu |
| X | 40045 | Play | command |
| Ctrl+X | 40185 | Open location | menu |
| Shift+X | 40029 | Open file(s) | menu |
| Z | 40044 | Previous track | command |
| Ctrl+Z | 40154 | Start of list | menu |

Verification of the band commands: sending 40267…40276 in order moved the slider thumbs
of, in order, the ten bands from 60 Hz to 16 kHz (one band per command), and sending
40279…40288 moved the same ten bands in the opposite direction; a band raised by one
command is lowered by the matching one. The direction of the band steps was verified by
measuring the thumb position (raise moves the thumb toward the top of the track). The
direction of the two preamp commands (40277, 40278) was **not** resolved.

Note: the equalizer window has no Shift/Ctrl variant for its transport keys except where
listed; the band and preamp keys are plain keys.

---

## 4. Global

The program also carries a global accelerator table. Its entries overlap the per-window
tables for the window and playback commands; the remaining entries address the
minibrowser. Whether the global table is active while a per-window table also applies is
provisional.

| Key | Command | Action | Confirmation |
|---|---|---|---|
| Ctrl+A | 40019 | Toggle always on top | command |
| Ctrl+D | 40165 | Toggle double size | command |
| Ctrl+E | 40186 | Toggle easy move | menu |
| Alt+E | 40040 | Toggle playlist editor | command |
| Alt+F | 40043 | Open the main menu | provisional |
| Alt+G | 40036 | Toggle equalizer | command |
| Ctrl+K | 40191 | Select visualization plug-in | command |
| Alt+K | 40221 | Configure visualization plug-in | menu |
| Ctrl+Shift+K | 40192 | Start/stop visualization plug-in | menu |
| Ctrl+P | 40012 | Preferences | menu |
| Alt+S | 40219 | Skin browser | menu |
| Ctrl+T | 40242 | Toggle elapsed/remaining time | command |
| Alt+T | 40298 | Toggle minibrowser | menu |
| Alt+V | 40328 | Toggle video window | menu |
| Num. | 40194 | Jump to file | menu |
| F1 | 40041 | About box | menu |
| F3 | 40318 | Playlist search (unconfirmed) | provisional |
| Ctrl+F4 | 40224 | Close the focused window | provisional |
| F5 | 40291 | Refresh skin | provisional |
| Alt+Enter | 40337 | Toggle video fullscreen | provisional |
| Ctrl+Tab | 40063 | Switch to the next window | provisional |
| Ctrl+W | 40064 | Toggle windowshade | command |
| Alt+W | 40258 | Show the main window | menu |
| Ctrl+L | 40302 | Minibrowser command (unconfirmed) | provisional |
| Ctrl+O | 40303 | Minibrowser command (unconfirmed) | provisional |
| Ctrl+R | 40301 | Minibrowser command (unconfirmed) | provisional |
| Ctrl+Alt+R | 40309 | Minibrowser command (unconfirmed) | provisional |
| Alt+Left | 40300 | Minibrowser command (unconfirmed) | provisional |
| Alt+Right | 40299 | Minibrowser command (unconfirmed) | provisional |

Note: `Ctrl+L` means "open location" in the main table but maps to a minibrowser command
in the global table — one of the conflicts mentioned above.

---

## 5. Mouse gestures

Row **source**:

- **verified** — the gesture was exercised on the running 2.95 in this audit (posted
  client-relative mouse messages, or real input inside the sandbox window) and its
  effect was read back;
- **menu** — the gesture is described by the program's own context-menu resources (the
  resource labels the click and modifier-click variants); the gesture itself was not
  exercised in this audit;
- **provisional** — inferred, not exercised and not labelled by the resources.

| Gesture | Action | Source |
|---|---|---|
| Click a clutterbar letter | see S02 §2 | verified |
| Click the time display | toggle elapsed/remaining time | verified |
| Drag the position bar | seek (position changes) | verified |
| Drag the volume slider | volume (the drawn slider moves) | verified |
| Drag the balance slider | balance (the drawn slider moves) | verified |
| Drag the title bar | move the window | verified |
| Double-click the title bar | toggle windowshade | verified (see note) |
| Double-click a playlist entry | play that entry | verified |
| Right-click the main window | main-menu popup | verified |
| Right-click a playlist entry | playlist entry context menu | verified |
| Right-click the long title | title context menu (file info, jump to file, jump to time, autoscroll songname) | verified |
| Click the previous button | previous track | menu |
| Shift+click the previous button | rewind 5 seconds | menu |
| Ctrl+click the previous button | start of list | menu |
| Click the play button | play / restart | menu |
| Shift+click the play button | open file(s) | menu |
| Ctrl+click the play button | open location | menu |
| Click the pause button | pause / unpause | menu |
| Click the stop button | stop | menu |
| Shift+click the stop button | stop with fadeout | menu |
| Ctrl+click the stop button | stop after current track | menu |
| Click the next button | next track | menu |
| Shift+click the next button | forward 5 seconds | menu |
| Ctrl+click the next button | end of list | menu |
| Click the eject button | open file(s) | menu |
| Shift+click the eject button | open directory | menu |
| Click the shuffle button | toggle shuffle | menu |
| Click the repeat button | toggle repeat | menu |
| Click the EQ button | toggle the equalizer window | menu |
| Click the playlist button | toggle the playlist editor | menu |
| Click a window button (menu/minimize/shade/close) | main menu / minimize / windowshade / close | provisional |
| Right-click the time display | elapsed/remaining menu | menu |
| Right-click the seek bar | seek context menu | menu |
| Right-click the EQ or playlist button | toggle that window | menu |
| Right-click the EQ window | EQ context menu (enabled, autoloading) | menu |
| Drag in the playlist | reorder / move entries | provisional |

Note (double-click on the title bar): a double-click enters windowshade from the normal
title bar at any tested height. From the **shaded** title bar it leaves windowshade only
outside a middle band: at 275 x 14, a double-click at y = 0, 2, 4 or y = 10, 12 toggled
back, while a double-click at y = 6 or 8 did not. The shade button (window button 3) and
the toggle command do leave windowshade at every height.

**Withdrawn:** an earlier row claimed that clicking the bitrate/sample-rate area cycles a
display mode. It was not reproducible on 2.95 (see S02 §5).

The right-click context menus are named in the program's menu resources; their contents
and order belong to S04.

---

## 6. Open questions

1. **How the four tables combine.** Whether the global table is always active, and which
   table wins where two tables bind the same key (e.g. `Ctrl+L`), is unconfirmed.
2. **The unlabelled commands.** `40318` (F3), `40224` (Ctrl+F4), `40291` (F5), `40334`
   (Alt+M), `40305` (Ctrl+Alt+N), `40337` (Alt+Enter), `40063` (Ctrl+Tab), `40064`
   (Ctrl+W), the volume keys, the playlist scroll/move keys, the font-size keys, the
   preamp-step keys and the minibrowser keys have no menu label; their actions are
   inferred. (`40064` was exercised as a command and does toggle windowshade.)
3. **Modifier-click gestures.** The click, Shift+click and Ctrl+click variants on the
   transport, shuffle, repeat, EQ and playlist buttons are documented by the program's
   own resources but were not exercised in this audit.
4. **Playlist drag and reorder.** Dragging entries in the playlist, and the drag-reorder
   behaviour, were not exercised.
5. **Global hotkeys.** Whether any of these work while another application has focus (a
   real system-wide hotkey) was not tested; the global table here means "active
   regardless of which Winamp window has focus", not necessarily "system-wide".
6. **`Alt+3` context.** The action text differs between the main window (view file info)
   and the playlist (file info); whether they are the same dialog is unconfirmed.
7. **Preamp step direction.** The two preamp commands (40277, 40278) moved the preamp
   slider but their direction was not resolved (see §3).
