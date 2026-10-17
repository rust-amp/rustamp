# Task card H3 — `rustamp-output` device-path robustness

**Background**
The audio-output crate (`rustamp-output`) is the only place that talks to real
devices (WASAPI, cpal) and the only place whose internal device model the test
suite can inspect without hardware (the null device). Five properties of that
path were unproven or wrong: (1) a failed `open` did not say which device it
tried nor which devices exist, (2) the render callback's real-time purity was
documented but only partially enforced, (3) a device that disappears or
changes format mid-run was spun on silently or reported opaquely, (4) the null
device diverged from the two real backends in ways that make its tests
misleading, and (5) the crate-level contract in `lib.rs` and the `OutputDevice`
trait docs contradicted the code. This card records the findings and the fixes.
Every change is inside `crates/rustamp-output/**`.

**Allowed sources**
`specs/**`, `docs/rfc/0001-core-contracts.md`, `crates/rustamp-core/src/audio.rs`
(the `OutputDevice` contract), the vendored `wasapi 0.26.0` and `cpal 0.18.2`
crate sources under `~/.cargo/registry`.
**Clean room:** never read Winamp source, SDK headers, disassembly, the 2.95
binary, `rustamp-research` or `rustamp-refbin`.

**Hard rules honoured:** no `cargo test` (VM-only); no host execution; no
window; no audio; no downloads; no `cargo add`; no new dependency; no git
writes; only `cargo fmt --all -- --check`, `cargo clippy --workspace
--all-targets --all-features -- -D warnings` and `cargo build --workspace` were
run. Every write is inside the worktree
`…\worktree\26de0e\wave41-output-robustness`; `git status --short` shows only
the four owned files plus this card.

---

## Findings (severity order)

### F1 — HIGH — `open` failures were not loud and did not name the device or its alternatives
- **Defect:**
  - `crates/rustamp-output/src/cpal_out.rs:96` (`find_output_device`, pre-change):
    a named device that matched nothing returned `cpal: no output device named
    like "X" (available: ["A", "B"])` — a Rust `Vec` `Debug` dump, not a
    human-readable list; an ambiguous name used a stray run of spaces and the
    same `{:?}` form.
  - `crates/rustamp-output/src/cpal_out.rs:184-185` (pre-change): a missing
    default device returned `cpal: no default output device` with no list of
    what does exist.
  - `crates/rustamp-output/src/wasapi_out.rs:267` (`open_endpoint`, pre-change):
    every failure returned a bare `e.to_string()` (or `COM init failed: {e}`)
    and the device name was sent as the literal `"wasapi"`; nothing named the
    endpoint that was tried and nothing listed the render endpoints present.
- **Fix:** new message builders that always name the target and list the
  alternatives:
  - `crates/rustamp-output/src/wasapi_out.rs:135` `open_failure_message`,
    `:148` `render_endpoint_names`, `:461` `open_endpoint` (now returns an
    `OpenError { code, device_name, message }`, `:127`) and `:177`
    `find_render_endpoint` for the new named-endpoint path.
  - `crates/rustamp-output/src/cpal_out.rs:109` `device_name`, `:118`
    `output_device_names`, `:127` `device_selection_message`, `:145`
    `no_default_device_message`, `:158` `find_output_device`, `:260` `open`.
- **Effect:** an absent, ambiguous or unusable device is a loud `Error::Output`
  that names the wanted device and lists the devices that exist; the backend
  never silently falls back to another device.

### F2 — HIGH — WASAPI spun on device loss / format change in silence
- **Defect:** `crates/rustamp-output/src/wasapi_out.rs:396` (pre-change) retried
  `client.start_stream()` every 5 ms forever on failure; `:441`
  (`write_available`) returned on a failing `get_available_space_in_frames()`;
  the `wait_for_event` timeout took `Err(_) => continue`. A removed endpoint or
  an invalidated client (format change) therefore produced *no* signal — the
  module doc even listed it as “Known limitation (M1)”. `close`
  (`:236`) could only report the earlier codes.
- **Fix:** a latched fatal error and a bounded failure streak:
  - `:75` `ERR_GONE`, `:81` `MAX_CONSECUTIVE_FAILURES = 64`, `:97`
    `FailureStreak`, `:120` `fail`, `:89` `error_text(ERR_GONE)`.
  - `:570` `render_loop` ticks the streak on `start_stream` failure, on a
    failed liveness probe after a `wait_for_event` timeout, and `:702`
    `write_available` ticks it on a failing buffer query; at the limit it
    latches `ERR_GONE` and the render thread exits. A success resets the streak.
  - `:323` `WasapiOutput::stream_error()` exposes the latch before `close`;
    `:426` `close` now names the device in its error.
