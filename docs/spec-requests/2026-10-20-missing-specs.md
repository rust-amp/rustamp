# Spec requests: the nine specs that do not exist yet

Filed 2026-10-20 by the orchestrator. **This is a request to the dirty room, not
work.** Under [CLEANROOM.md](../../CLEANROOM.md) §2 the dirty room runs and
automates Winamp 2.95, captures screenshots and timings and disassembles
binaries; the orchestrator and the implementers stay clean and **must not** read
Winamp, its SDK or its disassembly (§2, §5). §3 then audits the draft and
exports it to `specs/`.

So the requests below are phrased as **observable questions**: what a measurement
would have to settle, in words, without prescribing how it is measured.

## Why this list exists

Nine behaviours are implemented today as **documented own choices**, because no
spec settles them. Each one is honest about that in code and in
`docs/tasks/ROADMAP-to-winamp-parity.md`, but "own choice" is not parity: a
measurement can turn any item below into a fact.

| Spec | Blocks |
|---|---|
| S08 | the visualization analyser, its bands, its decay and its frame rate |
| S11 | `gen.bmp` and the `embed`/video window |
| S12 | ~~winamp.ini import and the key names it carries~~ **delivered: specs/S12-winamp-ini.md (audited 2026-10-20)** |
| S13 | ~~the card names it, no content survives~~ **delivered: `specs/S13-command-line.md` (audited 2026-10-20)** - the command line, single instance and the shell/drag-and-drop rules |
| S14 | playlist title formatting (`%1`, `$if`, …) |
| S16 | the Preferences page set and their controls |
| S18 | crossfade: length, curve, and what happens on a manual skip |
| S19 | streaming: buffering, the status line, and the failure states |
| S20 | the media library's columns, views and sort semantics |

## 1. S08 — the visualization analyser

**Implemented as an own choice** (`rustamp-vis`'s `analyser`, wired in
`rustamp-ui::visualization`): band count, band edges, decay per frame, peak
markers, oscilloscope sampling, the stale-frame timeout.

Questions a measurement can settle:

1. How many spectrum bands does the built-in analyser draw, and at which pixel
   columns? (Count the bands in a screenshot of a known signal, and the column
   extents.)
2. Which frequency does each band cover? (Feed a sine sweep, record which band
   peaks as a function of frequency; report the band edges as frequencies.)
3. What is the fall rate of a bar after a peak, in pixels per frame? (Feed a
   burst, then silence, and count the frames for a full-height bar to reach the
   baseline.)
4. Does the oscilloscope sample every Nth sample, or an averaged bucket? (Feed a
   signal that aliases differently under each hypothesis and say which one the
   picture matches.)
5. The frame rate of the classic vis window (and whether it changes with the
   spectrum/scope mode).
6. What does the window show while *nothing* is playing (flat line, frozen
   picture, background only)?

## 2. S11 — `gen.bmp` and the embed window

**Not implemented**: the embed/video window and the `gen.bmp` sheet have no
spec, so the window does not exist. The `gen_` plug-in kind loads and can be
configured (RFC 0020 covers the request).

Questions:

1. Which sprites does `gen.bmp` carry, at which rectangles (title bar, buttons,
   the frame edges), for the classic base skin?
2. The embed window's client size, its minimum size, and what happens when a
   general-purpose plug-in resizes itself.
3. Which of the bottom buttons exist in the embed window and what each does.
4. Whether `gen.bmp` is required for a window to open, and what is drawn when
   the skin has no `gen.bmp`.

## 3. S12 - `winamp.ini` - **DELIVERED** (specs/S12-winamp-ini.md)

**Not implemented**: no import, no key list.

Questions:

1. The key names a 2.95 installation writes for: the volume, the balance, the
   EQ enabled/autoload flags, the window positions, the skin, the last playlist,
   the output device, and the DSP/vis plug-in.
