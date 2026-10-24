# RFC 0020: the missing player → host plug-in commands

Status: **Accepted** (owner decision, 2026-10-20; second revision dated
2026-10-20, first draft 2026-10-19). Implemented by the integration lane; see
§11 for the acceptance checklist and §12 for the owner's decision.

**What the second revision adds.** The first draft proposed only
`Command::ConfigurePlugin`. Since then the host grew four more registry routes
that also name a plug-in and also have no wire request: `seek_input`,
`pause_input`, `unpause_input` and `stop_input` (`wave238`, `wave241`). They are
the same shape — a player→host request with a plug-in id — so they belong in
the **same** append run and behind the **same** version bump. Bumping 4 → 5 for
one variant and 5 → 6 next week for four would be two handshake breaks for one
kind of change.

## 1. The problem

The player can configure **one** kind of plug-in: the visualization plug-in.
`Command::ConfigureVisPlugin` asks "the *running* vis plug-in: show your
configuration". Every other kind — input, output, DSP, general purpose — has
no request at all, so a plug-in of those kinds can be loaded and run but never
configured from the player.

The gap is **only** the wire. The host side is ready (wave185):
`PluginRegistry::configure_plugin(plugin_id, kind)` opens the configuration
dialog of a *named* plug-in on the UI thread, and `configure_vis()` does the
same for the running vis plug-in. The host's own control router says so
outright (`crates/rustamp-plughost/src/control.rs:77-81`):

> The other plug-in kinds have **no** Configure request: naming a plug-in needs
> a payload the protocol does not carry, and a new `Command` variant is a wire
> change that needs an RFC (P3 §4 step 9). None is invented here; the
> host-side method that would serve it is `PluginRegistry::configure_plugin`.

So the request needs a payload that names a plug-in, the protocol needs a
variant that carries it, and the protocol version has to move so a peer that
does not know the variant is refused at the handshake instead of mis-decoding
it. That is what this RFC proposes.

### 1.1 The same gap on the transport routes

`wave238` and `wave241` added four registry routes that name a plug-in and act
on it: `PluginRegistry::seek_input(plugin_id, to_ms)`,
`pause_input(plugin_id)`, `unpause_input(plugin_id)` and
`stop_input(plugin_id)`. They are in-process only, exactly like
`configure_plugin` was before this RFC: the host's control router still accepts
only `Load` and `ConfigureVis`, so nothing outside the host can reach them.

Their semantics are settled by S09 and pinned by tests, and this RFC does **not**
re-open any of it:

| Route | S09 | What the host does |
|---|---|---|
| `seek_input(id, ms)` | §3.1 offset `0x54`, §3.6 | calls the input plug-in's `seek(ms)`; the **plug-in** flushes its output (`§3.7`, `§4.2` step 3) — the host must not flush |
| `pause_input(id)` / `unpause_input(id)` | §3.1 offsets `0x3c`/`0x40`, §3.6 | calls the input's `pause`/`unpause`; the plug-in passes it to its output |
| `stop_input(id)` | §3.5 step 6 | calls the input's `stop()`; the plug-in closes its own output |

So the wire shape is the same for all five requests: a plug-in id plus, where
the request has one, a small payload. That is why they share one append run.

## 2. Evidence — what exists on `main` today

All of the following is read from the tree, not from a measurement.