- **Format change = loss:** `AUTOCONVERTPCM` fixes the client's format for its
  lifetime, so a renegotiated endpoint mix format invalidates the client and its
  calls start failing; that surfaces as the same latched loss, and recovery is a
  new `open`. Turning the latch into an `Event::Error` needs an RFC (engine
  work), because the `OutputDevice` trait has no mid-run error hook
  (`crates/rustamp-core/src/audio.rs:320-353`).

### F3 — HIGH — cpal `start` left the object in a franken-state; `open` ignored a live stream
- **Defect:** `crates/rustamp-output/src/cpal_out.rs:203-217` (pre-change) built
  and played the stream with `?`: on either failure it returned `Err` but had
  already `take()`n `device`/`config`, leaving `self.device == None`,
  `self.config == None`, `self.stream == None` between “opened” and “not
  opened”. Separately, `open` (`:179`) never checked `self.stream`, so a second
  `open` while a stream was live replaced the device/config while the old
  stream kept running — two configurations, one object.
- **Fix:** `:297` `start` restores `device`/`config` on both build and play
  failure (the object stays consistently “open”), and names the device; `:260`
  `open` refuses while `self.stream.is_some()` with
  `cpal: already started; close the device before reopening`.

### F4 — MEDIUM — null `chunk_frames` raced the scratch resize (audio-thread panic)
- **Defect:** `crates/rustamp-output/src/null.rs:42,69` (pre-change) stored
  `chunk_frames` as an `AtomicU64` *outside* the `Inner` mutex; `:241` (`drive`)
  loaded it, then locked and sliced `scratch[..samples]`. `:205`
  (`set_chunk_frames`) stored the new chunk before resizing `scratch` under the
  lock, so a pump could observe the new chunk and the old, smaller scratch and
  slice out of bounds — a panic on the audio path.
- **Fix:** `crates/rustamp-output/src/null.rs:65` moved `chunk_frames` into
  `Inner` under the same mutex as `scratch`; `:249` `set_chunk_frames` writes
  both under one lock, `:281` `drive` and `:185` `latency` read it under the
  lock. A pump now sees the chunk and the buffer as one consistent pair.

### F5 — MEDIUM — the null device contradicted the two real backends
- **Defect:** `crates/rustamp-output/src/null.rs:85` (pre-change) accepted
  `sample_rate == 0` / `channels == 0` (WASAPI rejects a zero rate and clamps a
  zero channel count — `wasapi_out.rs:154`); a second `open` (`:85`) or `start`
  (`:104`) was silently accepted or a no-op where the real backends error;
  `played` was never re-based in `open` although RFC 0001 defines
  `played_frames()` as “since start” (`docs/rfc/0001-core-contracts.md:38`) and
  both real backends re-base it in `open`; and `:260` advanced the clock
  *before* the callback, so a callback could observe its own buffer in the clock
  — WASAPI counts a buffer only after `write_to_device` succeeds and cpal after
  the callback returns.
- **Fix:** `crates/rustamp-output/src/null.rs:115` `open` rejects an impossible
  format (`Error::InvalidArgument`), refuses a second `open`, and resets
  `played`; `:150` `start` refuses a second `start`; `:281` `drive` advances
  `played` *after* the callback returns. Module docs gain a “What the null
  device models (and what it does not)” section (`:20`) so its tests are read as
  claims about the real path.

### F6 — LOW — WASAPI `open`/`start`/`close` guards and format rejection
- **Defect:** `crates/rustamp-output/src/wasapi_out.rs:154` (pre-change)
  rejected only `sample_rate == 0` and silently clamped `channels == 0` to 1;
  `:197` `start` had no guard, so a second `start` queued a `Cmd::Start` the
  render loop would ignore while reporting success; `:236` `close` did not name
  the device.
- **Fix:** `:333` `open` also rejects `channels == 0`; `:380` `start` refuses
  when `self.started`; `:426` `close` names the device.

### F7 — LOW — crate docs contradicted the code
- **Defect:** `crates/rustamp-output/src/lib.rs:19` (pre-change) claimed
  `default_output()` “never fails — the caller can still fall back to
  `NullOutput`” and implied an opaque backend fallback; it did not say the
  backend is chosen *before* any open and is visible through
  `OutputDevice::name`, nor that the chosen backend never switches device.
