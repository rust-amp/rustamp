---
Spec ID: S-12
Title: winamp.ini keys and plugin.ini conventions
Sources consulted (category only): dirty room: a configuration file written by the reference binary during a scripted run; dirty room: plug-in folder listing; dirty room: disassembly of the configuration reader (owner-authorised under RFC 0021)
Author agent: orchestrator session (dirty-room role under RFC 0021, owner-authorised 2026-10-20)
Auditor: spec auditor subagent
Audit date: 2026-10-20
Dirty-room authorisation: RFC 0021 — the draft was authored in the dirty-room role, it is kept outside this repository, and no code was written from the dirty material.
Winamp version validated against: 2.95 (winamp.exe SHA-256 bca7a9fe1626b487d1b47bb5210670cf572d1a4cce23062076641d583b268561)
Status: audited
---

# S12 — winamp.ini keys and plug-in settings

The index title of this spec says "plugin.ini conventions" because that was the
open question. The answer is in §2: there is no separate `plugin.ini` in the
observed run — a plug-in's own keys live in the player's `winamp.ini`, in a
section named after the plug-in.

## 0. How to read this spec

Every statement here is about one configuration file and about the player's own
key table. Nothing in this spec is source code, a transliterated pseudo-code, a
copied comment or a disassembly listing.

| Where it comes from | What it means |
|---|---|
| §2, §3, §8 | **File fact** — read out of the `winamp.ini` the reference wrote: a key name, a value, the file's structure. |
| §4–§7 | **Reader fact** — taken from the player's own key table: a constant, a range or a rule that says what the program does with the file. |
| a row marked **inferred** | A reading of a key's name. Not observed behaviour: a hypothesis to be measured, never a fact to implement. |

Rules for an implementer:

- No key in §3 was observed to change the running program: the file was read,
  not exercised. The **inferred** rows name the readings that were explicitly
  flagged as guesses; the unmarked rows follow the key's name or an audited
  spec named in the row, and are equally unmeasured as behaviour.
- A key this spec does not mention is not covered by it. The key set comes from
  one scripted run (§10, open question 1).
- Where this spec's own counts do not add up (§5.3), the lists are the fact and
  the counts are open.

## 1. How this spec was obtained

Three artifacts stand behind this spec:

1. **One `winamp.ini` that the reference binary wrote itself** during a scripted
   run. It gives §2, §3 and §8.
2. **The listing of the plug-in folder** next to it. It gives the two plug-ins
   that §8 compares.
3. **The player's own key table** — the set of names the configuration reader
   recognises, together with the rules it applies to them — taken from the
   reference binary (a disassembly of the reader path, owner-authorised under
   RFC 0021). It gives §4–§7.

No source code, no SDK material and no external documentation were consulted.
For the file facts the file is the only artifact, so it records the state the
reference saved in that run — not necessarily the state of a fresh interactive
installation. The observed file is 2288 bytes: 76 keys in `[Winamp]` plus four
plug-in sections. Its `cwd` value names a temporary directory, which is what
marks the run as scripted rather than a fresh interactive install.

## 2. Shape of the file

- An INI file: `[Section]` headers, `key=value`, one pair per line, CRLF line
  endings, no quoting, no escaping. An empty value is written as `key=`
  (e.g. `last_shortdesc=`).
- The first section is `[Winamp]` and holds **all** the player's own settings.
- Every further section belongs to **one** plug-in and is named by the plug-in's
  own display name, not by its file name: `[WAV Writing Output Driver]` is
  `out_wave.dll`, `[Nullsoft Module Decoder]` is `in_mod.dll`, `[IN_WAVE]` is
  `in_wave.dll`.
- A plug-in's private keys therefore live in the player's `winamp.ini`; the
  observed run shows no separate `plugin.ini`.
- Lists are comma-separated without spaces (`eq_data=31,31,…,31`).
- Colours and binary blobs are hex strings without a prefix (`cfg_wfx=0100…`).
- Window positions are stored as an x then a y pair in `*_wx` / `*_wy` keys;
  sizes as `*_width` / `*_height`.

