---
Spec ID: S-02
Title: Main-window behaviour (transport state machine, clutterbar, title marquee, time display, bitrate/sample-rate, mono/stereo, button states)
Sources consulted (category only): dirty room: binary behaviour capture; dirty room: binary resource data (shortcut and menu tables); dirty room: shipped skin assets (test input); free: webamp (cross-check only)
Author agent: R-agent (dirty room, capture session 2026-10-14)
Auditor: Spec Auditor (dirty room)
Audit date: 2026-10-14
Winamp version validated against: 2.95 (winamp.exe SHA-256 bca7a9fe1626b487d1b47bb5210670cf572d1a4cce23062076641d583b268561, extracted from winamp295.exe SHA-256 ef1423999cbefd4f2a3a98acee2b194a4260268cb4517abda9f12963e866b7e5)
Status: audited
---

# S02 — Main-window behaviour

Scope: what the main window (275 x 116) *does* — the transport state machine, the
clutterbar buttons, the title marquee, the time display, the bitrate/sample-rate
readout, the mono/stereo indicator and the lit/pressed button states. Sprite
rectangles and window geometry are in S01; the shortcuts that trigger the same
actions are in S03; title-text formatting is S14.

Everything marked **verified** was produced in this audit by driving the running 2.95
with window messages (`WM_COMMAND`, `WM_WA_IPC`, posted client-relative mouse
messages) and reading the resulting state or pixels back; a few pressed/dragged
states were produced with real mouse input, always inside the sandbox window. The
verification method is stated per claim. Everything marked **provisional** was not
confirmed in this audit; **not determined** means it was not tested and no negative
result is implied.

Provenance note: an earlier capture session of this spec used synthesized desktop
clicks, so every claim it produced about click-driven behaviour was re-checked here
with window messages (or with guarded real input inside the sandbox window). Claims
that could not be reproduced are withdrawn and listed under open questions.

---

## 1. Transport state machine

The player has three observable transport states. `IPC_ISPLAYING` (`WM_WA_IPC` 104)
returns **1** for playing, **3** for paused and **0** for stopped. Verified by
reading the IPC in each state.

The state indicator is a 9 x 9 sprite at window (26, 28), taken from `playpaus.bmp`
(rectangles in S01):

| State | Sprite rect | Shape |
|---|---|---|
| playing | 0,0,9,9 | a right-pointing triangle |
| paused | 9,0,9,9 | two vertical bars |
| stopped | 18,0,9,9 | a filled square |

Transitions, verified by sending the corresponding command and reading `IPC_ISPLAYING`
back:

| From | Command | To |
|---|---|---|
| stopped | play | playing |
| paused | play | playing |
| playing | pause | paused |
| paused | pause | playing (pause is a toggle) |
| any | stop | stopped |

- Previous/next while **stopped** change the current playlist entry but do **not**
  start playback (the state stays stopped). Verified: `IPC_ISPLAYING` stays 0, and the
  playlist position (`WM_WA_IPC` 125) moves by one.
- Seeking (forward/back 5 s) while stopped has no effect on the state. Verified.
- With a file given on the command line the program starts playing it; the entry is
  also loaded (and shown in the marquee) when it is passed while stopped. Command-line
  behaviour itself is S13.

---

## 2. Clutterbar

The clutterbar is an 8 x 43 column at window (10, 22) (background from `titlebar.bmp`,
see S01). It carries five letter buttons stacked vertically, drawn at window x = 14.
The lit letter sprite is used while a button is held, or while a latched option is on.

| # | Letter | Drawn at | Behaviour | Type | Confirmation |
|---|---|---|---|---|---|
| 1 | O | 14,25 | opens the Options popup menu | momentary | verified |
| 2 | A | 14,33 | toggles **always on top** | latched | verified |
| 3 | I | 14,41 | opens the **file info box** for the current track | momentary | verified |
| 4 | D | 14,49 | toggles **double size** | latched | verified |
| 5 | V | 14,56 | opens the **visualization menu** | momentary | verified |

- **Latched** means the letter stays drawn in its lit sprite while the option is on.
  Verified for A (always-on-top on) and D (double size on); the lit letter is the same
  sprite whether the state was reached by a click or set in the saved state.
- **Momentary** means the letter is lit only while the button is held. Verified for all
  five letters: holding a letter lights it and releasing it clears it.
