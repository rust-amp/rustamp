# W10 batch review — adversarial review of the six Muse-wave lanes

**Reviewer role:** independent adversarial review. **Findings only — no code was changed,
nothing was committed.**
**Lane worktree:** `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave55-batch-review`
(the only tree read or written). `git status --short` in that worktree is empty after the
review; the only file added by this review is this document.

**The six lanes reviewed** (commits merged as one batch):

| Lane | Commit | Card |
|---|---|---|
| J2 Rust-themed built-in skin | `4a406f0` | `docs/tasks/J2-rust-theme-skin.md` |
| A12 output flush | `b04193d` | `docs/tasks/A12-output-flush.md` |
| N6 configuration | `b6d1d5f` | `docs/tasks/N6-config.md` |
| G3 wake timeout | `94f1ffa` | `docs/tasks/G3-wake-timeout.md` |
| P4 plugin DLL loader | `63619b9` | `docs/tasks/P4-dll-loader.md` |
| B3 decoder selection | `3ecc79b` | `docs/tasks/B3-decoder-selection.md` |

**Method / limits.** Everything below is from reading the tree plus the reproductions stated
with each finding. Per the task constraint **`cargo test` was not run**; the following were run
and are clean: `cargo fmt --all -- --check` (exit 0),
`cargo clippy --workspace --all-targets --all-features -- -D warnings` (exit 0, no warnings),
`cargo build --workspace` (exit 0). The one behavioural reproduction used the built
`target/debug/rustamp.exe --null` with `RUSTAMP_CONFIG` pointed inside the worktree (headless,
no window, no device); the scratch directory was deleted afterwards. Anything that needs a
window, an audio device or the VM is marked *unverifiable here*.

---

## 1. Claim table

Verdicts: **HOLDS**, **PARTIAL**, **FALSE**, **UNVERIFIABLE**.

### Lane J2 — Rust-themed built-in skin

| # | Claim | Verdict | Evidence (file:line) | Smallest reproduction |
|---|---|---|---|---|
| J2-1 | The built-in skin is complete: every **sheet** the loader asks for is present | **HOLDS** | `rustamp-skin/src/rust_theme.rs:53-75` covers all 19 arms of `webamp.rs:119-139` (`Sheet::ALL`); `skin.rs:192-207` reads exactly `Sheet::ALL` | all 19 `sheet_bytes` arms non-`None`; generator emits 33/33 files |
| J2-2 | Every sheet is at the **right size** | **HOLDS** | decoded headers match `rust_theme.rs` `SPEC_SHEET_SIZES` (`tests/rust_theme.rs:20-40`) | BMP header decode of all 19 assets: main 275×116 … video 234×119, all exact |
| J2-3 | All **eleven cursors** are present | **HOLDS** for the fixture set, **PARTIAL** vs the loader | `rust_theme.rs:144-156` ships 11; `webamp.rs:526-551` `CURSOR_NAMES` asks for **24**; `skin.rs:251-266` records **no** substitution for a missing cursor | 13 of the 24 loader-requested cursor names (`eqclose`, `mmenu`, `songname`, `winbut`, …) are absent from the built-in skin and silently resolve to nothing |
| J2-4 | A missing file still falls back to something legible | **HOLDS** | `skin.rs:105-116` (rust sheet, else `[0x17,0x18,0x1C]` fill); `skin.rs:132-143` (lookalike) | no magenta `PLACEHOLDER_RGBA` path is reachable for the built-in skins |
| J2-5 | The look-alike stays reachable (`--skin lookalike` / `builtin:lookalike`) | **HOLDS** functionally | `rustamp-app/src/skin.rs:38-46`, `103-114`; app test `selecting_lookalike_routes_to_lookalike_skin` | `choose(Some("lookalike"), None) == Lookalike` |
| J2-6 | The look-alike is still **verified** | **FALSE** *(closed by J4 `fc3c8c7`)* | `rustamp-skin/tests/lookalike.rs:5,10,28` imports and exercises `BuiltinDefaultSkin`; J2 never touched that file (`git diff --stat 4a406f0^ 4a406f0 -- crates/rustamp-skin/tests/` = `rust_theme.rs` only) | the J1 sprite/dimension tests silently retargeted to the Rust theme; `BuiltinLookalikeSkin` has no dimension test |
| J2-7 | The generator reproduces the committed assets | **HOLDS** | `generator/generate_skin.py:1371-1451` | in-memory re-run of all 33 `RUST_THEME_GENERATORS`: byte-identical except CRLF→LF (`.gitattributes text=auto`, `core.autocrlf=input`) |
| J2-8 | "verify_static_dimensions … **at build time**" | **FALSE** *(closed by W11 `f1b87b8`)* | `docs/tasks/J2-rust-theme-skin.md:100`; there is no `build.rs` in `crates/rustamp-skin/` | the check only runs when the script is executed manually |
| J2-9 | The generator README describes the generator | **FALSE** *(closed by J4 `fc3c8c7`)* | `generator/README.md:13-34` still describes the **look-alike**, its `assets/lookalike/` output and five non-spec sizes (titlebar 350, balance 47, monoster 56, eq_ex 56, gen 178) | `main()` writes to `RUST_THEME_DIR`; `LOOKALIKE_DIR` (`generate_skin.py:18`) is dead |
 
