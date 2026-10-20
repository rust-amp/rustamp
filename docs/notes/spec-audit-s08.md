# Spec audit S08 — the classic visualisation (the built-in analyser)

Lane: spec audit of the S08 draft (CLEANROOM.md §3, step 3).
Auditor: spec auditor subagent. Audit date: 2026-10-20.

| Artifact | Path in this worktree |
|---|---|
| Audited spec | `specs/S08-visualization.md` (new) |
| Index row | `specs/README.md` — S08 status `not started` → `audited` |
| This note | `docs/notes/spec-audit-s08.md` (new) |

No code was touched. `cargo fmt --all -- --check` stays green (exit 0), which
is expected: this lane changes Markdown only.

## The draft that was audited

The draft is kept outside the repository, under RFC 0021's sandbox path
(`D:\code\rustamp-sandbox\spec-drafts\S08-visualization.draft.md`, 4986 bytes,
last written 2026-10-20 10:31:57, 109 lines, SHA-256
`3ef54502d1da10fc81b72990c21c0829ff7326d9234bf885517ed8b4427a40e5`). It is the
only revision that existed at audit time. It was written from the **drawing
path of the built-in analyser** and from the **configuration file the reference
binary wrote**; the analyser itself was **not observed at runtime**, so the
draft contains no runtime observation at all.

## What the audit checked

- **CLEANROOM §4** — no source code, no pseudo-code transliterated from source
  or disassembly, no copied or paraphrased comments, no distinctive internal
  identifiers, no disassembly listings, no instruction-level descriptions.
- **CLEANROOM §3** — the provenance header is complete and filled in; the spec
  is behaviour, numbers and layout in prose and tables.
- **Epistemic status** — every item the draft called open is still open; no
  reading was promoted to a stated fact; one claim the draft stated flatly was
  demoted to a reading.
- **Internal consistency** — the areas, the band counts, the level scale, the
  fall-off table and the peak table were recomputed against each other. One
  contradiction surfaced in the peak table and two gaps in the geometry (§ *The
  numbers* below); none was smoothed.
- **Cross-references** — every cross-reference the draft made into the repo was
  checked against the file it names. One was wrong (§ *Cross-reference errors*),
  and S12 had already recorded three things the draft re-derived or contradicted.

## What stayed (unchanged in substance)

- The three analyser areas: **76 x 16** px (normal), **152 x 32** px (double
  size), **38 x 5** px (shaded).
- The band counts **10** (normal) and **5** (shaded), and the evidence for them:
  one peak marker per band, for the band indices 0…9 and 0…4 respectively.
- The **7.6 px** share per band (76 / 10 and 38 / 5).
- The level scale: sixteenths of a pixel, a stored 16 being one pixel of bar
  height.
- The rise rule (the bar jumps to the new level, instantaneously) and the fall
  rule (a fixed step per update).
- The fall-off table: the stored steps 3, 6, 12, 16, 32 and the px-per-update
  values 0.1875, 0.375, 0.75, 1.0, 2.0, the clamp to 0…4, and the statement
  that the table is the complete mapping with no interpolation.
- The peak rules: `sa_peaks` = 0 draws no markers, any other value draws one
  per band; the factors 1.05, 1.10, 1.20, 1.40, 1.60 with the same clamp; and
  the fact that the marker's step is not constant.
- The six setting names, the four the bar display reads, and the fact that
  `safire` and `saref` are not read by it.
- Every open item the draft had, in the draft's order (§ *The open list*).

## What was removed, and why

| Removed | Reason |
|---|---|
| The heading "S08 (DRAFT — not an audited spec, never hand this to an implementer)" | The file is now the audited spec; the warning was a property of the draft, not of the spec. |
| The draft's section "What this draft deliberately does not contain" | Meta-commentary about the draft. Its substance moved into §0 of the spec ("Nothing in this spec is source code, a transliterated pseudo-code, a copied comment, an internal identifier or a disassembly listing"), where it is a statement about the spec itself. The same move was made in the S12, S13 and S14 audits. |
| "the blit is `76 << 1` x `16 << 1`" (double-size row) | A code expression, and an instruction-level description that maps one-to-one onto machine code (CLEANROOM §4). The fact it carried — the area doubles — survives as "152 = 2 x 76 and 32 = 2 x 16". |
| "the drawing converts a stored value with a 4-bit shift" | The same: an instruction-level description. The behaviour it carried survives as the level scale ("a stored value of 16 is one pixel of bar height; the drawn height is the stored value divided by 16, rounded down"). |
| "the drawing stores `16 * level` directly" | A code expression. It survives as "the stored value becomes 16 times the new level". |
| The "Source" column of §1's table, and with it the pointers "the drawing path writes a peak marker only for band indices 0…9" and "the shaded path writes peak markers for indices 0…4 only" | Pointers into the dirty material. The behaviour survives in prose: "the display draws one peak marker per band, and in the normal window those markers are the band indices 0…9". |
| "the drawing path above is the bar display; the oscilloscope is a sibling path" | "Sibling path" describes the reference's internal structure. It survives as "the bar display; the oscilloscope is a separate mode". |
| "`safire` and `saref` are **not used by the drawing path**" | The same. It survives as "not read by the bar display". |
| "the drawing reads a per-band level array" (open item 1) | The same. It survives as "the display works on one level per band". |
| The name `GetPCMData` | See § *Cross-reference errors* item 5: the block is out of scope for this spec, and S09 §5.2's audited 2.95 vis module layout does not list such a member, so the name is not carried into `specs/`. |
| "The five settings are the five speeds the Preferences page offers." — **demoted, not deleted** | The drawing path establishes the five entries of the table; it says nothing about the Preferences page. The statement survives as a **reading** in §2.2, with the reason. |

