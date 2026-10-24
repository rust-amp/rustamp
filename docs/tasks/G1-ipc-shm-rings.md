# Task card G1 — Shared-memory PCM and visualization rings (Wave 2)

**Goal**
`rustamp-ipc` gets the bulk transport that the plugin host needs: a lock-free,
allocation-free SPSC ring in shared memory for PCM frames and one for
visualization frames, with an identical byte layout on x64 and i686, plus the
protocol addition that announces and opens a ring. When the card is done, one
process can stream PCM into a ring and the other reads it back without a lock,
without an allocation per frame and without copying twice.

**Lane** G — plugin host / IPC (`crates/rustamp-ipc/`, RFC 0003 in `docs/rfc/`).

**Owned paths**
- `crates/rustamp-ipc/**`
- `docs/rfc/0003-*.md` (new)
- Do **not** edit `Cargo.toml` at the repository root and do not touch any other
  crate. `rustamp-plugin-abi` is frozen (S09); `rustamp-core`'s audio traits are
  frozen by RFC 0001.
- Crate-local additions are fine in `crates/rustamp-ipc/Cargo.toml`. The
  workspace already provides `windows = "0.62"`; `rtrb` is available too if you
  want it for the single-process fallback.

**Allowed sources**
- `rustamp-core` and `rustamp-plugin-abi` in this repository.
- `docs/rfc/0001-core-contracts.md`, `specs/S09-plugin-abi.md`.
- Public documentation of the Win32 shared-memory API and of lock-free ring
  buffers in general.
- `docs/PLAN.md` section 2 (the process diagram and the tech stack).

**Forbidden sources**
- Winamp 5.x source, Winamp SDK headers or documentation, disassembly, the
  Winamp 2.95 binary, `D:\code\rustamp-research`. Nothing from them.
- Do not download anything (no `cargo add`, no git clone, no web fetching).
- Do not guess IPC **message semantics** (which `WM_WA_IPC` ids exist, what a
  plugin expects): that is spec S10 and is explicitly out of scope here. This
  card is transport only.

**Spec dependencies**
- S09 exists (static, runtime confirmation pending) and constrains the *data*
  that flows (576-sample vis data, PCM format). Read it, but do not implement
  any plugin call order.
- S10/S11 (IPC catalogue, embed windows) do not exist. The ring is designed so
  that it does not depend on them: it carries bytes, and whoever writes them
  later decides the semantics.

**Interface contract**
- **RFC 0003** describes, before you implement:
  1. the ring header layout as **facts** (magic, version, kind, capacity,
     frame size, the two monotonic counters, the padding rules) with a
     byte-offset table and its size on x86 and x86_64;
  2. the new `Message` variants that announce a ring (name/handle, kind,
     capacity, frame size) and that acknowledge or refuse it, and the
     `PROTOCOL_VERSION` bump that goes with it;
  3. the exact overrun/underrun rule: the producer **never** blocks and never
     overwrites unread data; on a full ring it drops whole frames and counts
     them; the consumer reads what is available and reports a partial frame as
     "not yet complete", never as data.
  Get the RFC reviewed in the report; it is the integration lead's call.
- The ring API must be usable by both processes and testable in one:
  a `RingProducer`/`RingConsumer` pair over a `MappedRegion` trait, with a
  Windows implementation (named file mapping, `CreateFileMappingW` /
  `MapViewOfFile`) and an anonymous in-process implementation for tests.
- `unsafe` is unavoidable for the mapping: every block needs a `// SAFETY:`
  comment stating the invariant. Keep the unsafe surface as small as possible
  (raw pointers confined to one module; safe API above it).
- No allocation and no lock on the write/read path. `read_into(&mut [f32])`
  and `write_from(&[f32])` must be allocation-free.

**What must work**
- **PCM ring:** interleaved `f32` frames, the frame size fixed when the ring is
  created (a format change re-creates the ring — do not support a mid-stream
  format change in the ring itself).
- **Vis ring:** one slot per `VisFrame` with a fixed slot size, so the consumer
  always sees a complete frame or nothing. Include the timestamp and a
  sequence number so the consumer can detect skipped frames.
- **Layout:** the header and slot layout are byte-identical on x64 and i686;
  prove it with compile-time assertions in the style of
  `crates/rustamp-plugin-abi`'s `x86_sizes` test, and make sure the counters
  are 64-bit and aligned so that a 32-bit producer and a 64-bit consumer agree.
- **Wrap-around** works for any capacity, including a capacity that is not a
  power of two (document the arithmetic you chose).
- Stale rings: a crashed peer must not leave a ring that a new run mistakes for
  live data. Use a generation/epoch field and document the rule.

**Acceptance tests**
- [ ] `cargo fmt --all -- --check` clean.
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean.
- [ ] `cargo test --workspace --all-features` green, with new tests for:
  - [ ] single-threaded round-trip: N frames written and read back byte-exact,
        including wrap-around at least twice;
  - [ ] multi-threaded soak: one producer thread and one consumer thread for a
        few million frames; every frame arrives exactly once, in order, and the
        drop counter plus the received count equals the sent count;
  - [ ] full-ring behaviour: the producer drops whole frames, never blocks,
        never overwrites unread data, and the counter is accurate;
  - [ ] a partial frame in the vis ring is not delivered;
  - [ ] **no allocation on the ring path**: a counting global allocator in a
        test binary that runs `write_from`/`read_into` many times and asserts
        zero allocations;
  - [ ] layout assertions for both architectures (the x86 ones run under
        `--target i686-pc-windows-msvc`);
  - [ ] a Windows named mapping round-trip: create, write, close, re-open by
        name, read (proves the cross-process path, not just the in-process one);
  - [ ] a stale/foreign region is rejected (wrong magic, wrong version, wrong
        frame size) with a clear error, never undefined behaviour.
- [ ] `cargo test -p rustamp-ipc --target i686-pc-windows-msvc` green — the ring
      must build and pass on the 32-bit side. If that needs a crate-local
      target-specific dependency, add it in `crates/rustamp-ipc/Cargo.toml`.
- [ ] `cargo test -p rustamp-plugin-abi --target i686-pc-windows-msvc` and
      `cargo build -p rustamp-plughost --target i686-pc-windows-msvc` still green.
- [ ] State the measured throughput (frames/s) and latency of the soak test in
      your report.

**Provenance notes**
- No third-party code is expected. If you take ring-buffer code from a free
  project (e.g. `rtrb`, which is a dependency you may use), note that you use
  it as a dependency rather than copying it; copying needs a `THIRD_PARTY.md`
  entry.

**Report back**
Files changed, the RFC 0003 summary with the header offset table, test results
and measurements, what you deliberately left out (message semantics, mid-stream
format change), and any contract question that needs a further RFC.