### Lane A12 — output flush

| # | Claim | Verdict | Evidence (file:line) | Smallest reproduction |
|---|---|---|---|---|
| A12-1 | `stop()` flushes the device | **HOLDS** | `rustamp-audio/src/engine.rs:686-687` | code + `tests/engine.rs:2069-2082` |
| A12-2 | A running `seek` flushes on drain | **HOLDS** | `engine.rs:931` in the `DecodeOut::Seeked` arm; drain ordering `decode.rs:811-829` | `decode.rs:819 await_drain()` → `Seeked` → engine flush → `decode.rs:828 wait_for_engine` |
| A12-3 | A seek while stopped flushes | **HOLDS** | `engine.rs:817-818` | — |
| A12-4 | `finish_run` flushes | **HOLDS** | `engine.rs:996-997` | — |
| A12-5 | "a stop no longer leaves the device buffer to replay" | **PARTIAL** — true on WASAPI + the null double, **false on the cpal fallback** *(closed by A13 `e2a8f3b`: cpal refuses with `Unsupported`, restart barrier enforced)* | `rustamp-output/src/cpal_out.rs:365-367` (`flush` is `Ok(())`), documented at `cpal_out.rs:12-14`; `default_output()` picks cpal when WASAPI is unavailable (`rustamp-output/src/lib.rs:57-65`) | a Windows box where `is_shared_mode_available()` is false still replays up to the backend buffer after stop |
| A12-6 | `flush` is called "nowhere it would drop audio that is still to be played" | **FALSE** on a **failed** seek *(closed by A13 `e2a8f3b`: SeekFailed sends no flush)* | `decode.rs:831-840` sends `DecodeOut::Seeked` on the error branch; `engine.rs:918-938` flushes **unconditionally** for every `Seeked` | a seek on a non-seekable/corrupt stream: `decoder.seek` → `Err` → engine `flush()` drops the device buffer while the decoder keeps playing from the old position |
| A12-7 | A seek still loses nothing | **HOLDS** on the happy path | drain → `Seeked` → flush → ack (`decode.rs:811-829`, `engine.rs:928-936`) means no post-seek frame is in the device buffer at flush time | — |
| A12-8 | The buffered-device test "proves" the counterfactual | **PARTIAL** | `tests/engine.rs:2133-2298` | the test asserts `flush()` clears a buffer it filled by hand; it never runs the without-flush case |

### Lane N6 — configuration

