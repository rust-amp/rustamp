# Spec audit S11 — embed window and `gen.bmp` sheet

Lane: spec audit of the S11 draft (CLEANROOM.md §3, step 3).
Auditor: spec auditor subagent. Audit date: 2026-10-20.

| Artifact | Path in this worktree |
|---|---|
| Audited spec | `specs/S11-embed-gen.md` (new) |
| Index row | `specs/README.md` — S11 status `not started` → `audited` |
| This note | `docs/notes/spec-audit-s11.md` (new) |

No code was touched. `cargo fmt --all -- --check` stays green (exit 0), which
is expected: this lane changes Markdown only.

## The draft that was audited

The draft is kept outside the repository, under RFC 0021's sandbox path
(`D:\code\rustamp-sandbox\spec-drafts\S11-embed-gen.draft.md`, 5076 bytes, last
written 2026-10-20 13:34:22, 98 lines of which 82 are non-empty). It is the
only revision that existed at audit time. It was written from three artifacts:
the reference's own sheet-name list, the BMP headers of the classic base skin
the reference ships, and the configuration file the reference wrote. **The
reference was not run for it**, and **no sprite rectangle was measured** — the
draft says so itself, in its scope line and in its last section.

## What the audit checked

- **CLEANROOM §4** — no source code, no pseudo-code transliterated from source
  or disassembly, no copied or paraphrased comments, no distinctive internal
  identifiers, no disassembly listings.
- **CLEANROOM §3** — the provenance header is complete and filled in; the spec
  is behaviour, sizes and file-format facts, in prose and tables.
- **Epistemic status** — every item the draft called open is still open; no
  reading was promoted to a stated fact; three statements the draft made as
  facts were demoted to readings or to their evidence: the `genex.bmp`
  purpose, the video window's "default" size, and the aspect ratio's
  "default 4:3".
- **Internal consistency** — the sheet-name list, the sizes table and the
  draft's per-sheet statements were cross-checked against each other, and against the
  audited specs that carry the same material (S01 §5, S17 §2 and §4, S12 §3.4
  and §3.6, S09 §5.3 and §7.3, S10 §12.2). Four contradictions surfaced; none
  was smoothed (§ *Contradictions* below).
- **Cross-references** — every cross-reference the draft made into the repo
  was checked against the file it names. Three were wrong (§ *Cross-reference
  errors* below).

## What stayed (unchanged in substance)

- All **six measured sizes** and their encodings: `gen.bmp` 194 x 109 (RLE8),
  `genex.bmp` 130 x 75, `video.bmp` 234 x 119, `mb.bmp` 234 x 119,
  `main.bmp` 275 x 116 (RLE8), `pledit.bmp` 280 x 186.
- All **eighteen sheet names** of the reference's list, and the fact that
  `genex.bmp` is named apart from that list.
- The two facts the draft flags as easy to get wrong: `gen.bmp` is not 275
  wide, and `video.bmp`/`mb.bmp` share a sheet size although the two windows
  do not.
- All **four settled behaviours**, in the draft's order, with the scope of each
  one now spelled out.
- All **nine open items**, in the draft's order.
- The draft's own declaration that no sprite rectangle was measured.

## What was removed, and why

Section numbers in the two tables below are the **draft's** numbering, unless
the row names the spec explicitly.

| Removed | Reason |
|---|---|
| The heading "S11 (DRAFT — not an audited spec, never hand this to an implementer)" | The file is now the audited spec; the warning was a property of the draft, not of the spec. Same move as in the S08, S13 and S14 audits. |
| The draft's section "What this draft deliberately does not contain" | Meta-commentary about the draft. Its substance moved into §0 of the spec ("Nothing in this spec is source code, a transliterated pseudo-code, a copied comment, an internal identifier or a disassembly listing") and into the scope line, which now says explicitly that the sprite rectangles are not in this spec. |
| "The reference's own sheet list names, in one table:" | The phrase pointed at how the reference stores the names internally. That is the reference's internal structure, which CLEANROOM §1 puts out of bounds; the fact an implementer needs — which eighteen names the reference holds, and that `genex.bmp` is named apart from them — is kept. |
| The bolding of `gen.bmp`, `video.bmp`, `mb.bmp` and `genex.bmp` inside the name list | Emphasis without information; the spec marks the load-bearing names in prose instead. |
| §2's enumeration of the `video_*` keys ("ratio as a fraction, vsync, overlays, auto-open/auto-close, OSD, YV12, stop-close, leave-fullscreen-on-stop") | A restatement of S12 §3.6, which already carries the key set and each key's meaning. The spec points at S12 instead. |
| §2's parenthetical "(The call itself is S09's; this spec covers what is drawn.)" | Wrong attribution, and it hid an open question — see § *Cross-reference errors* 2. The spec now says the request's shape is not in this spec and is open (open question 9, with question 13 naming the two candidate owners). |

