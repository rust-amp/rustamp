---
Spec ID: S-20
Title: Media Library plug-in (2.9x)
Sources consulted (category only): dirty room: string table of the media-library plug-in shipped with the reference; dirty room: plug-in folder listing
Author agent: orchestrator session (dirty-room role under RFC 0021, owner-authorised 2026-10-20)
Auditor: spec auditor subagent
Audit date: 2026-10-20
Dirty-room authorisation: RFC 0021 — the draft was authored in the dirty-room role, it is kept outside this repository, and no code was written from the dirty material.
Winamp version validated against: 2.95 (winamp.exe SHA-256 bca7a9fe1626b487d1b47bb5210670cf572d1a4cce23062076641d583b268561)
Status: audited
---

# S20 — the media library plug-in

## 0. How to read this spec

**The reference was not run for this spec, and the plug-in was not loaded.**
Everything below is read out of the **user-facing strings** of the media-library
plug-in that the reference ships, and out of the **listing of the plug-in
folder**. No behaviour was observed at runtime, so nothing here is a
measurement. Where a statement goes beyond what a string can carry on its own,
it is marked **reading** — a hypothesis, never a fact to implement.

| Where it comes from | What it means |
|---|---|
| §1–§6 | **String fact** — a spelling the plug-in carries: a caption, a view name, a prompt, a message, a configuration key name, a file name, a template string. |
| a statement marked **reading** | A reading of a name or of a message — what a string implies, not what was observed. |
| §7 | **Practical note** — a consequence that follows from the spec's own facts; nothing new. |
| §8 | **Open** — not established here; the measurement that would settle it is named. |

Nothing in this spec is source code, a transliterated pseudo-code, a copied
comment, an internal identifier or a disassembly listing. What the audit
changed, and what it found, is in `docs/notes/spec-audit-s20.md`.

**Scope.** This spec is about the **media library plug-in** that the reference
ships: the plug-in's identity, its views, its columns, its search, the playlist
it can extract from a selection, its persistent settings and its radio
directory. **CD playback is out of scope** — no CD path was examined, and
nothing in this spec describes one (§8, open question 7).

**This documents a plug-in — a peer feature, not a player contract.** The
reference's media library is a **separate general-purpose plug-in**; our own
library is a first-class part of the player. Everything below therefore
describes a **peer feature**. Where our library differs from it, that is our
own design and **not a deviation from the reference**.

**Answer to the open question in `docs/spec-requests/2026-10-20-missing-specs.md`
§9:** yes, 2.95 has a media library, and it is a **plug-in** — the reference
ships one that identifies itself as `Nullsoft Winamp Library v1.3`. It is not a
player feature, and it is not a page inside the player.

## 1. What the plug-in is

Facts:

- The reference ships a **general-purpose plug-in** whose name is
  `Media Library`; it identifies itself as `Nullsoft Winamp Library v1.3`.
- It carries the **window class name** `WinampLibraryPE`.
- It carries the **error-dialog caption** `Winamp Library Error`.
- It carries the two labels `Media Library` and `Toggle Library`.

Readings:

- A plug-in that registers a window under its own class has a window in its
  own right, not a page inside the player.
- The two labels are how the library is reached — but the plug-in's own string
  table cannot say **which control** carries them, nor whether the **player**
  carries the entry that opens the library (§8, open question 11).

## 2. Views

A fixed set of top-level views plus user-defined ones. The names the plug-in
carries:

| View | What the name says (reading) |
|---|---|
| `Local Media` | the indexed files on this machine |
| `Internet Radio` | the radio directory of §6 |
| `Playlists` | library-side playlists, not the player's playlist file |
| `New View` | a user-defined view, saved and named by the user |

Facts:

- A view can be **renamed** and **deleted**: the plug-in carries the two
  prompts `Rename playlist` and `Delete this view?`.
- The **last view** the window showed is remembered (§5, `last_view`).

Reading: the two prompts are the ones a rename and a delete put up. Note that
they use **two different words** — "playlist" in one and "view" in the other —
and the material does not say whether the two name the same object (§8, open
question 14).

## 3. Columns and the search

The plug-in knows these **seven field names**:

`title`, `artist`, `album`, `year`, `genre`, `track`, `ntracks`

Beside them it carries a **track-numbering mode** and a **column-resize mode**
(the setting behind `column_resize_mode`, §5).