| # | Claim | Verdict | Evidence (file:line) | Smallest reproduction |
|---|---|---|---|---|
| N6-1 | Honoured fields take effect at launch and persist | **HOLDS** for volume, balance, skin, double_size, always_on_top, windows, eq.state | `app.rs:186-188,216-222,263-270`; `window.rs:1288-1297,401-410,1117-1126` | fresh `--null` run writes and re-reads all of them |
| N6-2 | `ui.easy_move` is "honoured in drag handling" | **FALSE** *(closed by N8 `24b33e4`: wired at `app.rs:558-560`, `window.rs:1283-1293`)* | card `docs/tasks/N6-config.md:36`; `easy_move` is only read for the menu check (`app.rs:520-521`, `menu.rs:68`) and has **no** drag call site (`WindowEvent::Moved` at `window.rs:1370` only records a position; no `drag_window()`) | set `ui.easy_move = false`, drag the skinned body — nothing changes; it is nonetheless written to the file (`config.rs:287-290`) |
| N6-3 | Malformed file fails loudly and aborts startup | **HOLDS** | `config.rs:199-204,241-244`; `lib.rs:84-90` | `RUSTAMP_CONFIG=bad.toml` with `[audio` → exit 1, `… is malformed: TOML parse error…` |
| N6-4 | Unhonoured fields are omitted and warned about | **HOLDS** | `config.rs:114-186` (warnings), `254-341` (omission) | `replaygain`/`buffer_ms`/`shuffle` produce "not applied" warnings and do not appear in the saved file |
| N6-5 | The saved file round-trips | **HOLDS** for `Main`/`Equalizer`/`Playlist`/`Vis`/`Video`; **FALSE** for `WindowId::Gen(n)` *(closed: fixed in `c32cf7b`, closed as class in N9 `99b2cb3`)* | `config.rs:296-304` writes `id = "Gen({n})"` (a **string**); `WindowId` derives `Serialize`/`Deserialize` (`rustamp-proto/src/state.rs:12-33`), where a newtype variant is `{ Gen = n }` | see Finding **F1** — a config that loads fine makes the **next** launch abort |
| N6-6 | The fresh config's equalizer default matches the wire contract | **HOLDS** | `rustamp-proto/src/state.rs:72-81` `EqState::default().enabled = false`; `rustamp-core/src/config.rs:148-153`; `rustamp-audio/src/engine.rs:272` | fresh `--null` run saves `enabled = false`; `PlayerState::default().eq.enabled == false`. The UI's provisional `EqualizerState::default().enabled = true` (`rustamp-ui/src/equalizer.rs:352-359`) is overwritten by `app.rs:270 sync_eq_from_engine` and does **not** leak |

### Lane G3 — wake timeout

| # | Claim | Verdict | Evidence (file:line) | Smallest reproduction |
|---|---|---|---|---|
| G3-1 | The path is traced down to `WaitForSingleObject` | **HOLDS** | `ring.rs:908-953,878-895`; `wake.rs:135-166` | reading only |
| G3-2 | The measurement settles the question | **FALSE** *(closed by G5 `0f12340`: narrowed claim, extracted deterministic `timeout_ms` test, honest bounds in `ring.rs:1892`, host probe added)* | `ring.rs:1871-1918` | for the 10 ms case the lower bound is `max(10−16, 1) = 1 ms` (`ring.rs:1901-1903`); an early-return defect of up to 9 ms passes |
| G3-3 | The lower bound in `blocking_timeout_returns_timed_out` is honest | **PARTIAL** — honest about the tick, but **weakened** from `>= 8 ms` to `>= 1 ms` (`ring.rs:1833-1848`) *(closed by G5 `0f12340`: documented honestly as derived smoke floor)* | the assertion that failed in the VM was relaxed rather than the behaviour established |
| G3-4 | "the kernel timer interrupt fires at the next tick boundary (~7–8 ms later), evaluating the timer as expired" | **SUSPECT** *(closed by G5 `0f12340`: tick explanation withdrawn in `G3-wake-timeout.md:104-110`)* | card `docs/tasks/G3-wake-timeout.md:82-89` | a tick boundary 7.8 ms later is *before* the 10 ms expiry, so the stated mechanism does not explain an expiry at 7.8 ms; the MSDN wording ("may sleep for less than the specified length") supports the *conclusion* but not this mechanism |
| G3-5 | The ~7.8 ms VM figure is evidenced | **FALSE in-tree** *(closed by G5 `0f12340`: marked unevidenced single observation)* | `G3-wake-timeout.md:80` is the only place the number appears | no run log, capture or manifest for the G3 measurement exists in the repo |

