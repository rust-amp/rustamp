# Task card M6 — RFC 0006, unblocked parts: the media host crate and protocol (M2)

**Goal** Build what RFC 0006 needs that does **not** depend on the ring wake
mechanism: the new crate `rustamp-mediahost` with its two binaries, the protocol
types in `rustamp-proto`/`rustamp-ipc`, the version accounting and the layout
tests. RFC 0006 and RFC 0007 are **accepted**; implement them, do not redesign.

**Lane** M — plugin/media host. **Owned paths** `crates/rustamp-mediahost/**`
(new), `crates/rustamp-proto/**`, `crates/rustamp-ipc/**`, and the **root
`Cargo.toml`** for the new workspace member and `[workspace.dependencies]`
entries only. Nothing else: `rustamp-core`, `rustamp-audio`, `rustamp-ui`,
`xtask`, `docs/**` are findings, not edits.

**Allowed sources** `docs/rfc/0006-out-of-process-hosting.md` (your contract),
`docs/rfc/0007-blocking-consumer.md` (the wake design, accepted),
`docs/rfc/0003-shm-rings.md`, `docs/rfc/0004-plugin-licensing.md` (the permissive
invariant), `docs/notes/plugin-authors.md`, the existing `rustamp-ipc`/`proto`
code.

**Forbidden** Winamp source/SDK/disassembly/binary, `rustamp-research`,
`rustamp-refbin`. No downloads, no new crate versions (`cargo add` forbidden).

**Scope, in order**
1. `rustamp-mediahost` (GPL-3.0-or-later, **no §7 exception** — RFC 0006 §8) with
   two bin stubs `rustamp-dechost` and `rustamp-dsphost`, 64-bit, depending only
   on `rustamp-ipc` + `rustamp-proto`.
2. The RFC 0006 §7 wire types in `rustamp-proto`: `MediaLocationWire`,
   `DspStageDesc`, the wire spec struct; and the new `Message` variants in
   `rustamp-ipc` (`DecoderOpen/Opened/Seek/Seeked/Format/Stats/Eos/Error`,
   `DspConfigure/Ready/Reset/Error`), appended, with layout/round-trip tests.
3. **Version accounting:** RFC 0005 took `PROTOCOL_VERSION` to 2. RFC 0006's
   variants and RFC 0007's wake both ride that same v2 **while no peer ships v2**;
   if they cannot, they take 3. Decide with the evidence in the repo, document the
   reasoning in `message.rs`'s version history, and say which you chose.
4. The permissive-closure proof re-runs clean (the new crate is GPL; nothing
   permissive may depend on it).
**Out of scope:** the actual PCM transfer, the ring wake implementation, opening
a real decoder in the host, spawning processes. Stubs plus protocol only — the
PCM path is a later card and needs the lossless-delivery work.

**Acceptance** fmt/clippy clean; the **orchestrator** runs the suite in the test
VM (do not run tests yourself); i686 checks unaffected; no window, no spawned
child on the host.

**Report** files; the version decision with its reasoning; what is stubbed; the
closure proof; what the PCM card still needs.
