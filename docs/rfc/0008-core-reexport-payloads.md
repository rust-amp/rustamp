# RFC 0008: Re-export the RFC 0005 payload enums from `rustamp-core`

Status: **Accepted** on 2026-10-16 (owner approved writing it; implemented and
verified the same day).
Owner: Lane I (contracts). Depends on: RFC 0001 (core contracts), RFC 0004
(`rustamp-proto` is the permissive wire-type crate), RFC 0005 (which added the
three enums).

## 1. Why

RFC 0005 added three payload enums to `rustamp-proto`:

```rust
pub enum OpenMediaKind { Files, Location, Directory }
pub enum JumpTarget { File, Time }
pub enum PrefsPage { VisPlugin, VisOptions }
```

They are used by `Command` variants (`OpenMedia`, `OpenJump`, `OpenPrefsPage`),
so any crate that matches on those variants must name them. `rustamp-core`
re-exports `Command` from `rustamp-proto` at `rustamp_core::command::Command`,
but its `command` module re-exports an **explicit item list** — `Command` and
`SelectOp` only — so the three new enums are **not** reachable at
`rustamp_core::command::*`.

Today `rustamp-ui` (a GPL crate) imports them straight from `rustamp_proto`,
with a comment recording the gap. That works, but it makes `rustamp-core` an
incomplete facade: a consumer that reads the core docs, or that follows the
convention "consume the contract crate", silently cannot name the types it must
name to match on the variants it already sees.

## 2. What changes

`rustamp-core`'s `command` module re-exports the three enums in addition to the
two items it already re-exports:

```rust
pub use rustamp_proto::command::{Command, JumpTarget, OpenMediaKind, PrefsPage, SelectOp};
```

Nothing else moves, nothing is renamed, and no wire type changes. The items stay
defined in `rustamp-proto` (RFC 0004's invariant: the permissive crates must not
depend on a GPL crate — this RFC only widens a GPL crate's re-export, which is
the safe direction).

## 3. The rule this RFC establishes

`rustamp-core` re-exports **every public item of the wire types it re-exports**,
not a hand-picked subset. A payload type that a re-exported enum variant names is
part of the same public surface and must be reachable at the same path. If a
later RFC adds a payload type to a re-exported variant, the re-export list is
extended in the same change.

To stop the gap reopening silently, the implementation adds a test: for each
re-exported enum, a compile-time use of its payload types through the
`rustamp_core` path (a `use rustamp_core::command::*` that names them), so a
missing re-export fails the build rather than being noticed by a reader.

## 4. What does not change

- No `Command` variant, no wire byte, no `PROTOCOL_VERSION` (RFC 0005's v2 stands
  unchanged; RFC 0006 and RFC 0007 still ride it).
- `rustamp-proto` keeps the definitions; the permissive closure stays GPL-free.
- No crate outside `rustamp-core` must change. `rustamp-ui`'s direct
  `rustamp_proto` imports may stay or switch to the core path; the implementation
  card decides, and either is correct.

## 5. Acceptance criteria

1. `rustamp_core::command::{OpenMediaKind, JumpTarget, PrefsPage}` resolve.
2. The new test fails if any of them is removed from the re-export list.
3. `cargo test --workspace --all-features`, the i686 checks, `cargo xtask golden`
   and `cargo xtask ring-cross` stay green; the permissive-closure proof re-runs
   clean.
4. No file outside `crates/rustamp-core/**` needs an edit to keep compiling.

## 6. Open questions

| ID | Question |
|---|---|
| Q1 | Should `rustamp-core` instead re-export the whole `rustamp_proto::command` module (`pub use rustamp_proto::command::*`)? Simpler and future-proof, but it makes core's surface track proto's automatically, including items core may not want to promise. The explicit list with the test is the conservative choice; this RFC proposes that. |
