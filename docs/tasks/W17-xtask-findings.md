# W17: xtask findings closed (W16-F08, W16-F09, checklist step 10)

Status: implemented in `xtask/`; `cargo test` is run by the orchestrator only.

## W16-F08: full licence texts in the player notices

`player_third_party_notices()` in `xtask/src/package.rs` now embeds the complete
texts:

| Licence | Where it lands | Source of the text |
|---|---|---|
| MPL-2.0 (Symphonia) | section 4 of `player/THIRD_PARTY.md` | `xtask/licenses/MPL-2.0.txt` (373 lines, `include_str!`). It is the SPDX text of the `spdx` crate 0.10.6 from a backup copy of the local cargo registry, with the Rust raw-string wrapper and trailing blanks removed. It equals, modulo trailing whitespace, a second independent copy (`filepath-securejoin` 0.6.1 `LICENSE.MPL-2.0` in the local Go module cache). |
| Apache-2.0 | section 5 of `player/THIRD_PARTY.md` | the repository's own `LICENSE-APACHE` (`include_str!`). |

No network access was used. The live `~/.cargo/registry` holds no MPL text (the Symphonia
crates ship no licence file), which is why the SPDX copy was taken from the backup of
that registry. The host notices are unchanged (they ship `LICENSE-APACHE` beside them
and must stay MPL-free).

## W16-F09: stale README disclaimer

The generated release `README.md` no longer says the spectrum analyzer is zero or that
shuffle/repeat are unconnected. Section 4 now lists, as proven, that shuffle/repeat
drive the engine and that the built-in visualizer is fed from the playing audio. The
unproven list keeps the honest items (physical hardware, plugin UI wiring) and gains
"Visualizer Fidelity" (the response is an own choice, not matched to the reference) and
"Winamp visualization plugins are not hosted". The clean-machine bullet follows the
manifest flag (see below).

## Checklist step 10: offline PE import inspection

`xtask/src/pe_imports.rs` (hand-written, no dependency) parses the import directory
and the delay-load directory of a PE image and checks each module against an allowlist
of Windows system DLLs (plus `api-ms-win-*`/`ext-ms-win-*` API sets, minus
`api-ms-win-crt-*`). The C runtime (`vcruntime*`, `msvcp*`, `ucrtbase`, the UCRT
forwarders) is deliberately not allowed: the package policy is the static CRT.

It refuses, never passes, on: an unreadable file, a non-PE file, truncated or malformed
tables, an empty import directory, the wrong machine type (x64 expected for the player
and residents, x86 for the host), and any non-allowlisted module.

* `cargo xtask pe-imports [--allow a.dll,b.dll] [--machine x64|x86] <file>...`
* `cargo xtask package` inspects the built binaries before staging, then the staged
  copies; only then do the manifests carry `"artifact_imports_verified":true`
  (`ReleaseMetadata::imports_verified`; dry runs and fixtures keep `false`).
* `cargo xtask package --check` on an existing layout inspects its binaries first.

Limits: this is a static check of the import tables. It says nothing about
`LoadLibrary` at run time, and it does not prove a launch on a clean OS (README and
QUICKSTART still say so).
