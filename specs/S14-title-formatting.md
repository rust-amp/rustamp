---
Spec ID: S-14
Title: Title formatting
Sources consulted (category only): dirty room: user-facing help text of the input plug-ins shipped with the reference; dirty room: string table of the reference binary
Author agent: orchestrator session (dirty-room role under RFC 0021, owner-authorised 2026-10-20)
Auditor: spec auditor subagent
Audit date: 2026-10-20
Dirty-room authorisation: RFC 0021 — the draft was authored in the dirty-room role, it is kept outside this repository, and no code was written from the dirty material.
Winamp version validated against: 2.95 (winamp.exe SHA-256 bca7a9fe1626b487d1b47bb5210670cf572d1a4cce23062076641d583b268561)
Status: audited
---

# S14 — title formatting

## 0. How to read this spec

**The reference binary was not run for this spec.** Everything below is read out
of the help text that the shipped input plug-ins carry and out of the
reference's own strings. No behaviour was observed at runtime, so nothing here
is a measurement. Where a statement goes beyond what a help text or a string
can carry on its own, it is marked **reading** — a hypothesis, never a fact to
implement.

| Where it comes from | What it means |
|---|---|
| §1 | **String fact** — a fact about what the reference and its shipped plug-ins carry: which plug-ins ship the format language, and which tokens the player's own strings do not carry. |
| §2.1–§2.3 | **Help-text fact** — a behaviour the shipped plug-ins' own user-facing help text documents. That help text is the documentation a user reads; it is paraphrased here, never copied. |
| §2.4 | **Example fact** — a format string as it stands. |
| a statement marked **reading** | A reading of a fact, not observed behaviour. |
| §4 | **Open** — not established here; the measurement that would settle it is named. |

Nothing in this spec is source code, a transliterated pseudo-code, a copied
comment, an internal identifier or a disassembly listing.

**Scope.** This spec is about the title-format *language* that the shipped
input plug-ins carry. It settles what S12 §10 item 10 left open — the token
and function set of that language. It is **not** about a playlist-view format,
a preferences page, or the numbered `%1`–`%7` placeholder style; none of those
is covered here (§4, open question 8).

## 1. Where the formatting lives — it is not the player's

Facts:

- The reference has **no global title-format setting**: none of the keys the
  player itself recognises is a format string (the recognised key set is S12
  §5.1, and it carries no such key).
- The format language is a feature of **individual input plug-ins**. A plug-in
  that has it keeps its own format string in **its own section** of
  `winamp.ini`, beside that plug-in's other keys; the player does not interpret
  a plug-in's keys (S12 §8). The one format string the observed file carried
  is the module decoder's own key `cfg_title_format` (S12 §8.3).
- Of the input plug-ins the reference ships, two carry the language and two do
  not (the table below).
- The player's own strings carry **no field token and no function token** of
  this language.

| Plug-in | Carries the format language |
|---|---|
| the module decoder | yes |
| the Vorbis decoder | yes |
| the MP3 decoder | no |
| the WAVE decoder | no |

**Reading:** the playlist's text is whatever the input plug-in reports, and a
plug-in that carries the language can be configured to report something more
elaborate. The player's own silence about tokens is consistent with the player
leaving a reported title alone, but it does not prove that — see §4, open
question 9.

## 2. The language

The two plug-ins that carry the language ship the same help text, so it is one
dialect, and the function list of §2.2 is the complete set that help text
carries.

### 2.1 Fields

A field token is written `%name%` and inserts the field called `name`. Any tag
name is accepted — the help text's own example names an artist tag.

The field names the material carries: `title`, `filename`, `artist`, `album`,
`performer`, `part`, `tracknumber`, `comment`, `streamtitle` — **nine** names.
Eight of them appear in the strings of §2.4; `comment` appears in none of them,
so where that name was seen is open (§4, open question 10).

A field that the current track does not carry yields the **empty** string. The
conditional group of §2.3 is how a format hides that gap.

### 2.2 Functions

| Function | Behaviour |
|---|---|
| `$abbr(x)` | abbreviates `x`. What an abbreviation is remains open (§4, open question 1); the help text's example abbreviates an album name. |
| `$abbr(x,y)` | abbreviates `x` only when it is longer than `y` characters, otherwise inserts `x` unchanged |
| `$lower(x)`, `$upper(x)` | lower-cases / upper-cases `x` |
| `$num(x,y)` | inserts the number `x` padded with leading zeros to `y` characters — the documented use is track numbers, e.g. `$num(%tracknumber%,2)` |
| `$caps(x)` | upper-cases the first letter of every word and lower-cases the rest |
| `$caps2(x)` | like `$caps`, but leaves letters that are already upper-case alone |
| `$if(A,B,C)` | if `A` contains **at least one valid field**, inserts `B`, otherwise `C` |
| `$if2(A,B)` | equal to `$if(A,A,B)` |
| `$longest(A,B,…)` | inserts whichever of the arguments produces the **longest output string** |
| `$pad(x,y)` | pads `x` with spaces up to `y` characters |
| `$cut(x,y)` | truncates `x` to `y` characters |
| `$padcut(x,y)` | pads `x` to `y` characters and truncates it when longer |