No passage had to be cut for being source code, a transliterated pseudo-code, a
copied comment or a disassembly listing — the draft was already careful about
CLEANROOM §4. The names it carries are the ones interoperability requires: the
skin contract's file names and the configuration contract's key names.

## What was rewritten, and why

| Change | Reason |
|---|---|
| A new §0 "How to read this spec" with an evidence-class table (name fact / header measurement / reading / settled behaviour / open), the sentence "**The reference binary was not run for this spec**" and a scope paragraph | The draft mixed name-derived facts, header measurements and readings in one voice. The marker discipline is the one S08, S12, S13 and S14 use. The scope paragraph names what the spec does **not** cover — the sprite rectangles, and the embed request's shape. |
| The draft's §1 heading "The sheets the classic skin ships" → the spec's §1 heading "The sheet names the reference holds", plus a paragraph on the difference between the two | The heading was false for the list it introduced: the list holds `nums_ex.bmp`, which the classic skin does not ship, and leaves out `genex.bmp`, which it does. The draft's own §1 bullet (the `genex.bmp` asymmetry) and the base skin's measured list (S01 §5, S17 §3) made the mismatch visible. |
| The draft's §1 sentence "`genex.bmp` is **named separately**: its name occurs in the reference, but not in that list of eighteen." | The draft's wording ("named separately, outside that list") was kept, but split into the two facts it rests on: the name occurs, and it is not in the list. |
| The spec's §2 gained "These sizes and encodings agree with … S01 §5 … and S17 §4" | The draft stated the six sizes without saying that the repo's two audited specs carry the same measurements. That agreement is the strongest thing an implementer can be told about them. |
| The draft's §2 second bullet: "the video window's default is 275 x 232" → "In the configuration file the reference wrote, the video window was **275 x 232** and the minibrowser **350 x 348** (S12 §3.4). Those are the sizes observed in one run of the reference, not established defaults (S12 §10 item 8)." | Two fixes in one sentence: "default" was a promotion (S12 §10 item 8 says the defaults were not read), and the comparison the claim rests on was half missing — the draft never gave the minibrowser's size. |
| The draft's §2 second bullet: "different client sizes" → "the two windows do not", and "the sheet is a set of pieces" → "a sheet that is smaller than the window it dresses cannot be a full-window backdrop: it is a set of pieces" | "Client size" is not established: the keys name the window's width and height, and whether the saved size counts the drawn frame is unknown. The new phrasing makes the deduction from the two measured sizes explicit and points at open question 11. |
| The draft's §3 became the spec's §3 "What the sheets are for", marked **readings** | The draft stated the purposes in the same voice as the measurements. No rectangle was measured, so "the frame and buttons the player draws" is a reading from the sheet's name and its window. |
| The draft's §3 "`genex.bmp` — the sheet named outside the main list, i.e. an **extra** sheet for the same window" → "It reads as an **extra** sheet for the same window" | The draft's "i.e." turned a reading into a fact. What the sheet carries and when it is used is open in the draft's own list (items 2 and 6). |
| Each of the draft's four settled items gained a "rests on" line in the spec's §4 | The draft called them settled without saying what they rest on. Each rests on the configuration key meanings of S12 and, for item 2, on S17 §2's verified part — and S12 §10 item 11 says the behaviour of every key in S12 §3 is unmeasured. The statement survives; its evidence is now visible. |
| The draft's §4 item 3: "default 4:3" → "The values observed in the reference's own configuration file are **4** and **3**; that they are the *defaults* is **not** established (S12 §10 item 8)." | Same promotion as the window size. |
| The draft's §4 item 2: "a skin without them is still a skin (S17 owns the missing-file fallbacks), so the embed window must have a defined look without `gen.bmp`" → the verified part of S17 §2, its open part, and "the embed window looks like without `gen.bmp` is **open** here" | The draft's "must have a defined look" is a requirement the draft's own open item 5 contradicts. See § *Contradictions* 1. |
| A new §5 "Practical notes for an implementer" in the spec | Every statement follows from the spec's own facts: the sizes are the base skin's, not a format requirement; both encodings must be accepted; sheets are drawn opaquely; lookup folds case; do not derive the embed sprites from `main.bmp`; the video window's geometry comes from S12. Same pattern as S08 §5, S12 §9 and S14 §3. |
| The open list moved to the end and was renumbered | The task the spec exists for: facts first, open questions last. The draft's items kept their relative order (mapping below). |
| The header's SHA-256 written in upper case → lower case, and the long RFC 0021 line | House style: S08's, S12's, S13's and S14's headers spell the same hash in lower case and carry the same authorisation sentence. Same hash, same binary. |