- The Options popup that letter 1 opens is **not** the same popup as the main menu that
  opens on a right-click of the main window: the Options popup is smaller and contains
  the Preferences, Skins, Time elapsed/remaining, Always On Top, Double Size, EasyMove,
  Repeat and Shuffle entries. (Its contents belong to S04.)
- The file-info button opens the file-info dialog for the current track. With no track
  loaded the click produces no window.
- Whether the clutterbar can be hidden by a preference in 2.95 was not tested.

### 2.1 Marquee hint text while a control is held

While a control is held, the marquee area shows a short hint instead of the track
title. The hint is **press-driven**, not hover-driven: hovering a control with the
pointer does nothing. Verified hint strings, decoded from the rendered marquee against
`text.bmp`:

| Control held | Hint |
|---|---|
| clutterbar letter 1 (O) | `OPTIONS MENU` |
| clutterbar letter 2 (A) | `ENABLE ALWAYS-ON-TOP` |
| clutterbar letter 3 (I) | `FILE INFO BOX` |
| clutterbar letter 4 (D) | `ENABLE DOUBLESIZE MODE` |
| clutterbar letter 5 (V) | `VISUALIZATION MENU` |
| position bar | `SEEK TO: <time>/<length> (<percent>%)` |
| volume slider | `VOLUME: <n>%` |
| balance slider | `BALANCE: <CENTER or side>` |

- The transport buttons, the shuffle and repeat buttons, the EQ and playlist toggle
  buttons and the time display show **no** marquee hint, on press or on hover.
  Verified for each.

---

## 3. Title marquee

Area: window (111, 27) to about (265, 33), i.e. about 154 px, 5 px per character cell,
using the A–Z glyph row of `text.bmp` (see S01 §2.2).

**Content.** The marquee shows one of three things:

