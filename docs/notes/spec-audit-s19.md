# Spec audit S19 — streaming and ICY metadata display

Lane: spec audit of the S19 draft (CLEANROOM.md §3, step 3).
Auditor: spec auditor subagent. Audit date: 2026-10-20.

| Artifact | Path in this worktree |
|---|---|
| Audited spec | `specs/S19-streaming.md` (new) |
| Index row | `specs/README.md` — S19 status `not started` → `audited` |
| This note | `docs/notes/spec-audit-s19.md` (new) |

No code was touched. `cargo fmt --all -- --check` stays green (exit 0), which
is expected: this lane changes Markdown only.

## The draft that was audited

The draft is kept outside the repository, under RFC 0021's sandbox path
(`D:\code\rustamp-sandbox\spec-drafts\S19-streaming.draft.md`, 4906 bytes, last
written 2026-10-20 14:15:22, 90 lines). It is the only revision that existed at
audit time. It was written from the reference's **user-facing status strings**,
its **HTTP request template** and the `streamtitle` field of S14; **no stream
was played and the reference was not run for it**, so the draft contains no
runtime observation at all.

## What the audit checked

- **CLEANROOM §4** — no source code, no pseudo-code transliterated from source
  or disassembly, no copied or paraphrased comments, no distinctive internal
  identifiers, no disassembly listings.
- **CLEANROOM §3** — the provenance header is complete and filled in; the spec
  is behaviour, spellings and protocol facts, in prose and tables.
- **RFC 0021 §2 and §5** — the spec names its source **category** in the
  provenance header, so a later reviewer can see that the evidence is
  string-table-derived; and this note says what was removed, not only that the
  spec was approved.
- **Epistemic status** — every item the draft called open is still open; no
  reading was promoted to a stated fact; the draft's "two behavioural facts
  follow" was demoted to two **readings**, because no stream was played.
- **Internal consistency** — every claim in §1 of the draft was checked
  against the request template it quotes, and every claim in §4 and §5 against
  the strings and the template. Four contradictions surfaced (§ *Internal
  contradictions* below); none was smoothed.
- **Cross-references** — every cross-reference the draft made into the repo
  was checked against the file it names. Two were wrong (§ *Cross-reference
  errors* below).