| Fact | Where |
|---|---|
| `Command::ConfigureVisPlugin` is the only configure request on the wire; its doc says "Ask the loaded visualization plug-in to show its configuration. Id 40221." | `crates/rustamp-proto/src/command.rs:262-265` |
| `Command::ToggleVisPlugin` is the other vis-only request | `crates/rustamp-proto/src/command.rs:266-268` |
| The host acts on exactly one `CommandToHost` payload: `Message::CommandToHost(Command::ConfigureVisPlugin)` → `ControlInbound::ConfigureVis`. Every other `CommandToHost` payload falls into the catch-all arm and is ignored. | `crates/rustamp-plughost/src/control.rs:189-195` |
| `PluginRegistry::configure_plugin(plugin_id: u32, kind: PluginKind) -> Result<(), RegistryConfigError>` — opens the dialog of the plug-in with that id, on the UI thread, and compares the requested kind with the kind recorded at load time | `crates/rustamp-plughost/src/registry.rs:269-299` |
| `PluginRegistry::configure_vis()` — the same for "the vis plug-in that is running" | `crates/rustamp-plughost/src/registry.rs:301-315` |
| The refusals are already enumerated: `UnknownPlugin(u32)`, `WrongKind { expected, actual }`, `NoRunningVis`, `Plugin(PluginConfigError)` | `crates/rustamp-plughost/src/registry.rs:81-100` |
| A refusal is reported on **stderr**, because stdout is the frame stream | `crates/rustamp-plughost/src/main.rs:349-361` |
| `Message::CommandToHost(Command)` exists and is the accepted carrier for player→host commands | `crates/rustamp-ipc/src/message.rs:172-181` |
| `PROTOCOL_VERSION = 4` | `crates/rustamp-ipc/src/message.rs:35` |
| The last `Command` variant is `RemoveDeadFiles` at index **60 (0x3c)**; the next free index is 61 (0x3d) | `crates/rustamp-ipc/tests/command_wire.rs:323-331` |
| `Message::CommandToHost` is `Message` index **14 (0x0e)** | `crates/rustamp-ipc/tests/command_wire.rs:388-390` |
| The app's version gate: the supervisor's handshake barrier rejects a `Hello` whose `protocol_version != PROTOCOL_VERSION` (`record_failure()`, the child is reaped), and `HostGreeting::is_supported()` detaches the link for the same reason | `crates/rustamp-app/src/supervise.rs:779-797`, `crates/rustamp-app/src/host.rs:226-234` |
| After `DEFAULT_MAX_CONSECUTIVE_FAILURES` (3) rejected handshakes the supervisor enters `PersistentFailure` and stops replacing the host | `crates/rustamp-app/src/supervise.rs:527-528`, `:684-687` |
| The app never sends a `CommandToHost`: `HostStateLink` publishes state only, its `send` is private | `crates/rustamp-app/src/host.rs:486-530` |
| The app's handling of the vis request is still a note: `C::ConfigureVisPlugin \| C::ToggleVisPlugin => self.note(format!("{command:?} needs the plugin host (M3)"))` | `crates/rustamp-app/src/app.rs:1938-1940` |
| `PluginKind` lives in `rustamp-plugin-abi` and is **already on the wire twice**, in `Message::LoadPlugin` and `Message::PluginLoaded` | `crates/rustamp-plugin-abi/src/lib.rs:29-42`, `crates/rustamp-ipc/src/message.rs:96-117` |
| `rustamp-proto` has **no internal dependencies** today (only `serde`); `rustamp-ipc` depends on `rustamp-plugin-abi` | `crates/rustamp-proto/Cargo.toml`, `crates/rustamp-ipc/Cargo.toml`; RFC 0004 §3 |
| The public id 40221 is the **visualization** menu's "Configure plug-in..." entry (S03: "Configure visualization plug-in"); there is no public id for a generic configure request | `crates/rustamp-ui/src/menu.rs:1105-1111`, `crates/rustamp-ui/src/shortcuts.rs:333`, `specs/S03-shortcuts.md:82`, `specs/S04-menus.md:308` |
| The host's id table maps 40221 → `Command::ConfigureVisPlugin` | `crates/rustamp-plughost/src/commands.rs:121` |
| The card that identified the gap: "No wire-level 'configure plugin N'. The protocol has no such message (S10/RFC 0006), so routing a user-equivalent Configure request from the app needs an RFC and the contract lane." | `docs/tasks/P3-dll-acceptance.md:304-307` |

## 3. Decision — the proposal

### 3.1 One appended `Command` variant

