# W24 — `OleInitialize failed! RPC_E_CHANGED_MODE`: the backend probe ate the main thread's apartment

Date: 2026-10-18.
Worktree: `C:\Users\micro\.paseo\worktrees\3ikyavhh\paseo-wave121-ole`
Status: **RESOLVED in code (release blocker); only the owner machine can confirm the window now opens.**
Evidence: owner's real machine, packaged player, default output.
Related: `docs/RELEASE-NOTES-0.1.0.md` ("Virtual audio endpoints … are not
evidence of … behaviour on real hardware"), `docs/tasks/X5-smoke-window.md`,
`docs/tasks/H3-output-robustness.md`.

---

## 1. Boundary, ownership and environment

- **Owned paths**: `crates/rustamp-app/**`, `crates/rustamp-output/**`, this card.
- **Not touched**: `crates/rustamp-audio`, `crates/rustamp-core`,
  `crates/rustamp-ui`, `crates/rustamp-ipc`, `crates/rustamp-mediahost`,
  `crates/rustamp-plughost`, `crates/rustamp-vis`, `xtask`,
  `crates/rustamp-platform`, `docs/notes/vm-broker/**`, `docs/notes/vm-run-smoke.ps1`.
  The one change that belongs outside the boundary is written out in §8 and was
  **not** applied.
- No `cargo test`, no VM, no VM broker, no download, no new dependency, no git write.
- Cleanroom boundary kept: only our own sources and the sources of our own
  dependencies in `~/.cargo/registry` (`wasapi` 0.26.0, `cpal` 0.18.2,
  `winit` 0.30.13) were read.
- Gates (all run in this worktree):
  - `cargo fmt --all -- --check` → **exit 0**
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings` → **exit 0**
  - `cargo build --workspace` → **exit 0**

## 2. The evidence, and what the ordering proves

```text
rustamp: skin built-in default skin
rustamp: output Lautsprecher (Realtek(R) Audio) at 44100 Hz, 2 ch
thread 'main' panicked at winit-0.30.13/src/platform_impl/windows/window.rs:1174:
OleInitialize failed! Result was: `RPC_E_CHANGED_MODE`. Make sure other crates are
not using multithreaded COM library on the same thread or disable drag and drop support.
```

The second line is printed by `rustamp_app::run` **after** `App::new` returned
(`crates/rustamp-app/src/lib.rs:125-160`), i.e. after the output device was
chosen and opened; `window::run` is called after it. So the audio path ran
first on the main thread and the window creation was the victim, not the cause.

The device name also identifies the backend: `Lautsprecher (Realtek(R) Audio)`
is a WASAPI *endpoint friendly name* produced by
`Device::get_friendlyname()` in `WasapiOutput`, not a `cpal` device
description. The failing machine therefore took the `WasapiOutput` branch of
`default_output()` — our own WASAPI backend through the `wasapi` crate, not
cpal's WASAPI host.

## 3. Root cause: one line, on the wrong thread

There is no `CoInitialize`/`OleInitialize` anywhere in the workspace — correct.
The apartment is set by **our own** call to the `wasapi` crate's thin wrapper:

- `crates/rustamp-output/src/wasapi_out.rs:765` (pre-change `:752`), in
  `is_shared_mode_available()`:
  `if initialize_mta().ok().is_err() { return false; }`
- `wasapi-0.26.0/src/api.rs:73-75`:
  `pub fn initialize_mta() -> HRESULT { unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) } }`

Call chain, all on the **main thread**:

```text
main (rustamp-app/src/main.rs)
└─ rustamp_app::run                     (rustamp-app/src/lib.rs)
   └─ App::with_registry                (rustamp-app/src/app.rs:213)
      └─ create_output(OutputChoice::Default)        (app.rs:196-204)
         └─ rustamp_output::default_output()         (rustamp-output/src/lib.rs)
            └─ is_shared_mode_available()  ← CoInitializeEx(MTA) on the MAIN thread
   └─ window::run(app)                   → winit → OleInitialize → RPC_E_CHANGED_MODE → panic