- **Absence claims** — the draft makes three ("no timeout text", "no stall
  text", "no status-code text"). The repo carries **no** measurement of the
  reference's string table that could contradict them: the only string-table
  claims in the tree are S09 §1, S11, S13 and S14, none of which mentions the
  streaming strings. The absence claims therefore stand on the draft's own
  reading of that table, and the spec now labels them as facts about the
  **table** rather than about the behaviour (see § *What was rewritten*).

## What stayed (unchanged in substance)

- The request template, with its request line, its `HTTP/1.0` protocol line,
  its `Host:`, `User-Agent:` and `Accept: */*` lines and its slot for extra
  header lines.
- All **ten** phase entries of the status table — **twelve** texts — verbatim,
  with the leading newline of `Connection closed.` and the two-line shape of
  the two transfer texts.
- The two behavioural conclusions: the total is optional, and the counters are
  a progress display rather than a summary.
- The `streamtitle` field of the format language.
- The proxy path's two status texts.
- All **ten** open items, in the draft's order, with their own content.

## What was removed, and why

| Removed | Reason |
|---|---|
| The raw format fragment `…%s\r\n%sAccept:` from §1's bullet list | The only passage in the draft that reads as a **listing**: a literal from the binary's string table with printf conversion specifiers and CR/LF escapes. The fact behind it survives in prose — the template has one slot for extra header lines and two placeholder slots (§1 of the spec). |
| The heading "S19 (DRAFT — not an audited spec, never hand this to an implementer)" | The file is now the audited spec; the warning was a property of the draft, not of the spec. |
| The draft's section "What this draft deliberately does not contain" | Meta-commentary about the draft. Its substance moved into §0 of the spec, where it is a statement about the spec itself — the same move the S13 and S14 audits made. |
| "The request template is part of the binary" as the lead-in to §1 | A pointer into the dirty material. The evidence category belongs in the provenance header; §1 now states the fact about the reference and renders the template with its placeholders filled in. |
| "**two behavioural facts follow**" as the heading of §2's conclusions | No behaviour was observed — the draft says so itself in its own lead-in. The two conclusions are kept, in §2 of the spec, as **readings**. |
| "`Accept: */*` is **always** present" | The template carries the line; "always" is a claim about every request, which a template cannot establish. The spec keeps the line as a template fact and publishes the "always" question as open question 16. |
| "**HTTP/1.0**, one request per stream — no keep-alive, no chunked transfer, no `Range` request." | Three claims in one sentence: one of them (the protocol version) is a fact, the other two are readings, and the third contradicts the draft's own open item 9. Split up in §1 of the spec; the `Range` contradiction is published as open question 11. |
| "so a stalled host either blocks or is cut by a socket default" | An inference about behaviour, presented as a consequence of the string table. Kept as part of open question 1, where the table's silence is labelled as such. |
| "(no such string exists, so probably not)" from open item 3 | "Probably not" is a conclusion about behaviour inside an open item. The string-table fact stays; the conclusion became a reading; the question stays open. |
| The key spelling `config_proxy` | S12 §4.1: the reference's reader looks names up under a seven-character `config_` prefix, and **the name in the file is the name without that prefix**. `config_proxy` is therefore not the file's key. The spec uses `proxy` (S12 §5.1) and cites the prefix rule (S12 §4.1). |
| "the configuration carries the proxy string" | Not supported: the observed configuration file did **not** carry the key (S12 §5.2). §4 of the spec now says what was and was not observed, and the missing value became open question 13. |

No passage had to be cut for being source code, a transliterated pseudo-code, a
copied comment or a disassembly listing, apart from the raw format fragment
above. The strings it carries are the ones the rules allow: the reference's own
**user-facing text** and its **on-the-wire request**.

## What was rewritten, and why

| Change | Reason |
|---|---|
| A new §0 "How to read this spec" with an evidence-class table (template fact / string fact / reading / config fact / practical note / open), the sentence "**No stream was played for this spec, and the reference binary was not run**", and a scope paragraph | The draft mixed template facts, string facts and conclusions in one voice. The marker discipline is now the one S11, S13 and S14 use. The scope paragraph names what the spec does **not** cover (decoding, buffering, the seek bar, the time display, the output path — S18's), because the repo's own spec request expects a statement about those (`docs/spec-requests/2026-10-20-missing-specs.md` §8). |
| §1: the bullets became a facts list and a readings list | The draft's bullets mixed what the template establishes with what follows from it. The split makes it visible which of the draft's claims follow from the template and which do not. |
| §1: "the two `%s` slots" → "the template has **two placeholder slots**" | The `%s` conversion specifier is a property of the internal format string, not of the request; the count survives, the spelling does not. |
| §2: the table's heading "in the order the player shows them" → "The status texts", with the note that the phase column groups the texts and that **the order was not observed** | The draft claimed a runtime sequence from a string table. A string table does not carry the order in which the player shows the texts. Published as open question 15. |
| §2: the two conclusions became **readings**, each marked as such | They are conclusions drawn from two format strings, not observations. The draft itself records that no stream was played. |
| §2: the percentage text's `%%` explained in one clause | An implementer who copies the text verbatim needs to know that the doubled percent sign is how the format spells one literal percent sign. |
| §3: the ICY conclusion became a reading, with the tension against S14 §1 named | The field list is a **plug-in's** (S14 §1: the language belongs to individual input plug-ins, and the player's own strings carry no field token). The draft's step from the field name to "the player reads the ICY metadata block and its `StreamTitle` entry" is not carried by the draft's own material, and the entry's name `StreamTitle` is an inference, not a string the draft quotes. |
| §4: the proxy section now states the recognised key, the file spelling, the prefix rule and the **absence of an observed value** | The draft's "the configuration carries the proxy string" is contradicted by S12 §5.2. The spec now says what the tree knows and what the tree does not. |
| A new §5 "Practical notes for an implementer" | Every statement follows from the spec's own facts: the request, the two display variants, the counters, the proxy setting, and the rule that §6 must not be implemented as if settled. Same pattern as S12 §9, S13 §4, S14 §3. |
| The open list moved to the end and was renumbered | The task the spec exists for: facts first, open questions last. The draft's ten items kept their relative order (mapping below). |
| The header's SHA-256 written in upper case → lower case, and the long RFC 0021 line | House style: S11's, S12's, S13's and S14's headers spell the same hash in lower case and carry the same authorisation sentence. Same hash, same binary. |

## Internal contradictions found

1. **"no `Range` request" against the open extra-header slots.** §1 of the
   draft lists "no `Range` request" among the facts the template establishes,
   while its own open item 9 says the exact content of the two extra-header
   slots is not established. Both cannot stand: a `Range` line would be one of
   the extra headers. The spec keeps the template's own header lines as facts,
   demotes the consequence to a reading, and publishes the contradiction as
   open question 11.
2. **"one request per stream" against "whether a stream seek is possible at
   all".** A single request per stream and a seek that the server does not
   know about are the same claim twice, and the draft's own open item 7 asks
   whether a seek is possible at all. The spec keeps the template's shape as a
   fact and leaves the request count open (open question 15).
3. **"in the order the player shows them" against the evidence.** The draft's
   §2 heading claims a runtime sequence that a string table cannot carry.
   The spec groups the texts by the phase each names and publishes the
   sequence as open question 15.
4. **"no timeout text" — a fact about the table, drawn as a conclusion about
   the behaviour.** The draft's open item 1 mixes the two: the absence of a
   text in the string table is a fact, "so a stalled host either blocks or is
   cut by a socket default" is a conclusion, and "the value is not
   established" is a third thing again. The same mix appears in open item 3
   ("probably not") and in open item 4 (the 404/403/500 absence). The spec
   labels all three as facts about the **string table** and keeps the
   behavioural part open; open question 14 asks what such a silence is worth.

## Cross-reference errors found

1. **"`config_proxy`, S12 §3.2"** — wrong twice over. S12 §3.2 is "Volume,
   balance, EQ" (`volume`, `pan`, `use_eq`, `autoload_eq`, `preamp`,
   `eq_data`); it carries no proxy key. The proxy key is in **S12 §5.1** (the
   recognised key set, group "preferences, misc"), and **S12 §5.2** records
   that the observed file did **not** carry it. The `config_` prefix is
   **S12 §4.1**'s rule — the reader's lookup names carry it, the file's names
   do not. The spec now cites all three sections and says what was and was not
   observed. The claim "the configuration carries the proxy string" is
   published as open question 13.
