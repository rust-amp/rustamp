# Spec audit S13 — command-line switches, single instance, drag-and-drop rules

Lane: spec audit of the S13 draft (CLEANROOM.md §3, step 3).
Auditor: spec auditor subagent. Audit date: 2026-10-20.
Second pass: 2026-10-20 — a fact in the audited spec was found to be wrong
(`/ADD` **is** a switch). The spec is amended in place; this note records what
changed, why the first pass missed it, and what else in the spec rests on the
same kind of evidence.

| Artifact | Path in this worktree |
|---|---|
| Audited spec | `specs/S13-command-line.md` (new) |
| Index row | `specs/README.md` — S13 status `not started` → `audited` |
| This note | `docs/notes/spec-audit-s13.md` (new) |
| Second pass | `specs/S13-command-line.md` (amended in place) and this note. `specs/README.md` is unchanged — S13's row already reads `audited`. |

No code was touched. `cargo fmt --all -- --check` stays green (exit 0), which
is expected: this lane changes Markdown only.

## The draft that was audited

The draft is kept outside the repository, under RFC 0021's sandbox path
(`D:\code\rustamp-sandbox\spec-drafts\S13-command-line.draft.md`, 5260 bytes,
last written 2026-10-20 10:00:02, 104 lines). It is the only revision that
existed at audit time. It was written from the reference binary's switch table
and its shell-registration strings; **the reference was not run for it**, so
the draft contains no runtime observation at all.

## What the audit checked

- **CLEANROOM §4** — no source code, no pseudo-code transliterated from source
  or disassembly, no copied or paraphrased comments, no distinctive internal
  identifiers, no disassembly listings.
- **CLEANROOM §3** — the provenance header is complete and filled in; the spec
  is behaviour, spellings and file-format facts, in prose and tables.
- **RFC 0021 §2 and §5** — the spec names its source **category** in the
  provenance header, so a later reviewer can see that the evidence is
  disassembly-derived; and this note says what was removed, not only that the
  spec was approved.
- **Epistemic status** — every item the draft called open is still open; no
  reading was promoted to a stated fact; the draft's "observed" wording was
  corrected where nothing was observed.
- **Cross-references** — every cross-reference the draft made into the repo
  was checked against the file it names. Two were wrong (§ *Cross-reference
  errors* below).

## What stayed (unchanged in substance)

- Every switch spelling and the complete list of seven switches: `/CLASS=`,
  `/CONFIG=`, `/BOOKMARK`, `/INSTALL`, `/UNREG`, `/NOREG`, `/ONLYINSTALL`. —
  the seven spellings stand, but the word "complete" was the falsified part:
  the list has **eight** switches, `/ADD` was missing, and the completeness
  itself is now an open question (§ *Second pass* below, §5 item 13 of the
  spec).
- The forms: a leading `/`; the value directly after `=` with no space; the
  bookmark form and its `%s` template; several paths per command line, mixed
  with the switches. — the first pass's rendering of the bookmark command line
  (`"/BOOKMARK <path>"`) was superseded by the exact string
  `"%s" /BOOKMARK "%%1"`.
- The negative list: no `/ADD`, `/PLAY`, `/NEW`, `/EJECT` or `/TASKBAR` in
  this build, and an unknown token is a file argument. — **the `/ADD` part of
  this bullet is wrong and is corrected in the second pass below**; the other
  four names stand, scoped to the reference's switch list.
- The three ProgIDs `Winamp.File`, `Winamp.PlayList`, `Winamp.SkinZip`; the
  verb names `open`, `Play`, `Enqueue`, `ListBookmark`, `install`; the
  directory row's `Play` / `Enqueue` and its `Winamp.Play` / `Winamp.Enqueue`
  identifiers.
- The `…\shell\Play\DropTarget` and `…\shell\Enqueue\DropTarget` key shape.
- The `Enqueue` display-name fact; the "a playlist file has the same verbs as a
  media file" fact; the `SkinZip` → `install` fact.
- The single-instance shape: a registered window class, `/CLASS=` overriding
  it, a second start handing its arguments to the running instance.
