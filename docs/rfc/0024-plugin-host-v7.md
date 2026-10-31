# RFC 0024: the plug-in host's seventh protocol, and which DSP module runs

Status: **Draft** (2026-10-31). Written by the `lane/helper-plugins` lane for the
change it implements; the integration lead approves it. Owns the wire additions
`Message::EmbedSkin` and `Message::OutputClock`, the `PROTOCOL_VERSION` bump
6 → 7, and the `rustamp-core` field `PluginsConfig::dsp_module`.

Depends on: RFC 0003 (shared-memory rings, generation bumps), RFC 0004
(permissive crates and the GPL-free closure), RFC 0006 (out-of-process hosting;
the DSP helper's control channel), RFC 0020 (the last `Command` append run and
the version bump before this one).

## 1. The problem

Two gaps are one wire change, and bumping the version twice for them would be
two handshake breaks for one kind of change:

1. **The embed frame has no skin.** IPC 505 hands a plug-in a frame window it
   embeds its own window in. The frame is drawn from the skin's `gen.bmp`, but
   the process that makes it — the 32-bit `rustamp-plughost32` — must not link
   the GPL skin decoder (`rustamp-skin`), because it is one of the five
   permissive crates (RFC 0004). So the player has to hand the host the sheet's
   **pixels**, and the protocol has no message that carries a sheet.
2. **The player cannot see what an output plug-in has played.** An output
   plug-in that buffers (a device, a disk writer) plays later than the blocks it
   was given. The engine's clock is the device's clock, so without the plug-in's
   own `getOutputTime` the player ends a track while the plug-in still holds its
   end. The host reads that clock already, and the protocol has no message that
   carries it back.

