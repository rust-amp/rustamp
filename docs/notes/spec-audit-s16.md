# Spec audit S16 — the Preferences pages and their options

Lane: spec audit of the S16 draft (CLEANROOM.md §3, step 3).
Auditor: spec auditor subagent. Audit date: 2026-10-20.

| Artifact | Path in this worktree |
|---|---|
| Audited spec | `specs/S16-preferences.md` (new) |
| Index row | `specs/README.md` — S16 status `not started` → `audited` |
| This note | `docs/notes/spec-audit-s16.md` (new) |

No code was touched. `cargo fmt --all -- --check` stays green (exit 0), which
is expected: this lane changes Markdown only.

## The draft that was audited

The draft is kept outside the repository, under RFC 0021's sandbox path
(`D:\code\rustamp-sandbox\spec-drafts\S16-preferences.draft.md`, 8881 bytes,
last written 2026-10-20 15:41:47, 168 lines). It is the only revision that
existed at audit time. It was written from the reference binary's **dialog
resource strings** — the page labels and the per-option help texts — and
cross-checked against the configuration keys; **the reference was not run for
it and no window was opened for it**, so the draft contains no runtime
observation at all.

## What the audit checked

- **CLEANROOM §4** — no source code, no pseudo-code transliterated from
  source or disassembly, no copied or paraphrased comments, no distinctive
  internal identifiers, no disassembly listings.
- **CLEANROOM §3** — the provenance header is complete and filled in; the
  spec is behaviour, spellings and configuration-key facts, in prose and
  tables.
- **RFC 0021 §2 and §5** — the spec names its source **category** in the
  provenance header, so a later reviewer can see that the evidence is
  string-derived; and this note says what was removed and what was rewritten,
  not only that the spec was approved.
