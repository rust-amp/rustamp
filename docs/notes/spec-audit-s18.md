# Spec audit S18 — the output plug-in's fades, buffer and prebuffer

Lane: spec audit of the S18 draft (CLEANROOM.md §3, step 3).
Auditor: spec auditor subagent. Audit date: 2026-10-20.

| Artifact | Path in this worktree |
|---|---|
| Audited spec | `specs/S18-output-behaviour.md` (new) |
| Index row | `specs/README.md` — S18 status `not started` → `audited` |
| This note | `docs/notes/spec-audit-s18.md` (new) |

No code was touched. `cargo fmt --all -- --check` stays green (exit 0), which
is expected: this lane changes Markdown only.

## The draft that was audited

The draft is kept outside the repository, under RFC 0021's sandbox path
(`D:\code\rustamp-sandbox\spec-drafts\S18-output-behaviour.draft.md`, 5944
bytes, last written 2026-10-20 14:38:41, 124 lines). It is the only revision
that existed at audit time. It was written from the **user-facing strings** of
the DirectSound output plug-in that the reference ships and from that
plug-in's **configuration keys**; **the reference was not run for it and no
audio was played**, so the draft contains no runtime observation at all.

## What the audit checked

- **CLEANROOM §4** — no source code, no pseudo-code transliterated from source
  or disassembly, no copied or paraphrased comments, no distinctive internal
  identifiers, no disassembly listings.
- **CLEANROOM §3** — the provenance header is complete and filled in; the spec
  is behaviour, spellings and configuration facts, in prose and tables.
- **RFC 0021 §2 and §5** — the spec names its source **category** in the
  provenance header, so a later reviewer can see that the evidence is
  string-table-derived; and this note says what was removed and what was
  rewritten, not only that the spec was approved.
- **Epistemic status** — every item the draft called open is still open; no
  reading was promoted to a stated fact; seven of the draft's unmarked
  conclusions became **readings**, and one more was corrected outright
  (§ *The counts* below).
- **Internal consistency** — the key list, the counts and the nine open items
  were cross-checked against each other and against the draft's prose. Six
  contradictions surfaced (§ *Internal contradictions* below); none was
  smoothed.
- **Cross-references** — every repo fact the audited spec cites was checked
  against the file it names: S12 §3.2, §5.1, §8, §8.2, §8.3 and §4.1, S09 §4.1
  item 9, S03 §1, and the repo's own spec request §7.
- **Absence claims** — the draft makes one ("nothing in this plug-in mentions
  [a crossfade]") and derives a much wider one from it. The wider claim does
  not survive the search that produced it (§ *The absence claim* below); the
  narrow one is kept, scoped to the material.

## What stayed (unchanged in substance)

- The plug-in's identity as the **DirectSound output plug-in** that identifies
  itself as **v2.2.6**.
- **All eleven** configuration keys: `cfg_fade_start`, `cfg_fade_firststart`,
  `cfg_fade_seek`, `cfg_fade_pause`, `cfg_fade_stop`, `cfg_def_fade`,
  `cfg_logfades`, `cfg_fadevol`, `cfg_volume`, `cfg_buf_ms`, `cfg_prebuf2`.
- The fade report line **"Fade on <action> settings (<N> ms)"** — verbatim,
  including the millisecond unit.
- The constraint sentence, **verbatim**: *"Some fade times are bigger than
  buffer length; in order to get what you want, please increase buffer size to
  <N> ms."* This is the most load-bearing string in the draft and it survived
  the audit unchanged.
- The buffer report lines **"Active buffer size: <N> ms (<M> bytes)"**,
  **"Async buffer: <N> ms (<M> bytes)"**, **"Buffer locks done: <N>"**, and the
  write cursor and playback cursor on the status page.
- The prebuffer report lines **"Data buffered:"** and **"Total: <N> ms (<M>
  bytes)"**.
- The device and mixing report lines **"Device: "<name>""**, **"Mixing: <a>,
  primary buffer: <b><c>"** and **"supported, <N> free streams (<M> max)"**.
- **All six** failure messages, verbatim, with their situations.
- **All nine** open items, in the draft's order, with their own content — the
  only changes are to item 2 (which now names all eleven keys), item 7 (which
  now cites the repo's own `Stop with fadeout`) and item 9 (whose *conclusion*
  was pulled out of it) (§ *The open list* below).

## What was removed, and why