- The idempotence reading and the `/UNREG` pairing.
- Every open item of the draft, in the order the draft had them (§ *The open
  list* below).

## What was removed, and why

| Removed | Reason |
|---|---|
| The heading "S13 (DRAFT — not an audited spec, never hand this to an implementer)" | The file is now the audited spec; the warning was a property of the draft, not of the spec. |
| The draft's section "What this draft deliberately does not contain" | Meta-commentary about the draft. Its substance moved into §0 of the spec ("Nothing in this spec is source code, a transliterated pseudo-code, a copied comment, an internal identifier or a disassembly listing"), where it is a statement about the spec itself. |
| "The evidence is the binary's own **switch table and shell-registration strings**" from the body | A pointer into the dirty material. The evidence categories belong in the provenance header, where they now are; §0 states only what the reader of the spec needs to know (no runtime observation, two evidence classes, one marker for readings). |
| "Verbs **observed**" as the table's column head | "Observed" was wrong: nothing was observed, the verbs were read out of the reference's strings. The column head is now "Verbs", and the spec says plainly that the registration was not observed in a running shell. The draft's "A VM observation is needed" stays where the draft put it — on the in-window drop rule, which genuinely needs it. |
| "those behaviours belong to the shell verbs in §3, not to the command line" | The verb table of §3 has no verb that corresponds to `/EJECT` or `/TASKBAR`, so the sentence overstates what §3 provides. The spec now says the corresponding behaviours are reached through the shell verbs and that which verb stands in for which missing switch is not established. |
| "which is why dragging a file onto the player or onto the player's shortcut has two possible meanings" | It folded the in-window drop into the shortcut drop. The in-window drop is the draft's own open question 1 — it is not settled, and a sentence that says otherwise reads as a fact. The spec keeps the shortcut case and points the window case at the open list. |
| "The reference writes three verbs per file type and per directory" and "A playlist file has the same three verbs as a media file" | Both numbers are wrong; see the counts below. The spec publishes the table's own lists. |
| "The default class name is case-sensitive data of the binary" | Garbled beyond use. The fact kept is the case-sensitivity of the name, stated in one clause. |
| The parenthetical "`Winamp v1.x` appears in S10's catalog" | A wrong cross-reference (§ *Cross-reference errors* below). The name is now attributed to S09, which is where it is published. |
| The draft's parenthetical "the exact spelling is **not** recorded here because it is an interoperability name that S10 owns (`Winamp v1.x` appears in S10's catalog) — a spec that restates it in a second place invites drift" | The draft contradicted itself: it said the spelling is not recorded here and then wrote it twice (§1's example and this parenthetical). The spec records it **once**, in §2, with the pointer to the audited spec that owns it. The drift risk the draft wanted to avoid is now actually avoided. |

Nothing that reads as code, a transliterated pseudo-code, a copied comment or a
disassembly listing was found in the draft; no such passage had to be cut. The
draft was already careful about §4 — the only names it carries are the ones
interoperability requires (switch spellings, ProgIDs, verb names, the
`DropTarget` key shape, the `WM_DROPFILES` message name).

## What was rewritten, and why

