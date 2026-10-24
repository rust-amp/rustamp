# Spec audit S14 — title formatting

Lane: spec audit of the S14 draft (CLEANROOM.md §3, step 3).
Auditor: spec auditor subagent. Audit date: 2026-10-20.

| Artifact | Path in this worktree |
|---|---|
| Audited spec | `specs/S14-title-formatting.md` (new) |
| Index row | `specs/README.md` — S14 status `not started` → `audited` |
| This note | `docs/notes/spec-audit-s14.md` (new) |

No code was touched. `cargo fmt --all -- --check` stays green (exit 0), which
is expected: this lane changes Markdown only.

## The draft that was audited

The draft is kept outside the repository, under RFC 0021's sandbox path
(`D:\code\rustamp-sandbox\spec-drafts\S14-title-formatting.draft.md`, 5814
bytes, last written 2026-10-20 10:26:41, 94 lines). It is the only revision
that existed at audit time. It was written from the **user-facing help text** of
two input plug-ins that ship the format language, plus the reference binary's
string table; **the reference was not run for it**, so the draft contains no
runtime observation at all.

## What the audit checked

- **CLEANROOM §4** — no source code, no pseudo-code transliterated from source
  or disassembly, no copied or paraphrased comments, no distinctive internal
  identifiers, no disassembly listings.
- **CLEANROOM §3** — the provenance header is complete and filled in; the spec
  is behaviour, spellings and file-format facts, in prose and tables.
- **RFC 0021 §2 and §5** — the spec names its source **category** in the
  provenance header, so a later reviewer can see that the evidence is
  string-table- and help-text-derived; and this note says what was removed,
  not only that the spec was approved.
- **Epistemic status** — every item the draft called open is still open; no
  reading was promoted to a stated fact; the draft's self-declared "spec fact"
  was demoted to a reading.
- **Internal consistency** — the field list, the function list and the worked
  examples were counted and cross-referenced against each other. Six
  contradictions surfaced — three in the counts and three in the
  cross-references (§ *The counts* and § *Cross-reference errors* below); none
  was smoothed.
- **Cross-references** — every cross-reference the draft made into the repo
  was checked against the file it names. One was wrong (§ *Cross-reference
  errors* below), and one open item the draft should have reused already
  existed in S12.

## What stayed (unchanged in substance)

- The core claim: the reference has **no global title-format setting**; the
  format language belongs to **individual input plug-ins**, and each such
  plug-in keeps its own format string in its own section of `winamp.ini`.
- The four plug-ins and which of them carry the language: the module decoder
  and the Vorbis decoder do, the MP3 decoder and the WAVE decoder do not.
- The player's own strings carry no field token and no function token.
- All nine field names, all thirteen function rows (twelve names, `$abbr` in
  two forms), and every function behaviour as the draft stated it.
- The empty string as the value of a field the track does not carry.
- The conditional group's rule: inserted only when at least one valid field
  inside it was found, otherwise the whole block including the brackets
  disappears.
- All four format strings, including the two the draft quoted in two different
  sections.
- Every open item the draft had, in the draft's order (§ *The open list*
  below).

## What was removed, and why

| Removed | Reason |
|---|---|
| The heading "S14 (DRAFT — not an audited spec, never hand this to an implementer)" | The file is now the audited spec; the warning was a property of the draft, not of the spec. |
| The draft's section "What this draft deliberately does not contain" | Meta-commentary about the draft. Its substance moved into §0 of the spec ("Nothing in this spec is source code, a transliterated pseudo-code, a copied comment, an internal identifier or a disassembly listing"), where it is a statement about the spec itself. The same move was made in the S13 audit. |
| "Evidence, per plug-in shipped with the reference:" as the lead-in to §1's table | A pointer into the dirty material. The evidence categories belong in the provenance header, where they now are; §0 states the evidence classes once. |
| The quoted help-text sentence "inserts field named `<tagname>`, eg. `%artist%`" | A verbatim quote from the plug-ins' help text. The semantics survive as "the help text's own example names an artist tag"; the quoting does not. |
| §1's table column "Its own example", with the module decoder's and the Vorbis decoder's format strings | Those two strings are the subject of §2.4, where they appear once each with their provenance. Carrying them a second time in §1 is the duplication the house style avoids (S13's audit removed the same kind of duplication for the window class name). |
| "**Consequence for us, stated as a spec fact:** a compatible player does not need a global formatting engine." | A self-declared promotion. The consequence follows from §1's facts, but it is an inference, and the spec now carries it in §3 as a **reading**, not as a fact. |
| "The set below is complete for this build." | Overstated. What was read is the help text that two shipped plug-ins carry; the spec says the function list is "the complete set that help text carries", which is exactly what the evidence supports. |
| "These three are the best available test vectors: each exercises fields, `$if2`, `$abbr` and the conditional group at once." | Both halves are wrong: there are four strings, not three, and no single string uses `$if2` **and** `$abbr`. §2.4 now carries the four strings and a per-string coverage table. |

