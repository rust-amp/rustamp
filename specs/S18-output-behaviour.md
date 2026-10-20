---
Spec ID: S-18
Title: Output behaviors (fades, buffer, prebuffer)
Sources consulted (category only): dirty room: string table of the DirectSound output plug-in shipped with the reference; dirty room: a configuration file written by the reference
Author agent: orchestrator session (dirty-room role under RFC 0021, owner-authorised 2026-10-20)
Auditor: spec auditor subagent
Audit date: 2026-10-20
Dirty-room authorisation: RFC 0021 — the draft was authored in the dirty-room role, it is kept outside this repository, and no code was written from the dirty material.
Winamp version validated against: 2.95 (winamp.exe SHA-256 bca7a9fe1626b487d1b47bb5210670cf572d1a4cce23062076641d583b268561)
Status: audited
---

# S18 — the output plug-in's fades, buffer and prebuffer

## 0. How to read this spec

**The reference binary was not run for this spec, and no audio was played.**
Everything below is read out of the **user-facing strings** of the DirectSound
output plug-in that the reference ships, and out of the **configuration key
names** that plug-in carries. No behaviour was observed at runtime, so nothing
here is a measurement. Where a statement goes beyond what a string or a key
name can carry on its own, it is marked **reading** — a hypothesis, never a
fact to implement.

| Where it comes from | What it means |
|---|---|
| §1–§5 | **String fact** — a spelling the plug-in carries: a report line, a constraint sentence, an error message. |
| §2–§4 | **Key fact** — a configuration key name the plug-in carries, and the count of the keys. |
| a statement marked **reading** | A reading of a name or of a message — what a string implies, not what was observed. |
| §6 | **Practical note** — a consequence that follows from the spec's own facts; nothing new. |
| §7 | **Open** — not established here; the measurement that would settle it is named. |

Nothing in this spec is source code, a transliterated pseudo-code, a copied
comment, an internal identifier or a disassembly listing. What the audit
changed, and what it found, is in `docs/notes/spec-audit-s18.md`.

**Scope.** This spec describes the **output plug-in** — the DirectSound output
plug-in that the reference ships — and nothing else: its fade settings, its
buffer, its prebuffer, the report it shows and the messages it puts up. It is
**not** a description of the player. The player's own transport behaviour — the
transition from one track to the next, gapless playback, the player's own
volume path — is **not** covered here, and the one question about the player's
own fade stays open (§7, item 7). Where this spec says "the plug-in", it means
the DirectSound output plug-in; where it says "the player", it means the
player binary.

**What this spec does not settle: the crossfade between two tracks.** The
repo's own spec request (`docs/spec-requests/2026-10-20-missing-specs.md` §7)
asks for it; this spec can only report that the plug-in's own strings carry no
crossfade setting (§7, item 9), which is not the same as the reference having
none.

## 1. What the plug-in is

Facts:

- The reference ships a **DirectSound output plug-in** that identifies itself
  as **v2.2.6**.
- Its user-facing text comes from three surfaces: a **configuration page**, a
  **status report** and its **error messages**.
- It carries **eleven** configuration keys in all: six in the fade group (§2.1),
  three more knobs (§2.2), the buffer (§3) and the prebuffer (§4).

Reading: the plug-in owns the fades, the buffer and the prebuffer — they are
settings of the output module. What the **player** does with them is not
established here: the prebuffer policy stays open (§7, item 6), and so does
what the player does about the transition from one track to the next (§7, item
9).

Cross-check: **none** of the eleven keys is in the player's own recognised key
set (S12 §5.1), which is what S12 §8 says to expect — a plug-in's keys live in
the player's configuration file, in a section named after the plug-in, and the
player does not interpret them.

## 2. The fade settings

### 2.1 The fade group

| Key | What the name says (reading) |
|---|---|
| `cfg_fade_start` | the fade when playback starts |
| `cfg_fade_firststart` | the fade on the **first** start of a session; it is a separate key, so it can differ from `cfg_fade_start` |
| `cfg_fade_seek` | the fade on a seek |
| `cfg_fade_pause` | the fade on a pause — and, maybe, on its resume as well (§7, item 5) |
| `cfg_fade_stop` | the fade on a stop |
| `cfg_def_fade` | the fade used where an action has no value of its own |