## What was rewritten, and why

| Change | Reason |
|---|---|
| A new §0 "How to read this spec" with an evidence-class table (constant / reading / open), the sentence "**The analyser was not observed at runtime for this spec**", the clean-room sentence and the scope paragraph | The draft stated constants and readings in one voice. The marker discipline is now the one S12, S13 and S14 use. The scope paragraph names what the spec does **not** cover (the 576-entry plug-in block) and says where that item lives (§6, open question 6). |
| §1: the table now carries the area, the band count and the per-band share; the evidence moved into marked bullets | The draft's table mixed a fact, a source pointer and a derived value in one row. The band counts are constants; the 7.6 px share is a reading; the double-size band count is a reading (§ *The numbers*). |
| §1: the double-size band count kept as 10, but marked a reading and given an open question | The draft's evidence for that row covers the area, not the band count, and the geometry admits a second reading. The number is kept; the claim behind it is no longer stated flatly. |
| §2.2: the third column of the fall-off table is now headed "px per update (stored step / 16)" | The draft's column was a bare number. The division that produces all five values is now visible, so an implementer can reproduce the column instead of trusting it. |
| §3 was split into 3.1 (when markers are drawn), 3.2 (the factor table) and 3.3 (what the factor does, and what it does not settle) | The draft's §3 mixed the rule, the table and the contradictory direction claim in one passage. The settled part and the unsettled part are now in separate sections, and the contradiction is published as open question 11. |
| §3.3 states the draft's contradiction instead of smoothing it | "Decay" with a factor above 1, and "accelerates away from the bar as it falls", cannot both hold. The spec keeps the draft's wording visible and names what is missing (the multiplicand). |
| §4: "Six keys in the configuration file control this display" → "Six keys … are the analyser's own settings … The bar display reads **four** of the six", with a per-key table | The draft's sentence contradicted its own next paragraph, which says that two of the six are not used by the display. The per-key table also carries the observed value of each key as S12 §3.5 records it, and the consequence for the two enumerated settings. |
| §4: S12 §3.5's **inferred** reading of `sa` ("0 = oscilloscope") is named as a reading and not promoted | The draft said which value selects which mode is open; S12's row carries a hypothesis. Naming the hypothesis and leaving it open keeps the two specs consistent without turning a guess into a fact. |
| A new §5 "Practical notes for an implementer" | Every statement follows from the spec's own facts: the integer level, the quantisation of the drawn fall, the per-update units, the instantaneous rise, the area size, and the rule that §6 must not be implemented as if settled. Same pattern as S12 §9, S13 §4 and S14 §3. |
| The open list moved to the end and was renumbered | The task the spec exists for: facts first, open questions last. The draft's items kept their relative order (mapping below). |
| The header's SHA-256 written in upper case → lower case, and the long RFC 0021 line | House style: S12's, S13's and S14's headers spell the same hash in lower case and carry the same authorisation sentence. Same hash, same binary. |

## The numbers: what was checked and what came out

### Checks that passed

| Check | Result |
|---|---|
| 76 / 10 = **7.6 px** per band, 38 / 5 = **7.6 px** per band | **consistent.** The shaded area keeps the band width and halves both the band count and the height (16 px → 5 px). The two rows of §1 agree. |
| 152 = 2 x 76, 32 = 2 x 16 | **consistent** with the double-size claim, and with S01 §1's whole-window 2x scale. |
| Does "stored step / 16" reproduce **every** pixels-per-update value of the fall-off table? | **Yes, all five:** 3/16 = 0.1875, 6/16 = 0.375, 12/16 = 0.75, 16/16 = 1.0, 32/16 = 2.0. The column is the step divided by the level scale of §2.1, so it is reproducible and not a second set of numbers. |
| The fall-off table is ordered | **Yes:** 3 < 6 < 12 < 16 < 32, so the setting is a speed choice in the rising direction. |
| The peak factor table is ordered | **Yes:** 1.05 < 1.10 < 1.20 < 1.40 < 1.60, strictly increasing. |
| The two clamps | **Consistent with each other:** both are 0…4, both are "a value of 4 or more behaves like 4", and both five-entry tables are selected by a setting of the same shape. |
| The level scale against the fall-off table | **Consistent:** the table's px column follows from the sixteenths scale, and the rise rule (16 times the level) lands on the same grid. |

