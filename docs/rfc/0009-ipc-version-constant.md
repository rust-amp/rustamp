# RFC 0009: correct the IPC version constant

## Goal

The measured `IPC_GETVERSION` return value is **0x2905**, not the `0x2095` that the audited S09 §8.3 and the crate constant state (two independent sandbox runs agree). The two values sit on opposite sides of the `0x2900` threshold that plugins test, so a plugin built against S09 takes the wrong branch.

## Evidence

- Two independent S10 sandbox runs measured `IPC_GETVERSION` = **10501 = 0x2905**.
- The audited S09 §8.3 states `0x2095` ([SDK] tag — taken from the public plugin development headers, never measured against a running 2.95 binary).
- The crate constant `WINAMP_VERSION_295` is currently `0x2095`.
- Plugins test the return value against `0x2900` to decide whether 2.9+ IPCs exist.
  - `0x2095 < 0x2900` → plugin decides "pre-2.9 IPC surface", takes the wrong branch.
  - `0x2905 >= 0x2900` → plugin correctly detects 2.9+ IPC surface.

## Decision

Change the crate constant `WINAMP_VERSION_295` from `0x2095` to `0x2905`. The `IPC_GETVERSION` message number (0) is unchanged; only the *return value* of the query changes. The threshold `WINAMP_VERSION_2_9X_THRESHOLD` at `0x2900` stays as-is.

## Backwards compatibility

Plugins that currently compare the version against `0x2095` will now see `0x2905`. Since `0x2905 > 0x2900`, such plugins will now correctly enter the 2.9+ code path. This is a bug fix, not a breaking change — the previous value caused plugins to take the wrong branch on any host that implements the measured behaviour.

The old value `0x2095` is preserved in this RFC document as the claimed (but incorrect) SDK-derived value, for historical record and audit transparency.

## Cross-check (pending)

The decisive cross-check that remains is an in-process (plugin-DLL) caller that directly measures the return value of `IPC_GETVERSION` against a running Rustamp host. This has not yet been performed; until it is, the RFC stands on the two sandbox measurement runs.

## RFC acceptance

- The RFC exists in the house style (`docs/rfc/0009-ipc-version-constant.md`).
- The constant is updated to the measured value `0x2905` with a comment naming the RFC and the evidence.
- `cargo fmt --check` and `cargo clippy --workspace --all-targets --all-features -- -D warnings` are clean.
- No test suite is run (the orchestrator runs the suite in the Hyper-V test VM).
- No window is opened.
- No git write is performed.