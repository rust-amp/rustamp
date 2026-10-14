# Task card L1 — Implement the plugin-facing licensing split (RFC 0004)

**Goal**
Implement [RFC 0004](../../docs/rfc/0004-plugin-licensing.md): extract the
wire types into a new permissive `rustamp-proto` crate, relicense the
plugin-facing crates to `MIT OR Apache-2.0`, keep every other crate
GPL-3.0-or-later, and add the GPL-3.0 §7 plugin exception and the licensing
paperwork. **No behaviour, protocol or wire change.**

**Lane** Integration / licensing (touches `rustamp-core`, which the integration
lead owns — this card is the lead's approved change, see RFC 0004).

**Owned paths**
- `crates/rustamp-proto/**` (new)
- `crates/rustamp-core/**` — **only** the re-export and move described in
  RFC 0004 §3. No other change to core.
- `crates/rustamp-ipc/**`, `crates/rustamp-plughost/**`,
  `crates/rustamp-plugin-abi/**` (license metadata; for `ipc`/`plughost` also
  the dependency change)
- `Cargo.toml` at the repository root (new workspace member, nothing else)
- `LICENSE-MIT`, `LICENSE-APACHE`, `PLUGIN-EXCEPTION.md`, `THIRD_PARTY.md`,
  `CONTRIBUTING.md`, `.github/pull_request_template.md`, `docs/PLAN.md`,
  `CLEANROOM.md` — **only** the licensing edits RFC 0004 §4 lists.
- Do **not** touch `docs/rfc/**` (RFC 0004 is written and frozen),
  `docs/HANDOFF.md`, or any other crate.

**Allowed sources**
- `docs/rfc/0004-plugin-licensing.md` — your contract; read it first.
- `docs/notes/L-licensing-plugins.md` — the reasoning, the §7 exception draft
  text (§4.1) and the migration checklist (§4.4). Use its §4.1 text for
  `PLUGIN-EXCEPTION.md`.
- The repository itself, and the canonical license texts as they ship in
  dependencies already in the local cargo registry.

**Forbidden sources**
- Winamp 5.x source, Winamp SDK headers or documentation, disassembly, the
  Winamp 2.95 binary, `D:\code\rustamp-research`. Nothing from them.
- No downloads (no `cargo add`, no git clone, no web fetching). Take the
  canonical MIT and Apache-2.0 texts from a crate in the local cargo registry
  (several dependencies ship `LICENSE-MIT`/`LICENSE-APACHE`) — they must be
  unmodified.

**Spec dependencies**
- None. This card implements no player behaviour.

**What to do**
1. Create `rustamp-proto` and move the types RFC 0004 §3 lists out of
   `rustamp-core`, keeping the same module paths and re-exporting them from
   `rustamp-core` so **no caller changes**. Move the RFC 0003 layout asserts with
   the vis types (or keep an equivalent assert that fails the build if the sizes
   drift).
2. Cut `rustamp-ipc` and `rustamp-plughost` loose from `rustamp-core`: they
   depend on `rustamp-proto` and `rustamp-plugin-abi` only.
3. Set `license = "MIT OR Apache-2.0"` in the four permissive crates and add the
   canonical license files plus the per-crate pointer.
4. Add `PLUGIN-EXCEPTION.md` and reference it from the crate roots of the GPL
   crates that contain only our own code. **Do not** attach it to
   `rustamp-decoders` or to any crate carrying third-party GPL code — say in the
   report which crates you attached it to and why.
5. Do the paperwork of RFC 0004 §4: `THIRD_PARTY.md` section, `PLAN.md` §1/§2,
   `CLEANROOM.md` §1, and the inbound = outbound statement in `CONTRIBUTING.md`
   and the PR template.

**Acceptance tests**
- [ ] `cargo fmt --all -- --check` clean.
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean.
- [ ] `cargo test --workspace --all-features` green with the **same test count**
      as before the refactor (report the before/after numbers).
- [ ] `cargo test -p rustamp-ipc --target i686-pc-windows-msvc` and
      `cargo test -p rustamp-plugin-abi --target i686-pc-windows-msvc` green;
      `cargo build -p rustamp-plughost --target i686-pc-windows-msvc` green.
- [ ] `cargo xtask ring-cross` passes all four combinations.
- [ ] `cargo xtask golden` reports no mismatch.
- [ ] **Dependency proof:** show, from the dependency graph (`cargo tree` or
      `cargo metadata`), that `rustamp-proto`, `rustamp-plugin-abi`,
      `rustamp-ipc` and `rustamp-plughost` have no GPL-licensed crate in their
      dependency closure. Paste the evidence in the report.
- [ ] `PROTOCOL_VERSION` is still `1` and no test needed changing to keep the
      wire format — state explicitly that you changed no encoding.
- [ ] Report which crates received the §7 exception, and confirm that
      `rustamp_core`'s public paths are unchanged.

**Provenance notes**
- The canonical license texts are verbatim copies of standard licenses, which is
  how they are meant to be used; note their origin in the report.
- No third-party code is otherwise copied.

**Report back**
Files changed, the before/after test counts, the dependency-closure proof, the
list of crates carrying the §7 exception, anything that did not compile or that
you could not verify, and any place where RFC 0004 turned out to be ambiguous.