| Removed | Reason |
|---|---|
| The heading "S18 (DRAFT — not an audited spec, never hand this to an implementer)" | The file is now the audited spec; the warning was a property of the draft, not of the spec. |
| The draft's section "What this draft deliberately does not contain" | Meta-commentary about the draft. Its substance moved into §0 of the spec, where it is a statement about the spec itself — the same move the S13, S14, S19 and S20 audits made. |
| "the most useful sentence in the whole table" (on the buffer/fade constraint) | Editorial commentary with no evidentiary content. The sentence itself is kept and is now the load-bearing item of §2.3. |
| "So a fade is performed **inside the output buffer**" as a statement of fact | The constraint sentence does not say where the fade is performed; that is an inference from it. The spec keeps the inference as a **reading** and says what it rests on. |
| "a fade longer than the buffer cannot be honoured" as a bare statement | Same: it is the reading of the constraint sentence, now marked as one. |
| "**So the curve is selectable**" as a conclusion | It is a conclusion about what a toggle *means*, drawn from the key's name. It is a reading (§ *Internal contradictions* 2). |
| "(the other is presumably linear)" as an aside | The draft's own guess, stated as a parenthetical fact. The spec keeps it visible as a guess and asks the question in the open list (item 1). |
| "the user sets a time and the plug-in shows the byte size it actually got" as a restatement | It is a reading of the report line, not a restatement of it. Marked as a reading. |
| "the prebuffer is the amount of audio the plug-in wants **before** it starts playing" as a fact | The draft's own open item 6 asks whether the prebuffer is filled before the first sample leaves. Marked as a reading; item 6 stands. |
| "a second buffer region **next to** the main one" as a fact | The material carries two reported buffers; their relation is a reading. |
| "**Every one** of these is **named and actionable**" | It is false as written: two of the six messages only name the failure and say nothing to do about it (§ *The counts* below). |
| "a gap, not a feature" (on the crossfade) | An evaluation, and an assertion about the reference that the draft's evidence does not support. |
| "Fades belong to the output plug-in, **not the player**" as the heading of §1 | It contradicts the draft's own open item 7 (§ *Internal contradictions* 1). The heading now says what the material supports: the plug-in owns the fade **settings**. |
| "The DirectSound output of this build" → "the reference ships a DirectSound output plug-in" | The draft asserts that the build in question is the one the reference ships. The spec says what is observed. |

No passage had to be cut for being source code, a transliterated pseudo-code, a
copied comment or a disassembly listing. The names the draft carries are the
ones the rules allow: the plug-in's own **user-facing text** and its
**configuration key names**.

## What was rewritten, and why

| Change | Reason |
|---|---|
| A new §0 "How to read this spec" with an evidence-class table (string fact / key fact / reading / practical note / open), the sentence "**The reference binary was not run for this spec, and no audio was played**", a scope paragraph and the crossfade paragraph | The draft mixed string facts, key facts and conclusions in one voice. The marker discipline is now the one S11, S12, S13, S14, S19 and S20 use. The scope paragraph names what the spec does **not** cover — the player's transport behaviour — and the crossfade paragraph says in as many words that the spec does not settle the repo's own §7 request. |
| §1: the plug-in's identity, the three surfaces its text comes from, and the count of eleven keys | The draft never counted its keys; an implementer needs to know that the list is closed. The count is derived from the draft's own three lists. |
| §2: the fade group split into a key table with a "what the name says (reading)" column, plus an explicit count (five per action + one fallback = six) | The draft's table mixed carried names with the draft's own readings in one column, and its "one per action" did not agree with its six rows. |
| §2.2: the three knobs kept, each with its reading | Same reason as S20 §5: the status of each name is now visible in the table. |
| §2.3: the report line and the constraint sentence separated; the consequence marked as a reading | The draft ran the two together, so the reading looked like part of the quote. |
| §3, §4, §5: the report lines put into tables, the readings marked | Same reason as above; the strings stay verbatim. |
| §5: the "named and actionable" claim corrected to "four of the six" | The claim was false (§ *The counts*). |
| A new §6 "Practical notes for an implementer" | Every statement follows from the spec's own facts: the verbatim strings, the eleven keys and their section, the `cfg_` prefix, the buffer/prebuffer hints, the two volumes and the rule that §7 must not be implemented as if settled. Same pattern as S12 §9, S13 §4, S14 §3, S19 §5 and S20 §7. |
| The open list moved to the end and was renumbered | The task the spec exists for: facts first, open questions last. The draft's nine items kept their relative order (mapping below). |
| The header's SHA-256 written in upper case → lower case | House style: S11's, S12's, S13's, S14's, S19's and S20's headers spell the same hash in lower case and carry the same authorisation sentence. Same hash, same binary. |

## The key-spelling question (the check the task asked for)

The draft lists its keys with a `cfg_` prefix. The question is whether those
are the **file** spellings. The check:

- **S12 §8** — a plug-in's own keys live in the player's `winamp.ini`, in a
  section named after the plug-in, and the player does not interpret them.
- **S12 §8.3** — the one plug-in key the repo has observed **in a file**,
  `cfg_title_format` in `[Nullsoft Module Decoder]`, carries its prefix **in
  the file**. So the prefixed form is exactly what a plug-in's own key looks
  like when it is written down.
- **S12 §4.1** — the prefix rule (the player looks its own names up under a
  seven-character `config_` prefix, and the file carries the name without it)
  governs the **player's own** lookups. It does **not** transfer to a plug-in's
  section, so it can be used neither to confirm nor to deny the `cfg_` names
  here.

**Verdict:** the `cfg_` spelling of these eleven keys is **consistent** with
the file spelling of the one plug-in key the repo has observed, and the spec
says so, citing **S12 §8 and §8.3** — not §4.1. What is *not* established is
whether **these eleven names** are the file's spellings: the repo's observed
reference file carries **no section for `out_ds.dll`** (S12 §8.2), and it is
not established that the draft's plug-in is that file (open question 13). So
the repository cannot confirm them. That is published as open question 10,
with the measurement that would settle it (let the plug-in write its
configuration and read the file back).

## Internal contradictions found

1. **The draft's §1 heading against its own open item 7.** The heading says
   "Fades belong to the output plug-in, **not the player**", while item 7 asks
   "Whether the player has any fade of its own". The heading asserts what the
   item leaves open. The spec's heading says what the material supports — the
   plug-in owns the fade **settings** — and item 7 stays open. The repo also
   shows the player is not fade-free: its main window carries `Shift+V` →
   `Stop with fadeout` (S03 §1), which the spec now cites in item 7.
2. **"So the curve is selectable" against the draft's own open item 1.** §1
   states as a conclusion that there is more than one fade shape and that one
   of them is logarithmic, while item 1 asks which one `cfg_logfades` selects
   and what the other shape is. A toggle's name is not evidence of a choice
   between two shapes; the spec demotes the conclusion to a **reading** and
   item 1 stands.
3. **The fade count.** §1 says the plug-in has "its own fade settings, **one
   per action**" and then lists **six** rows. Five of the rows name an action
   (start, first start, seek, pause, stop); the sixth, `cfg_def_fade`, is the
   fallback. The spec publishes the counts the list supports: **five** keys per
   action, **plus** one fallback key, **six** in the fade group.
4. **The defaults enumeration against the key list.** Open item 2 asks for
   "Default values for every key above (buffer, prebuffer, the five fade
   times, the fade volume)" — that parenthesis names **eight** of the
   **eleven** keys and leaves out `cfg_def_fade`, `cfg_logfades` and
   `cfg_volume`. The spec's item 2 names all eleven.
5. **"Every one of these is named and actionable" against the six messages.**
   Two of them — `Error creating DirectSound buffer.` and `DirectSound
   doesn't appear to be installed on this system.` — only name the failure.
   The spec says "four of the six".
6. **The prebuffer sentence against the draft's own open item 6.** §3 states
   that the prebuffer "is the amount of audio the plug-in wants **before** it
   starts playing", while item 6 asks "is it filled before the first sample
   leaves". The spec marks the sentence a reading and leaves item 6 open.

## The counts

| Figure | Draft | Audited spec |
|---|---|---|
| Fade keys in the group | "one per action" over six rows | **five** per action, **plus** one fallback = **six** |
| Keys in the plug-in's configuration | not stated | **eleven** (six fade group + three knobs + buffer + prebuffer) |
| Keys whose default item 2 enumerates | eight of eleven | **all eleven** |
| Failure messages that carry a remedy | "**Every one**" | **four of six** |
| Counts stated twice | the fade count appears in §1's prose *and* in its table | the spec states each count once, next to the list it is derived from |

## The absence claim

The draft's open item 9 reads: *"nothing in this plug-in mentions one, so the
reference of this build has **no crossfade between tracks** — a gap, not a
feature."*

The evidence behind it is a search of **one plug-in's string table**. The
conclusion is about **the reference**. The two do not meet, and the repo says
so in two places:

- the repo's own spec request (`docs/spec-requests/2026-10-20-missing-specs.md`
  §7) asks for the crossfade — length, curve, behaviour on a manual skip — and
  lists it as **not implemented** on our side;
- the parity roadmap lists "Crossfade between tracks" under the modern
  features we do not have (`docs/tasks/ROADMAP-to-winamp-parity.md` §12).