Reading: `track` is the track number and `ntracks` the track count. What
`ntracks` counts — the album's tracks or the selected set — is not established
(§8, open question 10).

Facts:

- Searching shows `Searching... (N files found)`.
- The **column layout** — order, default widths, labels — is **not**
  established by the strings, and is open (§8, open question 1).

Reading: the count in the searching message is the number of files the search
matched, and it is known before the list is filled.

## 4. Extracting a playlist

The plug-in can write the current selection out as a **playlist**. Its own
wording carries:

| Where | String |
|---|---|
| the prompt before an existing playlist is overwritten | `Overwrite playlist (No will append)?` |
| the result of the extraction | `N tracks extracted` |
| the failure of one entry | `Error extracting track #N: <reason>` |

Reading: the prompt's own text says that answering "no" appends to the
existing file, and the per-entry failure text means one failed entry does not
abort the rest of the extraction.

### 4.1 The path template

The template strings the plug-in carries include `Artist - Album\Artist - Album`
and `## - Artist - Title`.

Reading: the backslash separates directories, so the template builds a folder
tree. Whether the backslash is literal on every platform or a separator that
follows the platform is open (§8, open question 8).

### 4.2 The template language

The template carries **bare words**, not `%name%` tokens. It is **not** S14's
language:

| | S14's title-format language | this template language |
|---|---|---|
| what it builds | the **title** an input plug-in reports | a **file-system path** for an extracted playlist |
| who owns it | the shipped **input** plug-ins (S14 §1) | the media-library **general-purpose** plug-in |
| a field is written as | `%name%` | a bare word, e.g. `Artist` |
| functions | `$function(...)` calls and `[ ... ]` groups (S14 §2.2, §2.3) | none is carried |
| the case of an inserted value | chosen by a function (`$upper`, `$lower`, S14 §2.2) | chosen by the **spelling of the word** — a reading, see below |

The tokens the plug-in carries:

| Token | Inserts (reading) |
|---|---|
| `Artist` | the artist with the default capitalisation |
| `ARTIST` | the artist in **all uppercase** |
| `artist` | the artist in **all lowercase** |
| `Album` / `ALBUM` / `album` | the album, the same three cases |
| `Title` / `TITLE` / `title` | the title, the same |
| `Genre` / `GENRE` / `genre` | the genre, the same |
| `year` | the album year |
| `#`, `##`, `###` | the track number, zero-padded to the number of `#` |

Facts about the table:

- The three spellings of a field are carried side by side — `Artist`,
  `ARTIST`, `artist` — and the same holds for album, title and genre.
- `year` is carried in **one** spelling only.
- The table covers **five** of the seven field names of §3. `track` and
  `ntracks` have **no word token**: the track number is addressed by the `#`
  forms.
- Whether the list is **complete** is not established (§8, open question 17).

Reading: the capitalisation of the word chooses the capitalisation of the
inserted value, and the number of `#` chooses the number of digits. The
material carries the three spellings; what the plug-in does with them is a
reading (§8, open question 12).

### 4.3 The sort failure message

The plug-in's error dialog carries the message

```
QSort failed and tried to update index outside range
```

This is a **user-facing message** — the library shows it in its own error
dialog (`Winamp Library Error`, §1). It is not an internal note and not a
debug text.

Reading: the failure it reports is a sort whose index bookkeeping went out of
range. The internal mechanism is not established and is not described here.

## 5. Persistent settings

