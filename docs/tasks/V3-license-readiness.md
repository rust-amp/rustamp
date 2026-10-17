# V3 — License readiness audit

Date: 2026-10-17. Baseline: `07ee09d`. Scope: documentation and licensing/CI
scripts only. No crate source, Cargo manifest or lockfile changed.

Working directory for **every command**:
`C:\Users\micro\.local\share\opencode\worktree\26de0e\wave37-license-audit`.
No reads or writes in the canonical checkout, research tree or reference-binary
tree. Cached freely licensed package manifests were read outside the worktree;
all file writes and the Clippy target directory were inside the worktree.

**Verdict:** the original gate was not a complete permissive-license gate.
The corrected gate passes locally, offline, for five roots × two Windows
targets with all workspace features. The inventory now covers all **289
external package versions (267 names)** in the unchanged lockfile. This is
metadata evidence, not legal clearance or a remotely observed CI result.

## 1. Findings, highest severity first

`baseline:path:line` below means the file at `07ee09d`, reproducible with
`git show 07ee09d:<path>`. Other references are to the corrected worktree.

| Severity / finding | Evidence | Fix / disposition |
|---|---|---|
| **High — copyleft and unknown licenses passed** | `baseline:.github/scripts/check-permissive-licenses.sh:58-64,106-122`: only GPL/AGPL/LGPL/GFDL substrings were rejected; any other nonempty text passed. Mock-Cargo receipts below show MPL-2.0, EPL-2.0, CDDL-1.0, `?`, `Unknown`, `NOASSERTION` returning 0. | `.github/scripts/license-audit.py:21-60,107-119`: positive identifier allowlist and expression grammar; null/empty/unknown/malformed/non-allowlisted licenses fail. Even copyleft alternatives in `OR` fail conservatively. |
| **High — optional-feature dependencies were not inspected** | `baseline:.github/scripts/check-permissive-licenses.sh:33-36,76`: default features only; comment deferred `--all-features` until someone noticed a new optional dependency. A lane could add a copyleft optional normal dependency and leave it disabled by default. | `.github/scripts/license-audit.py:63-71,179-180`: locked/offline/all-features metadata, filtered separately for both Windows targets. No manual closure maintenance. |
| **High — GPL compatibility was presented as sufficient** | `baseline:CONTRIBUTING.md:35-43`: “any GPL-compatible license works”; `PLUGIN-EXCEPTION.md:21-24,28-40`: our permission cannot cover third-party GPL material and its scope excludes decoders/skin. | `CONTRIBUTING.md:35-56,77-83` and `PLUGIN-EXCEPTION.md:55-58`: separate plugin-exception/combination review for copied **or linked** third-party copyleft. Permissive host dependencies must not impose copyleft on plugins. GPL-2.0-only is not GPL-3.0-compatible. **Human review remains necessary.** |
| **High — no Cargo dependency notices at all** | `baseline:THIRD_PARTY.md:8-14`: only copied webamp tables; `Cargo.lock:5-3068`: 312 packages, 23 workspace + 289 external versions. Example omissions: postcard (`Cargo.lock:1463-1474`), Symphonia (`Cargo.lock:2030-2050`), Windows (`Cargo.lock:2678-2688`). | `THIRD_PARTY.md:57-398`: direct purposes/consumers and complete exact name/version/declared-license/source inventory. `.github/scripts/license-audit.py:122-161`: compare metadata identities against the entire lockfile and compare the notice block exactly. Missing/stale/extra/wrong rows fail. |
| **Medium — coverage could be reduced to nothing** | `baseline:.github/scripts/check-permissive-licenses.sh:51,56,82-85,125-138`: caller-controlled root/target lists; no nonempty-closure assertion. Whitespace-only root override and successful empty Cargo output both returned 0 in fixtures. Additional permissive workspace roots were not discovered. | `.github/scripts/license-audit.py:75-91,169-171`: five mandatory roots must exist and retain MIT OR Apache-2.0; additional permissive members are discovered; unknown workspace licenses and either coverage override fail. Missing/malformed graph data fails rather than becoming an empty success. |
| **Medium — lockfile was mistaken for a local cache** | `baseline:.github/scripts/check-permissive-licenses.sh:12-14,76`: claimed no network while lockfile current, but used neither `--offline` nor `--locked`; cold checkouts still need index/manifests/sources and Cargo may update a lockfile. | `.github/scripts/license-audit.py:63-71,169`; `.github/workflows/ci.yml:24-32`: explicit Python requirement and CI fetch step, then offline/locked audit. No fetch was run in this lane. |
| **Medium — four/five split contradicted the manifests** | `baseline:CLEANROOM.md:21`; `baseline:CONTRIBUTING.md:17-19,37-40`; `baseline:docs/HANDOFF.md:13,58,170`; `baseline:THIRD_PARTY.md:25-26,43-45`; `baseline:docs/PLAN.md:7,57-59`; `baseline:PLUGIN-EXCEPTION.md:42-44`; `baseline:.github/pull_request_template.md:22`. `crates/rustamp-mediahost/Cargo.toml:12` is permissive already. | All active summaries corrected; historical RFC/task/research statements explicitly marked superseded (file list in §6). README already listed five; corrected its overbroad exception statement instead. |
| **Medium — Symphonia was misclassified as permissive** | `baseline:README.md:53`; `baseline:docs/HANDOFF.md:107`; `baseline:docs/rfc/0006-out-of-process-hosting.md:220-221`. Cached `symphonia-0.5.5/Cargo.toml:39` declares `MPL-2.0`; all 16 locked Symphonia-family packages declare MPL-2.0. | Corrected those documents; a future resident must preserve MPL obligations and must not enter the permissive framework closure. **Residual, deliberately not edited:** the same false example is in `crates/rustamp-mediahost/Cargo.toml:9-10`; manifests are out of this lane's write scope. |
| **Medium — runtime licensing is wider than Cargo normal edges** | `baseline:.github/scripts/check-permissive-licenses.sh:72-75` explicitly excludes build/dev edges. Metadata cannot enumerate arbitrary libraries linked by `build.rs`, code generated/copied into an artifact, or upstream declaration accuracy. The corrected traversal intentionally follows normal edges only (`.github/scripts/license-audit.py:112-117`). | `CONTRIBUTING.md:77-83`: mandatory review and notice/source obligations for build/dev/native/generated/copied material. The inventory includes build/dev/other-platform packages, but inclusion is **not** automatic license compatibility approval. |
| **Unproven — remote enforcement** | `git remote -v`: empty. `.github/workflows/ci.yml:12-14,46-48`: “required” is a job name, not a branch-protection setting; no `needs` orders these jobs. | Handoff corrected. Actions execution, cold-cache fetching, hosted tool availability and actual merge blocking remain unproven; no remote created and no workflow run claimed. |