| Change | Reason |
|---|---|
| A new §0 "How to read this spec" with an evidence-class table (string fact / table fact / reading / open) and the sentence "**The reference binary was not run for this spec**" | The draft mixed string-derived spellings, table-derived pairings and name readings in one voice, and used "observed" for things that were not observed. The marker discipline is now the same one S12 uses. |
| The switch table's third column head "Behaviour" → "What the reference pairs it with" | "Behaviour" claims more than the evidence carries. |
| §1: "the two value-taking switches spell the value directly after `=`, with no space (`/CLASS=Winamp v1.x`)" → "… the value runs to the end of the argument and may contain spaces (`/CLASS=<name>`)" | Keeps the shape the example carries (a value with a space inside it, no space after the `=`) without writing the class name a second time. |
| §1: a cross-reference to S02 §1 for what a file argument does | S02 (audited) measured that a file on the command line starts playing it, and that an entry passed while stopped is loaded and shown in the marquee. The draft said only that an unknown token is a file argument. The cross-reference states the measured part and names what it does not cover (several file arguments at once). |
| §2: the class name is now given once, with "**S09 is the normative home for that name**", plus the S09 and S01 pointers | Wrong attribution (below), and it removes the draft's own duplication. |
| §2: "The hand-over is a window message, so it is S10's subject, not this spec's." → "The hand-over is a window message. **Which message it is, is not stated here**: S10 covers the plugin-facing IPC channel … and not the single-instance hand-over, so no audited spec currently names it." | The draft delegated the message to a spec that does not carry it. The gap is now stated and listed as an open question instead of being hidden behind a cross-reference. |
| §3's table got a Handler column and the ProgID column was split off the directory row | The draft's single "ProgID" column held a ProgID in three rows and "a directory" in the fourth, with two further identifiers in a parenthesis that nothing explained. |
| §3: the counts are stated once, derived from the table (four / one / two) | The draft's prose contradicted its own table (§ *The counts* below). |
| §3: "Registration is idempotent by construction (the same keys are written), and `/UNREG` removes exactly those keys." → "**Reading:** registration writes the same key set each time, so registering again changes nothing, and `/UNREG` is the matching removal step for those keys. Neither was observed in a running shell." | "By construction" is a claim about the implementation, and "exactly" is a claim about a registry state nobody looked at. The inference survives as a reading; the overstatement does not. |
| §3: "The `Enqueue` label is the verb's display name." kept, with the display names of the other verbs named as not recorded | The draft stated one display name and left the others silently unstated. |
| §3: the `DropTarget` bullet now says what the entries carry is not recorded | The draft gave the key shape and nothing else; an implementer would otherwise have to guess the value. Named as an open question instead of being glossed. |
| A new §4 "Practical notes for an implementer" | Every statement in it follows from this spec's own facts: the command-line parsing shape, the class check before a second start, the completeness of the switch list, the verb registration with the `DropTarget` values, and the rule that §5 must not be implemented as if settled. Same pattern as S12 §9. |
| The open list moved to the end and was renumbered | The task the spec exists for: facts first, open questions last. The draft's items kept their relative order (the mapping is below). |
| The header's SHA-256 written in upper case → lower case | House style: S12's header spells the same hash in lower case. Same hash, same binary. |

## The counts: a discrepancy found in the audit

The draft's prose numbers and the draft's own table do not agree. The spec
publishes the table (the lists are the facts) and the counts derived from it.

| Figure | Draft's prose | The draft's own table |
|---|---|---|
| Verbs per media file | "three verbs per file type" | four — `open`, `Play`, `Enqueue`, `ListBookmark` |
| Verbs per playlist file | "the same three verbs as a media file" | four — the same four |
| Verbs per directory | "three verbs per file type and per directory" | two — `Play`, `Enqueue` |
| Position of `ListBookmark` | "the third verb" | it is the **fourth** of the four listed, or the third *beyond* `open` |
| Verbs per skin archive | not stated in prose | one — `install` |
| Identifiers in the table | three ProgIDs named in prose | three ProgIDs plus the directory row's `Winamp.Play` / `Winamp.Enqueue` |

The draft's prose counted three verbs where its own table lists four, and it
counted three verbs for a directory where its own table lists two. The most
likely cause is that `open` was left out of the count while it stayed in the
list. The spec states four / one / two, derived from the table, and does not
repeat the word "three" anywhere.

## Cross-reference errors found

1. **"an interoperability name that S10 owns (`Winamp v1.x` appears in S10's
   catalog)"** — the attribution is wrong. The class name is published as an
   ABI fact in **S09** §1 (the parent-window member) and **S09** §8 item 1
   (the window classes), and S01 §1 lists it in the window-geometry table. S10
   mentions it only in passing, S10's own header calls it a revised draft
   pending ratification, and the index row for S10 still reads "not started".
   The spec now names S09 as the normative home, points at S01 §1 as well, and
   writes the name down once.