Counts, taken from the table above:

- **five** of the six keys name a fade **per action** — start, first start,
  seek, pause, stop;
- the sixth, `cfg_def_fade`, is the fallback, not an action of its own;
- the fade group therefore holds **six** keys.

### 2.2 Three more knobs

- `cfg_logfades` — a toggle. The name says "logarithmic fades". **Reading:**
  the plug-in offers a choice of fade shape and this key selects the
  logarithmic one. The other shape is not named in the material — a linear
  counterpart is the natural guess, but nothing states it — and what the
  toggle does when it is off is open (§7, item 1).
- `cfg_fadevol` — **reading:** a volume the fade works against, separate from
  the main volume.
- `cfg_volume` — **reading:** the plug-in's own volume setting.

### 2.3 What the plug-in reports, and the one constraint it states

Facts:

- The plug-in reports the fade set as **"Fade on <action> settings (<N> ms)"**
  — the times are in **milliseconds**, one per action.
- The plug-in states this constraint in its own words (verbatim):

  ```
  Some fade times are bigger than buffer length; in order to get what you want, please increase buffer size to <N> ms.
  ```

**Reading:** a fade that is longer than the buffer length cannot be realised
inside that buffer, and the plug-in answers the conflict by naming the buffer
size it would need. It does not shorten the fade silently.

## 3. The buffer

Facts:

- `cfg_buf_ms` — the buffer length; the name says **milliseconds**.
- The plug-in reports the buffer as **"Active buffer size: <N> ms (<M> bytes)"**
  — a time and the byte size that time came to. **Reading:** the user sets the
  time, and the plug-in shows the byte size it ended up with.
- The status page shows a **write cursor** and a **playback cursor**, in
  bytes, and **"Buffer locks done: <N>"**. The two cursor lines' own wording
  is not recorded (§7, item 11). The lock count has no key behind it, so it is
  a counter the plug-in reports and not one of its settings (reading).
- The buffer report also carries **"Async buffer: <N> ms (<M> bytes)"** — a
  second buffer, reported next to the main one (reading).

## 4. The prebuffer

Facts:

- `cfg_prebuf2` — the prebuffer setting. The name says "prebuffer"; the
  trailing **2** is not explained by the material (§7, item 12).
- The status page reports **"Data buffered:"** and then **"Total: <N> ms
  (<M> bytes)"**.

**Reading:** the prebuffer is the amount of audio the plug-in wants to have
before it starts playing, so that a file does not begin with an immediate
underrun. Whether the plug-in fills the prebuffer before the first sample
leaves, and what it does when the device stalls, is open (§7, item 6).

## 5. The device, the mixing and the failures

Facts — the plug-in's report lines:

| What it reports | Text |
|---|---|
| the device | `Device: "<name>"` |
| the mixing mode | `Mixing: <a>, primary buffer: <b><c>` |
| the device list | `supported, <N> free streams (<M> max)` |

What the placeholders of the mixing line carry is not established (§7, item
11).

Facts — the failure messages, verbatim:

| Situation | Text |
|---|---|
| buffer creation | `Error creating DirectSound buffer.` |
| cooperative level refused | `Error setting DirectSound cooperative level; please shutdown other programs using the sound device` |
| device missing | `Device not found. Please select another device in configuration.` |
| bad driver | `Bad DirectSound driver. Please install proper drivers or select another device in configuration.` |
| no DirectSound | `DirectSound doesn't appear to be installed on this system.` |
| no device at all | `No DirectSound devices present. Please install soundcard drivers first.` |

**Reading:** four of the six messages tell the user what to do next; two only
name the failure — `Error creating DirectSound buffer.` and `DirectSound
doesn't appear to be installed on this system.`

## 6. Practical notes for an implementer

- Keep the plug-in's user-facing strings **verbatim** — they are the spellings
  the reference shows the user (§2–§5).
- The eleven keys belong to the output module's own configuration, not to the
  player's key set: none of them is in the player's recognised key set (S12
  §5.1), and S12 §8 says a plug-in's keys live in the player's configuration
  file, in a section named after the plug-in, and that the player does not
  interpret them.