## 2. Exact permissive set and exception scope (question 1)

| Current workspace crate | Manifest evidence | Own license |
|---|---|---|
| `rustamp-proto` | `crates/rustamp-proto/Cargo.toml:8` | MIT OR Apache-2.0 |
| `rustamp-plugin-abi` | `crates/rustamp-plugin-abi/Cargo.toml:8` | MIT OR Apache-2.0 |
| `rustamp-ipc` | `crates/rustamp-ipc/Cargo.toml:8` | MIT OR Apache-2.0 |
| `rustamp-plughost` | `crates/rustamp-plughost/Cargo.toml:8` | MIT OR Apache-2.0 |
| `rustamp-mediahost` | `crates/rustamp-mediahost/Cargo.toml:6-12,17-25` | MIT OR Apache-2.0; framework only, no resident dependency |

`Cargo.toml:3-26` has **23** workspace members (22 under `crates/`, plus
`xtask`), not the old handoff's 22-member snapshot. All other 18 manifests use
`license.workspace = true` (each `Cargo.toml:6`, including `xtask/Cargo.toml:6`),
inheriting `GPL-3.0-or-later` from `Cargo.toml:32`. The complete own-license
split, including previously omitted `xtask`, is `THIRD_PARTY.md:23-30`.

`rustamp-mediahost`'s metadata is inspected on **both** targets to catch
target-specific dependencies; this does not claim its intended x64-only
runtime has been built/run on i686 (`crates/rustamp-mediahost/Cargo.toml:22-25`).
There is no per-resident helper in `Cargo.toml:3-26` or `Cargo.lock` yet.