## 3. `[Winamp]` — observed keys

### 3.1 Skin and output

| Key | Observed | Meaning |
|---|---|---|
| `skin` | `base-2.91.wsz` | the skin file name (not a path) |
| `outname` | `out_wave.dll` | the selected output plug-in, by file name |
| `randskin` | `0` | **inferred**: pick a random skin at start |

### 3.2 Volume, balance, EQ

| Key | Observed | Meaning |
|---|---|---|
| `volume` | `0` | the volume, `0..=255` (S07 §5's slider law) |
| `pan` | `0` | the balance; see the note below |
| `use_eq` | `1` | the EQ's enabled state (the **On** button); S01 §3.1 links the ON indicator to this key and states that the link was not independently re-verified |
| `autoload_eq` | `1` | the EQ's auto-load state (the **Auto** button); S01 §3.1 measured that this key toggles exactly the AUTO indicator |
| `preamp` | `31` | the preamp in the slider's saved unit, `0..=63` (S01 §3.1: the thumb top sits at `y = 38` at value 0 and at `y = 89` at value 63) |
| `eq_data` | `31,31,31,31,31,31,31,31,31,31` | ten band values in the same unit; the count matches the equalizer's ten bands (S01 §3). Parsing and clamping: §6 |

**Note on `pan`.** `0` with `volume=0` is consistent with "centred", but one
sample fixes neither the range nor the direction of the key. The observable
that settles it is the main window's marquee hint: S02 §2.1 lists the hint row
as `BALANCE: <CENTER or side>`, and S07 §6 has since measured the control
itself — the left extreme reads `100% LEFT`, the centre reads `CENTER`, and the
right half was never reached. The engine-side value domain (signed, ±100 %
versus ±127) and the file's own encoding of the value are still open (S07 open
item O4). To settle the key: drive the balance with `WM_COMMAND` and read the
marquee back, then compare what the program writes to the file.

### 3.3 Playlist and transport

| Key | Observed | Meaning |
|---|---|---|
| `shuffle` | `0` | shuffle on/off (S15) |
| `repeat` | `0` | repeat on/off |
| `pladv` | `1` | **inferred**: advance the playlist when a track ends; the observed value `1` is the advancing state |
| `pe_fontsize` | `10` | the playlist font size |
| `shownumsinpl` | `1` | **inferred**: show the track number column in the playlist |
| `timeleftmode` | `0` | **inferred**: the time display shows elapsed (0) or remaining (1) |
| `autoscrollname` | `1` | **inferred**: scroll the title in the main window |
| `pilp` | `0` | unknown — no meaning was established; **open** |

### 3.4 Windows: position, size, shade, visibility

| Key | Observed | Meaning |
|---|---|---|
| `wx` | `26` | main window x |
| `wy` | `29` | main window y |
| `eq_wx` | `26` | equalizer x |
| `eq_wy` | `145` | equalizer y |
| `pe_wx` | `26` | playlist x |
| `pe_wy` | `261` | playlist y |
| `mb_wx` | `301` | minibrowser x |
| `mb_wy` | `29` | minibrowser y |
| `mb_width` | `350` | minibrowser width |
| `mb_height` | `348` | minibrowser height |
| `video_wx` | `26` | video window x |
| `video_wy` | `145` | video window y |
| `video_width` | `275` | video window width |
| `video_height` | `232` | video window height |
| `pe_width` | `275` | the playlist's width, unshaded |
| `pe_height` | `116` | the playlist's height, unshaded |
| `pe_height_ws` | `0` | **inferred**: the playlist's height while shaded |
| `windowshade` | `0` | main window shaded |
| `eq_ws` | `0` | equalizer shaded |
| `video_open` | `0` | video window visible |
| `mb_open` | `1` | minibrowser visible |
| `eq_open` | `1` | equalizer visible |
| `pe_open` | `1` | playlist visible |
| `mw_open` | `1` | main window visible |
| `minimized` | `0` | main window minimised |
| `aot` | `0` | always on top |
| `dsize` | `0` | double size |
| `easymove` | `1` | easy move |