### Lane P4 — plugin DLL loader

| # | Claim | Verdict | Evidence (file:line) | Smallest reproduction |
|---|---|---|---|---|
| P4-1 | A wrong-architecture DLL is refused before loading | **HOLDS** | `pe_preflight.rs:64-90` rejects `0x8664`/`0xaa64`/…; `loader.rs:364-366` runs it before `LoadLibraryExW` | synthetic x64 header → `Preflight(UnsupportedMachine{0x8664})` |
| P4-2 | A **hostile** DLL is refused before loading | **FALSE** *(closed: P5 `8a91563` preflight before mapping & P7 `e8134cf` approval store gate)* | `loader.rs:405-411` maps the image; Windows runs `DllMain` during `LoadLibraryExW`, i.e. **before** the export (`loader.rs:418-442`) or version (`loader.rs:462-471`) checks | any i386 PE with a `DllMain` executes code; a DLL that exports `winampGetInModule2` and returns version `0x100` then runs `init()` (`loader.rs:495-497`) |
| P4-3 | The preflight authorises the load | **FALSE — TOCTOU** *(closed: P5 snapshot & P7 `e8134cf` sealed private copy)* | `pe_preflight.rs:12-13` explicitly says "a future loader must revalidate the exact approved bytes it loads; this result must never authorise a later path-based load (TOCTOU)", but `loader.rs:365` reads bytes and `loader.rs:405-411` loads **by path** | swap the file between the read and `LoadLibraryExW`; the validated bytes are not the loaded bytes |
| P4-4 | Version masking accepts 2.x and 5.x input builds | **HOLDS** | `loader.rs:462-471`; `plugin-abi/src/common.rs:23-26` (`0x0F00_0000`/`0xF000_0000`) | `0x0F000100 & !0xFF000000 == 0x100` |
| P4-5 | The acceptance test proves host interaction | **UNVERIFIABLE here** | `tests/dll_acceptance.rs:68-155` (`#[ignore]`, `require_test_vm`) | VM only |

### Lane B3 — decoder registry selection

| # | Claim | Verdict | Evidence (file:line) | Smallest reproduction |
|---|---|---|---|---|
| B3-1 | Content beats extension regardless of registration order | **HOLDS** | `rustamp-core/src/audio.rs:276-285` (`c > *b`, numeric order) | `tests/robustness.rs:546-611` |
| B3-2 | A tie is broken by registration order, not refused | **HOLDS** | `audio.rs:280` (`c > *b` is strict, so the first winner is kept) | `tests/robustness.rs:503-543,614-685` |
| B3-3 | Unclaimed input is refused with `UnsupportedFormat(ext\|"unknown")` | **HOLDS** | `audio.rs:301-306`; `audio.rs:99-109` lowercases the extension | `tests/robustness.rs:688-726` |
| B3-4 | The implementation matches RFC 0013 | **HOLDS** (doc-only change to core) | `git show 3ecc79b -- crates/rustamp-core/src/audio.rs` = comments only | — |
| B3-5 | The tie rule is reachable | **PARTIAL** | reachable only via a test factory (`robustness.rs:448-501`); production `default_registry()` registers one factory (`rustamp-decoders/src/lib.rs`) | the tie/refusal policy is dead code until a second factory exists |
| B3-6 | RFC status vs the core docs | **PARTIAL (process)** | RFC 0013 is **Proposed** (`docs/rfc/0013-decoder-selection.md:3`) while the core contract comments now cite it as binding (`audio.rs:244-285`) | CONTRIBUTING.md requires an approved RFC before a `rustamp-core` contract change |

