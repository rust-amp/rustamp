# Spec audit S20 — the media library plug-in

Lane: spec audit of the S20 draft (CLEANROOM.md §3, step 3).
Auditor: spec auditor subagent. Audit date: 2026-10-20.

| Artifact | Path in this worktree |
|---|---|
| Audited spec | `specs/S20-media-library.md` (new) |
| Index row | `specs/README.md` — S20 status `not started` → `audited`, title changed (see below) |
| This note | `docs/notes/spec-audit-s20.md` (new) |
| Spec-request answer | `docs/spec-requests/2026-10-20-missing-specs.md` §9 and the S20 row of the table (updated) |

No code was touched. `cargo fmt --all -- --check` stays green (exit 0), which
is expected: this lane changes Markdown only.

## The draft that was audited

The draft is kept outside the repository, under RFC 0021's sandbox path
(`D:\code\rustamp-sandbox\spec-drafts\S20-media-library.draft.md`, 6579 bytes,
last written 2026-10-20 14:29:24, 140 lines). It is the only revision that
existed at audit time. It was written from the **user-facing string table** of
the media-library plug-in that the reference ships, plus the **plug-in folder
listing**; **the reference was not run for it and the plug-in was not loaded**,
so the draft contains no runtime observation at all.

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
  reading was promoted to a stated fact; four of the draft's unmarked
  conclusions became **readings**.
- **Internal consistency** — the field-name list, the token table, the settings
  key list and the ten open items were cross-checked against each other. Four
  contradictions surfaced (§ *Internal contradictions* below); none was
  smoothed.
- **Cross-references** — every cross-reference the draft made into the repo
  was checked against the file it names. One was wrong (§ *Cross-reference
  errors* below).