A new variant at the **end** of `Command`, after `RemoveDeadFiles`, per the
RFC 0005 §3 append rule (postcard encodes an enum by its declaration index, so
appending keeps every existing variant's bytes unchanged):

```rust
/// Ask the host to open the configuration dialog of the plug-in with this
/// id. The id is the one the host assigned in its `PluginLoaded` answer and
/// is valid only for that host process's lifetime. `kind` is the kind the
/// request claims; the host compares it with the kind it recorded when the
/// plug-in was loaded and refuses a mismatch. RFC 0020.
ConfigurePlugin {
    /// Host-assigned plug-in id.
    plugin_id: u32,
    /// The kind the request claims the plug-in has.
    kind: PluginKind,
},
```

Wire index: **61 (0x3d)**. Every existing variant keeps its index.

`ConfigureVisPlugin` **stays**. The two are complementary, not overlapping:
`ConfigureVisPlugin` means "the vis plug-in that is running", which names no
plug-in and needs no id; `ConfigurePlugin` names one. The vis plug-in can
also be configured through the new variant, by its id.

The new variant has **no public `WM_COMMAND` id** — S03 records 40221 as
"Configure **visualization** plug-in" and S04 §5 as the *visualization* menu's
"Configure plug-in..." entry; no id exists for a generic request. So under
RFC 0005 §3's host-forward rule the new variant is player-internal: the
host's id → `Command` table needs no row for it, and the host never forwards
it to the player. It travels in one direction only: player → host.

### 3.1.1 The four transport variants, appended after it

Same rule, same direction, same carrier: four more variants at indices **62**
to **65**, after `ConfigurePlugin` at 61.

```rust
/// Ask the host to seek the input plug-in with this id. The plug-in flushes
/// its own output at the new time (S09 §3.7); the host must not. RFC 0020.
SeekPlugin {
    /// Host-assigned plug-in id (valid for this host process only).
    plugin_id: u32,
    /// Target position in milliseconds, passed through unchanged (S09 sets
    /// no range).
    to_ms: i32,
},

/// Ask the host to pause the input plug-in with this id (S09 §3.1 offset
/// 0x3c; the plug-in pauses its output). RFC 0020.
PausePlugin { plugin_id: u32 },

/// The counterpart of `PausePlugin` (offset 0x40). RFC 0020.
UnpausePlugin { plugin_id: u32 },

/// Ask the host to stop the input plug-in with this id; the plug-in closes
/// its own output (S09 §3.5 step 6). RFC 0020.
StopPlugin { plugin_id: u32 },
```

`to_ms` is `i32` because that is the ABI's `seek(int ms)` type; the value is
passed through unchanged. A refusal (unknown id, wrong kind, missing slot) is
reported exactly like `ConfigurePlugin`'s, on stderr — see §3.5.

**Why they are not one variant with an operation enum.** A single
`PluginRequest { plugin_id, op: PluginOp }` would be one variant instead of
five, but it would put a *second* enum on the wire and make every future
request grow that enum — a shape change of an existing variant, which RFC 0005
§3 rules out. Five appended variants cost four index bytes in the enum and
nothing else, and each keeps its own doc comment and its own test.

### 3.2 The carrier

`Message::CommandToHost(Command)` — the existing carrier, unchanged. **No new
`Message` variant is proposed.** That is the reason the wire change is exactly
one `Command` variant and one version number.

### 3.3 The kind type — the one real design decision

The payload needs a kind. The kind type that the protocol already uses is
`rustamp_plugin_abi::PluginKind` (it is on the wire in `LoadPlugin` and
`PluginLoaded`), but `Command` lives in `rustamp-proto`, which RFC 0004 §3
declares to have **no internal dependencies**.

Three ways out:

| Option | What it costs |
|---|---|
| **A. `rustamp-proto` depends on `rustamp-plugin-abi`** and the payload uses `rustamp_plugin_abi::PluginKind` | One new (internal, permissive → permissive) dependency edge. Requires an erratum to RFC 0004 §3's "no internal dependencies" sentence. One kind type on the wire, exactly as today. |
| **B. A `PluginKind` mirror inside `rustamp-proto`** with a conversion at the boundary | Two definitions of a five-value enum, and therefore a second postcard encoding of the same concept. Drift risk of the kind RFC 0005 §13 already flags for the id tables. |
| **C. Move `PluginKind` into `rustamp-proto`**, re-export it from `rustamp-plugin-abi` | Inverts RFC 0004 §3's direction and makes the plugin-author-facing ABI crate depend on the protocol crate. `PluginKind` carries ABI facts (`export_name()`, `expected_version()`, `from_path()`) that belong to the ABI. |

**This RFC recommends A.** Both crates are `MIT OR Apache-2.0` (RFC 0004), so
the invariant that actually matters — no copyleft in the permissive closure
(RFC 0004 acceptance 4) — keeps holding; the license audit discovers the
roots and their closures automatically, so `rustamp-plugin-abi` simply joins
`rustamp-proto`'s closure. The cost is one sentence of erratum text, and the
gain is that the wire carries **one** kind type, as it already does twice.

If the owner prefers not to touch RFC 0004 §3, option **D** — drop the kind
from the payload and send `ConfigurePlugin { plugin_id: u32 }` only — is the
fallback: it is the smallest wire change and needs no dependency at all, at
the price of the request no longer stating which kind of plug-in it means.
The host's `WrongKind` check then becomes vacuous (the host would call
`configure_plugin` with the kind it recorded itself), and the frame loses its
self-describing property. D is discussed in §6.