2. **"The hand-over is a window message, so it is S10's subject, not this
   spec's."** — S10 does not cover it. S10's subject is the plugin-facing
   channel: `WM_WA_IPC` (§1.1), the `WM_COPYDATA` file-enqueue forms and the
   `WM_COMMAND` mechanism (§4, §9). Its message table has no single-instance
   hand-over, and no other audited spec has one either. The spec says the
   message is not identified here and lists it as open question 8 — the draft
   had delegated a gap to a spec that does not fill it.

Two more of the draft's cross-references checked out: its internal pointers
(§2, §3) resolve in the audited text; and S02 §1 does say "With a file given on
the command line the program starts playing it" and names S13 as the place for
the rest of the command-line behaviour — which is why the spec cites S02 §1 for
the file-argument case.

## The open list

Kept from the draft: the in-window drop rule (needs a VM observation and is the
next measurement), the drop of a folder, `/BOOKMARK` without a path, whether
`/CONFIG=` also moves the skin/plug-in directories, what the second instance
does when the first is busy, and whether `/CONFIG=` affects the class. The
draft's own order is kept, except that its item 4 was re-scoped (below). The
draft's §2 carried the last two of those as an inline "Open" bullet; they are
now items 5 and 6 of the list, so nothing is lost by the move.

Re-scoped:

- The draft's open item 4, "the exact default window class name (S10 owns
  it)", became open question 7 with a different content: S09 publishes the
  name, so the class name itself is no longer open — what is still open is the
  **default the reference's switch table carries** for `/CLASS=`, which was
  never read out. The class name itself appears in §2 with its pointer.

Added by the audit (each follows from something the draft itself records, and
none of them is a fact the draft did not contain):

| Added | Why |
|---|---|
| 7 — the default value of `/CLASS=` | The draft's item 4, re-scoped as described above. |
| 8 — which window message carries the hand-over | The draft delegated it to S10, which does not carry it (§ *Cross-reference errors*). |
| 9 — the exact quoting of the bookmark command line | The draft gives the registered form as `"/BOOKMARK <path>"` (quotes around the whole argument) and the template as `/BOOKMARK %s` (no quotes at all). The two writings disagree; the draft did not notice. |
| 10 — what `Winamp.Play` and `Winamp.Enqueue` are | The draft put them in the directory row's parenthesis without saying what they name — ProgIDs of the directory handler, or the verbs under another key. The spec keeps them and marks the question. |
| 11 — the display names of the remaining verbs | The draft recorded the display name of `Enqueue` only; `open`, `Play`, `ListBookmark` and `install` are unstated. |
| 12 — what the `DropTarget` entries carry | The draft gives the key shape `…\shell\<verb>\DropTarget` and no value; an implementer cannot register a working drop target from that alone. |

## What could not be resolved

1. **The verb counts.** The audit can only choose between the draft's prose and
   the draft's table; the table wins, because the lists are the data and the
   prose is a summary of them. If the table is the wrong one, a dirty-room
   re-read of the registration strings would say so.
2. **The draft's own duplication of the class name** (§1's example and §2's
   parenthetical). The spec resolves it by writing the name once; if the intent
   was never to publish it here, S09 §1/§8 and S01 §1 already do, and the
   spec's pointer is enough.
3. **Whether the switch table's pairings are behaviour.** The spec states them
   as table facts and marks the readings. Whether each switch does what its
   name says was not observed and cannot be observed from a string table — the
   VM run that §5's question 1 asks for would settle several rows at once.
4. **The directory case.** The draft's table has a row for it, but the
   ProgID column holds "a directory" rather than an identifier, and the two
   identifiers in the parenthesis are unexplained. Questions 10 and 12 keep
   that visible instead of guessing.
5. **Whether the reference registers the verbs under the ProgIDs or under the
   file-type keys.** The draft's key shape (`…\shell\Play\DropTarget`) is
   given with an elided prefix, and the spec keeps the elision: the full key
   path is not in the draft.

## Second pass (2026-10-20) — the `/ADD` correction

