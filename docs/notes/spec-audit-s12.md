# Spec audit S12 — winamp.ini keys and plug-in settings

Lane: spec audit of the S12 draft (CLEANROOM.md §3, step 3).
Auditor: spec auditor subagent. Audit date: 2026-10-20.

| Artifact | Path in this worktree |
|---|---|
| Audited spec | `specs/S12-winamp-ini.md` (new) |
| Index row | `specs/README.md` — S12 status `not started` → `audited` |
| This note | `docs/notes/spec-audit-s12.md` (new) |

No code was touched. `cargo fmt --all -- --check` stays green (exit 0), which
is expected: this lane changes Markdown only.

## The draft that was audited

The draft is kept outside the repository, under RFC 0021's sandbox path
(`docs/rfc/0021-dirty-room-in-session.md` on `main` describes the rule; the
RFC itself is not in this worktree's tree, because the worktree was cut before
that commit). The audited revision is the one that was on disk at audit time
(2026-10-20, 09:52, 220 lines), which is the revision that added the
disassembly-derived section — the earlier revision covered only the
configuration file. Both revisions were audited; the table below marks what
belongs to the disassembly-derived part.

## What the audit checked

- **CLEANROOM §4** — no source code, no pseudo-code transliterated from source
  or disassembly, no copied or paraphrased comments, no distinctive internal
  identifiers, no disassembly listings.
- **CLEANROOM §3** — the provenance header is complete and filled in; the spec
  is behaviour, constants, ranges and file-format facts, in prose and tables.
- **RFC 0021 §2 and §5** — the spec names its source **category** in the
  provenance header, so a later reviewer can see that part of the evidence is
  disassembly-derived; and this note says what was removed, not only that the
  spec was approved.
- **Epistemic status** — every **inferred** marker of the draft is kept in
  place; no guess was promoted to a stated fact; every item the draft called
  open is still in the open list.

## What stayed (unchanged in substance)

- Every key name and every value of the observed file, in the same grouping.
- The file's shape rules: INI layout, CRLF, no quoting, no escaping, empty
  value as `key=`, `[Winamp]` first, plug-in sections named by display name,
  comma-separated lists, hex without prefix, `*_wx` / `*_wy` and `*_width` /
  `*_height`.
- The file-level facts: 2288 bytes, 76 keys in `[Winamp]`, four plug-in
  sections, `cwd` naming a temporary directory.
- All **inferred** markers, in the same rows, with the same readings
  (`randskin`, `pladv`, `shownumsinpl`, `timeleftmode`, `autoscrollname`,
  `pe_height_ws`, the six analyser keys, the four marked video keys,
  `newverchk` / `newverchk2`).
- The observed title-format string and the four behavioural facts it
  establishes (the `$function(...)` / `%field%` shape, `$if2(a,b)`, the three
  field names, the conditional group).
- The plug-in-section observation: `out_wave.dll` is selected and has a
  section, `out_ds.dll` sits in `Plugins/` and has none.
- The playlist-versus-equalizer asymmetry note (`pe_width` / `pe_height` /
  `pe_height_ws` against positions and one shade flag).
- From the disassembly-derived part: the `config_` prefix rule, the
  default-on-missing-key rule, the 2048-byte string limit, the complete
  recognised key set (all 109 names, grouped as in the draft), the list of keys
  the observed file did not carry, the `eq_data` parsing and clamping rules,
  and the two-numbers-in-one-key rule for `prefs_last_page`.
- The draft's eight open questions, in their original order.

## What was removed, and why

| Removed | Reason |
|---|---|
| The heading "S12 (DRAFT — not an audited spec, never hand this to an implementer)" | The file is now the audited spec; the warning was a property of the draft, not of the spec. |
| The draft's section "What this draft deliberately does not contain" | Meta-commentary about the draft. Its substance moved into §0 of the spec ("Nothing in this spec is source code, a transliterated pseudo-code, a copied comment or a disassembly listing"), where it is a statement about the spec itself. |
| "(or a disassembly of the format engine)" as a way to settle the title-format token set | A hint about a dirty-room research path does not belong in a document handed to implementers. The fact it carried — the token set is not established — stayed (§8.3 and open question 10). |
| "found beside the binary under test" | A location hint. §1 now names the source categories only. |
| "The reference does not prevent it." (two plug-ins with the same display name) | The non-occurrence of a guard was not observed. The spec now says the observed file contains no disambiguation and gives no sign of a guard, and that this was not tested. |
| "the binary's own key table (a disassembly of the reader path)" from the draft's title line, and the heading "(disassembly facts)" | The provenance belongs in the header, not in the body of a spec an implementer reads. The header now carries it: the sources line names the disassembly category and a separate line names RFC 0021. §1 names the three artifacts and says which sections come from which. |