### 3.4 The host side (not a wire change)

The host's control router learns five more `CommandToHost` payloads, each
routed to the UI thread exactly like `ConfigureVis` is today (only the UI
thread may call into a plug-in):

| Payload | Host-local `ControlInbound` | Called |
|---|---|---|
| `ConfigurePlugin { plugin_id, kind }` | `ConfigurePlugin { plugin_id, kind }` | `registry.configure_plugin(plugin_id, kind)` |
| `SeekPlugin { plugin_id, to_ms }` | `SeekPlugin { plugin_id, to_ms }` | `registry.seek_input(plugin_id, to_ms)` |
| `PausePlugin { plugin_id }` | `PausePlugin { plugin_id }` | `registry.pause_input(plugin_id)` |
| `UnpausePlugin { plugin_id }` | `UnpausePlugin { plugin_id }` | `registry.unpause_input(plugin_id)` |
| `StopPlugin { plugin_id }` | `StopPlugin { plugin_id }` | `registry.stop_input(plugin_id)` |

All five routes already exist and are pinned headless (`wave185`, `wave238`,
`wave241`); this RFC adds **no** host behaviour, only the inbound routing. A
refusal — unknown id, wrong kind, no such slot, plug-in already dead — is
reported exactly as `configure_vis` reports it today: on **stderr**, because
stdout is the frame stream. Nothing is called through a null slot (S09 §0).

### 3.5 The app side (not a wire change)

The app must

1. remember the plug-ins the host reported — the `plugin_id`, the `name` and
   the `kind` from every `Message::PluginLoaded` — and
2. have a way to send a command to the host: today `HostStateLink` publishes
   state only and its `send` is private, so a public "send this command to
   the host" path has to be added.

Both are `rustamp-app` work. **Neither changes the wire**, so neither needs
the version bump; the version bump is needed for the `Command` variant alone.

### 3.6 What this RFC does not do

- It does not add a UI. Which menu entry, preference page or button offers
  "configure this plug-in" is a separate decision (and, for the
  preferences pages, a separate card).
- It does not add a reply. Q3 in §9 treats the reply as an open question;
  the proposal is fire-and-forget.
- It does not change `ConfigureVisPlugin`, `ToggleVisPlugin`, the framing,
  the ring layout, `PlayerState` or the `Event` set.
- It does not implement anything. This is a proposal.

## 4. The contract — what changes once this is accepted

| Item | Change |
|---|---|
| `rustamp-proto::command::Command` | five variants appended: `ConfigurePlugin { plugin_id, kind }` at 61 (0x3d), `SeekPlugin { plugin_id, to_ms }` at 62 (0x3e), `PausePlugin { plugin_id }` at 63 (0x3f), `UnpausePlugin { plugin_id }` at 64 (0x40), `StopPlugin { plugin_id }` at 65 (0x41) |
| `rustamp-proto`'s manifest | one internal dependency on `rustamp-plugin-abi` (option A) — plus the RFC 0004 §3 erratum |
| `rustamp-ipc::PROTOCOL_VERSION` | 4 → 5, with the version history extended |
| `rustamp-ipc::Message` | **unchanged** — `CommandToHost` already carries all five |
| `rustamp-plughost::control::ControlInbound` | five new host-local variants (the table in §3.4) |
| `rustamp-plughost::control::handle_message_inbound` | five more `CommandToHost` payloads routed to it; every other payload keeps falling into the catch-all arm |
| `rustamp-app` | remember the loaded plug-ins; add a public send path to the host; map the user's choice to the new commands |
| `rustamp-core` | re-export whatever `rustamp-proto` adds, at the existing paths (RFC 0004 §3) — no caller changes |
| `rustamp-ui` | nothing in this RFC (the entry point is a separate decision) |