2. **"(S14 §2.1)"** — **correct**, and it stays. S14 §2.1 is the section that
   lists the field names, and it lists `streamtitle` among the nine; S14
   §2.4's help-text illustration uses it. The audit checked the citation and
   found nothing to fix. What §2.1 does **not** say is that the player parses
   ICY metadata — that is the tension published as open question 12.
3. **"consistent with §4's proxy path"** — a forward reference inside the
   draft, and correct as far as it goes: §4 does describe the proxy path. The
   spec keeps the cross-reference and adds the missing half — the absolute
   request line is the proxy form (§1, §4).

## The open list

Kept from the draft, in the draft's order: the timeout value, the retry count,
the buffering policy, the status codes, the `Icy-MetaData` request header,
where the stream title is displayed, whether a stream seek is possible and
what the seek bar shows without a length, `http://` only or other schemes, the
exact content of the two extra-header slots, and whether the `Host:` header
includes the port.

Added by the audit. Each follows from something the draft itself records, and
none of them is a fact the draft did not contain:

| Added | Why |
|---|---|
| 11 — whether the request ever carries a `Range` header | The contradiction between §1's "no `Range` request" and the draft's own open item 9 (§ *Internal contradictions* 1). |
| 12 — whether the reference reads an ICY metadata block at all, and what feeds the `streamtitle` field | The step from a plug-in's field name to the player's behaviour is not carried by the draft's material, and it sits in tension with S14 §1 (§ *Internal contradictions* / §3). |
| 13 — the shape of the proxy setting | The observed configuration file carried no value for it (S12 §5.2), so neither the format nor the question whether the reference writes the key is answered. |
| 14 — whether a timeout exists at all, and whether the string table's silence settles it | A string table without a timeout text does not decide whether a timeout is applied; the same silence is what the draft's items 1, 3 and 4 lean on. Parallel to S14's open question 9. |
| 15 — the order of the status texts, and the number of requests per stream | The draft's §2 heading claimed a sequence the evidence cannot carry, and §1 claimed a request count (§ *Internal contradictions* 2 and 3). |
| 16 — whether the template's header lines are always sent exactly as shown | The draft asserted that `Accept: */*` is "always present"; a template shows one rendering, not every request. |