License metadata does not encode the additional permission. The actual scope
is `PLUGIN-EXCEPTION.md:28-40`: decoders and skin are deliberately excluded;
`xtask` is not listed either. Do not infer an exception for every GPL crate.

## 3. Original script, line by line; corrected coverage (question 2)

| Original line range | Audit result |
|---|---|
| 1-44 | Claims cargo-only, no-network-with-lock, four roots and complete default-feature coverage were misleading/stale. The fail-closed claim was stronger than its parser. Git Bash advice is valid here. |
| 45-56 | `set -euo pipefail` is useful. Actual defaults already contain **five** roots and **both** targets; `mediahost` was **not** omitted from the executed default list. Environment overrides can shrink or empty coverage. |
| 58-64 | Case-insensitive GPL/AGPL/LGPL/GFDL blacklist, not an all-copyleft/permissive decision. MPL/EPL/CDDL and proprietary/unknown strings pass. |
| 66-80 | Per-root/per-target `cargo tree -e normal` resolves the transitive **default-feature normal closure**, including the root and target-specific normal dependencies. Cargo failure exits 1. No offline, lock-preservation or all-features flag. |
| 82-112 | Separator/empty-license rejection is real. Empty emitted license fails. An unreadable/malformed manifest causing Cargo failure also fails at 76-79. However, whitespace, `?`, `Unknown`, `NOASSERTION` or other nonempty non-GPL text is not validated. Empty successful output yields zero packages and still passes. |
| 114-123 | Deny-list match fails with best-effort inverse-tree evidence; failure of that diagnostic deliberately cannot reverse the verdict. `MIT OR GPL` also fails (conservative policy). |
| 125-138 | Aggregates observed violations, but does not require any package/root/target to have been checked. Failure message omits mediahost; success message says five even when overridden. |

**Missing metadata precision:** no claim that the old script unconditionally
silently accepted absent metadata. It rejected a truly empty `{l}` and Cargo
errors; it silently accepted nonempty unknown placeholders. Every currently
locked package has a declared license, so a real missing-manifest-license
Cargo rendering was not reproduced without editing/creating a Cargo.toml
(forbidden here). The fixtures below isolate and prove the parser behavior.
The replacement uses JSON `license: null`/strings, not display placeholders;
null/empty/malformed/unknown are explicitly rejected.

Corrected normal graph: `.github/scripts/license-audit.py:94-119`. Cargo's
`dep_kinds.kind == null` is a normal edge; build/dev edges are not traversed.
Both targets and all workspace features: `:18,63-71,179-180`. Mandatory root
and additional-root discovery: `:14-17,75-91`. Allowlist is intentionally
conservative, not a full SPDX evaluator; valid but unreviewed identifiers or
`WITH` exceptions fail until reviewed. It is stricter than accepting a
permissive branch of an expression containing a copyleft alternative.

## 4. Offline/fresh-checkout requirements and receipts (question 3)