Consumers of the `Command` enum that match exhaustively need a new arm or a
catch-all: `rustamp-app/src/routing.rs` (the app's own set), the engine's
applier (its catch-all covers it) and the UI machine's prediction table (no
S02-visible effect → no prediction). RFC 0005 §14 already named
`#[non_exhaustive]` as a lane decision; it is still not made.

## 5. The wire

### 5.1 The bytes

Hand-derived from the definition and the postcard rules the existing wire
test documents (variant index as the first byte, `u32` as a LEB128 varint,
a fieldless enum as its declaration index). **Not measured** — the
implementation lane's test is the measurement.

| Value | Bytes |
|---|---|
| `Command::ConfigurePlugin { plugin_id: 7, kind: PluginKind::Visualization }` | `[0x3d, 0x07, 0x02]` |
| `Message::CommandToHost(Command::ConfigurePlugin { plugin_id: 7, kind: PluginKind::Visualization })` | `[0x0e, 0x3d, 0x07, 0x02]` |
| `Command::SeekPlugin { plugin_id: 7, to_ms: 12345 }` | `[0x3e, 0x07, 0xf2, 0xc0, 0x01]` |
| `Command::PausePlugin { plugin_id: 7 }` | `[0x3f, 0x07]` |
| `Command::UnpausePlugin { plugin_id: 7 }` | `[0x40, 0x07]` |
| `Command::StopPlugin { plugin_id: 7 }` | `[0x41, 0x07]` |
| `Message::CommandToHost(Command::StopPlugin { plugin_id: 7 })` | `[0x0e, 0x41, 0x07]` |

`0x3d` = 61 … `0x41` = 65, the appended declaration indices; `0x07` =
`plugin_id` 7 as a one-byte varint; `0x02` = `PluginKind::Visualization`
(declaration order: Input 0, Output 1, Visualization 2, Dsp 3, General 4);
`0xf2 0xc0 0x01` = postcard's zig-zag varint for `12345` (`24690` as LEB128).
Every earlier variant keeps its bytes, because the five are appended.

### 5.2 The version

`PROTOCOL_VERSION` 4 → 5. The reason is the one RFC 0005 §12 and RFC 0015
give: postcard encodes an enum by declaration index, so a v4 peer that
receives the new variant cannot decode it, and the project's rule is a clean
handshake rejection instead of a decode error. The version history in
`crates/rustamp-ipc/src/message.rs` gains a fifth entry:

> * `5`: adds the RFC 0020 `Command::ConfigurePlugin` variant.

`Message` itself does not change, so — unlike RFC 0015's `DspResetAck`, which
moved `DspError`'s index — no other message's bytes move.

### 5.3 What a v4 peer does

**A v5 player against a v4 host.** The host's `Hello` carries version 4; the
supervisor's handshake barrier compares it with `PROTOCOL_VERSION` and, on a
mismatch, records a failure and reaps the child
(`crates/rustamp-app/src/supervise.rs:779-797`). The link stays detached, so
the player never sends the new command; after three rejected handshakes the
supervisor enters `PersistentFailure` and stops replacing the host. The
failure is **loud and early**: no frame is ever mis-decoded.

**A v4 host against a v5 player.** The player is the side that enforces the
gate, so this is the same case seen from the other end: the player's link
detaches on the host's greeting and the host is replaced. Note that the host
itself does **not** compare the version in the player's `Hello`
(`crates/rustamp-plughost/src/control.rs:193` ignores `Hello`; the doc
comment above it overstates the check). The gate is therefore one-sided by
construction, and it is the player side that holds it.

**If a v5-only command ever reached a v4 host anyway**, the host's reader
thread would fail to decode the frame; a decode failure ends the reader
thread (`ControlExit::ChannelError`, exit code 1), the supervisor sees the
host die and replaces it, and the handshake fails again. This is the failure
mode the version bump exists to prevent, and it is why the bump is not
optional.

Player and host ship together from one build, so a mixed pair is a build
mistake, not a runtime condition.

## 6. Alternatives considered