Note the asymmetry: the playlist has both an unshaded size (`pe_width`,
`pe_height`) and a shaded height (`pe_height_ws`), while the main and equalizer
windows have only positions and one shade flag — their sizes are fixed by the
skin (S01 §1).

### 3.5 Spectrum analyser (input for S08)

| Key | Observed | Meaning |
|---|---|---|
| `sa` | `1` | **inferred**: the spectrum analyser is on (0 = oscilloscope) |
| `safire` | `4` | unknown — **open** |
| `saref` | `2` | unknown — **open** |
| `safalloff` | `2` | **inferred**: the bar fall-off speed, a small enumerated choice |
| `sa_peaks` | `1` | **inferred**: draw peak markers |
| `sa_peak_falloff` | `1` | **inferred**: the peak markers' fall-off speed |

These six keys are the player's own settings for the built-in analyser, so S08
can take the **setting names and the observed values** as file facts. Their
**value ranges** are not established by this one sample, and the mapping from a
setting to pixels and frames still needs a measurement; both belong to S08's
open list.

### 3.6 Video window (input for S11)

| Key | Observed | Meaning |
|---|---|---|
| `video_ratio1`, `video_ratio2`, `video_useratio` | `4`, `3`, `0` | the aspect ratio as a fraction (the first two), and whether the user overrode it (the third) |
| `video_vsync` | `0` | vertical sync |
| `video_aspectadj` | `1` | adjust the aspect ratio on resize |
| `video_overlays` | `1` | overlays |
| `video_ddraw` | `1` | DirectDraw |
| `video_updsize` | `1` | **inferred**: resize the window to the video |
| `video_autoopen` | `1` | open the window on video |
| `video_autoclose` | `1` | close it when the video ends |
| `video_noss` | `1` | **inferred**: disable screen savers while playing video |
| `video_osd` | `1` | on-screen display |
| `video_yv12` | `1` | the YV12 surface format |
| `video_stopclose` | `1` | **inferred**: close on stop |
| `video_remove_fs_on_stop` | `0` | **inferred**: leave fullscreen on stop |

### 3.7 Miscellaneous

| Key | Observed | Meaning |
|---|---|---|
| `cwd` | a temporary directory | the working directory at exit |
| `last_shortdesc`, `last_shorturl` | empty, empty | the last "shortcut" the player offered — a short description and a URL, both empty in the observed run |
| `prefs_last_page` | `0` | the last Preferences page index; the key carries a second number as well (§7) |
| `newverchk`, `newverchk2` | `1`, `1` | **inferred**: update checks (two separate ones) |

## 4. How the player reads a key

### 4.1 The names

- The names the player looks up carry a seven-character `config_` prefix;
  **the name in the file is the name without that prefix**. `config_volume` reads
  `volume=`, so a file that wrote `config_volume=` would not be found.
- Two kinds of read exist, one for integers and one for strings. Both follow
  the prefix rule above.

### 4.2 A missing key

A key that is not in the file yields the **default** that the caller passes, and
that default is the player's own initial value for the setting — not a value
read from the file. A missing key therefore leaves the program at its own
starting value.

### 4.3 String values

A string value is read through a 2048-byte staging buffer, and the caller
passes its own size; a value longer than the buffer is truncated by the profile
API. In practice a value beyond 2048 bytes is cut short.

## 5. The complete recognised key set

### 5.1 The list

The player recognises the following keys, spelled as they appear **in the
file** (that is, without the `config_` prefix). This is the complete set the
player recognises; which subset it *writes* is still open (§10, open question
1).

