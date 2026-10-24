# Task card X1 — `cargo xtask package`: ship the two binaries apart (M1)

**Goal** `xtask package` is a stub. Implement it so a release build produces the
two binaries **as separate artefacts**, in line with the licensing split: the
GPL player (`rustamp.exe`) and the permissive plugin host
(`rustamp-plughost32.exe`, i686) must be packageable independently, and the
permissive artefact must not contain a GPL binary. This is the last licensing
remainder (HANDOFF §5, item 6/19).

**Lane** X — tooling. **Owned paths** `xtask/**`. Do not touch any crate, the
root `Cargo.toml`, `docs/**` or `THIRD_PARTY.md`; report those needs instead.

**Allowed sources** `docs/rfc/0004-plugin-licensing.md` (the split and its
acceptance criteria), `README.md` (the License table, the five permissive
crates), `docs/HANDOFF.md` §4 (what each crate is), `xtask/src/main.rs` (the
existing subcommand shape), `PLUGIN-EXCEPTION.md`.

**Scope**
- A `package` subcommand that builds the release artefacts and lays them out in an
  output directory: the **player** (x86_64) and the **plugin host** (i686,
  `rustamp-plughost32.exe`) as **separate, independently shippable** items, each
  with the licence files that apply to it (the GPL text plus
  `PLUGIN-EXCEPTION.md` for the player; `LICENSE-MIT`/`LICENSE-APACHE` for the
  host), and a machine-readable manifest naming what went where.
- It must be **possible to ship the host alone** and to verify that the host
  artefact contains no GPL binary: add a check that fails if the host package
  would include anything from the GPL side.
- Follow the existing `xtask` conventions (subcommand parsing, error style,
  `run_status`, and the **no-window spawn** helper already in `xtask`).
**Out of scope:** installers, signing, a download step, and actually publishing
anything. No new dependency (`cargo add` forbidden); if a crate you need is not
already locked, report it.

**Hard rules** clean room (no Winamp source/SDK/disassembly/binary, no
`rustamp-research`/`refbin`); no downloads; no git write commands; **run no
tests** — the orchestrator runs the suite in the Hyper-V test VM; every spawned
child must use the existing no-window helper.

**Acceptance** fmt and clippy clean; the new subcommand compiles and its dry-run
path is exercised without producing an installer; the GPL-free check for the host
package exists and is demonstrated (or its impossibility is stated plainly).

**Report** files; the layout the command produces; how the host package is proven
GPL-free; the fmt/clippy results; what you could not verify without a release
build; confirmation that you ran no test, no window and no git write.