1. the program name when nothing is loaded (verified: `WINAMP 2.95`);
2. the current entry's display title, observed as `<playlist position>. <name>
   (<length>)`, for example `1. T1 (3:20)`. The bitmap font has no lowercase, so the
   name is drawn in capitals and the length is enclosed in **parentheses**. The exact
   formatting is S14;
3. a hint text while a control is held (see §2.1).

**Scrolling.** When the title is wider than the area it scrolls left. Measured from
consecutive window captures at about 60 frames/s: the text position is constant for
about 200 ms and then jumps by exactly one character cell (5 px), i.e. **5 px every
200 ms (25 px/s)**. Twelve consecutive steps measured 0.198–0.219 s apart (mean
0.204 s). Verified.

**Separator.** Between two copies of the title the marquee inserts a separator: three
asterisk glyphs with two blank character cells on each side, i.e. seven cells. Verified
by decoding the rendered glyph cells against `text.bmp`.

**Autoscroll switch.** The long-title context menu carries an "Autoscroll songname"
toggle (command id 40189). Sending that command stops the marquee (measured motion
0 px over 2 s) and sending it again resumes it (25 px/s). Verified.

**Scroll on drag.** With autoscroll off, dragging left or right inside the marquee
area scrolls the title text by hand (verified: dragging left advanced the visible text,
dragging right moved it back). The mapping from drag distance to pixels is not
determined.

**Idle.** With nothing loaded the marquee does not scroll (the program name fits).

---

## 4. Time display

Four 9 x 13 digit cells at window x = 48, 60, 78, 90, y = 26 (`numbers.bmp`, see S01
§2.3). The colon between the minutes and the seconds is part of `main.bmp`, not a
sprite.

| Condition | Display |
|---|---|
| playing | elapsed time, counting up |
| paused | the frozen time, blinking (below) |
| stopped | blank |
| nothing loaded | blank |
| remaining mode | the time until the end, with a minus sign at (38, 32) |

**Elapsed / remaining.** The time display toggles between elapsed and remaining. In
remaining mode a minus sign is drawn left of the time. Verified both by sending the
toggle command (40242, bound to Ctrl+T) and by posting a mouse click inside the time
display: in both cases the minus-sign region at (36, 30)–(46, 36) changes and nothing
else does. The click area is the display band around x = 36–96, y = 26–39.

**Pause blink.** While paused, the whole time display alternates between showing the
frozen time and blank. Measured over 8.5 s by sampling the display region: the visible
phase lasts about **1.30 s**, the blank phase about **1.30 s** (period about 2.60 s);
six complete half-cycles all fell within 60 ms of those values. The state indicator
(§1) stays lit throughout. Verified.

---

## 5. Bitrate and sample-rate display

Two numeral fields drawn with `text.bmp` (5 x 6 glyphs): the bitrate at window (111, 43)
and the sample rate at (156, 43). The static words `kbps` and `kHz` beside them are part
of `main.bmp`.

| Condition | Display |
|---|---|
| playing | bitrate and sample rate, e.g. `192` kbps, `44` kHz |
| paused | shown, frozen (same values as while playing) |
| stopped | blank |
| nothing loaded | blank |

Verified by capturing the region in each state.

**Click behaviour: withdrawn.** An earlier note claimed that the bitrate/sample-rate
area is clickable and cycles a three-state display mode. That claim came from a later
version's source and is **not confirmed on 2.95**: posted clicks at eight positions
across the band (x = 112–180, y = 46) produced no change in that band in any state.
Treat the area as non-interactive on 2.95 until a source that exercises it is found.

---

## 6. Mono / stereo indicator

Two labels at window y = 41: `mono` at (212, 41) and `stereo` at (239, 41), each with a
lit and a dim sprite (`monoster.bmp`, see S01).

| Condition | mono | stereo |
|---|---|---|
| stereo track playing or paused | dim | lit |
| mono track playing or paused | lit | dim |
| stopped | dim | dim |
| nothing loaded | dim | dim |

Verified by matching the rendered label regions against the sheet's lit and dim cells in
each state (mono and stereo test tracks).

---

## 7. Button states

All rows in this section are **verified** by holding the control with real mouse input
inside the sandbox window and matching the rendered pixels against the sheet; each
matched the named sheet rectangle with zero difference.

- **Transport buttons** (previous, play, pause, stop, next): the normal sprite is at
  `cbuttons.bmp` y = 0; the pressed sprite is the same x rectangle at y = 18. The
  pressed variant is drawn while the button is held.
- **Eject button**: normal at `cbuttons.bmp` (114, 0, 22, 16); pressed at (114, **16**,
  22, 16). Note the pressed row is y = 16, not y = 18, because this button is 16 px tall.
- **Window buttons** (menu, minimize, shade, close): in the **normal** state the four
  buttons are part of the title-bar body sprite (27, 0, 275, 14), as recorded in S01.
  While a button is **held**, it is drawn from a separate 9 x 9 cell of `titlebar.bmp`:
  menu (0, 9), minimize (9, 9), close (18, 9) and shade (9, 18), each at the button's
  window position. This closes S01's open question 1.
- **Shuffle and repeat**: the unlit sprite is at `shufrep.bmp` y = 0, the lit sprite at
  y = 30. Lit while the option is on.
- **EQ and playlist toggle buttons**: the closed sprite is at `shufrep.bmp` y = 61, the
  open sprite at y = 73; lit while that window is open.
- **Position-bar thumb**: while the thumb is held, it is drawn from `posbar.bmp`
  (278, 0, 29, 10) instead of the normal (248, 0, 29, 10). Verified.
- Pressing a transport button while the player is in another state acts as described in
  §1; the pressed sprite and the state change are independent.

---

## 8. Open questions

1. **Window-button pressed pixels.** The four 9 x 9 cells are now known (§7); what a
   *click* (as opposed to a hold) does to them at the pixel level was not measured
   beyond the hold.
2. **Clutterbar preference.** Whether 2.95 can hide the clutterbar, and how the drawn
   letters and hit areas behave when it is hidden, is untested.
3. **Scroll-on-drag mapping.** Dragging inside the marquee scrolls the title by hand
   (§3), but the ratio of drag distance to scroll distance was not determined.
4. **Bitrate/sample-rate click.** Withdrawn as a claim; the area behaved as
   non-interactive in this audit (§5).
5. **End-of-track behaviour.** What the indicator and the marquee do at the end of a
   track (auto-advance) was not measured.
6. **Hint strings for other skins.** The hint strings in §2.1 are the base skin's
   strings; whether a skin can change them is not known.
7. **Time-display blink exactness.** The blink period is about 2.60 s; the exact frame
   at which it flips relative to playback position was not determined.