**A1. Reuse `ConfigureVisPlugin` with a widened payload** (make it
`ConfigureVisPlugin { plugin_id: Option<u32>, kind: Option<PluginKind> }`).
**Rejected.** `ConfigureVisPlugin` already has a wire index (54 / 0x36) and a
public id (40221) and a place in the host's id table
(`crates/rustamp-plughost/src/commands.rs:121`). Changing its *shape* moves
the bytes of an existing variant: every peer that already decodes 0x36 as
the unit variant would mis-decode it. The append rule exists exactly to
prevent that, and RFC 0005 §3 makes it a rule, not a preference. The variant
would also have to keep meaning two different things ("the running vis
plug-in" and "this named plug-in"), and `Option` fields in a command that
the host forwards by id table are a poor fit for a command whose whole point
is to name one thing.

**A2. A separate `Message` variant** (e.g.
`Message::ConfigurePlugin { plugin_id, kind }`) instead of a `Command`
variant inside the existing `CommandToHost` carrier. **Rejected.** It would
add a `Message` variant — more wire surface, another index, another
`Message`-level compatibility question — for a request that is *already* a
command with a public route from the player to the host. RFC 0005 §12
proposed `Message::CommandToHost` precisely so that player→host requests
travel as commands, and the host's router already recognises that carrier.
The only reason to add a `Message` variant would be to add a **reply**, and
that is an open question (Q3 in §9), not a reason to move the request.

**A3. A `PluginKind` mirror in `rustamp-proto`** (option B in §3.3).
**Rejected** for the drift reason above: the wire would carry two encodings
of one concept, and the project has already been bitten by duplicated tables
(the UI's and the host's id tables, RFC 0005 §13).

**A4. No kind in the payload** (option D in §3.3). **Not rejected, but not
recommended.** It is the smallest possible wire change and needs no
dependency and no erratum. What it gives up: the request no longer says
which kind of plug-in it means, so the host's `WrongKind` refusal becomes
unreachable, and the frame stops being self-describing. The owner may well
prefer the smaller change; §10 lists the decision.

**A5. Reuse the existing `ToggleVisPlugin`/`ConfigureVisPlugin` ids for the
other kinds** (i.e. let 40221 mean "configure whatever plug-in is selected").
**Rejected.** The public ids are interoperability facts (S03), and 40221 is
recorded as the *visualization* menu's "Configure plug-in..." entry. Changing
its meaning would break the id table's contract with plugins and external
tools.

## 7. Licensing

- **No new dependency is added.** Option A adds an *internal* dependency
  (`rustamp-proto` → `rustamp-plugin-abi`); both crates are
  `MIT OR Apache-2.0` (RFC 0004), so the permissive closure stays
  copyleft-free and RFC 0004 acceptance 4 keeps holding. No crates.io crate
  is added, so `THIRD_PARTY.md` gains no new entry.
- The wire types stay in the permissive crates: the new `Command` variant in
  `rustamp-proto`, the version constant in `rustamp-ipc`. No GPL crate
  carries a wire type (RFC 0005 §13).
- The RFC 0004 §3 erratum is **text only**; it changes no license and no
  license file. The permissive set stays five crates
  (`rustamp-proto`, `rustamp-plugin-abi`, `rustamp-ipc`, `rustamp-plughost`,
  `rustamp-mediahost`).
- This RFC contains no code copied from any source, dirty or free; it
  describes the tree's own behaviour.

## 8. Test plan

### 8.1 Headless (the host; no window, no device, no spawned process)

| Test | Proves |
|---|---|
| `rustamp-ipc/tests/command_wire.rs`: a new test pinning the appended index (61 / 0x3d) and the hand-derived bytes, in the style of `remove_dead_files_is_appended_after_save_playlist` | the append rule holds: `RemoveDeadFiles` stays 0x3c and the new variant takes 0x3d |
| the same file: the new variant round-trips (`decode(encode(x)) == x`) for every `PluginKind` | the payload survives the round trip |
| the same file: `the_protocol_version_is_four` becomes `..._is_five` and the `Hello` construction is updated | the version moved with the variant |
| the same file: the pre-0005 vectors and every later append test stay green | no earlier byte string moved |
| `rustamp-app`: a `MemoryTransport` pair, a v4 host (the constant minus one) and the supervisor's handshake barrier | a v4 peer is **rejected**: the link stays detached, the failure is recorded, and no frame is written to the rejected peer |
| `rustamp-app`: the app's send path writes `Message::CommandToHost(Command::ConfigurePlugin { .. })` onto the transport | the app uses the accepted carrier, with the right payload |
| `rustamp-plughost`: `handle_message_inbound` maps `CommandToHost(ConfigurePlugin { .. })` to the new `ControlInbound` | the host routes the request to the UI thread; every other `CommandToHost` payload still falls into the catch-all arm |
| `rustamp-plughost`: the same for `SeekPlugin`, `PausePlugin`, `UnpausePlugin`, `StopPlugin` | the four transport requests reach the UI thread, and each calls its own registry route with the id from the frame |
| `rustamp-plughost`: a seek through the wire produces exactly one `flush`, and it comes from the plug-in (the `wave238` doubles) | the host still does not flush an output it does not own |
| `rustamp-plughost`: a wrong kind, an unknown id and a dead plug-in are refused for each route | nothing is called through a null slot, and the refusal is typed |
| `rustamp-plughost`: the registry refuses an unknown id, a wrong kind and a plug-in with no `config` slot (the `RegistryConfigError` cases) | nothing is called through a null slot (the existing in-process registry tests cover the `config` slot; the new one covers the id/kind routing) |

The headless tests are the ones that must run on the host; "the tests need
the VM" is never a reason to skip them.

### 8.2 VM-only acceptance (one run, in the interactive session)

One end-to-end case: a **real** plug-in's `config` runs because the player
asked for it. The `gen_` fixture already implements the Config window
(`fixture_config`, P3 §9.1 step 9), so the acceptance is:

1. the app loads the fixture through the production supervisor and learns its
   `plugin_id` from `Message::PluginLoaded`;
2. the app sends `Command::ConfigurePlugin { plugin_id, kind: General }`;
3. the host's UI thread calls the plug-in's `config`, the plug-in's Config
   window opens **owned by the host window**, and the guest closes it;
4. the host keeps pumping messages, the plug-in's trace agrees with the
   host's trace, and the host quits cleanly afterwards.

This is the only acceptance that needs a window, so it is `#[ignore]`d and
runs in the Hyper-V test VM's interactive session — never on the host, never
in session 0.

## 9. Open questions

| ID | Question |
|---|---|
| Q1 | **Does 2.95 configure a selected-but-not-running vis plug-in?** The wire does not need the answer: the new message names a *loaded* plug-in by a host-assigned id, and a plug-in that is selected but not loaded has no id, so the player cannot name it. The answer matters for the **UI** (is "Configure plug-in..." offered for a plug-in that is selected but not running?), which is a dirty-room question and out of scope here. |
| Q2 | **What does the player show when the plug-in is not loaded** (or the id is stale, or the kind is wrong)? With a fire-and-forget request the answer is "nothing": the host refuses, logs to stderr, and the user sees no dialog. A note in the player's status area would be friendlier but needs a reply path (Q3). |
| Q3 | **Should the reply be an event or fire-and-forget?** Fire-and-forget is the proposal: the host's `configure_vis` path already reports refusals on stderr, and the visible feedback of a *successful* configure is the dialog itself. A reply would need a new `Message` variant (and therefore its own wire change and version bump) and could only arrive *after* the modal dialog closed, so its value is limited to the failure cases. |
| Q4 | **Should the host also enforce the protocol version on the player's `Hello`?** The player enforces it (the supervisor's handshake barrier); the host ignores `Hello` entirely, although the doc comment above that arm claims otherwise. A one-sided gate is enough for a same-build pair, but the asymmetry is a trap for the next protocol change. **Follow-up, not this RFC.** |
| Q5 | **Should the new variant get a public `WM_COMMAND` id** so that a plug-in could ask the player to configure another plug-in? RFC 0005 §3's host-forward rule says a variant with a public id is forwarded by the host's table; S03/S04 record none for a generic request, and inventing one would be an interoperability claim this RFC cannot make. **Proposal: no id; the variant is player-internal.** |
| Q6 | **Should the plug-in id survive a host restart?** It cannot: the host hands out ids in load order starting at **one** (id 0 is never used, `crates/rustamp-plughost/src/registry.rs:149`, `:834`), so a restarted host re-uses the numbers for different plug-ins. The player must treat an id as valid only for the lifetime of the host process that issued it. Is that acceptable, or should the id be a random token? A random token is not a wire change of this shape (it stays a `u32`), but it changes the host's id assignment. |