Alongside them, one `rustamp-core` contract grows a field: a classic DSP library
may hold **several modules** (dsp_centercut's source has), and which one runs is
a user setting the player has nowhere to keep.

## 2. Evidence — what exists in the tree today

| Fact | Where |
|---|---|
| `PROTOCOL_VERSION` is **6**; `Message`'s last variant is `NowPlaying` at index **28 (0x1c)** | `crates/rustamp-ipc/src/message.rs:41-46`, `:275` |
| `Message::hello()` stamps `PROTOCOL_VERSION`; the supervisor rejects a `Hello` whose version differs and reaps the child | `crates/rustamp-ipc/src/message.rs:305-310`, `crates/rustamp-app/src/supervise.rs` |
| The host's control router turns `ControlInbound` into what the UI thread does; a variant it does not know has no arm | `crates/rustamp-plughost/src/control.rs` |
| `ResidentChain` is the helper's stage interface: `latency_frames`, `process`, `reset` | `crates/rustamp-mediahost/src/dsp.rs:60-84` |
| The DSP helper's pump owns the control channel and already polls it between blocks | `crates/rustamp-mediahost/src/dsp.rs:225-260` |
| The app holds a `HostStateLink` that publishes state, and a `publish_now_playing` beside it | `crates/rustamp-app/src/host.rs` |
| The skin crate decodes `gen.bmp` into an RGBA `RasterImage` (`Skin::sheet`, `RasterImage::as_rgba`) | `crates/rustamp-skin/src/skin.rs:341`, `crates/rustamp-skin/src/raster.rs:67` |
| `PluginsConfig` has `plugin_dir`, `vis_plugin`, `vis_preset`, `disabled` | `crates/rustamp-core/src/config.rs:207-221` |
| A classic DSP library reports its modules (`DspLibraryInfo`, each with a description), and a module is started by index | `crates/rustamp-plughost/src/loader.rs:956`, `:1410` |
| The stage description crosses the wire as `DspStageDesc { id, params }`; `params` is opaque to the transport (RFC 0006 §6) | `crates/rustamp-proto/src/host.rs`, RFC 0006 §6 |
| A frame keeps the kind it was created with; only a window created after the sheet is known can draw it | `crates/rustamp-plughost/src/embed_frame.rs` |
| webamp's `gen.bmp` rectangles are MIT and are already recorded in `THIRD_PARTY.md` | `THIRD_PARTY.md`, `crates/rustamp-plughost/src/embed_frame.rs` |

## 3. Decision

### 3.1 `PROTOCOL_VERSION` becomes 7

Two variants are **appended** to `Message`, after `NowPlaying`, per RFC 0005's
append rule (postcard encodes an enum by its declaration index, so appending
leaves every existing variant's bytes unchanged):

```rust
/// M: the skin's `gen.bmp` as pixels, for the embed frames plug-ins ask the
/// host for. The player decodes skins, the host only draws (RFC 0024). Empty
/// pixels: the skin has no such sheet, and the frames are plain windows.
EmbedSkin {
    width: u32,
    height: u32,
    /// 8-bit RGBA, top row first, `width * height * 4` bytes.
    rgba: Vec<u8>,
},
/// H: what an output plug-in has played so far, by its own clock: the
/// player's clock for a device that buffers (RFC 0024). Never falls.
OutputClock { played_ms: u64 },
```

Wire indices: **29 (0x1d)** and **30 (0x1e)**.

The version bumps although the additions are additive, for the same reason as
RFC 0003 §9 and RFC 0020 §3: a v6 peer would mis-decode them, and the project
rule is a clean `Hello` rejection rather than a decode error. A v6 host is
refused by the app's supervisor and a v7 host by a v6 app, on both sides.

### 3.2 The sheet crosses as pixels, not as a file

`EmbedSkin` carries the decoded sheet, not a path. The reasons are the ones
that keep RFC 0004's closure intact:

- The host must not link `rustamp-skin` (GPL) or any `.wsz` decoder. Pixels are
  a plain byte vector; no crate follows them.
- The player already decodes the skin for its own windows, so the pixels exist
  there and the decode is not repeated in a second process.
- A skin is a handful of 194 × 109 sheets at most; the message is sent **once
  per skin generation**, not per frame. `HostStateLink` keeps the generation it
  last sent and builds the pixels only when the generation is new
  (`publish_embed_skin`), and it forgets that on attach and on detach, so a
  reconnected host gets the current sheet.

`width = height = 0` and an empty `rgba` mean "this skin has no `gen.bmp`": the
host draws a plain system window, which is what the audio helpers (they are
sent no sheet at all) and a skin without the sheet get. A sheet whose length
does not match `width * height * 4` is refused by the host and treated the same
way; the frame keeps the kind it was created with, and a skinned frame takes a
new sheet at once (`set_skin` invalidates every frame of the thread).

### 3.3 `OutputClock` carries the plug-in's own clock

`ResidentChain` grows two defaulted methods (RFC 0006's interface, additive):

```rust
fn note(&mut self, _message: &Message) {}
fn played_ms(&mut self) -> Option<u64> { None }
```

`note` is how the player's messages reach the chain **in order with the
blocks** (the track, a pause, a configure request); `played_ms` is the chain's
clock. The helper's pump asks `played_ms` between blocks and while it waits for
input, and sends `Message::OutputClock` whenever the answer changed. `None`
means "no clock" (a DSP chain that is not an output), and nothing is sent then.

On the player's side `ControlPeer` keeps the last report in an `AtomicU64`
(`OutputClock`, with `u64::MAX` as "no report yet") so a reader on another
thread does not touch the control channel. `DspProcess::output_clock()` and
`DspProcess::control_sender()` hand both to the owner before the process moves
into its chain, which is why they exist on `DspProcess` and not only on
`DspSession`.

The player does not trust the number blindly: `PlayClock` bounds it by the
blocks it handed over and by a 10 s slack floor, and never lets it run back. A
plug-in without a clock, or one that stands still, therefore falls back on the
blocks, exactly as before this RFC.

`Message::CommandToHost(Command::PausePlugin/UnpausePlugin)` now also reaches an
**output** chain through the same `note`, which calls the plug-in's own
`pause`/`unpause`. RFC 0020 defined those commands for input plug-ins; this RFC
extends the meaning to the output plug-in of the audio path, with `plugin_id`
unused there (one plug-in per helper).

### 3.4 `PluginsConfig::dsp_module`

`rustamp-core`'s `PluginsConfig` gains one field, last in the struct so the
positional TOML and the serde default keep working:

```rust
/// Which module of the active DSP library runs (S09 section 6.3).
pub dsp_module: u32,
```

- `0` is the first module and the default, so an existing configuration keeps
  its behaviour and an unknown value never leaves the player without a stage.
- The app persists it as `plugins.dsp_module`, only when it is not `0`, and
  reads it in the same audit table as `plugins.plugin_dir` and
  `plugins.disabled` (`crates/rustamp-app/src/config.rs`, "The audit").
- The value is **per library**, not per file: the setting outlives a library
  that no longer has that module, and the helper then runs the first and traces
  it (`start_module`).

The module index crosses to the helper inside the stage's opaque `params`
(third line, after the DLL and the approval store), not as a new wire field:
RFC 0006 §6 makes `params` the stage's own serialization, and a new field would
be a second version bump for a value only the classic-DSP resident reads.

Two short-lived modes of `rustamp-plughost32` serve the setting while nothing
plays: `--dsp-modules <dll> --approvals <store>` prints one module name per
line (the player's `App::dsp_module_names`), and `--dsp-configure <dll>
--approvals <store> --module <n>` shows that module's dialog in a process of its
own and quits it, which is when many plug-ins write what the dialog changed
(`App::configure_dsp`). While a track plays, the dialog runs in the helper that
processes the audio instead (`DspChain::configure_running`), because a dialog
changes the instance it belongs to.

## 4. Backwards compatibility

- A v6 host and a v7 app refuse each other at `Hello`; neither mis-decodes.
- Every existing `Message` keeps its index and its bytes. The only changed
  bytes are `Hello.protocol_version`.
- `PluginsConfig` gains a field with a default; an old `rustamp.toml` loads
  unchanged, and the new key is written only when it is set.
- `ResidentChain`'s two methods are defaulted, so every existing chain (the
  test residents included) compiles and behaves as before.
- `DspStageDesc.params` is the stage's own serialization (RFC 0006 §6), so the
  classic-DSP resident's payload grew from two lines to three: DLL, approval
  store, module index. A two-line payload is refused loudly
  (`parse_stage` returns `None` → `OperationFailed("undecodable classic DSP
  stage")`), never read as "module 0". Both ends ship in the same binary, so no
  mixed pair exists in the field.

## 5. What this RFC does not decide

- **Whether the sheet is the right skin source.** The frame draws `gen.bmp`
  only. The title's letters come from the same sheet, as webamp does; no other
  sheet is read.
- **Embed flags.** `EMBED_FLAG_NO_CHILD_SIZING` and the scaleable flag are still
  ignored (they would need a spec). The frame fits its child on `WM_SIZE`,
  `WM_PARENTNOTIFY(WM_CREATE)` and `WM_SHOWWINDOW`.
- **The skin-change IPC event** to plug-ins, and `IPC_GET_EMBEDIF_NEW_HWND`.
- **`getOutputTime`'s exact semantics per plug-in.** The host reports the
  plug-in's answer as it is; the bound and the slack floor are the player's
  policy (`PlayClock`), not the contract's.
- **A drain on device close.** A natural stop relies on the clock; nothing
  flushes the plug-in's buffer when the device closes.
- **Where the module list is shown.** `App::dsp_module`, `App::set_dsp_module`
  and `App::dsp_module_names` are the seam; the preferences page that calls
  them is another lane's.

## 6. Acceptance

1. `PROTOCOL_VERSION` is 7, `Message::hello()` stamps it, and the wire tests
   assert both (`command_wire.rs`, `hosting_wire.rs`).
2. `EmbedSkin` and `OutputClock` round-trip, are appended at 0x1d/0x1e, and are
   in the property test's variant table (`tests/support/wire_gen.rs`,
   `MESSAGE_VARIANT_COUNT = 31`).
3. `HostStateLink::publish_embed_skin` builds the sheet once per generation and
   sends it once; a headless test asserts both (`host.rs`).
4. `PlayClock` has headless tests for the clock, the missing clock, the
   everything-at-once plug-in, the standing clock and a flush
   (`plugin_output.rs`).
5. `ResidentChain::note` and `played_ms` reach the running chain and the
   player's `OutputClock` without disturbing the block path
   (`dsp_control.rs`).
6. `PluginsConfig::dsp_module` round-trips through the config, is in the audit
   table, and is written only when set.
7. `cargo fmt --check`, `cargo clippy --workspace --all-targets --all-features
   -- -D warnings`, `cargo check --workspace --all-targets --all-features` and
   the workspace tests are green; the i686 host check is green.
8. What only the VM can show (a skinned frame with `vis_avs`, a DSP dialog in
   the running helper, the output clock under `out_wave`/`out_disk`, `vis_nsfs`
   loading) is covered by `#[ignore]`d tests and is listed in the lane's report.