### The contradiction found

The peak table is numerically clean and its prose is not. Two clauses of the
draft's §3 cannot both hold:

1. It calls the operation a **decay** of the marker's position, but every
   factor in the table is **above 1**. A multiplier above 1 grows the quantity
   it multiplies, so the marker's position does not decay — its *distance
   already travelled* would.
2. It says the peak **"accelerates away from the bar as it falls"**. "Falls"
   (descending) and "away from the bar" (moving away from it) are opposite
   directions, and the draft never says which way the marker travels.

Two readings fit the numbers equally well, and the numbers cannot decide
between them:

- the factor multiplies the marker's **height above the bar** → the marker
  rises away from the bar, accelerating upwards; then "falls" is the wrong
  word and the marker never descends;
- the factor multiplies the marker's **distance below the peak it fell from** →
  the marker descends towards the bar, accelerating; then "away from the bar"
  is the wrong word.

Both readings give the same table, the same ordering, and the same settled
property — the marker's step is not constant, so a falling marker visibly
speeds up. The spec keeps that property as a constant (§3.3) and publishes the
direction as open question 11. It was **not** smoothed into a fact in either
direction.

### The two gaps in the geometry

- **The double-size band count** is not covered by the draft's own evidence for
  that row (the row's evidence is the area), and the geometry admits both 10
  bands of 15.2 px and 20 bands of 7.6 px. The spec keeps 10 as a reading and
  publishes the question (open question 10).
- **The shaded level scale** is not stated: the normal area is 16 px tall and
  the shaded area 5 px, and the draft does not say whether the shaded path
  uses the same sixteenths-of-a-pixel scale or what a full bar is there
  (open question 12).

### A derived remark the spec adds

The step in §2.2's table counts **stored units, not pixels**. Because the
drawn height is an integer number of pixels, the visible fall is quantised: at
a step of 3 the drawn bar drops one pixel only every 16/3 ≈ 5.33 updates, at a
step of 12 every 16/12 ≈ 1.33 updates, and at a step of 32 it drops two pixels
per update. This follows arithmetically from the draft's two rules (16 stored
units per pixel, a fixed step per update) and is stated in the spec's §5
because an implementer who reads the table as pixel counts will draw a fall the
reference does not have.

## Cross-reference errors found

1. **"(S12 §2.5)"** — the wrong section. S12 §2 is "Shape of the file"; the six
   analyser keys are in **S12 §3.5** ("Spectrum analyser (input for S08)"), and
   the same six names stand in S12 §5.1's recognised key set. The spec now
   cites §3.5 and §5.1.
2. **"(S12 §10)"** for `safire` and `saref` — the right file, but the precise
   pointer is **S12 §10 item 2** ("recognised and observed, meaning unknown"),
   and S12 §3.5 marks the same two keys as open. The spec cites the item.
3. **S12 §3.5's deferral.** S12 §3.5 says the value ranges and "the mapping
   from a setting to pixels and frames" belong to S08's open list. This spec
   settles the **pixel** side for the two enumerated settings (the clamp is
   0…4, the two tables are the complete mapping) and leaves the **frame** side
   open; the ranges of the other four keys stay with S12 §10 item 3. The spec
   says so in §4 instead of restating S12's table.
4. **S12 §3.5's `sa` reading.** S12's row for `sa` carries an *inferred*
   reading ("0 = oscilloscope"), the draft says the mode mapping is open, and
   the two are consistent only because S12 marks its reading as a guess. The
   spec names the reading and does not promote it; the mode mapping stays open
   (open question 5).
5. **`GetPCMData`.** The draft names the function that hands the 576-sample
   block to a `vis_` plug-in. S09 §5.2's audited 2.95 vis module layout does
   not list such a member: its members are `description`, `parent_window`,
   `dll_instance`, `sample_rate`, `channels`, `latency_ms`, `delay_ms`,
   `spectrum_rows`, `waveform_rows`, `spectrum_data`, `waveform_data`,
   `config`, `init`, `render`, `quit` and `user_data`. The name is therefore
   not carried into `specs/`: the block is out of scope for this spec, and the
   ABI name belongs to S09. Open question 6 describes the block without it.
6. **S17 §6 — a cross-check that passed.** S17 gives the analyser's 24
   palette slots, among them the 16-colour spectrum gradient (slots 2–17) and
   the peak-dot colour (slot 23). The normal analyser's 16 px height matches
   the 16 gradient colours, and the shaded analyser's 5 px height matches the
   five oscilloscope colours (slots 18–22). The draft did not use this, and
   the spec does not turn it into a rule; the palette itself stays S17's.
