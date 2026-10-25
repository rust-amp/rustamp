# Audit note: S21 addendum 1 (what MilkDrop needs after start-up)

Audited 2026-10-25 by a dirty-room auditor agent (lane `rustamp-lanes\s21`).
**Verdict: PASS WITH CHANGES.** No source code, pseudo-code, listing or copied
comment was found. Six statements of the draft were wrong or incomplete and are
corrected in the spec (marked [C]); header-style identifiers were rewritten in
prose; every load-bearing claim carries [A] (re-derived from the binary), [N]
(not re-derived) or [C]. No VM run was made for the audit.

Unverified: everything after the language-service message 52 on the guest
(shader compile, canvas textures, `Present`), which d3dx9 library the guest
loaded, the guest's pixel-shader level and adapter. The clean room reads only
the spec and the implementation notes; the audit report stays in the dirty room.