| Requirement | Original / corrected behavior | This environment |
|---|---|---|
| Cargo + toolchain | Resolver only; no workspace artifact build/run needed by audit. Target std/MSVC linker not needed for metadata. | Installed; offline metadata resolved successfully. |
| Cargo.lock | Original could generate/update it; replacement requires it and passes `--locked`. | Present, unchanged; 312 package identities exactly equal metadata. |
| Cached registry index and package sources/manifests | Lockfile alone is insufficient. Replacement never downloads: `--offline`. | All needed packages cached; no download/fetch/install performed. |
| Python | New requirement: >= 3.11, stdlib (`tomllib`), no pip package. Bash entrypoint delegates to it. | Installed Python 3.14; direct and Git-Bash entrypoints pass. |
| Bash | Git Bash, not the WSL launcher in System32. Python direct invocation also works. | Ran `C:\Program Files\Git\bin\bash.exe`; PATH's bare bash points to System32. |
| Cold checkout | Approved cache population/vendor provisioning needed before offline audit. CI explicitly installs Python and `cargo fetch --locked`, then audits offline (`ci.yml:24-44`). | Cold checkout **not** exercised; would require network/cache provisioning. No network step run. |

Commands run from the worktree, with exit status 0 unless noted:

| Receipt | Result |
|---|---|
| `cargo metadata --offline --locked --format-version 1` and all-features variant | 312 packages/nodes; 23 workspace, 289 external versions / 267 external names. |
| `python .github/scripts/license-audit.py --inventory` | Complete 289-row inventory printed; manually applied as a documentation patch, no Cargo changes. |
| `C:\Program Files\Git\bin\bash.exe .github/scripts/check-permissive-licenses.sh` | Inventory matches; all ten root/target closures clean (counts below). |
| `python .github/scripts/check-license-audit.py` | **57** metadata-only regression checks pass: license grammar/null/unknown/copyleft alternatives, flags and platforms, injected bad normal dependency path, missing/relicensed/additional roots, stale/missing/duplicate inventory, Cargo failure, missing lock, coverage overrides. Only in-memory fixtures; no Cargo.toml edits or Rust suite. |
| Baseline script + shell-function mock Cargo, script loaded from `git show HEAD:...` into memory | `MIT` → 0; empty license → 1; `?`, `Unknown`, `NOASSERTION`, MPL-2.0, EPL-2.0, CDDL-1.0 → **0**; GPL-3.0-only and MIT OR GPL-3.0-only → 1. Empty successful output → **0**; whitespace root list → **0**. No real Cargo resolution in these legacy fixtures. |
| `cargo fmt --all -- --check` | Pass. |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | Pass; offline via `CARGO_NET_OFFLINE=true`, target explicitly inside worktree. Build/check, **not a test run**. |
| Installed PyYAML `safe_load` + job/command assertions | CI YAML parses; licenses job and fixture command present. Not actionlint/Actions execution. |
| `git diff --check` | Pass. |
| `git remote -v` | Empty. |

| Root | x86_64-pc-windows-msvc normal packages | i686-pc-windows-msvc normal packages |
|---|---:|---:|
| rustamp-proto | 8 | 8 |
| rustamp-plugin-abi | 8 | 8 |
| rustamp-ipc | 37 | 34 |
| rustamp-plughost | 38 | 35 |
| rustamp-mediahost | 38 | 35 |

Counts include roots/procedural-macro normal edges and are per-closure, not
distinct workspace-wide package totals.

## 5. Notice reconciliation and dependency rule (questions 4-5)