---

## 2. Findings by severity

### F1 — HIGH (proven): the configuration self-corrupts for `WindowId::Gen(n)`; the next launch aborts

> **Status (2026-10-18): CLOSED.** Fixed in `c32cf7b` by serializing `WindowId` via its derived serde representation, and closed as a whole class across all configuration types in commit `99b2cb3` (task card N9).

`crates/rustamp-app/src/config.rs:296-304` serialises a generic/plugin window id as a **string**
`"Gen({n})"`. `WindowId` derives `Serialize`/`Deserialize`
(`crates/rustamp-proto/src/state.rs:12-33`), where the newtype variant `Gen(u32)` has the TOML
representation `{ Gen = n }` — **not** a string. The unit variants (`Main`, `Equalizer`,
`Playlist`, `Vis`, `Video`) happen to serialise as plain strings, so only `Gen` is affected, and
every N6 round-trip test uses only unit variants (`crates/rustamp-app/tests/config.rs:45-73,266-294`,
`crates/rustamp-app/src/config.rs:452-480`), so the defect is invisible to the lane's own tests.

**Reproduction (proven, headless, no window):**

```
# valid config using the serde form the type actually accepts:
#   [ui]
#   [[ui.windows]]
#   id = { Gen = 3 }   x = 5   y = 6   visible = true  shaded = false  width = 275  height = 232
RUSTAMP_CONFIG=…\gen_serde.toml  target\debug\rustamp.exe --null     # exit 0, loads
# it now writes the file back with:  id = "Gen(3)"
RUSTAMP_CONFIG=…\gen_string.toml target\debug\rustamp.exe --null     # exit 1:
#   config … is malformed: … unknown variant `Gen(3)`, expected one of
#   `Main`, `Equalizer`, `Playlist`, `Vis`, `Gen`, `Video`
```

The first run accepts the config, the clean-exit save rewrites it into a form the same loader
rejects, and the **next** startup dies before opening anything. Severity is high because it is a
silent write of a value the program cannot read back, on the exact file lifecycle N6 exists to fix.

### F2 — MEDIUM (proven): the loader's PE preflight is defeated by a path-based load (TOCTOU)

> **Status (2026-10-18): CLOSED.** Closed by task card P5 (`8a91563`, passive snapshot preflight) and task card P7 (`e8134cf`, production approval store enforcing private sealed copy mapping before `LoadLibraryExW`).

`crates/rustamp-plughost/src/pe_preflight.rs:12-13` states the rule verbatim: "a future loader must
revalidate the exact approved bytes it loads; this result must never authorize a later path-based
load (TOCTOU)". `loader.rs:365` reads the file into memory and preflights those bytes, then
`loader.rs:405-411` calls `LoadLibraryExW` **on the path**, so the OS re-reads the file. The bytes
that were validated are not the bytes that get mapped. `canonicalize` is best-effort
(`loader.rs:393-399`); if it fails, a relative path plus `LOAD_WITH_ALTERED_SEARCH_PATH` falls back
to the standard search order, which widens the same gap into a DLL-planting vector. This is a
contract that two lanes use in two different ways: the preflight module defines the invariant, the
loader violates it.

### F3 — MEDIUM (proven): the flush fix does not reach the cpal backend, so stale replay survives there

> **Status (2026-10-18): CLOSED.** Closed honestly by task card A13 (`e2a8f3b` + `b40ef8d`): cpal `flush` now refuses with `Unsupported`, and `Engine::flush_output` enforces a fail-closed restart barrier.

A12's claim ("a stop no longer leaves the device buffer to replay", `docs/HANDOFF.md:231`) holds on
WASAPI (`wasapi_out.rs:617-678`) and on the null test double (`null.rs:182-188`) but **not** on the
cpal fallback, whose `flush` is a documented no-op (`cpal_out.rs:12-14,365-367`). `default_output()`
selects cpal whenever `is_shared_mode_available()` is false (`rustamp-output/src/lib.rs:57-65`), so
on such a machine a stop/seek still replays up to the backend buffer. The A12 card never names the
backend gap; it discusses only WASAPI retention and "what only a real device can prove"
(`docs/tasks/A12-output-flush.md:24-40,80-84`). Either the fix is backend-complete or the claim and
the handoff line must be narrowed to WASAPI.

