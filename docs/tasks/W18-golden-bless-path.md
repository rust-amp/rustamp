# W18: golden bless path through the VM

Status: code written, **not executed**. Nothing about this was executed against the guest.

## Problem

`tests/golden/lookalike/vis_stopped_tail.png` and `tests/golden/rust-theme/vis_stopped_tail.png`
were rendered under an old behaviour and must be re-blessed. Blessing is the
environment variable `RUSTAMP_BLESS=1`, read in `crates/rustamp-testkit/src/golden.rs`
(`assert_golden`, `== "1"`): the test writes the golden instead of comparing it.
The golden binaries are `golden_lookalike` and `golden_rust_theme`
(`crates/rustamp-ui/tests/golden_lookalike.rs`, `golden_rust_theme.rs`).

Two facts shape the design:

- The guest writes goldens under `C:\rustamp\code\Rustamp\tests\golden\` (that is
  `D:\code\Rustamp\tests\golden` through the guest's `subst`). The broker's `copyout`
  op only reads from `C:\rustamp\out\` (`vm-broker.ps1`, `copyout` case). So the guest
  script stages the two PNGs under `C:\rustamp\out\bless\` itself.
- `copyout` nests the source folder inside the destination (`Copy-Item ... -Recurse`
  into an existing directory), so the files land one level deeper than the destination.

## The host command

Run from the repository root, as the orchestrator. Quote the token so PowerShell
binds it as the `-Filter` value and does not parse it as a switch:

```
powershell -ExecutionPolicy Bypass -File docs\notes\vm-run-tests.ps1 -Filter '--bless:golden'
```

Add `-Reset` only if you want the baseline restored first (same convention as the
`--ignored` runs). The run does a compile, stage, start, copyin, guest run, and then
the bless copy-out.

## What each half does

**Guest half** (`docs/notes/vm-broker/guest/run-tests.ps1`):

- Parses a leading `--bless` token next to the existing `--ignored` branch. It sets
  `$bless = $true`, strips `^--bless:?` from `$filter`, and sets `$env:RUSTAMP_BLESS = '1'`.
  The token never reaches the binary-name filter.
- Without the token, `$env:RUSTAMP_BLESS` is never set and nothing else changes.
- After the binary loop, and only when `$bless`, it copies
  `tests\golden\{lookalike,rust-theme}\vis_stopped_tail.png` (under the staged repo)
  to `C:\rustamp\out\bless\{lookalike,rust-theme}\vis_stopped_tail.png`. A missing
  source is skipped, not invented.
- The result object is unchanged.

**Host half** (`docs/notes/vm-run-tests.ps1`):

- New parameter `-BlessOut`, default `D:\code\rustamp-vm\staging\out\bless-golden`.
- The host's `--bless` check uses the raw `$Filter`, like `--ignored`, and passes the
  token through `-GuestArgs` unchanged.
- New step `5b`, only when `$Filter -like '--bless*'`: it deletes `$BlessOut` if present,
  then runs `copyout -Source 'C:\rustamp\out\bless' -Dest $BlessOut`. It then checks the
  two expected files and prints `blessed: <path>  SHA-256 <hash>` for each.
- The final exit gate gains `-or $blessMissing -ne 0`, which is 0 on every run without the token.

## Where the files land on the host

```
D:\code\rustamp-vm\staging\out\bless-golden\bless\lookalike\vis_stopped_tail.png
D:\code\rustamp-vm\staging\out\bless-golden\bless\rust-theme\vis_stopped_tail.png
```

The repository's `tests/golden` is **not** changed by this path. The guest rewrote
its own copy only. Promoting the new images into `D:\code\Rustamp\tests\golden\`
is a separate, deliberate step after you have looked at them. It is not in this change.

## What could still go wrong

- **The filter is the only thing that narrows the run.** `--bless` with no name blesses
  the goldens in every binary the guest runs. The copy-out still only brings back the
  two images, but the guest-side goldens are rewritten everywhere. Use `--bless:golden`.
- **Quoting.** If `-Filter --bless:golden` is passed unquoted, PowerShell may try to bind
  `--bless:golden` as a switch. Keep the single quotes.
- **Copy-out failure.** If the guest did not produce the folder, the broker's `copyout`
  throws and the run stops with a non-zero exit. If the folder comes back but a file is
  missing, `BLESS MISSING` is printed and the run exits 1.
- **Stability.** A re-bless only proves the current output. Rerun the golden binaries
  without the token to confirm they now match.
- **Stale staging.** `$BlessOut` is deleted before the copy-out. Only that path is touched.
- **Ran gate.** The existing `ran -eq 0` gate still applies to `--bless:golden`. A name
  that matches no binary fails loudly.
- **Unverified PowerShell.** The change was checked only by the PowerShell parser
  (`ParseFile`, zero errors for both scripts). No function was executed.

## Verification done (host, no VM, no guest)

- `[System.Management.Automation.Language.Parser]::ParseFile` on both edited scripts:
  0 parse errors each.
- `git diff --stat`: `docs/notes/vm-broker/guest/run-tests.ps1` +29,
  `docs/notes/vm-run-tests.ps1` +28/-2.
- Not run: `cargo test`, the VM, the broker, any guest script. `cargo fmt`/`clippy`
  were not run either, since no Rust file was touched.

## Not executed

Nothing about this was executed against the guest.
