# Task card A2 — Independently re-measure the round-3 audit numbers (dirty room)

**Goal** The S01 round-3 export rests on offline numbers that **only the audit
produced**: the REM/SEL real-click pixel differences (4863 and 10673) and the
claim that **13** fresh-launch captures are byte-identical. The exporter explicitly
did not re-run them. Re-measure them independently and report whether they hold.

**Owned paths** `D:\code\rustamp-research\` — write one new file,
`audits\S01-round3-recheck.md`. Do not edit the audit, the drafts or the job.

**Read** `audits\S01-round3-audit.md` (the claims), the export run
`_runs\20261016-003014\` (`captures\manifest.jsonl`, the named captures), and the
earlier runs it compares against. `D:\code\Rustamp\specs\S01-window-geometry.md`
for what the spec now asserts (read-only).

**Measure, with your own tooling (Pillow or equivalent), tolerance zero:**
1. `o5real_rem_select` (079) vs `o5real_rem_click2` (081) → the differing-pixel
   count and its bounds. The audit says 4863.
2. `o5real_sel_select` (084) vs `o5real_sel_click2` (086) → the count and bounds.
   The audit says 10673.
3. The fresh-launch captures: are **13** of them byte-identical (SHA-256), and
   which files exactly? Name them.
4. The two comparison numbers the export also carries: the flat EQ case (6 of
   31900, bounds in the AUTO cell) and the preamp case (95 of 31900, bounds in
   the preamp column) — reproduce or refute.
5. Anything that does **not** reproduce: say so plainly, with your numbers.

**Hard rules** no live input, no reference-binary run, no sandbox launch; never
promote a gap to a fact; no downloads; no git write commands; leave the file
uncommitted; never descend into `rustamp-research\extracted\`.

**Report** per number: reproduced / refuted / not measurable, with your value and
method; the exact file list for the byte-identity claim; anything the audit got
wrong; whether the export's provenance stands.
