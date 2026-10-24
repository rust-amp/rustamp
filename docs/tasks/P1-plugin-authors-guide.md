# Task card P1 — Plugin-author licensing guide (M1)

**Goal**
Write `docs/notes/plugin-authors.md`: the plugin-author-facing document from
the licensing remainder (RFC 0004, HANDOFF open point on licensing paperwork).
It tells authors of classic plugins — proprietary, GPL, and other open-source —
under what terms their plugin runs with Rustamp, in plain language, with the
Ardour-style permission pattern as the model. Docs only; no legal advice.

**Lane** P — paperwork (docs only).

**Owned paths**
- `docs/notes/plugin-authors.md` (new file; the only file you may change).
- Everything else is **read-only**. If the guide needs something you cannot
  change from the draft, write it as an open question inside it.

**Allowed sources**
- `docs/notes/L-licensing-plugins.md` (the analysis, options and consequences;
  not legal advice), `docs/notes/SDK-licensing.md` (the SDK reading),
  `docs/rfc/0004-plugin-licensing.md` + errata (the split, the §7 exception),
  `PLUGIN-EXCEPTION.md`, `README.md` (License section), `THIRD_PARTY.md`.
- The Ardour permission pattern **as described in the L-note** — do not fetch
  Ardour texts from the web; if the L-note's description is insufficient, mark
  the passage provisional instead of researching externally.

**Forbidden sources**
- Winamp 5.x source, Winamp SDK headers or documentation, disassembly, the
  Winamp 2.95 binary, `D:\code\rustamp-research`, `D:\code\rustamp-refbin`.
  Nothing from them.
- Do not download anything. Do not run a reference binary. No web fetching:
  write from the repo's own notes and RFCs only.

**Scope**
- **In scope:** one guide covering — which plugin kinds exist (`in_`/`out_`/
  `vis_`/`dsp_`/`gen_`) and where they run (`rustamp-plughost32.exe`,
  permissive license); what proprietary-plugin authors must and must not do
  (ABI use, no SDK headers redistribution, support expectations); what
  GPL-plugin authors must know (their license governs their code; the §7
  exception covers our side only and cannot be granted for their code);
  what the §7 exception means in one paragraph with a pointer to
  `PLUGIN-EXCEPTION.md`; the decoder/DSP-helper story (RFC 0006, accepted) in
  two sentences for authors of GPL decoders/DSP; a visible **"this is not
  legal advice"** notice at the top; open questions (anything Ardour-pattern
  detail the L-note does not give you).
- **Out of scope:** changing any license text or exception; legal conclusions
  beyond restating the repo's own notes; API documentation for plugin authors
  (that is a separate future document — name it as a follow-up, do not write
  it).
- Never promote a gap to a fact. Mark every unverifiable detail provisional
  or as an open question.

**Acceptance tests**
- [ ] The guide exists at `docs/notes/plugin-authors.md`, carries the
      not-legal-advice notice, and contradicts nothing in the L-note, RFC 0004
      (with errata), `PLUGIN-EXCEPTION.md` or `README.md` (quote their terms
      exactly where repeated).
- [ ] Nothing outside the owned paths was modified (`git status --short` shows
      only the new file).
- [ ] No `cargo` build is required (docs only).

**Provenance notes**
- Written from the repo's own licensing notes; no external sources.

**Report back**
The guide path; per-audience coverage (proprietary / GPL / other OSS) in one
line each; anything you marked provisional or left as an open question; things
you could not verify from the allowed sources.