- **Fix:** `lib.rs:15` and `:44-56` now state: the backend is chosen before any
  device is opened and is visible via `name()`; once chosen it does not silently
  switch devices; a failed `open` names the device and lists the alternatives;
  the caller falls back to `NullOutput` explicitly.

---

## Render-callback purity (brief point 2)
The contract (`crates/rustamp-core/src/audio.rs:299-304`) requires the render
callback to fill the whole slice and not allocate, lock or log; RFC 0001
(`:34-38`) repeats it. Verified in this change and unchanged:
- cpal callback (`cpal_out.rs:312`): calls `render(buf)` then a single atomic
  `fetch_add` — no allocation, no lock, whole buffer filled by the engine’s
  `RenderFn` (silence-on-underrun is the engine’s duty).
- WASAPI callback (`wasapi_out.rs:682` `write_available`): passes the
  pre-allocated `samples`/`bytes` buffers to `render`, then one `write_to_device`
  and one atomic `fetch_add`. `render_loop` (`:570`) allocates only before the
  loop and is documented “Must not allocate, lock or log”.
- Null `drive` (`null.rs:281`): holds the `Inner` mutex, which is the test
  device’s documented audio-thread lock (`null.rs:16`); the real backends take
  no lock on the audio path.
No code change was needed here; the card records the property as verified.

---

## Tests added
Pure unit tests only: they open no device and start no audio, so they are safe
on any machine and were **not** run (VM-only rule). 16 new tests:
- `crates/rustamp-output/src/null.rs` (module at `:330`), 6 new:
  `open_rejects_a_format_no_device_can_grant`,
  `reopening_resets_the_played_clock_like_a_real_device`,
  `a_second_open_or_start_is_refused`,
  `the_clock_advances_after_the_callback_returns`,
  `the_chunk_size_is_the_reported_buffer_and_bounds_a_callback`,
  `latency_is_zero_before_open`.
- `crates/rustamp-output/src/wasapi_out.rs` (module at `:758`), 5 new:
  `an_open_failure_names_the_endpoint_and_lists_the_alternatives`,
  `an_open_failure_without_alternatives_says_so`,
  `device_loss_has_its_own_message`,
  `a_failure_streak_latches_only_at_the_limit`,
  `open_rejects_an_impossible_format_before_touching_a_device`.
- `crates/rustamp-output/src/cpal_out.rs` (module at `:407`), 5 new:
  `a_named_device_that_matches_nothing_lists_the_available_devices`,
  `an_ambiguous_named_device_says_how_many_and_which`,
  `a_named_device_lookup_with_no_devices_says_so`,
  `a_missing_default_device_lists_the_alternatives`,
  `open_rejects_an_impossible_format_before_touching_a_device`.

The device-*selection* helpers are pure for the same reason WASAPI’s
`open_failure_message`/`FailureStreak` are: they take plain names/strings, so
every message and the latch policy are testable without a device.

---

## Contract cross-references
- `crates/rustamp-core/src/audio.rs:320-353` — the `OutputDevice` trait:
  `open → start → (pause/resume/flush)* → close`; `played_frames()` “since
  `start`” (`:342-344`); `RenderFn` fills the whole slice (`:299-304`);
  `OutputFormat.buffer_frames` (`:306-313`).
- `docs/rfc/0001-core-contracts.md:34-38` — pull-based output;
  `played_frames()` is the audio clock.
- Contract-vs-code: the trait has **no** mid-run error hook, so a device loss
  cannot become an `Event::Error` here. Both real backends expose it out of band
  (`WasapiOutput::stream_error`, `CpalOutput::has_failed`) and report it from
  `close`; wiring it into the engine needs an RFC (engine work, not device
  work). This is recorded, not implemented.

---

## Verification
- `cargo fmt --all -- --check` — clean.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — clean.
- `cargo build --workspace` — succeeded.
- `cargo test` **not** run (VM-only rule); no window; no audio; no downloads; no
  dependency added; no git writes.
- `git status --short` shows only `crates/rustamp-output/src/{lib,null,wasapi_out,cpal_out}.rs`
  and this card.

**Diff size** (before the card): `cpal_out.rs` +249/−…, `lib.rs` +24/−…,
`null.rs` +185/−…, `wasapi_out.rs` +399/−… — 758 insertions, 99 deletions over
4 files. Final line counts: `cpal_out.rs` 428, `lib.rs` 57, `null.rs` 445,
`wasapi_out.rs` 761.