```

Two properties of that line make it the defect rather than bad luck:

1. It initialises COM **before** it looks at any device
   (`wasapi_out.rs:764-782`: `initialize_mta` → `DeviceEnumerator::new` →
   `get_default_device` → `get_iaudioclient` → `initialize_client`). So it
   consumes the thread's apartment **even when it answers `false`**.
2. `default_output()` called it inline, i.e. on whatever thread the caller
   happens to be — and the player's caller is the thread that goes on to own
   the event loop and the window.

Everything else was already right: the real endpoint is created and used only
on the render thread, which does its own `initialize_mta()`
(`wasapi_out.rs:467`, inside `open_endpoint`, called from `render_thread`), and
`WasapiOutput::open` only spawns that thread and waits for its answer. The
probe was the single piece of COM work left on the caller's thread.

## 4. What each side needs

| Side | Needs | Where |
|---|---|---|
| `winit` window (main thread) | a **single-threaded apartment**. `OleInitialize` = `CoInitializeEx(COINIT_APARTMENTTHREADED)` + OLE; on an MTA thread it returns `RPC_E_CHANGED_MODE`, and winit **panics** (not degrades) | `winit-0.30.13/.../windows/window.rs:1167-1178`; reached because `attributes.platform_specific.drag_and_drop` defaults to `true` |
| our WASAPI render thread | **MTA**, on *its own* thread — and it initialises it there itself | `wasapi_out.rs:467` |
| the shared-mode probe | MTA on the thread that runs it; nothing else, and it keeps no object alive past its return | `wasapi_out.rs:764-782` |
| `cpal` fallback | **either** apartment: `cpal` asks for **STA** and explicitly tolerates `RPC_E_CHANGED_MODE` | `cpal-0.18.2/src/host/com.rs:10-30` — "Try to initialize COM with STA by default to avoid compatibility issues with … winit (where drag and drop requires STA). This call can fail with `RPC_E_CHANGED_MODE` … That's OK though" |

So the apartments the two sides want are mutually exclusive **on one thread**,
and compatible on two. Nothing about WASAPI needs the *main* thread; drag and
drop does.

## 5. The two directions, honestly

**(A) Put the main thread into an STA first, before any audio work.**
Rejected. It does not survive contact with our own probe: on an STA thread
`initialize_mta()` returns `RPC_E_CHANGED_MODE`, `.ok()` is `Err`, and
`is_shared_mode_available()` returns **`false`** — so `default_output()` would
stop choosing `WasapiOutput` and hand back `CpalOutput` on exactly the machines
that work today. That is a different backend with different behaviour, not the
same device: `CpalOutput::flush` deliberately refuses with
`Error::Unsupported` (`cpal_out.rs:12-19`, A13 / `docs/tasks/W10-batch-review.md:57`),
so stop and seek would start replaying the endpoint tail again. Making it work
would mean *also* teaching the probe to accept `RPC_E_CHANGED_MODE` and
re-verifying shared mode from an STA, plus a `CoInitializeEx` of our own on the
main thread — which rustamp-app cannot do (`#![forbid(unsafe_code)]`) without
either a new API in rustamp-output or a `wasapi` dependency in the app. More
code, more risk, no benefit over (B).

