# Contributing to Rustamp

Thanks for helping. Please read this file and [CLEANROOM.md](CLEANROOM.md) before you start. The clean-room rules are binding.

## Workflow

- **Task cards.** Every piece of work starts from a task card (use the *task card* issue template). It names the goal, lane, owned paths, allowed sources, spec dependencies, interface contract and acceptance tests.
- **One agent or person per crate or area.** Don't edit another area's crate without coordinating. Owned paths are listed on the task card.
- **Small PRs to `main`.** Keep each PR focused on one task card. Use the pull request template; it includes the provenance checklist.
- **Contract changes go through an RFC.** Changes to shared contracts (`rustamp-core` types and traits, the `rustamp-ipc` protocol, public plugin ABI structs) need a short RFC in `docs/rfc/` first. The integration lead approves it.
- **CI must be green** before merge. The `windows` job is required. The `linux` job is informational until the cross-platform milestone.

## Licensing of contributions (inbound = outbound)

By submitting a contribution you license it under the license of the crate you
contribute to — **inbound = outbound**. Most crates are
**GPL-3.0-or-later**; the plugin-facing crates `rustamp-proto`,
`rustamp-plugin-abi`, `rustamp-ipc`, `rustamp-plughost` and `rustamp-mediahost` are
**MIT OR Apache-2.0** ([RFC 0004](docs/rfc/0004-plugin-licensing.md)). Do not
contribute code you cannot license under the target crate's license, and say so
if a contribution is meant to carry a different license.

This statement must hold before the first external contribution: once code from
outside has arrived under a crate's license, relicensing that crate needs every
contributor's consent.

## Dependencies

- **Prefer a well-maintained crate over a self-implementation** (owner rule,
  2026-10-17). Reach for an established, actively maintained crate instead of
  writing the algorithm yourself. Hand-rolling is the better choice only when no
  suitable crate exists, the crate is unmaintained, or it cannot meet a hard
  constraint here (the audio thread's no-allocation/no-lock rule, the MSRV, or a
  contract we must not change).
- **The license must be compatible, and it decides who may depend on what.**
  For GPL crates the dependency must be GPL-3.0-or-later compatible, **and must
  not defeat the plugin exception**: our §7 permission cannot grant rights in
  third-party GPL material (`PLUGIN-EXCEPTION.md`, permission and scope).
  Review the resulting player/plugin combination before introducing third-party
  copyleft, including through linking rather than copying; do not claim the
  exception extends to that dependency. A plain GPL-2.0-only dependency is not
  GPL-3.0 compatible. For the **five** permissive crates
  (`rustamp-proto`, `rustamp-plugin-abi`, `rustamp-ipc`, `rustamp-plughost`,
  `rustamp-mediahost`) only permissive licenses are allowed throughout their
  normal dependency closures, including optional features and target-specific
  dependencies. In particular, no dependency of the DLL-loading host may impose
  copyleft on a plugin; our GPL exception cannot cure that. The `licenses` job
  checks these closures for both Windows targets with all features against a
  conservative license-identifier allowlist. Unknown, missing, malformed and
  non-allowlisted licenses fail, even a copyleft branch in an `OR` expression.
  A legitimate new permissive identifier needs a documented license review
  before expanding the allowlist, not a per-crate owner approval. Check the
  license before adding, and
  check the crate's own manifest rather than a directory API: `rubato`, for
  example, has an empty license field on crates.io and `NOASSERTION` on GitHub,
  but declares `MIT OR Apache-2.0` in its `Cargo.toml`.
