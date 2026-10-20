---
Spec ID: S-11
Title: Embed-window API and gen.bmp layout
Sources consulted (category only): dirty room: string table of the reference binary; dirty room: the classic base skin shipped with the reference (BMP headers only); dirty room: a configuration file written by the reference
Author agent: orchestrator session (dirty-room role under RFC 0021, owner-authorised 2026-10-20)
Auditor: spec auditor subagent
Audit date: 2026-10-20
Dirty-room authorisation: RFC 0021 — the draft was authored in the dirty-room role, it is kept outside this repository, and no code was written from the dirty material.
Winamp version validated against: 2.95 (winamp.exe SHA-256 bca7a9fe1626b487d1b47bb5210670cf572d1a4cce23062076641d583b268561)
Status: audited
---

# S11 — the embed window and the `gen.bmp` sheet

## 0. How to read this spec

**The reference binary was not run for this spec.** Everything below comes from
three artifacts: the sheet names the reference holds, the BMP headers of the
classic base skin the reference ships, and the configuration file the reference
wrote. No window was watched while it ran, so nothing here is a runtime
measurement.

| Where it comes from | What it means |
|---|---|
| §1 | **Name fact** — the sheet file names the reference holds, and which of them the classic base skin ships. |
| §2 | **Header measurement** — a sheet's pixel size and compression, read out of the BMP header of the classic base skin. |
| §3 | **Reading** — what a sheet is for, inferred from its name and the window it belongs to. Never observed behaviour. |
| §4 | **Settled behaviour** — the four statements of §4; each item names what it rests on. |
| §5 | **Practical note** — a consequence that follows from §1–§4; nothing new. |
| §6 | **Open** — not established here; the measurement that would settle it is named. |

Nothing in this spec is source code, a transliterated pseudo-code, a copied
comment, an internal identifier or a disassembly listing. What the audit
changed, and what it found, is in `docs/notes/spec-audit-s11.md`.

**Scope.** This spec is about the **sheet inventory** of the embed
(general-purpose) window and the video window, the **sizes** of those sheets,
and the settings that place the windows.

**The sprite rectangles are not in this spec.** No rectangle was measured for
this spec, so no rectangle is stated here: the tables for `gen.bmp`,
`genex.bmp`, `video.bmp` and `mb.bmp` are open (§6, items 1, 2, 7 and 12). The
embed request's own shape is not in this spec either (§6, item 9).

## 1. The sheet names the reference holds

The reference's own sheet list names these **eighteen** sheets:

`titlebar.bmp`, `posbar.bmp`, `text.bmp`, `balance.bmp`, `volume.bmp`,
`numbers.bmp`, `nums_ex.bmp`, `shufrep.bmp`, `playpaus.bmp`, `monoster.bmp`,
`cbuttons.bmp`, `gen.bmp`, `video.bmp`, `mb.bmp`, `pledit.bmp`, `eq_ex.bmp`,
`eqmain.bmp`, `main.bmp`.

`genex.bmp` is **named separately**: its name occurs in the reference, but not
in that list of eighteen.

Two consequences are worth stating, because they are easy to get wrong:

- The reference's list of names is **not** the set of files the classic base
  skin ships. The base skin ships **eighteen** sheets, and they are the
  eighteen above **except `nums_ex.bmp`**, **plus `genex.bmp`** (S01 §5 carries
  the measured list of the base skin's sheets; S17 §3 covers the `nums_ex`
  substitution). `nums_ex.bmp` is a name the reference holds but the classic
  skin does not carry; `genex.bmp` is a file the classic skin carries whose
  name is not in the list.
- That asymmetry is a fact about the **names**, not about the loading: whether
  the reference loads `genex.bmp` through the same path as the other sheets is
  **open** (§6, item 10). Whether the reference holds further sheet names
  beyond these eighteen and `genex.bmp` is open as well (§6, item 14).

## 2. Measured sizes and encodings

Measured from the **BMP headers** of the classic base skin, 8 bits per pixel
throughout. S01 §5 carries the complete measured table for the base skin's
eighteen sheets; the six sheets below are the ones this spec's own material
measured.

| Sheet | Size (px) | Encoding |
|---|---|---|
| `gen.bmp` | **194 x 109** | RLE8 |
| `genex.bmp` | **130 x 75** | uncompressed |
| `video.bmp` | **234 x 119** | uncompressed |
| `mb.bmp` | **234 x 119** | uncompressed |
| `main.bmp` | 275 x 116 | RLE8 |
| `pledit.bmp` | 280 x 186 | uncompressed |

These sizes and encodings agree with the two audited specs that carry the same
measurements: S01 §5 (the base skin's sheet table) and S17 §4 (the list of the
base skin's RLE8 and uncompressed sheets).

Two facts follow, and both are easy to get wrong:

- **`gen.bmp` is not 275 wide.** The embed window's sheet is its own size, so
  an implementation must not derive its rectangles from `main.bmp`'s grid.
- **`video.bmp` and `mb.bmp` have the same size** — 234 x 119 each — although
  the two windows do not. In the configuration file the reference wrote, the
  video window was **275 x 232** and the minibrowser **350 x 348** (S12 §3.4).
  Those are the sizes observed in one run of the reference, not established
  defaults (S12 §10 item 8). A sheet that is smaller than the window it
  dresses cannot be a full-window backdrop: it is a set of pieces. What part
  of the window the pieces cover — inside the saved window rectangle, or around
  it — is **open** (§6, item 11).

## 3. What the sheets are for

These are **readings**: they follow from a sheet's name and the window it
belongs to, not from a measurement. No rectangle was measured, so nothing here
says what a sheet contains.

- `gen.bmp` — the frame the player draws around an **embedded general-purpose
  plug-in**. A `gen_` plug-in can ask the player for a window; the player
  decorates it, and the plug-in's own content sits inside. (The request
  itself is not in this spec — §6, item 9.)