| | |
|---|---|
| Trigger | A fact in the audited spec was found to be wrong: §1 said the reference carries no `/ADD`. |
| Evidence | The revised draft at `D:\code\rustamp-sandbox\spec-drafts\S13-command-line.draft.md`, which now carries `/ADD` as a real switch and a note on why the first pass missed it. |
| What was **not** done | No binary was run in this lane, no dirty material was read beyond that draft, no code was touched. The reference is still not run for this spec. |
| Changed | `specs/S13-command-line.md` (§0, §1, §3, §4, §5) and this note. `specs/README.md` is unchanged — S13's row already reads `audited`. |

### What was wrong

Two sentences of the audited spec were wrong, and the second was the first's
conclusion:

1. §1: "**No** `/ADD`, `/PLAY`, `/NEW`, `/EJECT` or `/TASKBAR` is in this
   build."
2. §1: "The reference recognises **exactly** these switches. The list is
   complete for this build."

`/ADD` is a real switch. The reference registers the `Enqueue` verb with the
command line `"%s" /ADD "%%1"`, so the switch means "enqueue, do not play" —
the counterpart of `/BOOKMARK`, whose verb is registered with the same shape,
`"%s" /BOOKMARK "%%1"`. The first pass put `/ADD` into the negative list and
therefore also published the completeness claim that the negative list
supported. Both are corrected.

The wrong fact was not a slip in the table: it was a **conclusion drawn from a
search that could not see the switch**.

### Why the first pass missed it — the quote-prefix trap

The first pass looked for the reference's switch names among the strings that
**begin with `/`**. The reference spells `/ADD` and `/BOOKMARK` inside a
registered command line, and there the string does not begin with the switch —
it begins with a quote, and the switch stands in the middle of it, behind the
program path:

```
"%s" /ADD "%%1"
"%s" /BOOKMARK "%%1"
```

A search restricted to `/`-prefixed tokens never sees either switch. The
result was a list that looked complete and was missing an entry — and, worse,
an absence claim that looked proven.

### The class of the error, and what it says about the rest of the spec

The miss is not a one-off; it is what happens when a **method is restated as
a fact**. Two things in S13 rested on that method:

1. **The completeness claim.** "Complete for this build" was the search
   method (`/`-prefixed strings) turned into a property of the reference. It
   has now been falsified once, by the switch that the same method could not
   see. The spec now states the list as the reference's own switch list and
   puts the question "is that every switch?" into §5 (item 13).
2. **The negative list.** "No `/PLAY`, `/NEW`, `/EJECT` or `/TASKBAR`" is the
   same absence claim about the same string population: a switch the
   reference spells quoted would not have been seen by that search either.
   The second pass therefore **scopes** those four names to the reference's
   switch list — the scope the evidence supports — and does not repeat the
   claim at the wider scope, because nothing in the tree supports it. The
   four names themselves are not contradicted by anything: the switch list
   gained one entry, it did not lose any.

Everything else in S13 that reads as "there is no X" was checked and does not
belong to this class:

| Claim in S13 | Kind of claim | Verdict |
|---|---|---|
| §1 "the list is complete for this build" | absence claim about the **binary** | **false** — corrected and scoped. |
| §1 "No `/PLAY`, `/NEW`, `/EJECT`, `/TASKBAR` is in this build" | absence claim about the **binary** | stands only at the scope "not in the reference's switch list"; the wider claim rested on the same narrow search and is no longer made. |
| §3 "the display names of the other verbs are not recorded" | absence in **our record** | stands. |
| §3 "What the `DropTarget` entries carry as their value is not recorded" | absence in our record | stands. |
| §2 "no audited spec currently names it" (the hand-over message) | absence in the **repository** | stands — a repository-wide search has no dirty-source blind spot. |
| §5 item 7 "the default the switch table carries … was not read out" | gap in our reading | stands. |
| §3 the verb counts (four / one / two) | completeness claim about the **verbs** per handler | **not independently established.** The counts come from the same kind of string reading that missed `/ADD`; nothing contradicts them and the table stands, but "four verbs per media file" is the table's content, not a proof that no fifth verb exists. Left as it is, and named here. |
| §4 "do not add others: the list is complete for this build" | instruction derived from a completeness claim | **corrected.** |