### F4 — MEDIUM (suspected, by code reading): the engine flushes on a **failed** seek and drops still-playable audio

> **Status (2026-10-18): CLOSED.** Closed by task card A13 (`e2a8f3b`): `Worker::apply_seek` sends `Seeked` only on decoder success; a failed seek sends internal `SeekFailed`, emitting no flush.

`crates/rustamp-audio/src/decode.rs:831-840` sends `DecodeOut::Seeked` on the *error* branch of
`decoder.seek`, with no drain and the current timeline. The engine's `Seeked` handler
(`engine.rs:918-938`) calls `self.output.flush()` unconditionally (line 931), re-bases the clock and
clears `pending_seek`. So a seek that the decoder refuses (non-seekable stream, corrupt index —
`seek_frames` never consults `is_seekable`, `engine.rs:804-830`) causes the device buffer to be
dropped while playback continues from the old position: up to one endpoint buffer of audio that was
still to be played disappears. This is exactly the case A12-6 says cannot happen. Not run here
(needs the engine harness / VM), hence suspected.

### F5 — MEDIUM (proven): `ui.easy_move` is advertised and persisted but not honoured

> **Status (2026-10-18): CLOSED.** Closed by task card N8 (`24b33e4`): wired via `App::easy_drag_main` (`crates/rustamp-app/src/app.rs:558-560`), `Windowed::try_easy_drag` (`crates/rustamp-app/src/window.rs:1283-1293,1460-1468`), and pinned by headless test `easy_move_decides_which_presses_drag_the_window`.

`docs/tasks/N6-config.md:36` claims `ui.easy_move` is "honoured in drag handling". There is no drag
path that reads it: `easy_move` is set from the config (`app.rs:266`), toggled (`app.rs:1272`) and
read only for the menu check (`app.rs:520-521`, `menu.rs:68`). `window.rs` has no `drag_window()`
call and its `WindowEvent::Moved` arms (`window.rs:1198,1370,1554`) only record positions. The value
is nevertheless written into the honest file (`config.rs:287-290`), which is the opposite of N6's
stated principle ("omit unhonoured settings so they are not advertised"). A second, softer instance
is `eq.state.auto`: the flag round-trips and lights the window's Auto cell, but no auto-preset
behaviour exists (S07-open), so it is carried rather than honoured.

### F6 — LOW/MEDIUM (proven): J2 silently removed the look-alike's only rendering verification

> **Status (2026-10-18): CLOSED.** Closed by task card J4 (`fc3c8c7`): `crates/rustamp-skin/tests/lookalike.rs` retargeted to `BuiltinLookalikeSkin` with dimension and sprite-fit tests restored.

`crates/rustamp-skin/tests/lookalike.rs` still imports and exercises `BuiltinDefaultSkin`
(lines 5, 10, 28), but J2 re-pointed `BuiltinDefaultSkin` from the look-alike to the Rust theme
(`skin.rs:89-116`) without touching that file (`git diff --stat 4a406f0^ 4a406f0 -- crates/rustamp-skin/tests/`
= `rust_theme.rs` only). The J1 dimension/sprite-fit tests therefore now cover the Rust theme, and
`BuiltinLookalikeSkin` is left with a single app-level smoke test. Its assets carry five non-spec
sizes (titlebar 350×87, balance 47×433, monoster 56×24, eq_ex 275×56, gen 178×86 vs
344/68/58/82/194) that happen to fit the sprite rectangles exactly — a fragile, now-untested
coupling. This is precisely the class of defect a single lane cannot see.

### F7 — LOW (proven): the G3 measurement does not settle the question it claims to settle