No passage had to be cut for being source code, a transliterated pseudo-code, a
copied comment or a disassembly listing — the draft was already careful about
CLEANROOM §4. The names it carries are the ones interoperability requires:
the field and function tokens of a published format language, the format
strings themselves, and the plug-ins' names.

## What was rewritten, and why

| Change | Reason |
|---|---|
| A new §0 "How to read this spec" with an evidence-class table (string fact / help-text fact / example fact / reading / open), the sentence "**The reference binary was not run for this spec**" and a scope paragraph | The draft mixed string-derived facts and help-text-derived facts in one voice. The marker discipline is now the one S12 and S13 use. The scope paragraph names what the spec does **not** cover (a playlist-view format, a preferences page, the `%1`–`%7` style), because the repo's own spec request expects those (§ *Cross-reference errors* below). |
| §1: "The player binary itself carries no field or function tokens." → "The player's own strings carry no field token and no function token of this language." | The draft pointed at the artifact; the spec states the fact about the reference. |
| §1: "So the playlist's text is whatever the input plug-in reports, and a plug-in that has the language can be configured to report something more elaborate." | "So" presented an inference as a consequence of fact. The spec marks it **reading** and points at open question 9, which is where the tension with the draft's own open item 7 lives. |
| §1: the claim "no global title-format setting" is now backed by S12 §5.1 (the player's own recognised key set carries no format-string key), and the per-plug-in claim by S12 §8 and §8.3 | The draft cited the wrong S12 section (§ *Cross-reference errors*). |
| §2.1: the field list's provenance claim was narrowed | The draft said the names were "observed in the shipped defaults and examples". `comment` is in neither — it appears in none of the four strings. The spec keeps the name, counts the list as nine, says that eight appear in the strings, and publishes the gap as open question 10. |
| §2.2: the `$abbr(x)` row now says that what an abbreviation *is* remains open | The row said "abbreviates `x`" with no pointer, next to a table whose other rows are exact. The threshold behaviour of the two-argument form stays a fact; the abbreviation rule is the draft's open item 1 and is now reachable from the row. |
| §2.2: "the help text states the identity" dropped from the `$if2` row; the function counts added | "The help text states the identity" is a pointer into the material, and the identity is the row's content. The row now also counts: twelve function names, `$abbr` in two forms. |
| §2.3: "the identity in §2.2 — `$if(A,A,)` is equivalent to `[A]`" → an explicit derivation from §2.2's `$if` row | The draft pointed at a section that does not state that identity; §2.2 states the `$if` semantics from which it follows. The spec now derives it in three sentences instead of citing it. |
| §2.4 became "The strings this spec carries", with four rows and a coverage table | The draft's "three" undercounted its own material, and its "each exercises … at once" was false. The table states, per string, which fields it uses and whether it uses `$if2`, `$abbr` and the conditional group — so an implementer can see the coverage instead of a claim about it. |
| A new §3 "Practical notes for an implementer" | Every statement follows from the spec's own facts: where the language belongs, the empty-string rule, the group's behaviour, the four test vectors, and the rule that §4 must not be implemented as if settled. Same pattern as S12 §9 and S13 §4. |
| The open list moved to the end and was renumbered | The task the spec exists for: facts first, open questions last. The draft's items kept their relative order (mapping below). |
| The header's SHA-256 written in upper case → lower case, and the long RFC 0021 line | House style: S12's and S13's headers spell the same hash in lower case and carry the same authorisation sentence. Same hash, same binary. |

## The counts: discrepancies found in the audit

| Figure | Draft | The draft's own material |
|---|---|---|
| Worked examples | "These **three** are the best available test vectors" | **four** distinct format strings are quoted in the draft: the two plug-in defaults, the help text's illustration (§2.4) and the help text's example of the conditional group (§2.3) |
| Coverage per example | "each exercises fields, `$if2`, `$abbr` and the conditional group **at once**" | no string uses `$if2` **and** `$abbr`: the two plug-in defaults use `$if2` only, the illustration uses `$abbr` only |
| Field names | nine names, "observed in the shipped defaults and examples" | **eight** of the nine appear in the strings; `comment` appears in none |
| Function names | thirteen table rows, no count | **twelve** distinct names — `$abbr` in two forms |
| Function names used in the strings | — | **two** of twelve (`$if2`, `$abbr`); the other ten appear only in the help text's list |
| Field names used in the strings | — | **eight** of the nine |

The draft's "at once" claim is the one that would have hurt an implementer most:
it invites a single test string to be written that exercises the whole
language, and no such string exists in the material. The spec publishes the
coverage table instead, so the gap between the language's size (twelve
functions) and the material's size (two functions in four strings) is visible.