The generalisation this correction forces on the spec pipeline: **in a spec
built from strings, a "there is no X" sentence is only as strong as the shape
of the search behind it.** S13's failure mode is the sharpest version of it —
the search looked for tokens beginning with `/`, and the two switches the
reference spells inside a quoted command line never began with `/`.

### What changed in `specs/S13-command-line.md`

| Change | Reason |
|---|---|
| Header: `Audit date: 2026-10-20` kept, a `Second audit pass:` line added | The change history has to be visible in the file itself; the first audit date is not overwritten. |
| §0: the string-fact row now also covers "a registered command line" | `"%s" /ADD "%%1"` is a new kind of string fact in this spec. |
| §1: the switch table gained the `/ADD` row, with its form `/ADD <path>`, its pairing (the `Enqueue` verb, §3) and its meaning marked as a **reading** of the name | The switch exists; what it *does* is still a reading, like `Play`/`Enqueue` in §3. |
| §1: "recognises **exactly** these switches … complete for this build" → "recognises the switches below" | The completeness claim was falsified. |
| §1: a new **Scope of the list** bullet | The list is what the reference's switch names yield; it is not proven to be every switch (§5 item 13). |
| §1: the negative list now names only `/PLAY`, `/NEW`, `/EJECT`, `/TASKBAR`, scoped to "the reference's switch list" | `/ADD` leaves the negative list — it is a switch. |
| §1: the sentence "the corresponding behaviours are reached through the shell verbs … rather than through the command line" now names the exception | `Enqueue` is reached through the command line (`/ADD`) *and* through the verb; it is the one case where a verb and a switch are the same thing. |
| §1: the forms bullets give the registered bookmark command line exactly (`"%s" /BOOKMARK "%%1"`), add the same shape for `/ADD`, and say that `/ADD` and `/BOOKMARK` take their path in the argument that follows the switch | The first pass rendered the bookmark command line as `"/BOOKMARK <path>"`; the revised draft gives the exact string. And with a third kind of switch form in the table, "the two value-taking switches" had become ambiguous. |
| §3: the `Enqueue` cell of all three verb rows carries the arrow to `/ADD`, and a new bullet says the `Enqueue` verb **is** `/ADD`, with its registered command line | The verb rows had to become consistent with the new switch. |
| §3: the `Play`/`Enqueue` reading bullet now says the `Enqueue` reading is corroborated by the switch name while `Play` stands on the name alone | The reading marker stays where it belongs. |
| §4: the parse rule mentions a switch that takes the following argument as its path; the completeness note points at §5 | `/ADD <path>` is one switch and one path, not two file arguments; and "the list is complete for this build" was the wrong instruction to hand an implementer. |
| §5: item 9 reworded to the exact registered form; item 11 now also asks for the other verbs' registered command lines; new items 13 and 14 | 13 is the completeness question the correction opened. 14 is the naming discrepancy the revised draft exposed (`Bookmark` vs. `ListBookmark`). The numbering of items 1–12 is unchanged, so the references in `crates/rustamp-app/src/cli.rs` (S13 §5.1 … §5.12) still point where they pointed. |

### What the correction does not change

- The other seven switches, their forms and their pairings stand.
- `/PLAY`, `/NEW`, `/EJECT` and `/TASKBAR` are still not in the reference's
  switch list. The list has one entry more than the first pass published, not
  fewer.
- The `Enqueue` verb, the `DropTarget` keys, the ProgIDs, the verb names and
  the counts of §3 stand.
- The single-instance shape of §2 stands.
- The open list keeps all twelve items; the correction added two and removed
  none.
- The `Bookmark` / `ListBookmark` discrepancy is **not** resolved by this
  pass: the revised draft says "the `Bookmark` verb", the audited table says
  `ListBookmark`. The spec therefore speaks of "the verb that registers
  `/BOOKMARK`" and leaves the name open (§5 item 14).

### Open points for the dirty room

