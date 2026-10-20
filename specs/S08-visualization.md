---
Spec ID: S-08
Title: Classic visualization (spectrum, oscilloscope, 576-sample data for plugins)
Sources consulted (category only): dirty room: disassembly of the analyser drawing path (owner-authorised under RFC 0021); dirty room: a configuration file written by the reference binary
Author agent: orchestrator session (dirty-room role under RFC 0021, owner-authorised 2026-10-20)
Auditor: spec auditor subagent
Audit date: 2026-10-20
Dirty-room authorisation: RFC 0021 — the draft was authored in the dirty-room role, it is kept outside this repository, and no code was written from the dirty material.
Winamp version validated against: 2.95 (winamp.exe SHA-256 bca7a9fe1626b487d1b47bb5210670cf572d1a4cce23062076641d583b268561)
Status: audited
---

# S08 — the classic visualisation: the built-in analyser

The index title of this spec names the 576-sample data for plug-ins. That part
is **out of scope here** (§0) and stays open (§6, open question 6).

## 0. How to read this spec

**The analyser was not observed at runtime for this spec.** The numbers below
come from the reference's own drawing path; the settings come from the
configuration file the reference wrote. Nothing here is a runtime measurement:
no frame of the reference was watched while it ran.

| Where it comes from | What it means |
|---|---|
| a **constant** | a value the reference's own drawing path uses: an area size, a band count, a table entry. It was read out of the reference, not seen on screen. |
| a **reading** | a statement this spec infers from a constant. Not observed behaviour: a hypothesis to be measured, never a fact to implement. |
| **open** | not established here; §6 names the question. |

Nothing in this spec is source code, a transliterated pseudo-code, a copied
comment, an internal identifier or a disassembly listing. What the audit
changed, and what it found, is in `docs/notes/spec-audit-s08.md`.

**Scope.** This spec is about the **built-in** analyser in the main window: the
bar display, its levels and its fall-off, and the peak markers above the bars.
The **576-entry sample block a `vis_` plug-in receives is out of scope here** —
it is listed among the open questions (§6, open question 6) so that it is not
lost, and S09 owns the plugin-facing ABI.

## 1. Geometry

The analyser's area and its band count depend on the state of the main window.

| State | Analyser area (px) | Bands | Share per band |
|---|---|---|---|
| normal main window | **76 x 16** | **10** | 7.6 px |
| main window, double size | **152 x 32** | 10 *(reading)* | 15.2 px *(reading)* |
| main window, shaded | **38 x 5** | **5** | 7.6 px |

What the table rests on:

- The area is 76 x 16 px in the normal window and 38 x 5 px while the main
  window is shaded. (constant)
- The band count is **10** in the normal window: the display draws one peak
  marker per band, and in the normal window those markers are the band
  indices **0…9**. In the shaded window the markers are the band indices
  **0…4** only — five bands. (constant)
- The double-size area is the normal area doubled: 152 = 2 x 76 and
  32 = 2 x 16. The whole window is scaled 2x in double-size mode (S01 §1);
  the area above is the analyser's own. (constant for the area)
- The band count is **not** fixed by the skin: it is 10 in the normal window
  and 5 while the main window is shaded. (reading)
- A band's column is a 76 / 10 = **7.6 px** share of the normal area. The
  shaded area gives the same share, 38 / 5 = 7.6 px: the shaded window keeps
  the band width and halves both the band count and the height. (reading)
- The exact split of a band's share into bar and gap is not established here
  — open question 4.
- **Reading (double size):** the double-size row keeps the band count of the
  normal window, so a band is 15.2 px wide there. The geometry alone does not
  decide this — 152 px fits 20 bands of 7.6 px just as well — and the
  evidence behind that row covers the area, not the band count. See open
  question 10.

## 2. Bar levels and the fall-off

### 2.1 The level scale

- A band's level is kept in **sixteenths of a pixel**: a stored value of 16 is
  one pixel of bar height, and the drawn height in whole pixels is the stored
  value divided by 16, rounded down. (constant)
- When the new level is at least the old one, the bar **jumps** to it: a rising
  bar is instantaneous, and the stored value becomes 16 times the new level.
  (constant)
- When the new level is lower, the bar falls by a **fixed step per update**.
  (constant)

### 2.2 The fall-off table

The step comes from a five-entry table selected by the `safalloff` setting. The
setting's value is clamped to **0…4**: a value of 4 or more behaves like 4.
(constant)

| `safalloff` | stored step | px per update (stored step / 16) |
|---|---|---|
| 0 | 3 | **0.1875** |
| 1 | 6 | **0.375** |
| 2 | 12 | **0.75** |
| 3 | 16 | **1.0** |
| 4 (or more) | 32 | **2.0** |

The third column is the second column divided by 16 — the level scale of
§2.1 — and all five values follow from that one division. The table is the
complete mapping: the step is looked up, never interpolated, and the stored
steps rise with the setting. (constant)

**Reading:** the five entries are the five speeds the Preferences page offers
for that setting. The five entries are established; the Preferences page
itself was not read.

## 3. Peak markers

### 3.1 When markers are drawn

- `sa_peaks` = 0 draws no peak markers; any other value draws **one per
  band**. (constant)

### 3.2 The factor table

A peak marker's position changes **multiplicatively** per update, by a factor
from a five-entry table selected by the `sa_peak_falloff` setting. The
setting's value is clamped to **0…4**: a value of 4 or more behaves like 4.
(constant)

| `sa_peak_falloff` | factor per update |
|---|---|
| 0 | **1.05** |
| 1 | **1.10** |
| 2 | **1.20** |
| 3 | **1.40** |
| 4 (or more) | **1.60** |