**(B) Keep the apartment-setting audio work off the main thread.** Chosen.
It is one function, it needs no `unsafe`, no new dependency and no new public
API, and it leaves every observable behaviour intact: the same probe, in the
same process, against the same endpoint, with the same client parameters, so
the same backend and the same device are chosen as today. The main thread then
has no apartment at all when `window::run` starts, and winit's `OleInitialize`
puts it into the STA it wants — drag and drop (`WindowEvent::DroppedFile`,
`crates/rustamp-app/src/window.rs:912`, the open-media dialog's drop-to-pick)
stays alive.

**(C) `with_drag_and_drop(false)` on the winit window attributes.**
Rejected: it silences the panic by deleting a feature the player implements and
the original has — dropping files onto the player and onto the open-media
dialog (`window.rs:905-915`). A fix that trades a shipped feature for a
one-line workaround is the wrong trade when (B) costs one thread.

**What the backend does when it does *not* get the apartment it wanted** — the
check that decides whether (B) is safe:

- WASAPI render thread: unaffected by the main thread's apartment. It is a
  fresh thread; its `initialize_mta()` succeeds; all endpoint interfaces are
  created and consumed there. The format negotiation result still travels back
  over an `mpsc` channel (only atomics and `RenderFn` cross the boundary, as
  the file's Threading note states), never a COM pointer.
- Probe thread: `initialize_mta()` on a fresh thread succeeds; the answer is a
  plain `bool`.
- `cpal` fallback (machines where shared mode is unavailable): its
  `com_initialized()` runs on whichever thread opens/starts the stream and
  tolerates both outcomes (§4). Before this fix it got `RPC_E_CHANGED_MODE` on
  the main thread — because the probe had set MTA even while answering
  `false` — and carried on; after the fix it is the one that puts the main
  thread into an STA, which winit then joins (`OleInitialize` → `S_FALSE`,
  which winit accepts: "ok … because it might happen that multiple windows are
  created on the same thread"). Either way: no behaviour change, no panic.
- Therefore no path exists in which the window opens and playback breaks. The
  device, the backend choice, the negotiated format and the render loop are
  byte-for-byte the code that produced the `44100 Hz, 2 ch` line on the owner's
  machine.

## 6. The change

| File | Lines (net) | What |
|---|---|---|
| `crates/rustamp-output/src/lib.rs` | +88 / −1 | `default_output()` calls the new `probe_shared_mode_off_thread()` instead of `is_shared_mode_available()` inline (`:82-89`); the helper spawns a named short-lived thread, joins it, and reports "not available" if it cannot be spawned or panics (`:110-117`); crate- and function-level docs state the apartment rule; new `#[cfg(all(test, windows))]` regression test (`:120-152`) |
| `crates/rustamp-output/src/wasapi_out.rs` | +16 / −2 | documents on `is_shared_mode_available()` (`:750-764`) that it initialises MTA on the *calling* thread before it looks for a device, must never be called on a thread that will own a window, and that `default_output()` is the safe entry point |
| `crates/rustamp-app/src/app.rs` | +9 | `create_output()` doc: this runs on the future window thread, so nothing here may initialise COM on it (`:186-196`) |
| `crates/rustamp-app/src/lib.rs` | +5 | comment at `App::new` (`:120-125`) naming the ordering invariant device-before-window |

Total: **4 files, +117 / −3 lines**; one behavioural change (the probe's
thread), the rest is the contract written down where the next change will read it.

The probe thread exits without `CoUninitialize`, exactly as the inline probe
did on the main thread; the difference is that the unbalanced initialisation
now dies with a thread that nobody needs again, instead of sticking to the
thread that has to create the window. Deinitialising explicitly was considered
and dropped: `is_shared_mode_available()` returns a bare `bool`, so the helper
cannot tell "MTA entered" from "entering it failed", and an unmatched
`CoUninitialize` is worse than a per-thread leak the OS reclaims at thread exit.

## 7. Regression guard (new test, device-independent)

`crates/rustamp-output/src/lib.rs:120-152`,
`default_output_leaves_the_calling_thread_free_for_an_sta`:

```rust
let device = default_output();
drop(device);
let entered = wasapi::initialize_sta();
if entered.is_ok() { wasapi::deinitialize(); }
assert!(entered.is_ok(), "…RPC_E_CHANGED_MODE…");
```

It asserts the exact property the owner's machine disproved — after choosing
the production backend, this thread can still enter the apartment winit needs —
and it **needs no audio endpoint**, because the old code initialised MTA before
it looked for one. So it would have failed on the old code on the owner's
machine *and* in a device-less guest. It opens no stream, plays nothing and
creates no window.

**Not run here**: `cargo test` belongs to the orchestrator VM. `cargo clippy
--workspace --all-targets --all-features` compiled it (exit 0), so it builds;
it has not been executed.

## 8. How this could have been caught before shipping

1. **The guard in §7 would have caught it in the ordinary suite**, on any
   machine, with no endpoint. That is the real answer: the property is about a
   *thread*, not about hardware, and testing it as a hardware property is what
   made it invisible.
2. **The coverage hole was a split, not a missing device.** The guest *does*
   have an endpoint (`docs/RELEASE-NOTES-0.1.0.md`: "Real audio endpoint:
   reference playback | pass"), and `tests/real_playback.rs` drives the
   production `default_output()` path there — but it opens **no window**
   (`real_playback.rs:30-31`, stated in its own header). The window tests and
   the packaged smoke open a window but never a device: the smoke starts the
   packaged player with `--no-audio`
   (`docs/notes/vm-broker/guest/run-smoke.ps1:237`:
   `$psi.Arguments = '--no-audio "' + $Fixture + '" --seconds 120'`), which is
   `NullOutput` and never calls `default_output()`. No test in the suite ever
   did *device then window on one thread*, which is the whole defect.
3. **Change needed outside the ownership boundary — NOT APPLIED.** In
   `docs/notes/vm-broker/guest/run-smoke.ps1`, add a second player start that
   uses the **default** output on the guest's endpoint, after the existing
   `--no-audio` window step and with the same window wait, i.e. a start whose
   arguments are `'"' + $Fixture + '" --seconds 20'` (no `--no-audio`), checked
   with the existing `SmokeWin` window wait and the documented title rule from
   `docs/tasks/X5-smoke-window.md`. That step, and only that step, exercises
   device-then-window in the packaged binary; on the released build it would
   have failed with the owner's panic. It needs the guest endpoint to be present
   (it is) and it plays audible audio in the guest, so it belongs behind the
   same VM gate as the audio tests. The same hole in the test suite could be
   closed inside the app crate instead (a window test that uses
   `OutputChoice::Default`), but that test would need a real endpoint and would
   play audio, so it would be `#[ignore]`d and VM-only — strictly weaker than
   the guard in §7.

## 9. What only the owner machine can prove

The defect was never reproducible here and still is not: this worktree may not
run the VM, and the panic needs a thread that first opened a *real* shared-mode
endpoint and then created a window. From code and dependency source it is
proven that (a) the only apartment-setting call in the workspace is
`initialize_mta()` at `wasapi_out.rs:467` (render thread, correct) and `:765`
(the probe), (b) the probe ran on the main thread before `window::run`, and
(c) it no longer does. What **only the owner machine can show**:

- that the player now reaches its window at all — `rustamp: output Lautsprecher
  (Realtek(R) Audio) at 44100 Hz, 2 ch` followed by a visible `Rustamp` window
  instead of the `OleInitialize` panic;
- that the device line is **unchanged** (same endpoint name, 44100 Hz, 2 ch),
  i.e. the backend choice really did survive moving the probe to another thread
  on Realtek hardware;
- that audio is **audible** on that hardware and that transport, seek and stop
  behave (the WASAPI `flush` path), which no virtual endpoint and no static
  argument can establish;
- that drag and drop works on the real desktop (a file dropped on the player
  starts playing), which is the feature direction (C) would have silently cost;
- whether any *other* component on the owner's machine — a shell extension, an
  input or DSP plugin loaded into the player, an audio driver helper — claims
  the main thread's apartment before winit does. If the same panic survives
  this fix, that is the remaining suspect, and the next step is to log the
  apartment (`CoGetApartmentType`) immediately before `window::run` on that
  machine.