- The `cfg_` prefix on these names is **consistent** with the one plug-in key
  the repo has observed in a file: `cfg_title_format`, in `[Nullsoft Module
  Decoder]`, carries its prefix **in the file** (S12 §8.3). S12 §4.1's prefix
  rule — the player looks its own names up under a seven-character `config_`
  prefix, and the file carries the name without it — governs the **player's
  own** lookups and does **not** transfer to a plug-in's section. Whether
  these eleven names are the file's spellings is still open (§7, item 10).
- The buffer and the prebuffer are the two values the player hands to an
  output plug-in: the output module's `open` takes a buffer hint and a
  prebuffer hint, and the ABI says the prebuffer must not exceed the buffer
  (S09 §4.1, item 9). The plug-in's two settings are therefore the two hints
  it is given (reading).
- Two volumes exist and they are not the same key: the player's own `volume`
  (S12 §3.2, the range `0..=255`) and the plug-in's `cfg_volume`. How the two
  compose is open (§7, item 8).
- Everything in §7 is unknown. A behaviour that §7 lists must not be
  implemented as if it were settled.

## 7. Open questions

Kept from the draft, in the draft's order:

1. **The fade curve(s)** — what `cfg_logfades` selects, and what the other
   shape is. The draft assumes two shapes; the key's name shows at most that a
   logarithmic shape is named.
2. **The default value of every key** — all eleven: the buffer, the
   prebuffer, the five fade times, the fallback fade, the fade volume, the
   plug-in's own volume and the curve toggle.
3. **The fade unit's granularity** — whether a fade of `N` ms is linear in
   time or in steps per buffer block.
4. **What a fade does on a seek** — does the old position fade out and the new
   one fade in, or only one of the two? (The key's name says "on seek"; the
   behaviour is not established.)
5. **Pause and resume** — one setting for both, or only the pause?
6. **The prebuffer policy** (§4).
7. **Whether the player has a fade of its own.** The player is not fade-free:
   its main window carries `Shift+V` → `Stop with fadeout` (S03 §1). Whether
   that fade is the same thing as the plug-in's `cfg_fade_stop`, or a second
   one, is open.
8. **The order of volume application** — the plug-in's `cfg_volume` against
   the player's own `volume`.
9. **Crossfade between two tracks.** The plug-in's own strings carry no
   crossfade setting and nothing in them mentions one. That is an absence
   claim about **one plug-in's string table**, not about the reference: a
   crossfade could live in the player, in a Preferences page, or in another
   output plug-in. The repo's spec request
   (`docs/spec-requests/2026-10-20-missing-specs.md` §7) asks the question,
   and the repo holds no measurement of it either way. **What would settle
   it:** the player's own string table and Preferences page (is there a
   crossfade setting at all?), and a runtime measurement of the reference —
   play two tracks of known length back to back and measure the silence
   between them, then repeat with a manual "next"; or measure the gap between
   two tracks in a gapless continuation.

Added by the audit. Each follows from something the draft itself records, and
none of them is a fact the draft did not contain:

10. **Whether the eleven key names are the file's spellings, in which file and
    in which section.** The draft's sources are the plug-in's string table and
    "a configuration file written by the reference", and the draft does not
    say which of the two carries the key list. The repo's observed reference
    file carries no section for the DirectSound output plug-in `out_ds.dll`
    (S12 §8.2) — and it is not established that the draft's plug-in is that
    file (§7, item 13) — so the file form of these names is not established
    here. Settled by letting the plug-in write its configuration and reading
    the file back.
11. **The exact wording of the status page's cursor lines, and of the mixing
    report's placeholders.** The draft names a write cursor, a playback cursor
    and a mixing line, but not their spellings.
12. **What the trailing `2` in `cfg_prebuf2` distinguishes.**
13. **Which file the plug-in is.** The draft calls it "the DirectSound output
    plug-in shipped with the reference" and never names the file; the repo's
    folder listing names `out_ds.dll` in `Plugins/` (S12 §8.2), and it is not
    established that the string table came from that file.