2. The value syntax of each (integer range, string escaping, booleans as 0/1 or
   words).
3. Which keys a *fresh* installation writes, and which it only writes after the
   user changes the setting.
4. Whether a missing key falls back to a documented default and what that
   default is.

## 4. S13 - unknown content - **DELIVERED as specs/S13-command-line.md**

**Answer:** S13 is the command line, single instance and the shell/drag-and-drop rules. It is delivered as `specs/S13-command-line.md` (audited 2026-10-20): seven switches (`/CLASS=`, `/CONFIG=`, `/BOOKMARK`, `/INSTALL`, `/UNREG`, `/NOREG`, `/ONLYINSTALL`), the single-instance rule, and the `Play`/`Enqueue`/`ListBookmark`/`open` verbs that make drag-and-drop from Explorer work.


## 5. S14 — playlist title formatting

**Not implemented**: the playlist shows the file name or the tag title, no
formatting language.

Questions:

1. The placeholder set (`%1`, `%2`, …) and what each maps to.
2. The `$if`, `$if2`, `$num`, `$pad`-style constructs, if they exist in 2.95 at
   all (they may be a 5.x feature — **precedence rule §6: 2.95 wins**).
3. Where the format string is entered and stored (a preferences page? a per-view
   setting? a global one?).
4. The behaviour on a missing field (literal text? empty? a fallback?).

## 6. S16 — the Preferences pages

**Partly implemented** as a native dialog with pages built from the tree
(`rustamp-platform::prefs_dialog`); the page set, their order and their controls
are own choices.

Questions:

1. The full page tree (the left-hand tree in 2.95), its order and its nesting.
2. For each page: the controls it carries and their labels, in the reference's
   own words.
3. Which pages are absent in 2.95 (the 5.x set is larger — precedence §6).
4. The dialog's size, and whether it is resizable.

## 7. S18 — crossfade

**Not implemented**: fade-out on stop exists (`StopWithFadeout`), but no
crossfade between tracks.

Questions:

1. The crossfade length range and its default, and where it is configured.
2. The curve (linear, equal-power, something else) — measured on a
   constant-amplitude signal, as the gain of each track against time.
3. What happens on a manual "next"/"previous" and on a seek during a crossfade.
4. Whether the crossfade applies to a gapless continuation or only to a
   different-track transition.
5. Whether the two tracks' sample rates must match for a crossfade to happen.

## 8. S19 — streaming

**Not implemented**: local files only.

Questions:

1. Which protocols 2.95 accepts, and the timeout it uses before giving up.
2. The buffering behaviour: when does playback start, how much is buffered, and
   what the status line shows while buffering.
3. The failure states and their messages (unreachable host, 404, a stream that
   ends, a stream that stalls).
4. Whether the length/seek controls behave differently on a stream (and what the
   seek bar shows when the length is unknown).

## 9. S20 — the media library

**Implemented as a modern own design** (index, search, cover column, roots);
2.95 has no media library in the modern sense, so this may be "no reference
behaviour" rather than a gap.

One line from the dirty room settles it: **does 2.95 have an equivalent view
(the "Load" dialog's file view? the playlist's own columns?), or is this a
deliberate extension?** If it is an extension, the card closes as "own feature"
and the parity list should say so.

## What the dirty room should return

For each item: an audited spec under `specs/` with the provenance header
`specs/README.md` describes, the measurement method, and — where a number is
stated — the run that produced it. §4 of `CLEANROOM.md` applies: behaviour,
numbers and layouts are allowed; code, pseudo-code, comments and internal
identifier names are not.

## What this request deliberately does not ask for

- Any reading of Winamp's source, SDK headers or disassembly by an implementer
  or by the orchestrator.
- A spec for the **plugin ABI**: S09 already carries it as facts, and the host
  is implemented against it.
- A spec for the **IPC** or the **shortcuts/menus**: S10, S03, S04 and S05 all
  exist and are implemented.