| Comparison | Evidence / result |
|---|---|
| Every external dependency's name, exact version and license | **Entire list:** `THIRD_PARTY.md:106-398` (289 rows). Exact lock identity equality, not a Windows-only subset: `.github/scripts/license-audit.py:129-137`. Licenses come from cached manifests; Cargo.lock itself has no license fields (`Cargo.lock:5-13` is the first example). |
| Missing before audit | **All 289 rows** of that list were missing from the baseline's 47-line notice. This includes every direct dependency in `THIRD_PARTY.md:65-82` and all their transitives, build/dev and foreign-platform dependencies. |
| Stale/wrong existing dependency entries | No prior Cargo entries existed, so no prior version/license entries to update. Existing webamp source attribution retained, not claimed re-audited against upstream. |
| Missing/wrong own split | Baseline omitted mediahost and xtask; exception summary failed to mention skin exclusion and confused copied source with dependencies. Corrected `THIRD_PARTY.md:23-55`. |
| Important exact license distinctions | 16 Symphonia 0.5.5 packages: MPL-2.0, not permissive (`Cargo.lock:2030-2222`; cached `symphonia-0.5.5/Cargo.toml:39`). `r-efi` 5.3.0/6.0.0 has MIT OR Apache-2.0 OR LGPL-2.1-or-later (cached manifests `:39`; `Cargo.lock:1512-1522`), not LGPL-only. cpal 0.18.2 is Apache-2.0 (cached manifest `:34`); zip 9.0.3 is MIT (`:50`); unicode-ident has an **AND Unicode-3.0** obligation (`:36`). Exact expressions retained. |
| Planned crates | `rubato` is absent from this Cargo.lock; earlier license research is not evidence it is a current dependency. No stale “already used” entry added. |
| How the old rule could be bypassed | MPL normal dependency; disabled optional GPL normal dependency; new permissive member missing from fixed list; build/native linked code outside normal edges; GPL-compatible third-party GPL added to player without considering exception. Original script and policy lines in §1 identify each gap. |
| Smallest safe policy/mechanism fix | Positive allowlist + null/grammar validation; all-features/both-target locked graph; mandatory + discovered roots without overrides; exact lock inventory; explicit human build/native/copied-code and GPL-side combination review. No per-crate owner approval reinstated. |

Allowlist passage proves only declared licenses for supported Windows normal
closures. It does not prove full GPL-side compatibility, third-party extension
of our exception, license-text accuracy, maintenance quality, native-library
licensing or distribution compliance. Build/dev packages are recorded but not
automatically required to be permissive by this normal-closure gate.

## 6. Files corrected / added

| Change | Paths |
|---|---|
| Current five-crate split, contribution/dependency procedure and scope | `CLEANROOM.md`, `CONTRIBUTING.md`, `PLUGIN-EXCEPTION.md`, `THIRD_PARTY.md`, `docs/HANDOFF.md`, `docs/PLAN.md`, `.github/pull_request_template.md` |
| Historical lists explicitly superseded, original decision history preserved | `docs/rfc/0004-plugin-licensing.md`, `docs/notes/L-licensing-plugins.md`, `docs/tasks/L1-licensing-split.md`, `docs/tasks/V1-gpl-free-ci.md` |
| Symphonia MPL classification; README exception scope | `README.md`, `docs/HANDOFF.md`, `docs/rfc/0006-out-of-process-hosting.md` |
| Offline checker/complete notice enforcement and reproducible fixtures | `.github/scripts/check-permissive-licenses.sh` (replaced), `.github/scripts/license-audit.py` (new), `.github/scripts/check-license-audit.py` (new) |
| Fresh-run prerequisites and fixture invocation | `.github/workflows/ci.yml` |
| This evidence card and quotable lane checklist | `docs/tasks/V3-license-readiness.md` (new) |

## 7. Dependency lane checklist (question 6; quote into a prompt)

1. **Work only in the assigned worktree.** Do not read/write the canonical,
   research or reference-binary trees. No tests/app/plugin binaries on the host;
   tests belong in the Hyper-V test VM. No commit unless separately authorized.
2. Choose a maintained crate; inspect its **actual manifest and license files**
   for the exact resolved version, not just an API/directory label. Record
   name/version/license and manifest/license-file `path:line` evidence. Check
   enabled features, transitives and Windows x86_64/i686 target variants.
3. For permissive roots, require only permissive licenses throughout the normal
   closure. Do not let any dependency of plughost impose copyleft on a plugin.
   For GPL crates, require GPL-3.0 compatibility **and** review the resulting
   plugin combination: Rustamp's §7 permission cannot cover upstream GPL code.
   Review build/dev tools, generated/copied source and native linked libraries
   separately; preserve notices/source obligations. An unknown license is not
   an approval. A new allowlist identifier needs evidence/review, not a bypass.