**Twelve** function names, `$abbr` in two forms. `$if` and `$if2` take field
arguments; the others take strings.

Nesting is supported, and the strings of §2.4 use it: `$abbr` sits inside a
conditional group, inside another conditional group.

### 2.3 The conditional group

Text in square brackets is inserted **only when at least one valid field inside
it was found**; otherwise the whole block, brackets included, disappears.

The help text's own example is the rule in one line:

```
%artist% - [%album% / ]%title%
```

The same rule written as a function is `$if(A,A,)`: §2.2's `$if` row inserts
its second argument when the first holds a valid field and its third otherwise,
so `$if(A,A,)` inserts `A` when `A` holds a valid field and the empty string
otherwise — exactly what the bracket group does.

### 2.4 The strings this spec carries

**Four** format strings are available: two plug-ins' own defaults and two from
the help text.

| Where | Format string |
|---|---|
| the module decoder's own default, observed in `winamp.ini` (S12 §8.3) | `$if2(%title%,%filename%)[ (%part%)]` |
| the Vorbis decoder's own default | `[$if2(%performer%,%artist%) - ]$if2(%title%,%filename%)` |
| the help text's illustration | `[%artist% - ][$abbr(%album%,10)[ %tracknumber%] / ]%title%[ %streamtitle%]` |
| the help text's example of the conditional group (§2.3) | `%artist% - [%album% / ]%title%` |

What each one exercises:

| String | Fields it uses | `$if2` | `$abbr` | Conditional group |
|---|---|---|---|---|
| the module decoder's default | title, filename, part | yes | – | yes |
| the Vorbis decoder's default | performer, artist, title, filename | yes | – | yes |
| the help text's illustration | artist, album, tracknumber, title, streamtitle | – | yes | yes (nested) |
| the conditional group's example | artist, album, title | – | – | yes |

Between them the four strings use **eight** of the nine field names of §2.1
and **two** of the twelve function names of §2.2. The other ten function names
appear only in the help text's list, not in any of the strings.

## 3. Practical notes for an implementer

- The language belongs to the plug-in that reports the title. **Reading** (the
  consequence §1's facts point at): an implementation that wants the feature
  attaches it to that plug-in and gives it its own setting beside the
  plug-in's other keys (S12 §8). Do not invent a global formatting engine —
  the reference has no global title-format setting.
- A missing field yields the **empty** string (§2.1). The group hides the gap;
  it does not fill it.
- The conditional group is a group, not a delimiter: the brackets disappear
  together with the block (§2.3).
- The four strings of §2.4 are the available test vectors — two plug-in
  defaults and two help-text examples.
- Everything in §4 is unknown. A behaviour that §4 lists must not be
  implemented as if it were settled.

## 4. Open questions

1. **The exact rule of `$abbr`** — does it take the first letters of the
   words, a fixed prefix, or a single letter? The help text says only
   "abbreviation". Measurable by configuring `$abbr(%album%)` with a known
   album and reading the reported title.
2. **`$num` with a non-numeric field** (empty string, or text).
3. **`$caps`/`$caps2` on non-ASCII text** — locale dependent, needs a probe.
4. **`$cut`/`$pad` on a value that contains the separator characters** (a `]`
   inside a tag, an unmatched bracket) — the escaping rule is unknown.
5. **Whether an unknown function name is inserted literally or drops the tail.**
6. **The maximum format-string length** each plug-in accepts.
7. **Whether the player's own title show uses any of this** when the plug-in
   reports a plain "artist - title" — the MP3 decoder has no language, so a
   playlist row for an MP3 is whatever that plug-in reports; whether the player
   post-processes it was not established.
8. **The numbered placeholder style `%1`–`%7`.** §1 records that the player's
   own strings carry no field or function token, and no plug-in's help text
   documents a numbered placeholder; the set and its mapping are not
   established here. The repo's spec request for S14 asks for exactly this set
   (`docs/spec-requests/2026-10-20-missing-specs.md` §5, question 1).
9. **Whether the absence of tokens in the player's own strings settles
   question 7.** A string table that carries no token does not prove that the
   player leaves a reported title untouched; a runtime observation is needed.
10. **Where the `comment` field name was seen.** It is in the field list of
    §2.1 but in none of the four strings of §2.4, so the list's own claim
    about its origin does not hold for that name.
11. **What counts as a "valid field"** for `$if` and for the conditional group:
    a tag the track carries, or a field whose value is non-empty. §2.1 says a
    field the track does not carry yields the empty string; the two readings
    differ for a tag that is present but empty.
12. **Where the shipped defaults live.** The module decoder's default was
    observed in `winamp.ini` (S12 §8.3); the Vorbis decoder's default was not
    observed in that file — the observed file's plug-in sections are those of
    the WAV output driver, the module decoder and the WAVE decoder (S12 §2).
    Whether the Vorbis default lives in the plug-in's own resources or in a
    section that the observed run never wrote is open, and so is whether other
    shipped plug-ins carry further defaults.
13. **How `$longest` breaks a tie** between two arguments of equal output
    length.
