# Task card V1 — CI check: permissive crates stay GPL-free (M1)

**Historical card, superseded (2026-10-17):** current requirements are in
`V3-license-readiness.md`: five mandatory roots including `rustamp-mediahost`,
automatic additional-root discovery, all features, both Windows targets,
permissive allowlist and complete lockfile notices. The implementation now
uses Cargo metadata plus Python >= 3.11 (stdlib), with explicit CI cache
population before its offline/locked check. The original four-root,
cargo-only/GPL-substring instructions below are not the current gate.

**Goal**
Add a CI job that fails the build if any GPL-licensed crate enters the
dependency closure of `rustamp-proto`, `rustamp-plugin-abi`, `rustamp-ipc` or
`rustamp-plughost` (RFC 0004 acceptance 4, the load-bearing invariant of the
licensing split). The check uses only cargo itself — no new tools, no new
dependencies.

**Lane** V — verification (CI).

**Owned paths**
- `.github/workflows/**`
- Do **not** edit anything else. In particular do not touch crates, `xtask`,
  `docs/**` or the root `Cargo.toml`. If the check needs something outside
  CI configuration, report it as a finding instead of editing it.

**Allowed sources**
- `docs/rfc/0004-plugin-licensing.md` (acceptance 4 and §3: which crates are
  permissive and why the invariant exists).
- `.github/workflows/ci.yml` (the existing workflow to extend),
  `docs/HANDOFF.md` §7 (the verification gate this joins).
- Public GitHub Actions + `cargo tree` documentation.

**Forbidden sources**
- Winamp 5.x source, Winamp SDK headers or documentation, disassembly, the
  Winamp 2.95 binary, `D:\code\rustamp-research`, `D:\code\rustamp-refbin`.
  Nothing from them. (Irrelevant for CI work, stated for completeness.)
- Do not download anything locally. The CI job itself must install nothing
  beyond the Rust toolchain the workflow already uses: implement the check
  with `cargo tree -p <crate> -e normal --format "{p} || {l}"` (plus
  `--target i686-pc-windows-msvc` for `ipc`/`plughost`) and fail on any
  `GPL`/`AGPL`/`GFDL` license string, in the style of the L1 lane's proof.

**Scope**
- **In scope:** a new CI job (or step, whichever fits `ci.yml`) that runs the
  closure check for all four permissive crates on both targets and fails with
  a readable message naming the offending package; a local runnable form of
  the same check (script or documented one-liner) so contributors can run it
  before pushing — as a file under `.github/` if a script is needed, never in
  `xtask` (not owned).
- **Out of scope:** running CI itself (there is no GitHub remote, so the
  workflow cannot be observed end-to-end — state that); changing the deny
  policy beyond license strings (no `cargo-deny`, no advisory DB); touching
  any crate's dependencies to satisfy the check.
- Never promote a gap to a fact. If `cargo tree`'s license output is
  unreliable for some package (missing license metadata), fail loudly on
  unknown rather than silently passing — and say so.

**Acceptance tests**
- [ ] The workflow YAML is valid (verify with a YAML parser or
      `actionlint`-equivalent if already available locally — do not download
      one; otherwise careful manual review and state the method).
- [ ] The check logic is executed locally against the current tree and passes:
      no GPL strings in any of the four closures, both targets where
      applicable (this is the L1 proof, re-run).
- [ ] A forced-failure demonstration: temporarily simulate an offending
      package (without committing the simulation) and show the check fails
      with the package named — or state precisely why that demonstration was
      impossible.
- [ ] `git status --short` in your worktree shows only `.github/` changes.

**Provenance notes**
- None (our own config).

**Report back**
Files changed; the check's exact logic; the local re-run results (per crate,
per target); the forced-failure demonstration or why it was impossible; the
YAML validation method; anything you could not verify (including that GitHub
itself has not run it — no remote exists).