The plug-in stores its own keys in the player's configuration file, in a
**section named after the plug-in** (S12 §8: a plug-in's section is named by
its display name, and **the player does not interpret the plug-in's keys**).

| Key | What the name says (reading) |
|---|---|
| `last_view` | the last view the window showed |
| `column_resize_mode` | how the columns resize |
| `viewplayonemode` | whether a view plays on a single click rather than a double click |
| `remembersearch` | whether the search box's content is remembered; remembered only when set |
| `radio_last_upd` | — see below |
| `maxradiostream` | the maximum number of radio streams |
| `radio_upd_freq` | how often the radio directory is updated |

Facts and cross-checks:

- There are **seven** keys. **None** of them is in the player's recognised key
  set (S12 §5.1) — which is what S12 §8 says to expect, because the player
  does not interpret a plug-in's keys.
- The names are spelled **without** a prefix. S12 §4.1 states the prefix rule
  for the **player's own** lookups: the reader looks a name up under a
  seven-character `config_` prefix, and the file carries the name without it.
  That rule does not transfer to a plug-in's own section — the one plug-in key
  the repo has observed in a file, `cfg_title_format` in
  `[Nullsoft Module Decoder]` (S12 §8.3), carries its prefix **in the file**.
  Whether the seven names above are the file's spellings is therefore **open**
  (§8, open question 13).
- The section the keys live in is not established either: the plug-in carries
  two candidate names, `Media Library` and `Nullsoft Winamp Library v1.3`
  (§8, open question 13).

`radio_last_upd` is in the list and used nowhere else in the material: what it
records is open (§8, open question 15).

## 6. The radio directory

Facts:

- The plug-in ships or loads a radio list file named `radio.vmd`.
- It carries the message `Fetching Playlist`.

Readings:

- It has an **update frequency** and a **maximum stream count** — the settings
  `radio_upd_freq` and `maxradiostream`. What the two mean is open (§8, open
  question 5).
- `Fetching Playlist` is the message it shows while a playlist is loaded from
  the radio directory rather than from disk.

Open: the **`radio.vmd` format** itself (§8, open question 5).

## 7. Practical notes for an implementer

- The library is a **separate plug-in with its own window** (§1). Our own
  library is a first-class part of the player, and the differences between the
  two are our own design (§0).
- Keep the plug-in's user-facing strings **verbatim** — they are the spellings
  the reference shows the user (§1–§6).
- The template language of §4.2 is **not** S14's language: bare words, no `%`
  and no `$`, no functions. Do not reuse S14's formatter for it.
- The three spellings of a field name are carried side by side. An
  implementation that wants the case rule has to implement it itself, and the
  rule itself is still a reading (§4.2, open question 12).
- None of the seven keys is the player's own (S12 §5.1); they belong to the
  plug-in's section, which the player does not interpret (S12 §8). Write them
  **without** a `config_` prefix only once the measurement settles the
  question — see open question 13.
- Everything in §8 is unknown. A behaviour that §8 lists must not be
  implemented as if it were settled.

## 8. Open questions

Kept from the draft, in the draft's order:

1. The **column order, default widths and labels** of the Local Media view.
2. The **default view set** and which views are created on a first run.
3. The **sort semantics** (per column, tie-breaks, case folding) — the
   `QSort` message shows that a sort exists, not how it orders.
4. The **user view's query language** ("New View") — this is the library's
   own language and nothing in the string table describes it.
5. The **`radio.vmd` format** and the meaning of `radio_upd_freq` /
   `maxradiostream`.
6. Whether the library **indexes on its own** (a background scan) or only on
   demand — the reference predates the "add a folder" model we implemented.
7. **CD playback** — out of scope for this spec; nothing was examined.
8. Whether the path template's **backslash** is literal on every platform or a
   directory separator that follows the platform.
9. **How the library's playlist and the player's playlist relate** (does an
   extracted playlist land in the player's playlist, on disk, or both?).
10. The **`ntracks`** column's meaning: the track count of the album, or of
    the selected set.

Added by the audit. Each follows from something the draft itself records, and
none of them is a fact the draft did not contain:

11. **Which control carries `Media Library` and `Toggle Library`**, and
    whether the **player** itself carries an entry that opens the library.
    The plug-in's own string table carries the two labels; it cannot place
    them in the player's menu. Settled by looking at the player with the
    plug-in loaded.
12. **The capitalisation rule of the template language.** §4.2 states it as a
    reading: the material carries `Artist`, `ARTIST` and `artist` side by
    side, but whether the spelling chooses the case of the inserted value —
    and what an unlisted spelling such as `Year` or `YEAR` does — is not
    established. Settled by extracting a playlist with a template that uses
    all three spellings and reading the file back.
13. **Whether the seven key names are the file's spellings, and in which
    section they live.** S12 §4.1's prefix rule governs the player's own
    lookups, and the observed plug-in key `cfg_title_format` (S12 §8.3)
    carries its prefix in the file. Settled by letting the plug-in write its
    configuration and reading the file back.
14. **Whether "playlist" and "view" name the same thing.** The rename prompt
    says `Rename playlist`, the delete prompt says `Delete this view?`.
15. **What `radio_last_upd` records.** It is in the key list of §5 and
    nowhere else in the material.
16. **The track-numbering mode** — its values and their effect.
17. **Whether the token list of §4.2 is complete**, and what `track` and
    `ntracks` do in a template (neither has a word token).