7. **S01 §1 — the double-size scale.** The draft's "the area doubles with the
   window" agrees with S01 §1's whole-window 2x scale. The spec cites S01 for
   the window rule and gives the analyser's own area.
8. **S09's deferrals.** S09's data notes and its open items U-17 (the 75-byte
   analyser row encoding) and U-18 (how the host slices PCM into spectrum
   windows) are delegated to S08. The draft's open item 1 names the same gap
   but does not close it: the transform that fills the bands was not read, so
   neither U-17 nor U-18 is answered. They stay in S09's open list.

## The open list

Kept from the draft, in the draft's order: the band frequency edges; the
update rate; the oscilloscope's sampling; the bar/gap split inside a band's
7.6 px share; which `sa` value selects which mode and whether a third mode
exists; the 576-entry plug-in block (out of scope here, S09 owns the ABI);
whether the shaded analyser uses the same band set (5 of 10) or a different
transform; `safire` and `saref`; and what the display does when nothing plays.

Added by the audit. Each follows from something the draft itself records, and
none of them is a fact the draft did not contain:

| Added | Why |
|---|---|
| 10 — the band count in double size | The double-size row's evidence covers the area, and 152 px fits 20 bands of 7.6 px as well (§ *The two gaps in the geometry*). |
| 11 — the direction of the peak marker's travel | The draft's own wording contradicts itself; the table cannot decide between the two readings (§ *The contradiction found*). |
| 12 — the level scale of the shaded analyser | 16 px against 5 px, with the draft's sixteenths scale stated only for the normal path. |
| 13 — where the analyser area sits in the main window | The spec gives the area's size; S01 records no analyser rectangle, so the position is not established. |

## What could not be resolved

1. **The peak marker's direction.** Unresolvable from the numbers; only a
   dirty-room re-read of the multiplicand or a runtime capture settles it.
2. **The double-size band count.** 10 and 20 both fit the area.
3. **The shaded level scale.** Not stated in the draft.
4. **The area's position in the window.** Not in the draft and not in S01.
5. **Whether the Preferences page really offers five speeds per setting.** The
   draft states it; the drawing path cannot show it, so it stays a reading.
6. **The `sa` mode mapping.** S12's reading ("0 = oscilloscope") is a
   hypothesis, and the draft's mode question stays open.
7. **The 576-entry block.** Out of scope by the task's own definition; S09
   owns the ABI and the data members.

## Notes for the orchestrator (not this lane's files)

- **The repo's own implementation choices differ from this spec on the
  load-bearing numbers.** `docs/tasks/S08-vis-impl.md` §3 chose 19 bands of
  4 px pitch (3 px bar + 1 px gap) in a 76 x 16 area, a linear fall of 24
  rows/s, and a 300 ms peak hold followed by a 16 rows/s fall. This spec says
  **10** bands of 7.6 px in the normal window, a **fixed step per update**
  (0.1875 … 2.0 px per update depending on `safalloff`), and a
  **multiplicative** peak factor. The area size matches the repo's own area;
  the band count, the fall model and the peak model do not. The card's
  numbers are the repo's own choices and this lane did not touch the card.
- **The position of the area is still the repo's own choice** — the card uses
  (24, 43) and this spec does not establish it (open question 13).
- **`docs/spec-requests/2026-10-20-missing-specs.md` §1 asks six questions.**
  This spec answers two of them in part: the band count (Q1 — 10 in the normal
  window, 5 shaded; the pixel columns are 7.6 px shares, and the bar/gap split
  inside a share stays open) and the fall rate (Q3 — a fixed step per update
  and its conversion to px per update; the *frame* rate the question asks for
  is open). The band frequencies (Q2), the oscilloscope sampling (Q4), the
  frame rate (Q5) and the idle rendering (Q6) stay open.
- **`docs/tasks/S08-measurement.md` Q7** asks whether the bar fall-off is
  linear in updates or exponential per update. The spec's §2.2 table is a
  **fixed step per update** (linear in updates) and the §3.2 table is
  multiplicative — so the "linear in updates" hypothesis is the one the
  constants support for the bars, and the capture design can use it as the
  reference hypothesis. A capture still needs the update rate (open question
  2) to turn the per-update step into px/s.
- **`specs/S09-plugin-abi.md`'s U-17 and U-18** are not answered by this spec
  (§ *Cross-reference errors* item 8). The 75-byte analyser row and the PCM
  slicing are outside the draft's evidence.
- No file outside the three artifacts above was touched.

## Verification

- `cargo fmt --all -- --check` → exit 0 (green; this lane changed Markdown
  only).
- `git status --short` → the three artifacts above, plus nothing else.