## Cross-reference errors found

1. **"S12 §2.6"** for the video keys — the wrong section, and a section that
   does not exist (S12's §2 has no subsections). The video window's position,
   size and visibility are in **S12 §3.4** (`video_wx`, `video_wy`,
   `video_width`, `video_height`, `video_open`); the video behaviour keys are
   in **S12 §3.6**. The spec cites both.
2. **"The call itself is S09's"** — S09 does not own the embed call. S09
   §5.3 ("The plugin owns its window (or embeds one via the embed API, S11)")
   and §7.3 ("creates windows (embedded via S11)") both point the embedding
   surface **at this spec**, and the general-purpose struct S09 §7.1 lays out
   carries no embed call at all. S10 §12.2 lists the embed IPC ids — 505
   (`IPC_GET_EMBEDIF`), 532 and 533 — as host answers and marks them never
   probed. The spec now says the request's shape is open and names the two
   candidate owners (open question 13).
3. **"S17 owns the missing-file fallbacks"** — correct as a pointer, but the
   draft used it as if it settled the case of a missing `gen.bmp`. S17 §2
   verifies the fallback for `main.bmp` and `titlebar.bmp` only and leaves the
   per-file matrix **not determined**. The spec keeps the pointer and marks
   the open part (open question 5).

## Contradictions found

| # | Contradiction | Resolution |
|---|---|---|
| 1 | §3 of the draft ("Behaviour that is settled") settled "the embed window must have a defined look without `gen.bmp`", while §4 of the same draft ("What is open") listed "what is drawn when the skin has no `gen.bmp`" as open. | The fallback rule stays settled — with S17 §2's verified part named, and its open part named too. The embed window's look without `gen.bmp` stays **open** (open question 5). The draft's "must" was a requirement, not an observation. |
| 2 | §1's heading said "the sheets the classic skin ships", but the list holds `nums_ex.bmp` (which the classic skin does not ship) and leaves out `genex.bmp` (which it does). | The list and the file set are now two separate statements (§1), with the difference spelled out and S01 §5 / S17 §3 named as the sources for the file set. |
| 3 | "the video window's default is 275 x 232" — S12 §3.4 records 275 x 232 as the **observed** value in one configuration file, and S12 §10 item 8 says the defaults were not read. | Demoted to "observed in one run", for the window size and for the aspect ratio alike (§4 item 3). |
| 4 | "`video.bmp` and `mb.bmp` are the same size although the two windows have different **client sizes**" — the draft quoted only one of the two sizes, called it a default, and the minibrowser's size was missing; and "client size" is not established for a window whose size the configuration file stores as the window's width and height. | Rewritten with both observed sizes, the "observed, not default" marker, and the word "client" dropped; whether the saved size counts the drawn frame is published as open question 11. |

## The counts: the lists against each other

| Check | Result |
|---|---|
| Sheets named in the draft's "What the sheets are for" section | `gen.bmp`, `genex.bmp`, `video.bmp`, `mb.bmp` — **all four appear in the spec's §1** (`genex.bmp` as the separately named one). No sheet is used there that §1 does not name. |
| Names in §1's list | **eighteen**, plus `genex.bmp` named apart from them: **nineteen** names the reference holds. |
| Rows in the spec's §2 sizes table | **six** of the nineteen names; the other thirteen have no size in this spec. S01 §5 carries the complete measured table for the base skin's eighteen sheets — of which the base skin ships **every** name of §1's list except `nums_ex.bmp`, **plus** `genex.bmp`. |
| Encodings in the spec's §2 table | All six agree with S01 §5 and with S17 §4's list of the base skin's RLE8 sheets (`main`, `gen`) and uncompressed sheets (`pledit`, `genex`, `video`, `mb`). |

## The open list

Kept from the draft, in the draft's order: the sprite rectangle table for
`gen.bmp`; the same for `genex.bmp` and what selects it; the embed window's
client size and minimum size and what happens on a resize request; which
bottom buttons exist in the embed window and what each does; what is drawn
when the skin has no `gen.bmp`; whether `genex.bmp` is required for the window
to open; the video window's chrome table in `video.bmp` and its minimum size;
whether the embed window is the same window as the video window; the `gen_`
embed request's shape.

Added by the audit. Each follows from something the draft itself records, and
none of them is a fact the draft did not contain:

| Added | Why |
|---|---|
| 10 — whether `genex.bmp` is loaded through the same path as the other sheets | The name is not in the reference's list of eighteen while the file is in the base skin (§ *Contradictions* 2). The name's absence is a fact about the names, not about the loading. |
| 11 — what the sheet's pieces cover (inside the saved window rectangle, or around it) | The deduction in §2 — a sheet smaller than the window cannot be a backdrop — needs this to become actionable. |
| 12 — the sprite table in `mb.bmp` | The draft's scope excludes every sprite table, but `mb.bmp` is one of the four sheets the spec measures, and item 7 asks the same question of `video.bmp`. |
| 13 — which surface owns the embed request | The contradiction in § *Cross-reference errors* 2: S09 points embedding at this spec, S10 lists the embed IPC ids as never-probed host answers. |
| 14 — whether the reference holds further sheet names beyond the eighteen and `genex.bmp` | The list is what the draft read; nothing establishes that it is complete, and an implementer could otherwise read it as the complete skin contract. |

## What could not be resolved

1. **Every sprite table** (items 1, 2, 7, 12). No rectangle was measured, and
   none can be derived from a sheet's size. The repo already carries a
   **provisional, free-source** table for `gen.bmp`
   (`crates/rustamp-skin/src/webamp.rs`, every row marked "provisional: webamp;
   S11 out of scope") — a candidate for a measurement to confirm or refute, not
   evidence for this spec.
2. **Whether `genex.bmp` is loaded at all** (item 10), and what selects it
   (item 2).
3. **The embed request's shape** (items 9 and 13). The dirty room must
   establish it, and it must then say which of S09, S10 or this spec owns it.
4. **Whether the embed window and the video window are the same window**
   (item 8) — the draft's "What the sheets are for" section already
   distinguishes their sheets, but nothing establishes whether the reference has
   one window or two.
5. **The client size, the minimum size and the resize behaviour** (item 3).
6. **The completeness of the sheet-name list** (item 14).

## Notes for the orchestrator (not this lane's files)

- **The repo's own spec request for S11 is not answered.**
  `docs/spec-requests/2026-10-20-missing-specs.md` §2 asks four questions:
  the `gen.bmp` sprite rectangles, the embed window's client and minimum size
  and the resize behaviour, the bottom buttons, and whether `gen.bmp` is
  required and what is drawn without it. This spec answers **none** of them
  as a fact — it settles the sheet inventory, the sizes and the encodings, and
  it states the four questions as open items. That is the honest outcome of a
  draft that measured no rectangle; the request's entry for S11 stays open and
  the row in that file was not touched by this lane.
- **The theme's sheets need no change.** `docs/tasks/J2-rust-theme-skin.md`
  and `crates/rustamp-skin/generator/` already generate `gen.bmp` (194 x 109),
  `genex.bmp` (130 x 75), `mb.bmp` and `video.bmp` (234 x 119 each) from
  S01 §5's sizes, which are the sizes this spec confirms.
- **S01's pointer stays valid.** S01 §5 and §7 item 9 say the `gen.bmp` /
  `genex.bmp` sprite tables belong to S11. This spec does not deliver them;
  the pointer is still correct.
- **S10's rows stay as they are.** S10 §12.2 marks ids 505, 532 and 533 as
  "host (embedding is S11)" and never probed; this spec leaves the request's
  shape open, so those rows do not change.
- **The host's window class names live in S09 §8.1** — it records `Winamp Gen`
  ("frames for embedded windows") next to `Winamp MB` and `Winamp Video`. The
  draft carries nothing about window classes, so this spec does not restate
  them; an implementer who needs the class name has S09 for it.
- **S12's video rows are unchanged** by this lane; the spec only cites them.
- No file outside the three artifacts above was touched.

## Verification

- `cargo fmt --all -- --check` → exit 0 (green; this lane changed Markdown
  only).
- `git status --short` → the three artifacts above, plus nothing else.