A crossfade could live in the player, in a Preferences page, or in another
output plug-in; the DirectSound plug-in's strings cannot rule that out. This
is the S13 lesson in its second form: **a "there is no X" sentence is only as
strong as the shape of the search behind it.**

So the spec keeps the narrow claim ("the plug-in's own strings carry no
crossfade setting and nothing in them mentions one"), drops the wide one, and
publishes the question — with what would settle it: the player's own string
table and Preferences page, and a runtime measurement of the reference (two
tracks of known length back to back, measuring the silence between them; then
the same with a manual "next"). The absence of a crossfade in **this** plug-in
is **not** published as a fact about the reference, and the draft's open item
9 stays open in its place.

## The open list

Kept from the draft, in the draft's order, with their own content: the fade
curve(s); the default value of every key; the fade unit's granularity; what a
fade does on a seek; pause and resume; the prebuffer policy; whether the
player has a fade of its own; the order of volume application; the crossfade
between two tracks.

Changed in the audit:

- Item 2 now enumerates **all eleven** keys (the draft named eight).
- Item 9 keeps the question and loses the draft's conclusion; the conclusion
  is replaced by the scope of the search that produced it and by the
  measurement that would settle it.
- Item 7 now cites the repo's own `Stop with fadeout` (S03 §1) as the reason
  the player is not fade-free.

Added by the audit. Each follows from something the draft itself records, and
none of them is a fact the draft did not contain:

| Added | Why |
|---|---|
| 10 — whether the eleven key names are the file's spellings, in which file and in which section | The draft names two sources (the plug-in's string table and "a configuration file written by the reference") and does not say which carries the key list; the repo's observed file has no section for `out_ds.dll` (S12 §8.2), and it is not established that the draft's plug-in is that file. |
| 11 — the exact wording of the status page's cursor lines and of the mixing report's placeholders | The draft names a write cursor, a playback cursor and a mixing line, but not their spellings. The strings an implementer would have to reproduce are not in the draft. |
| 12 — what the trailing `2` in `cfg_prebuf2` distinguishes | A key in the draft's list that nothing explains; the name implies a counterpart the material does not show. |
| 13 — which file the plug-in is | The draft calls it "the DirectSound output plug-in shipped with the reference" and never names the file; the repo's folder listing names `out_ds.dll` in `Plugins/` (S12 §8.2), and it is not established that the string table came from that file. |

## What could not be resolved

1. **The fade shapes.** `cfg_logfades` names a logarithmic shape; the other
   one is not named in the material. Only the plug-in's own configuration page
   would show it.
2. **The defaults.** Not in the strings. A fresh configuration file would show
   them.
3. **The prebuffer and the buffer policy.** Nothing in the material describes
   when the plug-in starts playing or what it does on a stall.
4. **The crossfade.** The search behind the draft's conclusion could not see
   the player and the other output plug-ins; only a runtime measurement can
   close it.
5. **The file form of the eleven key names.** Only the file the plug-in writes
   settles it.
6. **The `cfg_prebuf2` suffix and the plug-in's file name.** Both are naming
   questions the material does not answer.

## Notes for the orchestrator (not this lane's files)

- The repo's spec request `docs/spec-requests/2026-10-20-missing-specs.md` §7
  ("S18 — crossfade") is **not** answered by this spec. The spec answers the
  *fade* half of the request (the plug-in's five per-action fade keys, the
  millisecond unit and the buffer/fade constraint) and leaves the crossfade
  half open, with the measurement that would settle it. The request file was
  **not** touched by this lane.
- `docs/tasks/A1-audio-engine.md` (line 37) says S18 "does not exist yet" and
  that out_ds-style fades and prebuffering come with M2. The file was **not**
  touched by this lane; the spec exists now, but with a long open list, so
  "do not implement them" is still the right instruction until §7 is measured.
- `docs/tasks/ROADMAP-to-winamp-parity.md` still lists S18 among the missing
  specs (§ *Specs Still Needed*). It was **not** touched by this lane.
- `docs/HANDOFF.md` collects the open spec requests; its S18 entry
  ("S18 (fades/prebuffering)") was **not** touched by this lane.
- The fade stage in `crates/rustamp-dsp` is an equal-power ramp, and the
  engine applies `StopWithFadeout`. The spec licenses no change to either:
  the fade curve, the default times and the prebuffer policy are all open
  (§7, items 1–3 and 6), and the fade the reference's plug-in offers is a
  **plug-in's** fade, not the player's (§0, scope).
- No file outside the three artifacts above was touched.

## Verification

- `cargo fmt --all -- --check` → exit 0 (green; this lane changed Markdown
  only).
- `git status --short` → the three artifacts above, plus nothing else.