- `genex.bmp` — the sheet named outside the list of eighteen (§1). It reads as
  an **extra** sheet for the same window; what it carries, and when it is used
  instead of `gen.bmp`, is **open** (§6, items 2 and 6).
- `video.bmp` — the video window's chrome; `mb.bmp` — the minibrowser's. The
  two share a sheet size (§2) but not a window size.
- The **video window's position, size, visibility and behaviour** are the
  `video_*` keys that S12 §3.4 and §3.6 record. This spec does not restate
  them.

## 4. Behaviour that is settled

Four statements, each with what it rests on.

1. **The embed window exists only while a plug-in asks for it.** The video
   window exists while a video plays and the visibility settings say so
   (`video_open`, `video_autoopen`, `video_autoclose` — S12 §3.4, §3.6).
   *Rests on:* the meanings of the configuration keys. The request's own shape
   is open (§6, item 9).
2. **The sheets are optional in a skin.** S17 §2 verifies that a skin with
   `main.bmp` and `titlebar.bmp` deleted still loads and still draws, and that
   the fallback is **per element**: the missing element comes from the built-in
   default skin, the rest of the skin still comes from the directory. S17 §2
   leaves the **per-file matrix** open — which missing sheet falls back to
   which built-in sheet, and whether any sheet's absence is fatal, was not
   exercised. What the embed window looks like without `gen.bmp` is **open**
   here (§6, item 5). *Rests on:* S17 §2, verified part and open part.
3. **The video window's aspect ratio is kept as a fraction** — `video_ratio1`
   and `video_ratio2`, with `video_useratio` recording whether the user
   overrode it (S12 §3.6). The values observed in the reference's own
   configuration file are **4** and **3**; that they are the *defaults* is
   **not** established (S12 §10 item 8).
4. **A video window the user closed stays closed for that video**, unless
   `video_autoopen` opens it on the next one. *Rests on:* the meanings of
   `video_autoopen`, `video_autoclose` and `video_stopclose` (S12 §3.6).

## 5. Practical notes for an implementer

- The sizes in §2 are the **classic base skin's** sizes, not a format
  requirement: a skin may ship any size, and a loader must read the size from
  the file (S01 §5 makes the same point).
- A loader must accept **both** RLE8-compressed and uncompressed BMPs:
  `gen.bmp` is RLE8, `genex.bmp`, `video.bmp` and `mb.bmp` are not (S01 §5,
  S17 §4).
- The classic sheets are drawn **opaquely**; there is no per-sheet colour key
  (S01 §6, S17 §4).
- Sheet lookup folds case (S17 §1).
- Do not derive the embed window's sprites from `main.bmp`: `gen.bmp` has its
  own size (§2) and its own sprite table, and that table is not in this spec.
- The video window's geometry and behaviour come from the `video_*` keys
  (S12 §3.4, §3.6), not from this spec.
- Everything in §6 is unknown. A behaviour that §6 lists must not be
  implemented as if it were settled.

## 6. Open questions

Kept from the draft, in the draft's order:

1. **The sprite rectangle table for `gen.bmp`** — title bar, the buttons, the
   frame edges, in the 194 x 109 sheet. This is the heart of the spec and
   needs the drawing path, not the sheet size.
2. **The same for `genex.bmp`**, and what selects it.
3. **The embed window's client size and minimum size**, and what happens when
   a plug-in asks to resize itself.
4. **Which bottom buttons exist** in the embed window and what each does;
   whether they are the same set as the main window's.
5. **What is drawn when the skin has no `gen.bmp`** (a plain frame? the
   plug-in's own window with no decoration?).
6. **Whether `genex.bmp` is required** for the window to open.
7. **The video window's chrome table** in `video.bmp`, and its minimum size.
8. **Whether the embed window is the same window as the "video window"**, or a
   separate one with its own sheet.
9. **The `gen_` embed request's shape** — whether the player or the plug-in
   owns the frame. §3 states only that the player decorates.

Added by the audit. Each follows from something the draft itself records, and
none of them is a fact the draft did not contain:

10. **Whether `genex.bmp` is loaded through the same path as the other
    sheets.** Its name does not occur in the reference's list of eighteen names
    (§1) while the file is present in the classic base skin (S01 §5, S17 §4).
    Whether the reference loads it by name, by a separate rule, or not at all
    is not established.
11. **What the sheet's pieces cover.** The sheets are smaller than the windows
    they dress (§2), so they cannot be backdrops; whether the frame is drawn
    inside the saved window rectangle or around it — and therefore what the
    saved size counts — is not established.
12. **The sprite table in `mb.bmp`.** This spec excludes every sprite table,
    but `mb.bmp` is one of the four sheets it measures, and the minibrowser's
    chrome table is as open as the video window's (item 7).
13. **Which surface owns the embed request.** S09 §5.3 and §7.3 point the
    embedding surface at this spec, and S10 §12.2 lists the embed IPC ids
    (505, 532, 533) as host answers that were never probed. The request's
    shape is open in both.
14. **Whether the reference holds further sheet names** beyond these eighteen
    and `genex.bmp` (§1).