> **Status (2026-10-18): CLOSED.** Closed by task card G5 (`0f12340`): `docs/tasks/G3-wake-timeout.md` rewritten to narrow the claim and withdraw the tick-explanation mechanism (`G3-wake-timeout.md:90-118`), deterministic `timeout_ms` contract test extracted and pinned (`crates/rustamp-ipc/src/shm/wake.rs:145,507`), honest test bounds documented (`crates/rustamp-ipc/src/shm/ring.rs:1837,1858,1892`), reproducible host probe added (`crates/rustamp-ipc/examples/wake_timeout_probe.rs`), and the ~7.8 ms figure explicitly marked as an unevidenced single observation.

`ring.rs:1871-1918` is a real measurement loop, but its sub-tick tolerance is
`max(requested − 16 ms, 1 ms)` (`ring.rs:1901-1903`), so for the 10 ms case the only assertion is
"took at least 1 ms". A regression that returns early by up to 9 ms would pass, and the previously
failing `>= 8 ms` bound was relaxed to `>= 1 ms` (`ring.rs:1840-1843`). The card's mechanistic
explanation (`G3-wake-timeout.md:82-89`) is self-inconsistent: a tick boundary "~7–8 ms later" is
before the 10 ms expiry, so it cannot be the moment the timer is evaluated as expired. The MSDN
wording the card cites does support the conclusion ("may sleep for less than the specified length"),
so the conclusion is plausible — but it is asserted, not demonstrated, and the ~7.8 ms figure has no
artefact in the tree. The honest statement is: the test proves the call enters the kernel and does
not return instantly; it cannot separate an early kernel return from an implementation bug.

### F8 — LOW (proven): a hostile i386 DLL reaches `DllMain` and the entry point

> **Status (2026-10-18): CLOSED.** Closed by task cards P5 (`8a91563`) and P7 (`e8134cf`): whole-image SHA-256 approval store gate (`crates/rustamp-plughost/src/approval.rs`) precedes mapping; honestly documented as an ABI loader, not a sandbox.

The preflight is architecture-only (`pe_preflight.rs:64-90`); any 32-bit PE passes, is mapped, and
has its `DllMain` executed by `LoadLibraryExW` before the export and version checks
(`loader.rs:405-471`). The module doc does require trusted code (`loader.rs:356-359`), but the P4
card's §4 "Machine & Preflight Verification" reads as a safety barrier. The truthful statement is
that the loader is an ABI loader, not a sandbox, and nothing rejects a hostile i386 DLL.

### F9 — LOW (proven): cursors get no fallback and no substitution record

> **Status (2026-10-18): CLOSED.** Closed by task card J4 (`fc3c8c7`): `resolve_cursor_fallbacks` (`crates/rustamp-skin/src/skin.rs:468`) added with fallback and substitution records.

Sheets are filled from the fallback and reported (`skin.rs:209-210,393-414`); cursors are neither:
`skin.rs:251-266` pushes a cursor only when the container has it, with no `Substitution` and no
`SkinFallback` call. With `CURSOR_NAMES` at 24 names (`webamp.rs:526-551`) and 11 shipped, 13
loader-requested cursor names resolve to nothing silently. No consumer requests a cursor yet, so the
impact is latent — but it is a second "contract used two ways" (sheets vs cursors) and it makes
J2's "every cursor the loader asks for" literally false.

### F10 — LOW (proven): documentation left stale by the batch

> **Status (2026-10-18): PARTIALLY CLOSED.** Bullets 1 and 2 closed by J4 (`fc3c8c7`) and W11 (`f1b87b8`). Bullet 3 code comment in `crates/rustamp-app/src/skin.rs:6` persists (unowned `.rs` path).

- `crates/rustamp-skin/generator/README.md:13-34` still documents the look-alike generator, its
  `assets/lookalike/` output and five non-spec sizes; `generate_skin.py` now writes only the Rust
  theme and `LOOKALIKE_DIR` (`generate_skin.py:18`) is dead.
- `docs/tasks/J2-rust-theme-skin.md:100` says the dimension check runs "at build time"; there is no
  `build.rs`.
