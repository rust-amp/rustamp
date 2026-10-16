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

## 0. How to read this spec

Evidence grades used:

| Grade | Meaning |
|---|---|
| **settled** | Direct measurement in a sandboxed 2.95 run, with the step or capture named |
| **provisional** | Measured, but its meaning is not established; the missing piece is named |
| **open** | No evidence of any kind; the capture that would settle it is named |
| **visual reading** | A string, row label or shape read from a capture by eye or by OCR, not produced by the comparator |
| **corroboration** | Two independent measurements agreeing, where neither alone would settle the point |

Runs interpreted here (all at dpi as recorded; landmark gate passed in each):

| Run | Job | Result |
|---|---|---|
| `20261016-055602` | End-of-list measurement (17/17 steps ok, landmark gate passed) | Settled end-of-list state: indicator stopped, `IPC_ISPLAYING = 0`, time, kbps/kHz and mono/stereo all stopped, on the last entry, via the open-error skip path. `eol_state`, `eol_stable`, `eol_play_after`, `sac_before`, `sac_arm`, `sac_arm2` share one SHA-256 (`D25CE145F8DD26D2…`). The whole run yields only three distinct window frames. |
| Other claims in §§2–7 | 2026-10-14 audit session | Claims verified by driving the running 2.95 with window messages and guarded real input inside the sandbox window. No individually named run IDs per claim; methods per claim are stated but without gate verdicts or step tables. |

---

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

### 1.1 End of the list

When the **last entry of the list is consumed**, the player stops. This is the
open-error skip path (a 3-entry playlist whose entries point at absent files;
no track ever plays to a natural end — see the caveat below), measured in run
`20261016-055602` (17/17 steps ok, landmark gate passed). The end-of-list
state is the **ordinary stopped state**: `IPC_ISPLAYING` (step 003,
`ipc_isplaying_after_list`) returns 0 and the observable indicator, time, kbps/kHz
and mono/stereo regions all report stopped (below).

Facts, each with the step or capture it rests on:

- **End state is stopped.** The state indicator is the stopped square: 0 px of
  difference against the recorded stopped capture `captures\m06_main_stopped_main.png`
  in the indicator region (24, 28, 11, 9), but 38 px against the recorded playing
  capture `m04_main_playing_main.png` and 35 px against the recorded paused capture
  `m05_main_paused_main.png`. `IPC_ISPLAYING = 0` (`ipc_isplaying_after_list`, 003).
  Evidence: `eol_state` (001). Cross-check: `playpaus.bmp` stopped sprite at
  (18, 0, 9, 9) (§1).
- **Static and reproducible.** `eol_stable` (002) is byte-identical to `eol_state`
  (001) (0 differing pixels). The run took one shot per step and every post-list
  launch produced the same window: `eol_state`, `eol_stable`,
  `ipc_isplaying_after_list`, `eol_next_back`, `eol_play_after`, `sac_before`,
  `sac_arm`, `sac_unarm`, `sac_arm2`, `sac_ctl_base`, `repeat_base` share one
  SHA-256 (`D25CE145F8DD26D2…`), and the twin gate probes (`smoke\a_main.png` /
  `smoke\b_main.png`) are byte-identical too. The whole run yields only **three**
  distinct window frames.
- **Ends on the last entry.** The window title reads `3. s02r2 cc - Winamp
  [Stopped]` at every post-list step (`eol_state`, `eol_play_after`, `sac_before`,
  `sac_ctl_base`, `repeat_base`). All three entries are present: `eol_prev` (004)
  moved 3→2 and `eol_next_back` (005) moved 2→3.
- **Only the marquee row differs from a recorded stopped capture.** Whole-window
  difference between `eol_state` (001) and `m06_main_stopped_main.png` is 95 px,
  all of them inside (111, 27)–(167, 32), the marquee row.
- **Prev / next at the end move the marquee only.** `eol_prev` (004) differs
  from `eol_state` (001) by 22 px, bounds (111, 27)–(164, 32) — the marquee
  row only; entry 3→2; the indicator (24, 28) is untouched. `eol_next_back`
  (005) differs from `eol_prev` (004) by the same 22 px and the resulting window
  is byte-identical to `eol_state` (001). Playlist position (`WM_WA_IPC` 125)
  moves by one; transport stays stopped.
- **Play at the end blinks the indicator and re-stops.** `eol_play` (006) differs
  from `eol_next_back` (005) by 38 px, all inside the indicator box
  (24, 28)–(34, 36); the indicator region compares at 0 px to the recorded
  playing capture `m04_main_playing_main.png`. `eol_play_after` (007) is
  byte-identical to `eol_state` (001). The current entry is not reset (title stays
  `3. s02r2 cc …`).