4. Update the **owning `crates/<crate>/Cargo.toml`**, root
   **`Cargo.toml` `[workspace.dependencies]`** if shared, and the resolved
   **`Cargo.lock`**. Ordinary compatible Rust dependency acquisition needs no
   per-crate owner approval; lane-specific no-download restrictions still win.
   If the cache is insufficient and downloads are forbidden, report the blocker;
   do not silently use an unlocked/different resolution.
5. Update **`THIRD_PARTY.md`**: direct purpose/consumer, all added/changed
   transitive versions/licenses, source/notice obligations, snapshot date/count.
   Run `python .github/scripts/license-audit.py --inventory`; review and replace
   only the marked generated block. Retain copied-source attributions. A new
   workspace crate also requires the own-license split/docs to be updated.
6. From the worktree root, run **exactly**
   `bash .github/scripts/check-permissive-licenses.sh` with Git Bash on Windows,
   or `python .github/scripts/license-audit.py`. Prerequisites: Python >= 3.11,
   Cargo, current lockfile and cached package manifests/index/sources. The check
   is offline/locked, all-features and both Windows targets; never set
   `RUSTAMP_LICENSE_CRATES`/`RUSTAMP_LICENSE_TARGETS`, weaken the policy or use
   `--inventory` as a passing-check receipt. Root/closure discovery is automatic.
7. If changing checker/policy code, also run
   `python .github/scripts/check-license-audit.py`. Run required fmt/Clippy
   checks within lane restrictions; obtain **VM** test evidence from the
   integration lead, never run `cargo test` on the host.
8. Report metadata only: exact dependency/version/license; reviewed
   manifest/license-file references; normal/build/dev/native/copy classification;
   consumer/features/targets; direct/transitive lockfile delta and notice diff;
   GPL-side exception review if relevant; **full license-check output and exit
   code**; any unverified/network/VM blockers. Finish with
   **`git status --short` in the assigned worktree** and confirmation of no
   forbidden reads/writes, host tests or unauthorized commit.

## 8. What remains unproven (question 7)

- **No GitHub remote / no observed Actions run.** Fresh Windows runner setup,
  actual Python/Cargo/cache/fetch integration, credentials/network behavior,
  service-side job results and required-check branch protection are unproven.
  YAML parsing/local script passage are not substitutes. Jobs are parallel;
  naming a job “required” does not block merges by itself.
- No cold-cache offline success claimed; missing packages should fail. No
  network/download/install performed to manufacture fresh-checkout evidence.
- No test suite or GUI/plugin runtime evidence produced. No `cargo test`,
  reference binary, player or host application executed. Clippy was only the
  explicitly permitted compilation/static check. The 57 Python checks are
  metadata fixtures, not Rust tests or VM validation.
- No audit of every upstream license text, native link input or generated
  artifact; no legal conclusion that the exception covers third-party GPL.
- `crates/rustamp-mediahost/Cargo.toml:9-10` still has the false “Symphonia is
  permissive” comment. Reported, not edited: Cargo.toml is out of write scope.
- Final boundary evidence: `git status --short` run **inside this worktree**;
  only the paths in §6 are changed/untracked. This cannot independently prove
  another tree is clean; that tree was deliberately not inspected. No git
  writes, no commit, no source/manifest/lockfile edits.

Final `git status --short` snapshot in the assigned worktree (initial status
was empty):

```text
 M .github/pull_request_template.md
 M .github/scripts/check-permissive-licenses.sh
 M .github/workflows/ci.yml
 M CLEANROOM.md
 M CONTRIBUTING.md
 M PLUGIN-EXCEPTION.md
 M README.md
 M THIRD_PARTY.md
 M docs/HANDOFF.md
 M docs/PLAN.md
 M docs/notes/L-licensing-plugins.md
 M docs/rfc/0004-plugin-licensing.md
 M docs/rfc/0006-out-of-process-hosting.md
 M docs/tasks/L1-licensing-split.md
 M docs/tasks/V1-gpl-free-ci.md
?? .github/scripts/check-license-audit.py
?? .github/scripts/license-audit.py
?? docs/tasks/V3-license-readiness.md
```
