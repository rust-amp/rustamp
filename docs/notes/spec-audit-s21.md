# Audit note: S21 (what a Winamp 5.x plug-in expects of its host)

Audited 2026-10-25 by a dirty-room auditor agent (lane `rustamp-lanes\s21`).
**Verdict: PASS WITH CHANGES.**

* No source code, transliterated pseudo-code, disassembly listing or copied
  comment was found in the draft. The changes were factual corrections and
  wording: instruction-level phrasing rewritten as behaviour, SDK-looking names
  replaced by neutral ones (the spec carries a naming table), a real-world path
  meaning relabelled as a reading, and six corrected facts about which plug-in
  sends which message.
* Every claim in the spec carries a mark: audited (re-derived from the
  binaries), not re-derived, or corrected.
* The VM survey was re-run once: identical to the earlier table (3 of 14 load,
  seven die at the same places).
* **Unverified:** nothing was measured with a host that answers the 5.x
  requests, so everything past the first fault point of each plug-in is a
  reading. What a real 5.x host does with no service manager, and the ranges of
  the random generators, need a run of the real player.

The audit report itself and the working material stay in the dirty room
(`D:\code\rustamp-lanes\s21\audits\`); the clean room reads only the spec and
the implementation notes.