- **Time display, kbps/kHz, mono/stereo.** At the end of the list they read as in
  the stopped state: time digits blank (band (46, 26, 53, 13) 0 px against `m06`,
  218 px against `m04`), kbps/kHz blank (band (111, 38, 70, 12) 0 px against
  `m06`), mono and stereo both dim (band (212, 38, 40, 10) 0 px against `m06`).
  Evidence: `eol_state` (001). Whether the elapsed/remaining display mode changed on
  the way to the end is **provisional** (no minus sign is present in the post-list
  state, but the run never toggles the mode and never captures the pre-end state) —
  see open question 10.
- **Stop-after-current has no visible indicator.** Posting `WM_COMMAND` 40157
  (`sac_arm`, 009) produces a window **0 / 31900 px** identical to `sac_before`
  (008) — whole window, including the transport row and the repeat/shuffle LEDs.
  That this means "no visible indicator" and not "command ignored" is **decided**
  by the delivery control: `sac_ctl_repeat` (013) posts `WM_COMMAND` 40022
  (`&Repeat`) down the same posted path after the same restart and differs from
  `sac_ctl_base` (012) by **6 / 31900 px**, bounds (216, 94)–(218, 95) (the
  repeat LED), all six pixels (16, 90, 0) → (0, 214, 0); the bootstrap recorded
  `expect_change=true satisfied`. Posted `WM_COMMAND` is therefore handled in this
  reference, and 40157's 0-px identity is the mode's (absent) indicator. The three
  reproduction steps are byte-identical to their named references with
  `expect_same` satisfied: `sac_before` (008) = `eol_state`, `sac_unarm` (010) =
  `sac_before`, `sac_arm2` (011) = `sac_arm`. Whether stop-after-current is a
  toggle (arming twice returns to off) or a one-shot, whether any menu checkmark
  exists, and whether it has any effect at a real track end, are **open** — see
  open questions 8 and below. Evidence: `sac_before`/`sac_arm`/`sac_unarm`/
  `sac_arm2` (008–011), `sac_ctl_base`/`sac_ctl_repeat` (012/013).
- **Nothing persists across a restart.** `sac_before` (008) is byte-identical to
  `eol_state` (001); a `restart` force-kills Winamp, so `winamp.ini` is not
  rewritten and no toggle carries over. Evidence: `sac_before` vs `eol_state`.