- `crates/rustamp-app/src/skin.rs:4-7` and `src/lib.rs:12` still call the built-in default the
  "look-alike default skin (`BuiltinDefaultSkin`, task card J1)"; it is now the Rust theme.

### F11 — LOW (proven): a malformed config is printed twice

> **Status (2026-10-18): VERIFIED STILL OPEN.** Both print sites exist and fire (`crates/rustamp-app/src/lib.rs:87` and `crates/rustamp-app/src/main.rs:93`). Verified by code reading.

`crates/rustamp-app/src/lib.rs:87` prints the error and returns `Err`; `crates/rustamp-app/src/main.rs:69`
prints it again. Cosmetic, but the "informative message" the card promises arrives duplicated.

### F12 — LOW (proven): the buffered-device flush test proves less than its doc-comment claims

> **Status (2026-10-18): MOOT/SUPERSEDED.** Superseded by A13 (`e2a8f3b`) test doubles in `tests/flush_contract.rs`.

`crates/rustamp-audio/tests/engine.rs:2128-2298` fills a mock device buffer by hand, calls `stop()`
and asserts the buffer is now empty. It never exercises the without-flush counterfactual the comment
advertises ("would replay old audio"), so it pins the call, not the consequence.

### Boundary-level note (error swallowed at a boundary)

Not defects by themselves, but the same shape recurs and is worth naming because the batch adds
call sites: `wasapi_out.rs:620,628,673,676` discard `reset_stream`'s result (the card accepts this),
`host_window.rs:449-451` silently drops an IPC-query record when the `RefCell` is busy, and
`decode.rs:822-828` discards the `Seeked` send and the `wait_for_engine` result. None is new to this
batch, but F1 and F4 are both places where a swallowed failure turns into a silent wrong state.

---

## 3. What cannot be verified without hardware or the VM

- **A12 / F3:** the real WASAPI `Stop → Reset → Start` sequence, the audible replay length without
  flush, and driver `Reset` compliance (`docs/tasks/A12-output-flush.md:80-84`). The cpal
  no-op (F3) is provable by reading, but whether a given cpal host retains its buffer is not.
- **P4:** the entire DLL acceptance test (`tests/dll_acceptance.rs`, `#[ignore]`, `require_test_vm`):
  mapping `in_openmpt.dll`, export resolution, `init()`, IPC queries answered at the host window,
  and clean `quit()`. Also the sign-extension behaviour of the window procedure on i686. The
  wrong-architecture refusal (P4-1) is covered by synthetic headers and holds on the host.
- **G3:** the actual per-run elapsed times in the guest, and whether the ~7.8 ms figure reproduces
  at all. Only the VM has the timer behaviour the card describes; the host test was not run per the
  task constraint.
- **J2:** the fixture comparison (`tests/rust_theme.rs:210-311`) skips without `RUSTAMP_TESTSKINS`;
  the claim that `base-2.91.wsz` carries exactly 18 sheets + 11 cursors is therefore unverified here
  (the assets' own dimensions were verified directly).
- **N6:** the windowed half (always-on-top level, main/EQ/playlist placement and shade restored on a
  real screen) needs a window; only the headless `--null` lifecycle was exercised here.
- **J2/A12/N6 windowed rendering and audio:** no golden test was run (`cargo xtask golden` needs the
  VM/test skins) and no audio was played.

## 4. Review hygiene

- Only `docs/tasks/W10-batch-review.md` was written. `git status --short` in the review worktree is
  empty (verified after removing the scratch config directory and a `__pycache__` left by the
  read-only generator re-run).
- No file under `D:\code\Rustamp`, `D:\code\rustamp-research` or `D:\code\rustamp-refbin` was read
  or written. No code was changed. No commit was created.
- `cargo test` was not run (task constraint). `cargo fmt --all -- --check`,
  `cargo clippy --workspace --all-targets --all-features -- -D warnings` and `cargo build --workspace`
  are clean.