The five factors are strictly increasing, so the higher the setting, the larger
the factor. (constant)

### 3.3 What the factor does — and what it does not settle

Because the factor multiplies the marker's distance each update, the marker's
step is **not constant**: with a factor above 1 the change grows with every
update, which is why a falling peak marker visibly speeds up instead of
descending at a constant speed. (constant, as to the growth of the step)

The **direction** of the marker's travel is not settled, and the draft's own
wording contradicts itself about it: it calls the operation a *decay* of the
marker's position although every factor is above 1 — a multiplier above 1 grows
what it multiplies — and it says the peak *accelerates away from the bar as it
falls*, where "falls" and "away from the bar" name two opposite directions.
What is missing is the multiplicand: the marker's height above the bar, or its
distance below the peak it fell from. The five factors and their order are the
settled part of this section; the direction is open question 11.

## 4. The settings that drive the display

Six keys in the configuration file are the analyser's own settings: `sa`,
`safire`, `saref`, `safalloff`, `sa_peaks`, `sa_peak_falloff`. The set, the
observed values and the file facts are S12 §3.5; the same six names stand in
S12 §5.1's recognised key set. The bar display reads **four** of the six.

| Key | What the display does with it |
|---|---|
| `sa` | selects the display mode. The observed value is `1` (S12 §3.5). Which value selects the bar display and which the oscilloscope — and whether a third mode exists — is open (open question 5). S12 §3.5 carries an **inferred** reading of this key ("0 = oscilloscope"); this spec does not promote it. |
| `safalloff` | selects the fall-off step table of §2.2. The observed value is `2` — a stored step of 12, that is 0.75 px per update. |
| `sa_peaks` | 0 draws no peak markers, any other value draws one per band (§3.1). The observed value is `1`. |
| `sa_peak_falloff` | selects the peak factor table of §3.2. The observed value is `1` — a factor of 1.10 per update. |
| `safire`, `saref` | **not read by the bar display.** Their meaning is unknown: S12 §3.5 marks both as open, and S12 §10 item 2 lists them together with `pilp` as recognised, observed and of unknown meaning. |

S12 §3.5 left the value ranges of these keys, and the mapping from a setting
to pixels and to frames, to this spec. §2 and §3 settle the **pixel** side for
the two enumerated settings: the clamp is 0…4 and the two tables are the
complete mapping. The **frame** side — how many updates a second the analyser
runs at — is open (open question 2), and so is the range of the other four keys
(S12 §10 item 3).

## 5. Practical notes for an implementer

- The level is an integer count of sixteenths of a pixel (§2.1), so the drawn
  bar height is an integer number of pixels.
- The fall-off step counts **stored units, not pixels**. At 16 stored units per
  pixel, the step of 3 is 0.1875 px per update, so the drawn bar drops one
  pixel only every 16/3 ≈ 5.33 updates; the step of 16 is one pixel per update
  and the step of 32 is two pixels per update. Carry the step in stored units:
  the visible fall is quantised by the pixel grid, and the numbers in §2.2's
  table are stored steps.
- The two enumerated settings of §2.2 and §3.2 are **per update**, not per
  millisecond. The update rate is open (open question 2), so no rate per
  second follows from this spec yet.
- A rising bar is instantaneous; only the fall is stepped (§2.1).
- The analyser area is 76 x 16 px in the normal window (§1). Where that area
  sits inside the window is not established here — open question 13.
- The bar/gap split inside a band's 7.6 px share is open (open question 4),
  and so is the mapping of bands to frequencies (open question 1).
- Everything in §6 is unknown. A behaviour that §6 lists must not be
  implemented as if it were settled.

## 6. Open questions

Kept from the draft, in the draft's order:

1. **The band frequency edges.** The display works on one level per band; the
   transform that fills those levels — the window size, the FFT length and the
   mapping from its bins to the ten bands — is not established here.
2. **The update rate.** The fall-off is per *update*; how often the analyser is
   updated, and whether that changes with the display mode, is not
   established.
3. **The oscilloscope.** Its sampling — every Nth sample, or an average per
   column — is a separate mode and was not read.
4. **The bar/gap split** inside a band's 7.6 px share.
5. **Which `sa` value selects which mode**, and whether a third mode exists.
6. **The 576-entry block a `vis_` plug-in receives**: its block size, its
   channel interleaving, and whether it is pre- or post-DSP. This is **out of
   scope for this spec** (§0); S09 §5.2 owns the plugin-facing ABI and the
   data members.
7. **Whether the shaded-window analyser uses the same band set** (5 of 10) or
   a different transform.
8. **`safire` and `saref`** (S12 §10 item 2).
9. **What the display does when nothing plays** — flat bars, frozen, or
   cleared.

Added by the audit. Each follows from something the draft itself records, and
none of them is a fact the draft did not contain:

10. **The band count in double size.** 152 px fits 10 bands of 15.2 px and 20
    bands of 7.6 px alike, and the evidence behind the double-size row covers
    the area, not the band count. The spec keeps 10 as a reading (§1).
11. **The direction of the peak marker's travel** (§3.3): the draft calls a
    factor above 1 a decay, and says the marker accelerates away from the bar
    as it falls.
12. **The level scale of the shaded analyser.** The normal area is 16 px tall
    and the shaded area 5 px; whether the shaded path uses the same
    sixteenths-of-a-pixel scale (§2.1), and what a full bar is there, is not
    established.
13. **Where the analyser area sits in the main window.** This spec gives the
    area's size (§1); S01 records no analyser rectangle, so the position is
    not established either.