1. **The placeholder spelling.** The revised draft writes the registered
   command lines with `%%1`. The first pass had rendered the same string as
   `"/BOOKMARK <path>"`. The spec carries `%%1` verbatim, as the draft gives
   it; a re-read should confirm the exact placeholder (one `%1` or two).
2. **`Bookmark` vs. `ListBookmark`** (§5 item 14) — one verb or two.
3. **The completeness of the switch list** (§5 item 13) — the dirty room
   should read the switch names out of the reference in a shape that is not
   biased by a leading `/` or a leading quote.
4. **The completeness of the verb table** — the four / one / two counts of §3
   come from the same kind of reading that missed `/ADD`.
5. **The measurement in `docs/HANDOFF.md`** — see below.

### Downstream: what the correction breaks (not this lane's files)

1. **The implementation is now non-conformant.** `crates/rustamp-app/src/cli.rs`
   was written to the wrong §1: its module docs say "`/ADD`, `/PLAY`, `/NEW`,
   `/EJECT` and `/TASKBAR` are **not** switches here", and two of its tests
   assert that `/ADD` is a file argument
   (`an_unknown_slash_token_is_a_file_argument` and
   `the_switches_the_reference_does_not_have_are_file_arguments`). With the
   corrected spec, `/ADD` has to be parsed as a switch that takes the path
   behind it, the second test loses `/ADD` from its list, the conformance
   table's row "§1 no `/ADD`, `/PLAY`, `/NEW`, `/EJECT`, `/TASKBAR` |
   matches" has to go, and the module's "the `/` family stays exactly the
   spec's list" sentence has to be re-worded. This lane owns no code.
2. **`docs/tasks/F2-single-instance.md`** — its "Reconciliation with S13"
   section states the same wrong behaviour ("`/ADD`, `/PLAY`, `/NEW`,
   `/EJECT` and `/TASKBAR` are file arguments, not errors") and closes with
   "All twelve stay open"; the open list has fourteen entries now. The card
   is not this lane's file.
3. **The tree already held a measurement that contradicts the audited spec.**
   `docs/HANDOFF.md` (entry of 2026-10-16, run `20261016-212527`) records:
   "`/ADD` enqueues without playing and the one-letter title keeps the marquee
   static (both hypotheses confirmed by this run after two refused runs)". That
   is a runtime observation of the reference with `/ADD` on the command line,
   and it was in the tree while the spec was audited — the first pass's "no
   `/ADD` in this build" was contradicted by the repository itself. The spec
   still carries `/ADD` on the draft's string evidence and claims no
   measurement, because §0 states that the reference was not run for this
   spec and promoting a handoff summary to a spec fact needs the run record
   behind it. The dirty room should reconcile the two: either the measurement
   joins S13 — and §0, the header's evidence classes and the "reading" marker
   on `/ADD` change with it — or it stays a handoff note and S13 keeps the
   string evidence.

## Notes for the orchestrator (not this lane's files)

- `docs/tasks/F2-single-instance.md` says "S13 does not exist as a spec" and
  states its behaviours as own choices; the spec now exists, but it is a
  string-derived spec with a long open list, so F2's own-choice framing is
  still the right one until the open questions are measured.
- `docs/spec-requests/2026-10-20-missing-specs.md` §4 asks what S13 was meant
  to cover. This spec answers it: the README's title — command-line switches,
  single instance, drag-and-drop rules.
- Neither file is owned by this lane and neither was touched.
- The index rows for S10 and S13 disagree with the tree (S10's file exists,
  its row still reads "not started"); S13's row is fixed here.
- The second pass (§ *Second pass*) changes the conformance picture for
  `crates/rustamp-app/src/cli.rs` and for the F2 card: both were written
  against the wrong §1 and need a follow-up lane. Neither file is owned by
  this lane and neither was touched.

## Verification

- `cargo fmt --all -- --check` → exit 0 (green; this lane changed Markdown
  only).
- `git status --short` → the three artifacts above, plus nothing else.
- Second pass: `cargo fmt --all -- --check` → exit 0 (green; Markdown only).
- Second pass: `git status --short` → `specs/S13-command-line.md` and
  `docs/notes/spec-audit-s13.md`, plus nothing else.