- **Epistemic status** — every item the draft called open is still open; no
  reading was promoted to a stated fact. Two of the draft's unmarked
  conclusions became **readings** (the shuffle morph, the Internet
  connection's mutual exclusivity), and one draft claim was **falsified**
  (the "every option has a key" claim, below).
- **Internal consistency** — the label inventory of §1 was cross-checked
  against the option sections of §3 in both directions. They do not cover
  each other: **five labels have no section and five sections have no label**
  (§ *Internal contradictions* below). The draft's own pointer to the skin
  page pointed at the wrong section.
- **Cross-references** — every cross-reference the draft made into the repo
  was checked against the file it names. **Three were wrong** (§ *Cross-
  reference errors* below); the rest checked out.
- **Absence claims** — the draft's "the tree is open" and "the video page's
  texts are truncated" claims were checked against the repository's own
  records. The tree claim is **complicated by a measurement the repo already
  holds** (§ *What the repository already holds* below).

## What stayed (unchanged in substance)

- All **ten** page labels, in the draft's order: `Setup`, `File types`,
  `Options`, `Display`, `Plug-ins`, `Input`, `Output`, `Visualization`,
  `DSP/Effect`, `General Purpose`.
- The three **inferred** parents (Setup → File types; Options → Display and
  the skin page; Plug-ins → Input, Output, Visualization, DSP/Effect,
  General Purpose) — now marked as a reading.
- **All 26 options**, their help sentences **verbatim**, and the grouping
  into Setup/File types, Options/Display, Options/skins, Setup/agent and
  network, Internet connection, Plug-ins, Language pack and Video.
- The three **Internet-connection** sentences and the three values
  **Idle / Normal / High** of the priority choice.
- The **language pack** pair of strings, the **video page**'s two shortened
  sentences, the **playlist removal** question and the four **EQ preset**
  menu labels.
- The two **configuration key names** the draft names: `prefs_last_page` and
  `shuffle_morph_rate`.
- The **menu entry and shortcut** of §2.
- All **seven open items**, in the draft's order, with their own content.

## What was removed, and why

| Removed | Reason |
|---|---|
| The heading "S16 (DRAFT — not an audited spec, never hand this to an implementer)" | The file is now the audited spec; the warning was a property of the draft, not of the spec. |
| The draft's section "What this draft deliberately does not contain" | Meta-commentary about the draft. Its substance moved into §0 of the spec ("Nothing in this spec is source code, a transliterated pseudo-code, a copied comment, an internal identifier or a disassembly listing"), where it is a statement about the spec itself — the same move the S13, S14, S19 and S20 audits made. |
| The fenced code block holding the ten page labels | A listing of strings in a code block reads as a dump. The labels are user-facing text and are now an inline list in §1. |
| The draft's §4 heading "What this settles for us" and its first bullet ("Our Preferences is a modern design, not a clone. The reference's page set is fixed and cramped; ours is ours.") | A design statement about **our** UI, not a fact about the reference. Its substance moved into §0 as the scope paragraph; the editorial judgement ("cramped") was dropped, because it is an evaluation and not an observation. |
| "The evidence is the reference's own **dialog resource strings** … plus the configuration keys" from the body | A pointer into the dirty material. The evidence categories belong in the provenance header, where they now are; §0 states only what the reader needs to know (no runtime observation, no window opened, three evidence classes, one marker for readings). |
| The draft's phrase "**the configuration keys**" in §3.2 and §4 where it named the key set as "the same set" | The claim was false (§ *The falsified claim* below); it is replaced by the explicit cross-check of §4. |

Nothing that reads as code, a transliterated pseudo-code, a copied comment or
a disassembly listing was found in the draft; no such passage had to be cut.
The names the draft carries are the ones the rules allow: the reference's own
**user-facing text**, its **page labels**, and **configuration key names**
(which are the file format, already published by S12).

## What was rewritten, and why

| Change | Reason |
|---|---|
| A new §0 "How to read this spec" with an evidence-class table (string fact / key fact / reading / practical note / open), the sentences "**The reference binary was not run for this spec, and no window was opened**" and "**Layout and control types are out of scope**" | The draft mixed string facts and conclusions in one voice. The marker discipline is now the one S12, S13, S18, S19 and S20 use. The scope line names what the spec does **not** cover — the dialog's control types, their order and the geometry. |
| §1: the ten labels as an inline list, plus a new §1.2 that cross-checks the labels against the sections in **both** directions | The draft's label list and its option sections do not cover each other. The spec publishes the mismatch as two tables instead of leaving an implementer to discover it. |
| §1: the inference of the parents marked **reading**, and the sentence "The exact tree is **open**" moved to the open list | The draft stated the parentage as "the most plausible parents, marked inferred" and then called the tree open — two statements that sit awkwardly together. The spec keeps the inference as a reading and keeps the question open. |
| §2: the `prefs_last_page` citation corrected (S12 §3.4 → S12 §3.7 and S12 §7) and the second number marked open | The draft cited the wrong S12 sections (§ *Cross-reference errors*) and stated the two-number rule without the fact that the second number's meaning is not established (S12 §10, open question 7). |
| §2: `Ctrl+P` now carries the command id 40012 and the pointer to S03 §1; the menu row points at S04 §4.1 row 1 | The draft said only "the menu table names it". The audited specs name the id, and the id is the public `WM_COMMAND` contract. |
| §3.2: the shuffle-morph paragraph rewritten as a **reading**, with the tension against S15 §1 stated and the measurement that S15 would need named; the `shuffle_morph_rate` citation corrected (S12 §3.2 → S12 §5.1) | The draft's paragraph stated an algorithm ("shuffle is not a jump from one random order to the next, it **morphs**") and called it "an S15 fact". The evidence is one help sentence. (§ *The shuffle morph* below.) |
| §3.3, §3.5, §3.6, §3.8: the table column heads and the row kinds made explicit; §3.5's "mutually exclusive" marked a reading; §3.6's priority row marked as a set of choice values and its button row as a label | The draft's tables mixed three kinds of string — help sentences, choice values and button labels — under one column head, and stated mutual exclusivity as a fact. |
| §3.4: the typo note kept, with the attribution marked as the draft's | The draft's parenthetical "(the typo "when checks" is the reference's)" is **kept**, because the quote is carried verbatim with the typo in it. The auditor did not re-read the string out of the binary, so the attribution rests on the draft — the spec says so. |
| §3.7, §3.8: the language-pack and video groups given their own tables | The draft's video group was prose with two sentences in it; both are options of the video page and belong in a table like the rest. |
| A new §4 with the full option/key cross-check, and the draft's §4 claim replaced by it | The draft's claim was false. §4 publishes the check instead of the claim (§ *The falsified claim*). |
| A new §5 "Practical notes for an implementer" | Every statement follows from the spec's own facts: our page tree is our own choice, the help texts are to be kept verbatim, one key per option is not to be assumed, the key names are the file's spellings, a morphing shuffle is not to be implemented from this spec. Same pattern as S12 §9, S13 §4, S18 §6, S19 §5. |
| The open list moved to the end and was renumbered; four items added | The task the spec exists for: facts first, open questions last. The draft's seven items kept their relative order (mapping below). |
| The header's SHA-256 written in upper case → lower case, and the long RFC 0021 line | House style: S11's, S12's, S13's, S14's, S18's, S19's and S20's headers spell the same hash in lower case and carry the same authorisation sentence. Same hash, same binary. |

## Internal contradictions found

1. **The label inventory and the option sections do not cover each other.**
   The draft's §1 lists ten labels; its §3 has eight option groups. Checked in
   both directions:
   - **Five labels have no section**: `Input`, `Output`, `Visualization`,
     `DSP/Effect`, `General Purpose`.
   - **Five sections have no label**: the skins page (§3.3), the
     agent-and-network group (§3.4), the Internet-connection group (§3.5),
     the language pack (§3.7) and the Video page (§3.8).
   The draft noticed neither half. The spec publishes both halves as two
   tables in §1.2 and asks the open question 8, which of the five unlabelled
   groups are pages of the Preferences dialog at all.
2. **§1's pointer to the skin page names the wrong section.** The draft's §1
   says "the skin-related page's strings (§3.6)"; §3.6 is **Plug-ins**, and
   the skin page's strings are in **§3.3**. Corrected.
3. **§3.2's shuffle paragraph against §5's open list.** The draft's §3.2
   states the morph as "a behaviour we had not recorded" and calls it "an S15
   fact", while the draft's own §5 asks for the remaining pages' option lists
   and the video page's texts — the morph is not among the open items. The
   contradiction is resolved in the direction the audit requires: the morph
   stays **open** (open question 11) and the paragraph becomes a reading.
4. **§4's "the key list is the same set" against the draft's own option
   count.** The draft lists 26 options and says the key list is the same set;
   S12's recognised list holds 109 names. The draft does not notice. See
   below.

## The falsified claim

The draft's §4 said: *"Every option above has a **key** in S12 (the key list
is the same set), so a setting that has a key but no page is a gap on our
side."*