Nothing that reads as code, a transliterated pseudo-code, a copied comment or a
disassembly listing was found in the draft; no such passage had to be cut. The
only Winamp-internal names the spec carries are the file's own key and section
names (the public file format), the shipped plug-ins' display names and DLL
file names, the `config_` prefix (see below), and cross-references to audited
specs.

One name was deliberately **not** imported: S09 §1 lists a host-internal
setting name for the selected output plug-in and hands its semantics to S12.
That name is an internal identifier, not part of the public interface, so the
spec uses the file's own key `outname` and nothing else.

## What was rewritten, and why

| Change | Reason |
|---|---|
| §0 now states the provenance rule explicitly: §2, §3 and §8 are file facts, §4–§7 are reader facts, a row marked **inferred** is a hypothesis. | The draft mixes two very different evidence classes in one document; an implementer must be able to tell them apart at a glance. No marker was added or removed. |
| "Every key is read through one of two helpers: an integer reader and a string reader." → "Two kinds of read exist, one for integers, one for strings." | "Helpers" names an internal structure that the file format does not need. |
| "Both compare the requested name against the **seven-character prefix `config_`** and, when it matches, skip those seven characters before asking the profile API for the key." → "The names the player looks up carry a seven-character `config_` prefix; the name in the file is the name without that prefix." | The string-handling mechanics (compare, then skip seven characters) map onto the machine code. The prefix is a **constant** and the lookup rule is a behaviour, and both survive the rewrite. The constant itself stays, because §4 allows constants and because S09 §1 already publishes the same `config_*` names from the host string table; the observable consequence (`config_volume=` in a file is not found) is kept verbatim. |
| "that default is whatever the player's global held when the read ran" → "the player's own initial value for the setting". | The phrasing describes a storage location rather than the behaviour. |
| "The string reader has a 2048-byte staging buffer and the caller passes its own size" → "A string value is read through a 2048-byte staging buffer, and the caller passes its own size". | Same content, phrased as a read of a value rather than as a description of a reader object. The observable (a value beyond 2048 bytes is cut short) is now stated outright. |
| "The reader path names the following keys" → "The player recognises the following keys". | "The reader path" is a pointer into the disassembly; the behavioural statement is what a spec needs. |
| "the separator is skipped as one character" → "the separator counts as one character". | The original phrasing describes the scan step; the rewrite keeps the consequence (`1,2,3` and `1,,3` differ). |
| "bits 8 and above are kept in a second global" → "the bits from bit 8 upwards carry a second number, which the player keeps separately". | "A second global" names a variable; the bit layout is the fact and stays. |
| The two generalisations in §8 are now split into an **Observed** bullet and a **Reading** bullet: the "written only while the plug-in is loaded" rule (§8.2) and the display-name collision (§8.1). | Both are inferences from one sample; stated flatly they read as observed behaviour. The observations they rest on stayed next to them. |
| "so S08 can state their setting names and value ranges as facts" → "the setting names and the observed values as file facts … their value ranges are not established by this one sample". | The draft's own table gives one value per analyser key, so no range was observed. This is the one place where a claim was corrected rather than removed: it stops an unmeasured range from being published as observed. |
| `pladv`: "(manual advance in our config)" → "the observed value `1` is the advancing state". | The original parenthetical described the harness's configuration, not the program. |
| `use_eq` / `autoload_eq`: the rows pointed at "S07 §1: the `On` / `Auto` button". In S07, §1 only lists the saved-state keys; the On and Auto buttons are S07 §3, and the measured key-to-indicator links are in S01 §3.1. The rows now cite S01 §3.1, which records that `autoload_eq` toggles exactly the AUTO indicator and that the `use_eq` / ON-indicator link is provisional (not independently re-verified). | A wrong cross-reference sends an implementer to the wrong place; the corrected one also carries the caveat the audited spec attaches to the `use_eq` link. |
| `preamp`: "S01 §3" → "S01 §3.1" for the thumb-y values 38 and 89. | The mapping is in §3.1. |
| The `pan` note now names the measured marquee wording (`CENTER`, `100% LEFT`; `100% RIGHT` expected but not measured) from S07 §6 and states that the file's own encoding is still open. The draft's method hint (drive the balance with `WM_COMMAND`, read the marquee back) was kept as it stands. | S02 §2.1's `BALANCE: <CENTER or side>` is a placeholder; the question stays open, but the implementer now sees which measurement already exists. `WM_COMMAND` is a public Windows message name and stays. |
| The windows table was split to one key per row. | Same values and meanings, easier to read and to diff against a real file. |
| Section renumbering: the draft's §3 became the spec's §4–§7, the draft's §4 became §8, the draft's §5 became §10. | The reader facts needed their own sections; the open list had to come last. |
| §9 "Practical notes for an implementer" was added. | Every statement in it is derived from this spec's own facts and open list: plain INI parsing, writing the names without the prefix, round-tripping keys the player does not recognise (§5.3), clamping only where a range is known, the 2048-byte cut, and both writing strategies remaining possible. |
| Open questions 9 (the counts), 10 (the title-format token set) and 11 (no key's behaviour was observed) were added; the draft's eight questions kept their order. | The draft stated the token-set gap in §3 but did not list it; question 11 states the limit the draft only implied through its markers; question 9 carries the count discrepancy found in this audit. |

## The counts: a discrepancy found in the audit

The draft's prose numbers and the draft's own lists do not agree. The spec
publishes the lists (they are the facts) and the arithmetic, and puts the
discrepancy into open question 9 rather than picking one figure silently.

| Figure | Draft's prose | The draft's own lists |
|---|---|---|
| Recognised keys | 105 | 109 names in the group table |
| Observed file | "held 76 of them" (of the recognised) | 36 recognised keys absent → 109 − 36 = 73 recognised keys present; the file holds 76 keys in `[Winamp]` |
| Keys itemised by name in the draft's §2 | — | 72; `shuffle_morph_rate` is recognised and not absent, yet not itemised; three further file keys are named nowhere |

The spec states all five numbers in §5.3 and says which of them disagree. A
recount in the dirty room is needed before an implementer relies on any of
them.

## What could not be resolved

1. **The count discrepancy above.** Not resolvable from the draft: it needs the
   file and the key table side by side.
2. **The draft's marker discipline is inconsistent.** Rows such as `dsize`,
   `aot`, `easymove`, `windowshade`, `video_vsync` and `video_useratio` read
   the key's name exactly like `randskin` or `shownumsinpl`, which carry the
   **inferred** marker. The audit brief was to keep the markers exactly as the
   draft had them, so they were kept, and §0 states what an unmarked row is
   worth. A dirty-room revision should mark the name-read rows uniformly in
   one pass rather than row by row.
3. **`pilp`, `safire`, `saref`** — recognised and observed, meaning unknown.
   Kept in the open list; not resolvable from the key table.
4. **The plug-in sections' key inventory.** Only two keys outside `[Winamp]`
   are named in the draft (`cfg_title_format` and the `cfg_wfx` hex example).
   The spec therefore states the plug-in-section *conventions* and not the
   sections' key lists.
5. **Whether a `plugin.ini` exists at all.** The observed run shows a plug-in's
   keys inside `winamp.ini`; nothing else was inspected. The spec's statement
   is scoped to that run.
6. **`cwd`'s exact meaning** (working directory at exit versus at start) is
   the draft's reading and stays a reading; it is tied to open question 6.
7. **The defaults the integer read falls back to** are named as the player's
   own initial values but their values were not read (the draft's open
   question 8). Nothing in the repo states them either.

## Verification

- `cargo fmt --all -- --check` → exit 0 (green; this lane changed Markdown
  only).
- `git status --short` → the three artifacts above, plus nothing else.