## 10. What the owner must decide

1. **Accept the wire change at all** — one appended `Command` variant and
   `PROTOCOL_VERSION` 4 → 5. Without this, the plug-in kinds other than the
   vis one stay unconfigurable from the player, and the host's
   `configure_plugin` stays unreachable from outside the host process.
2. **Which kind type** (§3.3): **A** (recommended — `rustamp-proto` depends
   on `rustamp-plugin-abi`, plus an RFC 0004 §3 erratum), **B** (a mirror in
   `rustamp-proto`; drift), **D** (no kind in the payload; smallest wire, no
   dependency, no erratum, but the request stops being self-describing and the
   host's `WrongKind` refusal becomes unreachable).
3. **Reply or not** (Q3 in §9): fire-and-forget (proposed) or a new `Message`
   variant — the latter is a second wire change and a second bump.
4. **Whether the four transport variants ride along** (second revision,
   **proposed: yes**). The player and the host ship together, so a single bump
   is cheap now and expensive later; the project has bumped 1 → 2 → 3 → 4 for
   exactly this reason, and the four routes are already written and tested.
   Saying no means a second handshake break later for the same kind of change.
5. **Whether `to_ms` is `i32` or `u32`.** S09's slot is `seek(int ms)` —
   `i32` is the ABI's type and is passed through unchanged; S09 settles no
   range, so a negative value is the plug-in's business. Proposal: `i32`.

## 11. RFC acceptance (for the implementation lane, once this RFC is accepted)

- [x] The variant is appended after `RemoveDeadFiles` with a doc comment, and its
  index is pinned in a wire test (`command_wire.rs::the_plugin_requests_are_appended_after_remove_dead_files`, `hosting_wire.rs` total count = 66).
- [x] `Message` is unchanged; the request travels in `Message::CommandToHost` (verified by bytes `[0x0e, 0x3d, 0x07, 0x02]` and the carrier test).
- [x] `PROTOCOL_VERSION` is 5, the version history names RFC 0020, and the
  version-gate tests cover a peer one version below (`hello_barrier_rejects_a_peer_one_version_below` checks `PROTOCOL_VERSION - 1`).
- [x] The host routes the new payload to the UI thread and refuses an unknown id,
  a wrong kind and a missing `config` slot (`control.rs::the_named_plugin_requests_reach_the_ui_thread...`, `the_three_refusals_are_reported_and_call_nothing`, `an_unknown_id_is_refused_by_every_transport_route_and_calls_nothing`).
- [x] The app remembers the loaded plug-ins (id, name, kind) and sends the
  request through the accepted carrier (`app.rs::the_named_plugin_requests_reach_the_host_through_the_accepted_carrier`, `the_four_transport_requests_go_out_over_the_same_carrier`).
- [x] `rustamp-core` re-exports what `rustamp-proto` adds, at the existing paths (`command.rs` re-exports `PluginKind`; `command_reexports.rs` proves it names the ABI's own type).
- [x] The permissive closure stays copyleft-free; `THIRD_PARTY.md` is unchanged (the proto→plugin-abi edge is internal, both permissive).
- [x] `cargo fmt --all -- --check`,
  `cargo clippy --workspace --all-targets --all-features -- -D warnings` and
  `cargo test --workspace --all-features` are green; the window-shaped
  acceptance runs in the Hyper-V test VM, not on the host.

## 12. Owner decision (recorded 2026-10-20)

**Accepted.** The owner accepted the revision as proposed and chose **option A**
for the kind type (section 3.3): `rustamp-proto` takes one internal dependency
on `rustamp-plugin-abi` and reuses the ABI's kind type, with an **RFC 0004
section 3 erratum** recording that the proto crate names the ABI's type instead
of mirroring it. The answers to section 10 in full:

1. **The wire change is accepted**: one appended `Command` variant travelling in
   `Message::CommandToHost`, and `PROTOCOL_VERSION` 4 -> 5.
2. **Kind type: A** (as above).
3. **Fire-and-forget** (Q3): no reply message, no second wire change.
4. **The four transport variants ride along** (second revision): one bump for
   configure plus seek/pause/unpause/stop.
5. **`to_ms` is `i32`**: S09's slot is `seek(int ms)`, and a negative value is
   the plug-in's business.

The implementation lane works from section 11.