## What could not be resolved

1. **Whether the reference reads ICY metadata.** Unresolvable from a field
   name in a plug-in's format language. It needs a capture against a server
   that sends an ICY metadata block.
2. **The timeout.** The string table's silence is the strongest statement
   available from this material — and it is a statement about the table.
   Whether a timeout is applied, and with what value, needs a stalled-host
   measurement.
3. **The proxy setting's format.** The observed file carried no value, so
   nothing in the repo constrains it. S12 §5.1 lists `inet_mode` in the same
   group as `proxy`; whether the two are related (a direct/proxy switch) is
   not established here and is a lead for the dirty room, not a claim of this
   spec.
4. **The order of the status texts.** Only a capture of a running player
   against a server that accepts, refuses, stalls and closes would settle it.
5. **What the two extra-header slots carry.** The draft's own open item 9;
   the contradiction with §1's "no `Range` request" (open question 11) is
   only a symptom of it.

## Notes for the orchestrator (not this lane's files)

- `docs/spec-requests/2026-10-20-missing-specs.md` §8 asks S19 four
  questions. This spec answers **none** of them as a measurement and gives
  the shape of all four: question 1 (protocols and timeout) — the template
  shows `http://` with an HTTP/1.0 request line; the timeout is open
  question 1, the other schemes open question 8. Question 2 (buffering) —
  open question 3. Question 3 (failure states and their messages) — the
  messages exist in §2 of the spec (`Error connecting to host`,
  `Error connecting to proxy`, `Connection to host lost`,
  `Connection closed.`), but which failure produces which message, and what
  a 404 does, are open question 4. Question 4 (length and seek controls) —
  open question 7. The file itself was not touched by this lane.
- `docs/PLAN.md` line 79 names `ureq`/`reqwest` for "HTTP streams with ICY
  metadata" and line 186 names "HTTP/Icecast/SHOUTcast streaming with ICY
  titles" as roadmap items; `docs/tasks/ROADMAP-to-winamp-parity.md` lists
  "ICY metadata (station names, track info)" as **not** done. Nothing in this
  spec contradicts those: the spec settles what the reference's own strings
  and request template say, and open question 12 says that whether the
  reference parses ICY metadata at all is not established. The roadmap's own
  framing is unaffected.
- `crates/rustamp-ui/src/title_format.rs` already carries the nine field
  names of S14 §2.1, `streamtitle` included, and the help-text illustration
  of S14 §2.4. This spec does not change that list, and it does not license a
  claim that the player populates the field from an ICY block (§3).
- No file outside the three artifacts above was touched.

## Verification

- `cargo fmt --all -- --check` → exit 0 (green; this lane changed Markdown
  only).
- `git status --short` → the three artifacts above, plus nothing else.