## Cross-reference errors found

1. **"(S12 §4)"** — the wrong section. S12 §4 is "How the player reads a key"
   (the `config_` prefix rule, the missing-key default, the 2048-byte string
   buffer). It says nothing about plug-in sections. The claim the draft wanted
   it to support — the formatting is per plug-in, not global — is supported by
   **S12 §8** ("Each plug-in's section is named by its display name and its
   keys are the plug-in's own; the player does not interpret them") and **S12
   §8.3** (the observed `cfg_title_format` key in the module decoder's own
   section), and the "no global setting" half is corroborated by **S12 §5.1**:
   the player's own recognised key set carries no format-string key. The spec
   now cites those three sections.
2. **The token-set item was already in S12.** S12 §10 item 10 reads: "The
   title-format language beyond the four facts in §8.3 — the full token and
   function set is not established by this one string. S14 carries it." The
   draft never mentions it. The spec's §0 now says what it settles instead of
   restating the gap, and S12's four facts (§8.3) are consistent with §2.1,
   §2.2 and §2.3 — the conditional group, the field tokens and `$if2` agree.
3. **§1 against the draft's own open item 7.** §1 says the player's own
   strings carry no token; open item 7 asks whether the player post-processes a
   plain "artist - title" at all. A string table that carries no token does
   not prove that the player leaves a reported title untouched. Published as
   open question 9 rather than smoothed into a fact in either direction.
4. **The Vorbis decoder's default against S12 §2 and §8.3.** S12 §2 lists
   the observed file's plug-in sections: the WAV output driver, the module
   decoder and the WAVE decoder. The Vorbis decoder has no section there, so
   its default was not observed in that file. Published as open question 12
   (where the shipped defaults live, and whether other plug-ins carry further
   ones).
5. **The repo's spec request expects more than the draft covers.**
   `docs/spec-requests/2026-10-20-missing-specs.md` §5 asks S14 four
   questions. This spec answers three of them: the `$if`, `$if2`, `$num` and
   `$pad` constructs **do exist in 2.95** (shipped by the input plug-ins, so
   the precedence rule §6 is satisfied from the 2.95 side); the format string
   is stored **per plug-in** in that plug-in's own `winamp.ini` section, not on
   a preferences page and not globally; and a missing field yields the **empty
   string**. It does **not** answer question 1, the `%1`–`%7` placeholder set.
   That gap is published as open question 8 and in the orchestrator notes
   below.

## The open list

Kept from the draft, in the draft's order: the exact rule of `$abbr` (with the
draft's own measurement recipe), `$num` with a non-numeric field,
`$caps`/`$caps2` on non-ASCII text, `$cut`/`$pad` on a value that contains the
separator characters, an unknown function name, the maximum format-string
length, and whether the player's own title show uses any of this when the
plug-in reports a plain "artist - title".

Added by the audit. Each follows from something the draft itself records, and
none of them is a fact the draft did not contain:

| Added | Why |
|---|---|
| 8 — the `%1`–`%7` numbered placeholder style | The repo's spec request asks S14 for this set (§ *Cross-reference errors* 5); the draft's evidence covers only the plug-ins' tag-name language. |
| 9 — whether the token silence settles the draft's item 7 | The contradiction between §1 and the draft's own open item 7 (§ *Cross-reference errors* 3). |
| 10 — where the `comment` field name was seen | The field list's own provenance claim does not hold for that name (§ *The counts*). |
| 11 — what counts as a "valid field" | §2.1 says a field the track does not carry yields the empty string; §2.2's `$if` turns on "at least one valid field". A tag that is present but empty splits the two readings, and the draft settles neither. |
| 12 — where the shipped defaults live | The Vorbis decoder's default was not in the observed file (S12 §2), and the draft says nothing about where it is instead (§ *Cross-reference errors* 4). |
| 13 — how `$longest` breaks a tie | The function is defined by "the longest output string"; equal-length arguments are undefined. |

## What could not be resolved

1. **The `$abbr` rule.** Unresolvable from a help text; the draft's own
   measurement recipe stands.
2. **Whether `comment` is a field at all.** The draft lists it, no string uses
   it, and the help text's list of tag names was not read for this spec. Either
   it is a documented tag or it is an error in the list; a dirty-room re-read
   would say which.
3. **Whether the four strings are the complete set of shipped defaults.** The
   draft says the two plug-ins that carry the language ship the same help
   text; it does not say that no other plug-in ships a default string, and the
   observed `winamp.ini` only covers one of them.
4. **Which reading of "valid field" is right.** Both readings fit every
   string of §2.4.
5. **The `%1`–`%7` placeholder style.** Entirely outside the draft's evidence.
   §1 records that the player's own strings carry no token of any kind, which
   is the strongest statement available from this material — but a numbered
   placeholder need not appear as a string in the binary, so the question
   needs its own measurement.

## Notes for the orchestrator (not this lane's files)

- `docs/spec-requests/2026-10-20-missing-specs.md` §5 question 1 is **not**
  answered by this spec. Questions 2, 3 and 4 are answered as summarised
  above; the file itself was not touched by this lane.
- `crates/rustamp-meta/CONTRACT_NOTES.md` asks S14 to say which fields are
  used, the fallbacks for missing fields, and whether the format string is
  per-entry or global. From §1 and §2.1: the field names are the nine of
  §2.1, the fallback for a missing field is the empty string, and the format
  string is **per plug-in** — neither global nor per entry: the plug-in
  formats the track it reports. The crate's own request also lists raw inputs
  (album artist, year, genre, disc numbers) that this language does not name.
- `crates/rustamp-playlist/CONTRACT_NOTES.md` asks whether
  `PlaylistEntry::title` is the raw or the formatted string. S14's shape — the
  formatter lives in the plug-in and formats the track it reports — points to
  raw tag values in and a formatted display string out, but the caching
  question is S06's, and this spec does not decide it.
- `docs/PLAN.md` describes `rustamp-meta` as handling "Winamp 2 title
  formatting (%1–%7 style)", and `docs/HANDOFF.md` collects the open S14
  request as "is `PlaylistEntry::title` the formatted or the raw title; which
  fields feed `%1`–`%7`". Both are half-answered at best; the `%1`–`%7` half
  is open question 8. The crate description is ahead of the spec.
- The tree names S14's subject in more places than these three: `docs/PLAN.md`
  (§ "rustamp-meta" and the milestone list), `docs/tasks/E1-playlist-io.md`,
  `docs/tasks/M1-meta-tags.md`, `docs/tasks/E2-playlist-meta-audit.md`,
  `docs/tasks/ROADMAP-to-winamp-parity.md`, `docs/rfc/0002-audio-contracts.md`
  and `docs/rfc/0005-app-commands.md` all defer something to S14. None of
  them was changed by this lane; the notes above cover the ones that make a
  claim about what S14 will contain.
- No file outside the three artifacts above was touched.

## Verification

- `cargo fmt --all -- --check` → exit 0 (green; this lane changed Markdown
  only).
- `git status --short` → the three artifacts above, plus nothing else.