- **Repeat can be armed by a posted click.** The re-aimed click at body centre
  (223, 96) on the repeat button lit the LED: `repeat_arm` (015) differs from
  `repeat_base` (014) by 6 / 31900 px, bounds (216, 94)–(218, 95), (16, 90, 0)
  → (0, 214, 0). The earlier 0-px result at (210, 89) was a point-placement
  error (clicking the sprite's top-left corner), not a negative capability.
  Evidence: `repeat_base` (014), `repeat_arm` (015).
- **Repeat wrapping stays open.** `repeat_play` (016) differs from `repeat_arm`
  (015) by 38 px (indicator box only) and then `repeat_after` (017) differs from
  it by 73 px, bounds (24, 27)–(164, 48); the window was still changing at capture
  time, the LED stayed lit (0, 214, 0), and the title read `3. s02r2 cc - Winamp
  [Stopped]`. Whether repeat wraps to the first entry at the end of the list is
  therefore **not settled**. Evidence: `repeat_play` (016), `repeat_after` (017).
  Follow-up: a media-enabled run plus a job `landmark_region` excluding the
  position bar at client (16, 72, 248, 10).

Caveat: the measurement reaches only the **open-error skip** path. The fixture's
three entries point at `C:\s02r2-media\*.mp3`, which does not exist in the
guest, so every entry takes the open-error → advance path and no track ever plays
to a natural end. The natural end of a played track, the intermediate skip
captures, and stop-after-current's effect on a track end are **not measured** here
(see open questions 5 and 8). Whether the reference skips a dead entry and
continues rather than stopping at it is likewise **provisional** (the
`r_o4_60.m3u` precedent shows the skip directly; this run shows only its
consequence — the list ended on entry 3 with all three entries dead).

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
5. **End-of-track behaviour.** The end-of-list *state* is now settled and is the
   ordinary stopped state (§1.1): indicator stopped, `IPC_ISPLAYING = 0`, time,
   kbps/kHz and mono/stereo all stopped, on the last entry, via the open-error
   skip path (`eol_state`, `ipc_isplaying_after_list`, `eol_stable`,
   `eol_next_back`). Still open:
   - **Natural end of a played track.** No entry in that measurement could be
     opened, so only the open-error advance path is reachable; whether a track
     that actually played stops the same way is **not determined** (cannot be
     measured with this harness). Follow-up: map a media folder into the sandbox,
     or ship a small guest-local file, **and** declare a job `landmark_region`
     excluding the position bar — the posbar track sits at client (16, 72),
     248 x 10, inside the default region rows 49–115 — e.g. rows 49–71.
   - **Repeat wrapping at the end of the list** (item 9 below).
   - **Stop-after-current's effect on a track end** (item 8 below).
6. **Hint strings for other skins.** The hint strings in §2.1 are the base skin's
   strings; whether a skin can change them is not known.
7. **Time-display blink exactness.** The blink period is about 2.60 s; the exact frame
   at which it flips relative to playback position was not determined.
8. **Stop-after-current (mode, indicator, persistence, effect).** The mode is reached
   by `WM_COMMAND` 40157 (`ToggleStopAfterCurrent`, RFC 0005 §5.1). Findings of
   §1.1: arming changes **no visible on-screen control** — `sac_arm` (009) is 0 /
   31900 px identical to `sac_before` (008), whole window; delivery is **decided**
   (not "command ignored") because the control `sac_ctl_repeat` (013) posts
   `WM_COMMAND` 40022 down the same path after the same restart and lights the
   repeat LED at (216, 94)–(218, 95), 6 / 31900 px, (16, 90, 0) → (0, 214, 0).
   - `sac_before` (008) ≡ `eol_state` (001); `sac_unarm` (010) ≡ `sac_before`
     (008); `sac_arm2` (011) ≡ `sac_arm` (009) — `expect_same` satisfied on all
     three.
   - **Open — toggle vs one-shot.** Arming twice round-trips to byte-identical
     captures (all four states ≡ `D25CE145…`), which is the expected result for a
     toggle with no indicator **but also what "the command did nothing" looks
     like** without the delivery control. The three steps assert identity only;
     they do **not** prove the toggle. RFC 0005's `Toggle*` convention is not
     evidenced by this run.
   - **Open — on-screen / menu checkmark.** The main menu was never opened in this
     audit; a right-click of the stop button (its context menu carries
     `Stop after &current`, id 40157) was not exercised (no right-button gesture in
     the harness). S-04 owns the menu.
   - **Open — effect on a track end.** No track ends naturally in this measurement;
     arming, unarming and the indicator are measured, the effect is not.
     **Cannot be measured with this harness** (§1.1 caveat).
   - **Settled — no persistence.** `sac_before` (008) is byte-identical to
     `eol_state` (001): a `restart` force-kills Winamp, `winamp.ini` is not
     rewritten, nothing carries over.
   - **Settled — no `PlayerState` IPC id.** The reference exposes no IPC id for the
     mode; S-10 owns that surface.
9. **Repeat wrapping at the end of the list.** A posted click at (223, 96) **does**
   arm repeat (`repeat_arm` (015): 6 / 31900 px, bounds (216, 94)–(218, 95),
   (16, 90, 0) → (0, 214, 0); the earlier 0-px reading at (210, 89) was a
   point-placement error). But the final capture `repeat_after` (017) was still
   changing vs `repeat_play` (016) (73 px, bounds (24, 27)–(164, 48)) and the title
   still read `3. s02r2 cc - Winamp [Stopped]`, so repeat wrapping is **not
   decidable** from this run. Open also:
   - **Whether a posted `WM_COMMAND` 40022 is the right id for repeat in this
     binary.** The id comes from the resource tables (S-03/S-04, S-03 binds `R`
     to repeat); the step proves that id lights the LED on this posted path, but
     the internal Repeat mode bit is not confirmed by any menu state (still pixel
     evidence, not a menu state — the pixels at (216, 94)-(218, 95) are the
     recorded shuffle/repeat sprite, `captures\m11_main_shuffle_repeat_main.png`).
   - Whether an on-screen/menu checkmark exists (main menu never opened).
   - **Not tested here:** real (`SendInput` / `mouse_event`) clicks; all inputs
     were posted messages. Real-input confirmation is a separate claim.
   - Follow-up: the same media-enabled run as item 5, or a mid-run arm primitive.
10. **Elapsed/remaining time mode at the end.** No minus sign is present in the
    post-list state, and the minus-sign area (36, 30)–(46, 36) is part of the
    region in which `eol_state` and `m06` agree. Whether the mode changed on the
    way to the end is **provisional**: the run never toggles the display mode and
    never captures the pre-end state. Settle with a two-step probe (command 40242,
    or a click in the display band x = 36–96, y = 26–39). Evidence: `eol_state`
    (001).
11. **Open-error skip as a captured transition.** This run consumed lists of dead
    entries; the skip itself was never captured, only its consequence (the list
    ended on entry 3 with all three entries dead, and 3→2→3 walked all present
    entries). Whether the reference skips a dead entry and continues (rather than
    stopping) is therefore **provisional** on the reference side, from the
    `r_o4_60.m3u` precedent. Cannot be measured with post-list captures.
    Evidence: `eol_state`, `eol_prev`, `eol_next_back`; `r_o4_60.m3u` runs
    20261015-160722 / -163106.
12. **Real-mouse confirmation.** Every input in the runs above was a posted
    message; a real-input (`SendInput` / `mouse_event`) confirmation is a separate
    claim, recorded here so it is not re-opened (no run performed one).
13. **End-of-list IPC event.** Whether the reference exposes an IPC id for the
    end-of-list event is unowned (RFC 0005 O15, still open); this audit contributes
    no IPC read beyond `IPC_ISPLAYING = 0` as a cross-check (`ipc_isplaying_after_list`,
    step 003). S-10 owns that surface.