Checked against **S12 §5.1** — the complete set of names the player
recognises, spelled as they appear in the file — the claim is **false in both
directions**.

| Direction | Result |
|---|---|
| Option → key | 6 of the 26 options match a key name outright (`pladv`, `shuffle_morph_rate`, `randskin`, `langpack`, `video_vsync`, `video_aspectadj`). 5 more have a **candidate** that rests on the name alone (`bifont`; `snap`/`snaplen`; `sticon`/`taskbar`/`whichicon`/`whichicon2`/`minst`; `defext`/`extlist`/`check_ft_startup`; `visplugin_priority`/`priority`). 4 sit in a **count mismatch** (`newverchk` + `newverchk2` for one option; `inet_mode` for three settings). **11 have no key of that name at all** — associate audio CDs, retake file types, metadata read timing, skin list, skin directory, usage reporting, agent, QuickLaunch, visualization directory, DSP/effect directory, flush plug-in name cache. |
| Key → option | S12 §5.1 recognises **109** names; the great majority are not Preferences-page options (window geometry, equalizer, playlist, minibrowser, video, analyser, plug-in selection). **30** of them, plus the six plug-in-selection keys, the observed file did not carry at all (S12 §5.2). |

The spec therefore publishes the **cross-check** (§4) instead of the claim,
and the practical note is narrowed to the statement that survives: a setting
that has a key **and** appears in the option inventory of §3 may be one we
are missing; a key outside that inventory is not a gap until the option it
belongs to is found.

A second, smaller mismatch is worth naming: **one option with two keys** and
**three settings with one key**. An implementer who assumes one key per
option will get both wrong.

## The shuffle morph

