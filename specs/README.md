# Specs

This directory holds the clean-room specifications that implementers work from. A spec describes observable behavior, timings, coordinates, constants and ABI facts. It contains no code and no source-derived pseudo-code. The rules for what a spec may contain are in [CLEANROOM.md](../CLEANROOM.md), section 4.

Specs reach this directory only after the Spec Auditor has signed off. Each file starts with the provenance header below.

## Index

| ID | Title | Status |
|---|---|---|
| S01 | Window geometry, sprite rects and pixel layout deltas | audited |
| S02 | Main window behaviors (clutterbar, title marquee, time display, kbps/kHz, mono/stereo, button states) | audited |
| S03 | Keyboard and mouse shortcut table, per window | audited |
| S04 | Menu trees (main, right-click, playlist buttons, EQ presets) | not started |
| S05 | Docking and snapping rules | not started |
| S06 | Playlist editor (selection, drag-reorder, scrolling, search, resize steps) | not started |
| S07 | Equalizer (band centers, filter design, slider-to-dB mapping, preamp, `.eqf` formats) | not started |
| S08 | Classic visualization (spectrum, oscilloscope, 576-sample data for plugins) | audited |
| S09 | Plugin ABI (in_/out_/vis_/dsp_/gen_ structs, versions, threading, call order) | audited – static; runtime confirmation pending |
| S10 | `WM_WA_IPC` and `WM_COMMAND` catalog | not started |
| S11 | Embed-window API and `gen.bmp` layout | audited |
| S12 | winamp.ini keys and plugin.ini conventions | audited |
| S13 | Command-line switches, single instance, drag-and-drop rules | audited |
| S14 | Title formatting | audited |
| S15 | Shuffle algorithm | not started |
| S16 | Preferences pages and options | not started |
| S17 | Skin-loading quirks (missing files, palette and BMP quirks, nums_ex fallback, pledit.txt, region.txt) | audited |
| S18 | Output behaviors (fades, buffer, prebuffer) | not started |
| S19 | Streaming and ICY metadata display | audited |
| S20 | Media Library (2.9x) and CD playback (later milestones) | not started |

## D-notes

D-notes are behavior observations about foobar2000's pipeline. They describe what foobar2000 does, not how it is built, and they copy nothing.

| ID | Title | Status |
|---|---|---|
| D01 | foobar2000 component model, ReplayGain, gapless and seeking strategy (behavior only) | not started |

## Provenance header template

Every spec file starts with this header. Fill in all fields. Use categories for sources, not file names or paths.

```markdown
---
Spec ID: S-xx
Title: <short title>
Sources consulted (category only): <e.g. dirty room: binary behavior capture; dirty room: disassembly; free: webamp>
Author agent: <agent name or ID, dirty room>
Auditor: <Spec Auditor agent name or ID>
Audit date: YYYY-MM-DD
Winamp version validated against: 2.95 (<build or hash of the binary used, if known>)
Status: <draft | audited | validated>
---
```

Rules for the header:
- `Winamp version validated against` must name 2.95 for any behavioral spec. A spec that cites only 5.x sources is not validated (see CLEANROOM.md, section 6).
- `Audit date` is empty until the Spec Auditor signs off. A file without an audit date does not belong here.