- **Absence claims** — the draft makes three ("not established by the
  strings", "nothing in the string table describes it", "the format is open").
  The repo carries no measurement of the *plug-in's* string table that could
  contradict them, so the absence claims stand on the draft's own reading of
  that table; the spec keeps them as statements about the **material**.

## What stayed (unchanged in substance)

- The plug-in's identity: a **general-purpose plug-in** named `Media Library`
  that identifies itself as `Nullsoft Winamp Library v1.3`, with the window
  class name `WinampLibraryPE` and the error-dialog caption
  `Winamp Library Error`.
- All **four** view names — `Local Media`, `Internet Radio`, `Playlists`,
  `New View` — and the two prompts `Rename playlist` and `Delete this view?`.
- All **seven** field names — `title`, `artist`, `album`, `year`, `genre`,
  `track`, `ntracks` — and the track-numbering mode and the column-resize
  mode.
- The searching message `Searching... (N files found)`.
- Both path-template strings, `Artist - Album\Artist - Album` and
  `## - Artist - Title`, and the whole **token table**, including the `#` /
  `##` / `###` forms.
- The three extraction strings — `Overwrite playlist (No will append)?`,
  `N tracks extracted`, `Error extracting track #N: <reason>`.
- The message `QSort failed and tried to update index outside range`.
- All **seven** configuration keys — `last_view`, `column_resize_mode`,
  `viewplayonemode`, `remembersearch`, `radio_last_upd`, `maxradiostream`,
  `radio_upd_freq` — and the two marked readings that go with
  `remembersearch` and `viewplayonemode`.
- The radio directory file `radio.vmd` and the message `Fetching Playlist`.
- All **ten** open items, in the draft's order, with their own content.

## What was removed, and why

| Removed | Reason |
|---|---|
| The heading "S20 (DRAFT — not an audited spec, never hand this to an implementer)" | The file is now the audited spec; the warning was a property of the draft, not of the spec. |
| The draft's section "What this draft deliberately does not contain" | Meta-commentary about the draft. Its substance moved into §0 of the spec, where it is a statement about the spec itself — the same move the S13, S14 and S19 audits made. |
| "One **internal** failure has a user-visible message worth recording because it describes a real hazard" | The framing called the message an internal failure and put the audit's attention on the hazard. The message is a **user-facing** message of the library's own error dialog (§4.3 of the spec), and that is how the spec now presents it. |
| "— a sort that loses its index mapping" (the gloss on the `QSort` message) | A mechanism gloss. The message says in plain words that a sort failed and an index went out of range; the internal mechanism is not established and is not described in the spec. The reading survives in §4.3, marked as such. |
| "the plug-in does not create the menu entry's look" | Neither a fact nor a reading: "look" is not a property of a menu entry. Dropped. |
| "the column-resize mode" from §3's list of what the plug-in knows | It is the same thing as the settings key `column_resize_mode`, which §5 already carries. The spec names it once, in §5, and §3 refers to it. |

No passage had to be cut for being source code, a transliterated pseudo-code, a
copied comment or a disassembly listing. The names the draft carries are the
ones the rules allow: the plug-in's own **user-facing text**, its **window
class name** and its **configuration key names**.

## What was rewritten, and why

| Change | Reason |
|---|---|
| A new §0 "How to read this spec" with an evidence-class table (string fact / reading / practical note / open), the sentence "**The reference was not run for this spec, and the plug-in was not loaded**", a scope paragraph and the peer-feature paragraph | The draft mixed string facts and conclusions in one voice. The marker discipline is now the one S11, S13, S14 and S19 use. The scope paragraph names what the spec does **not** cover (**CD playback**), and the peer-feature paragraph says that this documents a **plug-in**, so our own library's design is **not a deviation** from it. |
| The title: "Media Library (2.9x) and CD playback (later milestones)" → "Media Library plug-in (2.9x)" | The spec does not cover CD playback at all, so a title promising it would mislead an implementer. The index row carries the same change. |
| §1: "It is reached from the player's menu (`Media Library`, and a `Toggle Library` command)" → the two labels are carried, and a reading says the plug-in's own string table cannot place them in the player's menu | The evidence is the **plug-in's** string table. It carries the two labels; it cannot say which control shows them, nor whether the *player* carries the entry that opens the library. Published as open question 11. |
| §1: "i.e. it is a window in its own right, not a page inside the player" → a **reading** | "i.e." presented an inference as a restatement of the class-name fact. A plug-in that registers a window under its own class is in its own right; the spec says so as a reading. |
| §2: the four views kept as names, their glosses moved into a "what the name says (reading)" column | The draft's table mixed the carried names with the draft's own readings in one column. |
| §3: `track` and `ntracks` glosses demoted to a reading, and the "the track count" gloss dropped | The draft's own open item 10 asks what `ntracks` counts; the §3 gloss pre-empted it. Open question 10 keeps the question. |
| §3: "the count is known before the list is filled" → a reading | The message carries a count; the order in which the count and the list appear is an inference from the wording, not an observation. |
| §4: the three extraction strings moved into a table of "where / string", and the two consequences ("so 'no' appends", "without aborting the rest") demoted to one reading | The draft stated two behavioural conclusions as facts from three message texts. |
| §4.1: "a backslash separates directories, so the template builds a folder tree" → a reading, with the literal-vs-platform question left to the draft's own item 8 | The draft settled in a parenthesis what its open item 8 leaves open. The spec now says the separator role is a reading and that the platform question is open. |
| §4.2: "not S14's `%field%` language" → a five-row **contrast table** | The draft's one-line contrast was imprecise: S14's language is not only `%field%`, it also has `$function(...)` calls and `[ ... ]` groups (S14 §2.2, §2.3), it belongs to the **input** plug-ins, and it builds a **title**. This one is a **path template** in a **general-purpose** plug-in, with bare words and no functions. The table states the contrast precisely. |
| §4.2: the token table kept verbatim, its "Inserts" column labelled a reading, and the case rule demoted to a reading | The draft stated "the capitalisation of the word chooses the capitalisation of the value" as a fact, although its own material only carries the three spellings side by side. The spec keeps the table, marks the column, and publishes the rule as open question 12. |
| §4.2: three facts added about the table itself | They follow from the draft's own two tables: the three spellings are carried side by side, `year` in one spelling only, and the table covers **five** of the seven field names — `track` and `ntracks` have **no** word token. |
| §4.3: the `QSort` message given its own section, presented as a **user-facing message** of the library's error dialog | See § *What was removed* above. |
| §5: the section reference corrected, the prefix rule made explicit, and the cross-check against S12 §5.1 added | The draft cited "S12 §4" for the plug-in's section, but S12 §4 is "How the player reads a key". The plug-in-section rule is **S12 §8** and the prefix rule is **S12 §4.1**. The spec cites both, adds the observed `cfg_title_format` counter-example (S12 §8.3) and states the cross-check that none of the seven keys is in the player's recognised key set (S12 §5.1). |
| §5: the key table gains a "what the name says (reading)" column, and `radio_last_upd` is marked as unexplained | The draft's §5 listed the keys and then explained two of them in prose; the table makes the status of each name visible, and the seventh key — which the draft never mentions again — is published as open question 15. |
| §6: the update frequency and the maximum stream count demoted to a reading | The draft asserted them as facts in §6 and left their meaning open in its item 5. The spec keeps one and points at open question 5. |
| A new §7 "Practical notes for an implementer" | Every statement follows from the spec's own facts: the peer-feature status, the verbatim strings, the "not S14's language" rule, the seven keys and their section, and the rule that §8 must not be implemented as if settled. Same pattern as S12 §9, S13 §4, S14 §3, S19 §5. |
| The open list moved to the end and was renumbered | The task the spec exists for: facts first, open questions last. The draft's ten items kept their relative order (mapping below). |
| The header's SHA-256 written in upper case → lower case, and the long RFC 0021 line | House style: S11's, S12's, S13's, S14's and S19's headers spell the same hash in lower case and carry the same authorisation sentence. Same hash, same binary. |

## Internal contradictions found

1. **The backslash in §4 against the draft's own open item 8.** The draft's §4
   says in a parenthesis that "a backslash separates directories, so the
   template builds a folder tree", while item 8 asks whether the backslash is
   literal on every platform or a separator that follows the platform. The
   first half is a reading, the second half is open. The spec keeps the
   reading and leaves the platform question open (open question 8).
2. **The update frequency in §6 against the draft's own open item 5.** §6 says
   the plug-in "has an update frequency and a maximum stream count", while
   item 5 leaves the meaning of `radio_upd_freq` / `maxradiostream` open. The
   spec demotes the statement to a reading and points at open question 5.
3. **The "ntracks (the track count)" gloss against the draft's own open item
   10.** The gloss states what the column is, and item 10 asks what it counts.
   The spec keeps the field name and drops the gloss into a reading; item 10
   stays open (open question 10).
4. **The `QSort` message as an "internal failure" against its own text.** The
   draft called it "one internal failure" and then quoted a message that is
   addressed to the user. The spec presents it as the user-facing message it
   is (§4.3) and keeps the mechanism open.

## Cross-reference errors found

1. **"S12 §4" for the plug-in's settings section.** S12 §4 is "How the player
   reads a key" (§4.1 the `config_` prefix rule, §4.2 the missing-key
   default, §4.3 the 2048-byte string buffer). It says nothing about
   plug-in sections. The rule the draft wanted is **S12 §8** ("Each plug-in's
   section is named by its display name and its keys are the plug-in's own;
   the player does not interpret them"). The spec now cites S12 §8 for the
   section, **S12 §4.1** for the prefix rule and **S12 §8.3** for the observed
   `cfg_title_format` counter-example.

   The prefix rule itself needed the check the task asked for. The seven names
   are spelled **without** a prefix, which is how the file spells the
   **player's own** keys (S12 §4.1). But that rule governs the player's
   lookups, and the one plug-in key the repo has observed in a file carries
   its prefix **in the file** (`cfg_title_format`, S12 §8.3). So the rule
   cannot be transferred to the library's keys by analogy: whether the seven
   names are the file's spellings is **open** (open question 13), together
   with the name of the section they live in.

   The second half of the check came out clean: **none** of the seven names is
   in the player's recognised key set (S12 §5.1), which is exactly what S12
   §8 says to expect for a plug-in's own keys.
2. **"S12 §4" for the plug-in's section — also the wrong citation for the
   `last_view` claim in §2.** §2's "the window remembers the last view" is a
   reading of the `last_view` key, and the spec now says so and points at §5.
3. **The repo's spec request expects more than the draft covers.**
   `docs/spec-requests/2026-10-20-missing-specs.md` §9 asks whether 2.95 has
   an equivalent view or whether the library is a deliberate extension. The
   draft answers it in one line; the spec answers it in §0 and the request
   file now records the answer: **2.95 has a media library, as a plug-in**,
   and the columns, views and sort semantics the request asked about stay
   open (open questions 1–3).

## The open list

Kept from the draft, in the draft's order: the column order, default widths
and labels; the default view set; the sort semantics; the user view's query
language; the `radio.vmd` format and the meaning of `radio_upd_freq` /
`maxradiostream`; whether the library indexes on its own; CD playback; the
backslash in the path template; how the library's playlist and the player's
playlist relate; the meaning of the `ntracks` column.

Added by the audit. Each follows from something the draft itself records, and
none of them is a fact the draft did not contain:

| Added | Why |
|---|---|
| 11 — which control carries `Media Library` / `Toggle Library`, and whether the player carries the entry | The evidence is the plug-in's own string table (§ *What was rewritten*). |
| 12 — the capitalisation rule of the template language | The draft stated it as a fact; the material carries only the three spellings (§ *What was rewritten*). |
| 13 — whether the seven key names are the file's spellings, and in which section | S12 §4.1's prefix rule does not transfer to a plug-in's section (§ *Cross-reference errors* 1). |
| 14 — whether "playlist" and "view" name the same thing | The two prompts use two different words (§2 of the spec). |
| 15 — what `radio_last_upd` records | A key in the list that no section of the draft explains. |
| 16 — the track-numbering mode's values and effect | §3 names the mode; nothing describes what it does. |
| 17 — whether the token list is complete, and what `track` / `ntracks` do in a template | Two of the seven field names have no word token (§4.2 of the spec). |

## What could not be resolved

1. **Whether the player carries a menu entry for the library.** The plug-in's
   own string table carries the two labels and cannot place them. Needs the
   reference with the plug-in loaded.
2. **The capitalisation rule.** Three spellings side by side are the strongest
   statement the material carries, and whether they choose the case of the
   inserted value is the next step. The dirty room's own measurement recipe
   (extract a playlist with all three spellings, read the file back) stands.
3. **The spelling of the seven keys in the file, and the section they live
   in.** S12 §8.3 shows that a plug-in's key can carry a prefix in the file,
   so the prefix rule does not decide it. Only the file the plug-in writes
   settles it.
4. **The sort semantics.** The `QSort` message shows that a sort exists, not
   how it orders. The draft's own item 3 stands unchanged.
5. **The `radio.vmd` format.** Nothing in the material describes it; the file
   itself would.
6. **The column layout and the default view set.** Not in the strings; they
   need the window.

## Notes for the orchestrator (not this lane's files)

- `docs/spec-requests/2026-10-20-missing-specs.md` §9 is answered and the
  file was updated by this lane: 2.95 has a media library, as a **plug-in**.
  The card closes as "own feature" for the parity list, with the plug-in's
  behaviour documented as a peer reference.
- `docs/tasks/ROADMAP-to-winamp-parity.md` (§ *10. Media Library* and the
  "S20: Media Library (M5)" row) still lists S20 as missing. It was **not**
  touched by this lane, but its "missing specs (…, S18-S20)" line is now one
  item out of date: S20 is audited. Whether the parity list should record the
  library as an own feature is the orchestrator's call.
- `docs/HANDOFF.md` collects the open spec requests; the S20 entry there is
  likewise not touched by this lane.
- `crates/rustamp-ui`'s library is a first-class part of the player; the spec
  does not license any change to it. The spec's §0 says in as many words that
  the differences between the reference's plug-in and our library are our own
  design, not a deviation.
- The reference's library plug-in is a **general-purpose** plug-in, so the
  spec's content is about a third-party-style plug-in rather than about the
  player's own behaviour. If the orchestrator wants the *player's* side (does
  the player carry a menu entry, does the player receive the plug-in's
  playlists?), that is a separate measurement, not this spec.
- No file outside the four artifacts above was touched.

## Verification

- `cargo fmt --all -- --check` → exit 0 (green; this lane changed Markdown
  only).
- `git status --short` → the four artifacts above, plus nothing else.