The draft's §3.2 said: *"The last one is a **behaviour we had not recorded**:
shuffle is not a jump from one random order to the next, it **morphs** — the
order drifts over time, and the setting controls how fast. That is an S15
fact (the shuffle's **shape**) …"*

That is a claim about the **algorithm**, and the evidence is one **help
sentence**: *"This controls how quickly the playlist order changes (morphs)
over time, when shuffle is on."* The sentence is a fact about the text; the
drift is a reading of it.

It also collides with an audited spec: **S15 §1** describes shuffle as a
**permutation without replacement**, generated when playback starts or when
shuffle is toggled on — a model in which the order does not drift. A morphing
order is a different traversal model, so the two cannot both be right, and
nothing in the draft or in the repo settles which is.

The spec therefore: keeps the sentence verbatim as a string fact; marks the
morph as a **reading**; states the collision with S15 §1; names the
measurement S15 would need (play with shuffle on, record the playback order
over time at two different morph-rate settings, see whether the order drifts
or stays a fixed permutation — plus the setting's range, unit and default,
which S12 §10 item 3 leaves open for every numeric key); and keeps the
question open as open question 11. The practical note §5 adds: **do not
implement a morphing shuffle from this spec.**

## Cross-reference errors found

1. **"`prefs_last_page`, S12 §3.4"** — S12 §3.4 is "Windows: position, size,
   shade, visibility" and carries no such key. The key is in **S12 §3.7**
   ("Miscellaneous"), and the rule the draft quotes — one integer whose low
   byte is the page and whose bits from bit 8 upwards carry a second number
   — is **S12 §7** ("`prefs_last_page` — two numbers in one key"), whose
   second number is **open** (S12 §10, open question 7). The spec cites **S12
   §3.7** and **S12 §7** and carries the open marker.
2. **"the configuration key `shuffle_morph_rate` (S12 §3.2)"** — S12 §3.2 is
   "Volume, balance, EQ". The key is in **S12 §5.1** (the transport group),
   and S12 §5.3 records that it is recognised but **not itemised** in S12 §3
   — one of the counts S12 itself flags as open. The spec cites **S12 §5.1**
   and **S12 §5.3**.
3. **"the skin-related page's strings (§3.6)"** — §3.6 is *Plug-ins*; the
   skin page's strings are in **§3.3** (see *Internal contradictions* 2).

Two more of the draft's cross-references checked out and are kept: **S12
§10 item 8** is indeed the item that says the defaults were not read, and
**S20** does say that the media library is a separate general-purpose
plug-in with a window of its own and not a page inside the player (S20 §0,
§1).

## What the repository already holds

The owner rule ("before you write *X does not exist*, search the repo for a
measurement of X") applies here, and it changes the open list. The tree
already carries **partial measurements of the reference's Preferences
window**, taken in other lanes and **not** part of this spec's evidence:

| Where | What it records |
|---|---|
| `docs/HANDOFF.md` (VM run `20261016-204109`) | the Preferences family is **one 443 × 336 frame** with pixel-distinguishable pages; **bare `Ctrl+P` opens the Setup page**; the skin-browser command opens the **Preferences frame on the Skins page, not a separate window** |
| `docs/rfc/0005-app-commands.md` §7 (measurement of 2026-10-15) | command **40191** opens the dialog at **Plug-ins > Visualization**; command **40317** opens the same dialog at **Options > Visualization** |

Three consequences:

1. The draft's open item 1 ("the tree's parentage") is **not** untouched by
   what the repo holds: `Ctrl+P` → **Setup** and the skin browser → **Skins**
   name two pages and their relation to the frame, and the RFC's two paths
   name two more relations.
2. The draft's **inference** (§1.1, Plug-ins → Visualization) is
   **corroborated** by the RFC's 40191 path — and **complicated** by its
   40317 path, "Options > Visualization", which does not follow from the ten
   labels at all (the draft puts `Display` and the skin page under `Options`
   and `Visualization` under `Plug-ins`).
3. The draft's open item 2 asks for "the dialog's size" as something that
   "needs the window (a screenshot), not the strings" — the size **has** been
   measured in another lane.

The spec keeps both items open (the run record behind the RFC's measurement
is not in this repository, and a handoff summary is not a spec fact — the
same rule the S13 audit applied to the `/ADD` measurement), and adds §1.3, a
pointer to both records, so that the next measurement starts from them
instead of repeating them. The dirty room should reconcile the two.

## The open list

Kept from the draft, in the draft's order: the tree's parentage; control
types and layout; the video page's full texts; the remaining pages' option
lists; the music library's settings; whether the priority choice is
page-level or per-plug-in; the default value of every option.

Added by the audit. Each follows from something the draft itself records, and
none of them is a fact the draft did not contain:

| Added | Why |
|---|---|
| 8 — the page set behind the labels: which pages exist beyond the ten labels, and which of the five unlabelled groups are pages at all | The draft's label inventory and its option sections do not cover each other (§ *Internal contradictions* 1). |
| 9 — the encoding of the three Internet-connection settings in `inet_mode`, and whether the three texts describe one choice or three | §3.5 gives three sentences and S12 gives one key; the draft asserted mutual exclusivity without evidence. |
| 10 — which of `newverchk` and `newverchk2` the update-check option writes, and what the second is for | §4 shows two keys for one option; S12 §3.7 marks both as inferred. |
| 11 — whether the shuffle order drifts over time at all, and what the morph rate does | §3.2's text and S15 §1's permutation model disagree (§ *The shuffle morph*). |

Nothing was removed from the open list and no item was promoted to a fact.

One re-scope inside a kept item: the draft's open item 4 offered the
checkbox label "Always on top" as an example of a short label that leaves no
long string behind. The example is the draft's illustration and **not** a
verbatim reference label — the reference's own menu reads `Always On Top`
(S04 §4.1 row 5) — so the spec names it as such.

## What could not be resolved

1. **The tree.** The spec publishes the ten labels and the eight groups and
   the mismatch between them; which is which can only be settled by the
   window. The repository's own records (§1.3) narrow it but do not settle
   it, and their two "Visualization" paths disagree with each other.
2. **The typo "when checks".** The quote is carried verbatim with the typo,
   as the draft gives it; the auditor did not re-read the string, so the
   attribution to the reference rests on the draft. The dirty room should
   confirm the exact spelling before an implementer copies the sentence.
3. **The option → key mapping.** §4's second column is a **reading of a
   name** throughout; only six of the 26 rows are outright matches and the
   eleven rows with no key are the exceptions to the draft's claim. Which
   key an option really writes can only be settled by changing the setting in
   the reference and reading the file back.
4. **Whether the unlabelled groups are pages.** "Language pack", "agent and
   network", "Internet connection" and "Video" may be pages, tabs, groups or
   dialogs; the strings do not say.
5. **The completeness of the option lists.** The draft's own open item 4
   says it: a page whose options carry short labels leaves no long string
   behind, so a page may hold options that no string in the table records.
   §3 is an inventory of what the strings carry, not a proof that it is
   everything the reference offers.

## Notes for the orchestrator (not this lane's files)

- `docs/tasks/ROADMAP-to-winamp-parity.md` still lists S16 as **not started**
  (§ *Still missing* and the § S16 rows). It was **not** touched by this
  lane — it is not one of this lane's three files — but it is now one line
  out of date: S16 is audited, with the open list above.
- `docs/HANDOFF.md` collects the open spec requests; the S16 entry there is
  likewise not touched by this lane.
- `docs/spec-requests/2026-10-20-missing-specs.md` asks, for S16, that "the
  labels have to arrive in the reference's **own words**" and that the
  measurement read the reference's resources. This spec answers the first
  half: the ten page labels and the option help texts are in the reference's
  own words. The second half — the page **tree**, the controls and their
  labels — is what the dirty room still owes, and §1.3 says which existing
  records to start from.
- `crates/rustamp-platform/src/prefs_dialog.rs` carries the preferences
  dialog, and `docs/notes/BETA-readiness.md` (P2, BETA-10) says the
  preference pages are skeletons. This spec licenses no change there; it
  gives the inventory against which a missing option can be seen.
- The draft's claim that every option has a key in S12 was false (§ *The
  falsified claim*). If any lane has implemented "one key per option" from
  that claim, it needs a follow-up — the spec's §4 is the corrected
  cross-check.
- No file outside the three artifacts above was touched.

## Verification

- `cargo fmt --all -- --check` → exit 0 (green; this lane changed Markdown
  only).
- `git status --short` → the three artifacts above, plus nothing else.