| Group | Keys |
|---|---|
| skin, output, cwd | `skin`, `outname`, `randskin`, `cwd`, `defext` |
| main window | `wx`, `wy`, `mw_open`, `minimized`, `aot`, `dsize`, `easymove`, `windowshade`, `keeponscreen`, `snap`, `snaplen`, `parent`, `hilite` |
| equalizer | `eq_wx`, `eq_wy`, `eq_open`, `eq_ws`, `eqdsize`, `use_eq`, `autoload_eq`, `preamp`, `eq_data` |
| playlist | `pe_wx`, `pe_wy`, `pe_open`, `pe_width`, `pe_height`, `pe_height_ws`, `pe_fontsize`, `shownumsinpl`, `pladv`, `addtolist` |
| minibrowser | `mb_wx`, `mb_wy`, `mb_open`, `mb_width`, `mb_height` |
| video window | `video_wx`, `video_wy`, `video_open`, `video_width`, `video_height`, `video_ratio1`, `video_ratio2`, `video_useratio`, `video_vsync`, `video_aspectadj`, `video_overlays`, `video_ddraw`, `video_updsize`, `video_autoopen`, `video_autoclose`, `video_noss`, `video_osd`, `video_yv12`, `video_stopclose`, `video_remove_fs_on_stop` |
| transport | `volume`, `pan`, `shuffle`, `repeat`, `shuffle_morph_rate`, `timeleftmode`, `autoscrollname` |
| spectrum analyser | `sa`, `safire`, `saref`, `safalloff`, `sa_peaks`, `sa_peak_falloff` |
| plug-ins | `visplugin_num`, `visplugin_name`, `visplugin_autoexec`, `visplugin_priority`, `dspplugin_num`, `dspplugin_name` |
| preferences, misc | `prefs_last_page`, `last_shortdesc`, `last_shorturl`, `proxy`, `inet_mode`, `browserbrand`, `langpack`, `extlist`, `newverchk`, `newverchk2`, `check_ft_startup`, `usecursors`, `fixtitles`, `ttips`, `ascb_new`, `taskbar`, `splash`, `sticon`, `riol`, `minst`, `whichicon`, `whichicon2`, `disvis`, `priority`, `ospb`, `bifont`, `rofiob`, `pilp` |

### 5.2 The keys the observed file did not carry

The reference can leave a recognised key out entirely. The observed file did
**not** carry these: `bifont`, `eqdsize`, `check_ft_startup`, `usecursors`,
`fixtitles`, `keeponscreen`, `snap`, `snaplen`, `parent`, `hilite`, `disvis`,
`minst`, `riol`, `ttips`, `ascb_new`, `taskbar`, `splash`, `sticon`,
`whichicon`, `whichicon2`, `addtolist`, `rofiob`, `priority`, `ospb`, `proxy`,
`inet_mode`, `langpack`, `extlist`, `defext`, `browserbrand` — and all six
plug-in-selection keys.

### 5.3 Counts, and where they do not add up

The lists are the fact; the counts that go with them do not add up, and the
difference is open (§10, open question 9).

| Count | Value |
|---|---|
| Names in the recognised list of §5.1 | 109 |
| Names in the absent list of §5.2 | 30 + the six plug-in-selection keys = 36 |
| Recognised keys therefore present in the file | 109 − 36 = 73 |
| Keys of `[Winamp]` that §3 itemises by name | 72 |
| Keys the file held in `[Winamp]` | 76 |

The three figures that disagree: the recognised list holds 109 names while the
draft's prose gave the same set as 105; `shuffle_morph_rate` is recognised and
not in the absent list, yet §3 does not itemise it; and three further keys of
the file are named neither in §3 nor in the recognised list. A recount in the
dirty room is needed before an implementer relies on any of these numbers.

## 6. `eq_data` — parsing and clamping

The ten band values are parsed from the comma-separated string with these
behavioural guarantees:

- exactly **ten** values are taken; the parse stops at the first non-numeric
  character;
- a value below 0 becomes **0**, a value above **63** becomes **63**;
- a missing or unparsable entry leaves the band at whatever it was — the parse
  stops, it does not skip to the next entry;
- the separator counts as one character, so `1,2,3` and `1,,3` behave
  differently: an empty entry stops the parse.

## 7. `prefs_last_page` — two numbers in one key

The one integer that carries **two** numbers: its **low byte** is the
Preferences page index restored at start, and the bits from bit 8 upwards carry
a second number, which the player keeps separately. The second number's
meaning was not established — **open**.

## 8. Plug-in sections

