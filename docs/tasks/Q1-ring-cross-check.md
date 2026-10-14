# Task card Q1 — Cross-bitness ring check and RFC 0002 cleanups

**Goal**
Close the one real verification gap of RFC 0003 (no end-to-end test with a
32-bit producer against a 64-bit consumer) with a runnable check, and do the two
small cleanups RFC 0002 left behind.

**Lane** Q — QA harness, with a small piece of lane B (decoders).

**Owned paths**
- `xtask/**`
- `crates/rustamp-ipc/examples/**` and `crates/rustamp-ipc/tests/**` — **not**
  the ring implementation in `crates/rustamp-ipc/src/`. If you believe the ring
  itself is wrong, report it; do not change it.
- `crates/rustamp-decoders/**` (only the reader-wrapper cleanup below)
- Do **not** edit `Cargo.toml` at the repository root, `docs/HANDOFF.md`,
  `docs/rfc/**` or any other crate.

**Allowed sources**
- The repository: `crates/rustamp-ipc` (RFC 0003 is accepted; read
  `docs/rfc/0003-shm-rings.md`), `crates/rustamp-decoders`,
  `docs/rfc/0002-audio-contracts.md`, `xtask/src`, `crates/rustamp-testkit`.
- Public documentation of `std::process`, `cargo` targets and the Win32
  shared-memory API.

**Forbidden sources**
- Winamp 5.x source, Winamp SDK headers or documentation, disassembly, the
  Winamp 2.95 binary, `D:\code\rustamp-research`. Nothing from them.
- No downloads (no `cargo add`, no git clone, no web fetching). Use the crates
  already in the workspace; `std::process` is enough for the runner.

**Spec dependencies**
- None. This card implements no player behaviour.

**Part 1 — the cross-bitness check**
- Add a small **peer binary** to `rustamp-ipc` (an `examples/` binary is the
  simplest home; it must not be a test, so `cargo test` does not run it) with a
  command line along these lines:
  - `ring_peer produce --name <name> --kind pcm|vis --frames <n> --capacity <n> [--frame-size <n>]`
  - `ring_peer consume --name <name> --frames <n>`
  - The producer creates the ring, writes `n` frames of a deterministic,
    position-dependent pattern, prints the ring name and the generation on
    stdout, then **stays alive** until the consumer has finished (poll
    `state`, or wait for a timeout with a clear error) so the mapping does not
    disappear underneath the consumer.
  - The consumer opens the ring by name (retry briefly while the producer is
    still starting), reads exactly `n` frames, verifies every value against the
    same pattern, and prints a machine-readable result line such as
    `OK received=<n> dropped=<d>`. Any mismatch or short read is a non-zero
    exit with a precise message.
  - Cover both kinds: the PCM ring and the vis ring (`seq`/`timestamp_us` plus
    the 2×576 arrays).
- Add an `xtask` subcommand `cargo xtask ring-cross` that:
  - builds the peer for **both** `x86_64-pc-windows-msvc` and
    `i686-pc-windows-msvc` (locate the binaries under
    `target/<triple>/debug/examples/`),
  - runs both directions (32-bit producer → 64-bit consumer and the reverse)
    for both ring kinds,
  - prints a short table of the four runs with the frames and drops,
  - exits non-zero if any run fails, and skips with a clear message on a
    non-Windows host.
- It must be deterministic: fixed frame counts, a fixed capacity, a bounded
  timeout, no sleeping-based flakiness beyond the short open-retry.

**Part 2 — the RFC 0002 cleanups**
- `rustamp-decoders`: RFC 0002 made `ReadSeek` require `Send + Sync`, so the
  `Mutex<Box<dyn ReadSeek>>` wrapper around the symphonia reader is no longer
  needed. Remove it, keep the type's behaviour identical, and update the wording
  in `crates/rustamp-decoders/CONTRACT_NOTES.md` that still says `Send` only.
  All 25 decoder tests (including the bit-exact WAV ones and the corrupt-packet
  tests) must stay green and unchanged in their expectations.

**Acceptance tests**
- [ ] `cargo fmt --all -- --check` clean.
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean.
- [ ] `cargo test --workspace --all-features` green, with the decoder suite
      unchanged apart from the cleanup.
- [ ] `cargo xtask ring-cross` passes and really runs all four combinations —
      prove it in the report with the printed table, and say explicitly that it
      did not silently skip.
- [ ] `cargo test -p rustamp-ipc --target i686-pc-windows-msvc` green.
- [ ] `cargo test -p rustamp-plugin-abi --target i686-pc-windows-msvc` green.
- [ ] `cargo build -p rustamp-plughost --target i686-pc-windows-msvc` green.
- [ ] `cargo xtask golden` reports no mismatch.

**Provenance notes**
- No third-party code is copied. If you add a dependency, stop and report it
  first.

**Report back**
Files changed, the printed table of the four runs, what the check would have
caught (e.g. did you sanity-check it by breaking something on purpose?), the
decoders cleanup, and anything you could not verify.