- **Adding a Rust dependency no longer needs a per-crate owner approval**
  (owner rule, 2026-10-17), but the license check and the record are not
  optional: update the owning `Cargo.toml` (and root `[workspace.dependencies]`
  when shared), include the resolved `Cargo.lock` in the change, and **record every new
  dependency version**, including transitives, in `THIRD_PARTY.md`. Record the
  direct dependency's purpose/consumer separately from the generated inventory.
  Print the replacement inventory with
  `python .github/scripts/license-audit.py --inventory`, review it, and replace
  only its marked block. Then run
  `bash .github/scripts/check-permissive-licenses.sh` from the checkout root
  (Git Bash on Windows), or `python .github/scripts/license-audit.py` directly.
  Root/closure discovery is automatic; do not add package names to a manual
  closure or set coverage overrides. Requires Python >= 3.11, Cargo, a current
  lockfile and cached manifests/index/sources. It is offline/locked and builds
  nothing; a cold checkout needs an explicitly authorized `cargo fetch --locked`
  first (ordinary dependency acquisition is covered by the owner rule above).
  Report the exact name/version/license, manifest/license-file evidence,
  direct/transitive lock delta, consumers/features/targets, inventory diff and
  check exit/output. Run tests only through the Hyper-V test-VM workflow, never
  on the host.
- **The automatic check is not legal clearance.** It covers normal edges, not
  build/dev edges, arbitrary native libraries linked by `build.rs`, copied code,
  license-text correctness or GPL-side plugin-exception compatibility. Review
  those explicitly, including generated code and all enabled feature variants;
  preserve required notices and source obligations. Record that review in the
  lane's evidence. A passing metadata check is not evidence that an upstream
  declaration describes every bundled file or that an artifact is compliant.

## Coding standards

- **Formatting:** `cargo fmt --all`. The config is in `rustfmt.toml`.
- **Toolchain and MSRV:** stable Rust, currently **1.99**. The workspace declares `rust-version = "1.99"` so that current crate releases stay installable; raise it together with the toolchain instead of pinning older dependencies. Note that raising the MSRV *activates* MSRV-gated Clippy lints that were previously suppressed (`as_chunks`, `is_multiple_of`, `collapsible_if` with let-chains); expect mechanical fallout and run `cargo clippy --fix --allow-dirty` for the machine-applicable ones.
- **Lints:** `cargo clippy --workspace --all-targets --all-features -- -D warnings`. No warnings are allowed; fix them or justify a narrow `#[allow]` in the code with a comment.
- **Audio thread:** no heap allocations, no locks (`Mutex`, `RwLock`), no blocking I/O and no logging on the audio thread. Communicate through the lock-free rings in `rtrb`.
- **`unsafe`:** every `unsafe` block or `unsafe fn` must be preceded by a `// SAFETY:` comment that states the invariant that makes it sound. PRs without it are rejected.
- **Public API:** document public items. Keep the crate's public surface small.
- **No copied code** from Winamp or any dirty source. Free third-party code needs attribution and a THIRD_PARTY entry (see CLEANROOM.md, section 7).

## Testing

- **Unit tests** are expected for every new module. Put them next to the code in `#[cfg(test)]` modules, or in `tests/` for integration tests.
- **Property tests** where they fit (playlist operations, shuffle, title formatting, parsers).
- **Golden tests** (image and audio comparisons) run later via `cargo xtask golden`. Every window or renderer change should add or update a golden case once the harness exists. Golden tests run with both the original base skin and the look-alike default skin.
- Run `cargo test --workspace --all-features` **in the Hyper-V test VM, never on the host**, before you open a PR. Put the VM results in the PR's test evidence section; a host compile/check is not a test run.
- **`RUSTAMP_TESTDATA`** points at a directory holding the `classic-base` skin used by the golden tests. **It is unset by default, and the `classic-base` cases skip silently when it is** — set it locally to run them. The free webamp checkout contains the original base skin (`base-2.91`); point the variable at its parent directory. A skipped golden case is not a passed one: if your change touches the renderer, run the golden tests with the variable set and say so in the PR.

## Provenance (required)

Rustamp is clean-room. Each PR must:

1. Say which specs (`specs/S-xx-*.md`, `specs/D-xx-*.md`) and free sources it used.
2. Confirm that you did not view Winamp source, SDK material or disassembly.
3. List any third-party code copied, with source, license and attribution added.

The checklist is in `.github/pull_request_template.md`. If you are unsure whether something is allowed, ask before you write the code. See [CLEANROOM.md](CLEANROOM.md).

## Commit and PR hygiene

- Write commit messages in the imperative mood, with a short subject line.
- Rebase onto `main` rather than merging it into your branch.
- Keep generated files and binaries out of the repository. Reference binaries and original skins live outside the repos.