Each plug-in's section is named by its display name and its keys are the
plug-in's own; the player does not interpret them. Two facts are worth
recording because they constrain our host.

### 8.1 The section name

- **Observed:** the section name is the plug-in's display name (§2): `[WAV
  Writing Output Driver]`, `[Nullsoft Module Decoder]`, `[IN_WAVE]`.
- **Reading:** two plug-ins with the same display name would therefore collide.
  The observed file contains no disambiguation and gives no sign of a guard
  against the collision; this was not tested.

### 8.2 When a section is written

- **Observed:** `out_wave.dll` is selected and has a section; `out_ds.dll` sits
  in `Plugins/` and has none.
- **Reading:** a plug-in's keys are written only while the plug-in is loaded, so
  a section's presence is not a registry of installed plug-ins.

### 8.3 The title format string

One observed key is a **title format string**:

```
[Nullsoft Module Decoder]
cfg_title_format=$if2(%title%,%filename%)[ (%part%)]
```

It was observed in the module decoder plug-in's own section, so it is that
plug-in's format string. Whether the player itself uses the same formatting
language for its window title is not established here; that is S14's subject.

Behavioural facts this one string establishes:

- The language is `$function(...)` calls over `%field%` tokens.
- `$if2(a,b)` yields `a` when `a` is non-empty, otherwise `b`.
- `%title%` and `%filename%` are fields; `%part%` is a third field.
- `[ ... ]` is a conditional group: it renders only when the fields inside it
  have values.

The full token and function set is **not** established by this one string; it
needs its own measurement, and S14 stays open.

## 9. Practical notes for an implementer

- Parse the file as a plain INI: values run to the end of the line, there is no
  quoting and no escaping.
- Write the key names **without** the `config_` prefix (§4.1).
- Round-trip what you read: preserve keys outside the recognised set of §5 as
  well, and preserve unknown sections. The file held more keys than the player
  recognises (§5.3), and the plug-in-section write rule is not established
  (§10, open question 5), so a writer must not drop what it did not understand.
- Clamp only where a range is known: `volume` is `0..=255`, `preamp` is
  `0..=63`, and an `eq_data` value is clamped to `0..=63` with exactly ten
  entries taken (§6). Every other numeric key's range is open (§10, open
  question 3).
- A value longer than 2048 bytes is cut short (§4.3).
- The file is written by the program itself. Whether it rewrites the whole file
  or patches it on exit is open (§10, open question 6), so treat both as
  possible.

## 10. Open questions

1. **Which subset of the recognised keys the player writes.** The set of keys it
   *knows* is settled (§5); the observed file held 76 keys, of which the lists
   account for 73, and one scripted run cannot say whether the rest are written
   only after a change, written by a different path, or never written.
2. **`pilp`, `safire`, `saref`** — recognised and observed, meaning unknown.
3. **Ranges and clamping for every numeric key.** Anchor points: `volume`
   (0..255), `preamp` (0..63), `eq_data` (0..63, ten values, §6). The rest are
   unestablished.
4. **`pan`** — range and direction.
5. **The plug-in-section write rule** (does a plug-in's section survive
   deselection? this file cannot show it).
6. **Whether the player rewrites the whole file or patches it** on exit (the
   mtime of untouched keys would show it).
7. **The high part of `prefs_last_page`** (§7).
8. **The defaults** the integer read falls back to. The read returns the
   caller's default, and that default is the player's own initial value; it is
   not stated here because it was not read. A fresh install's file would show
   the same values, and that is the cheapest measurement for them.
9. **The counts in §5.3.** The recognised list holds 109 names (the draft's
   prose gave 105), the file held 76 keys, §3 itemises 72 of them by name, and
   three keys of the file are named nowhere. A recount in the dirty room is
   needed.
10. **The title-format language beyond the four facts in §8.3** — the full
    token and function set is not established by this one string. S14 carries
    it.
11. **The behaviour of every key in §3.** None of them was observed to change
    the running program in this session; the **inferred** markers name the
    readings that were explicitly flagged as guesses, the unmarked rows are
    equally unmeasured as behaviour.
