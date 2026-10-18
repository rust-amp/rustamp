# W17 Immediate Visualizer Clear on Stop (Finding W16-F06 Closure)

**Status:** Implemented in `crates/rustamp-ui` and `crates/rustamp-app`. Static gates pass (formatting, clippy, build). Runtime tests, golden updates, and live-player verification are reserved for the orchestrator's Hyper-V VM.

## 1. Background and Finding W16-F06

Adversarial review finding **W16-F06** identified a discrepancy between the claim in `docs/tasks/S08-vis-impl.md` (S08-C2: "really clear the screen after a stop") and the actual implementation:
- In `crates/rustamp-ui/src/visualization.rs`, handling `TransportState::Stopped` zeroed `targets` and set `state.input = VisInput::Missing`, but did not clear `bars` and `peaks`.
- In `VisMode::Scope`, the waveform immediately vanished because `draw_visualization` checked `state.input == VisInput::Fresh`.
- In `VisMode::Spectrum`, `draw_visualization` drew any non-zero rows/peaks regardless of transport state. Bars decayed at 24 rows/s and peaks held for 300 ms then fell at 16 rows/s, remaining visible for up to ~1300 ms after stop.
- In `crates/rustamp-app/src/visualization_tests.rs:213`, `stopping_playing_or_paused_cannot_retain_scope_or_stale_bars` avoided testing Spectrum mode on stop by explicitly switching to `VisMode::Scope` right before `Command::Stop`, asserting `assert_static_pixels`, waiting 2000 ms for the hidden bars to finish decaying, and only then switching back to Spectrum. The test therefore asserted a weaker claim than its name and the S08-C2 claim stated.

## 2. Decision on Honest Behaviour

1. **Specification Analysis:**
   - `specs/S08-vis.md` is open / unwritten.
   - `specs/S02-main-window-behaviour.md` specifies the stopped state as stopped/static. No specification requires or implies a decaying spectrum tail after a stop command.
   - The decaying tail after stop was an unmeasured "own choice" made in S08-vis-impl that contradicted the card's explicit claim S08-C2.
2. **Honest Behaviour:**
   - Screen clearing upon stop must apply uniformly across all visualization modes (`Spectrum`, `Scope`, `ScopeDots`).
   - Decay is valid **only while playback continues** (`TransportState::Playing`), e.g. when frames lag or after input expires during playback.
   - Upon transport stop (`TransportState::Stopped`), both spectrum bars and peaks must be cleared immediately (zeroed), and input marked as `VisInput::Missing`. Render output immediately matches the static background/grid baseline without any lingering bars or peak markers.

## 3. Implementation Details

### `crates/rustamp-ui/src/visualization.rs`
- In `Visualizer::advance(now_ms, transport)`:
  When `transport == TransportState::Stopped`, the visualizer immediately resets to its default clean state (`*self = Self::default()`), preserving the active `mode` and recording `last_ms = Some(now_ms)` and `last_transport = TransportState::Stopped`. This immediately zeroes `bars`, `peaks`, `targets`, and `hold_until`, centers `scope`, and sets `input = VisInput::Missing`.
- Updated doc comment on `Visualizer::advance` to clarify that stop clears the screen immediately in all modes.
- In unit test `stop_and_reset_do_not_replay_scope_or_retain_old_rate_envelopes`:
  Iterated over both `VisMode::Spectrum` and `VisMode::Scope`. Verified that `vis.advance(0, TransportState::Stopped)` immediately results in `vis.state.bars == [0; SPECTRUM_BANDS]` and `vis.state.peaks == [0; SPECTRUM_BANDS]`.

### `crates/rustamp-app/src/visualization_tests.rs`
- In `stopping_playing_or_paused_cannot_retain_scope_or_stale_bars`:
  - Removed mode switching prior to stop.
  - Parameterised across both `[VisMode::Spectrum, VisMode::Scope]`.
  - For each mode, tested stopping directly from `Playing` as well as stopping from `Paused`.
  - Directly asserted `assert_static_pixels(&mut app)` immediately after `app.submit(Command::Stop)`, proving immediate screen clear in both modes.
  - Asserted `app.state().visualization.bars == [0; SPECTRUM_BANDS]` and `peaks == [0; SPECTRUM_BANDS]` immediately after stop, and verified they remain zeroed 2000 ms later.

## 4. Verification and Scope Compliance

- Owned paths touched:
  - `crates/rustamp-ui/src/visualization.rs`
  - `crates/rustamp-app/src/visualization_tests.rs`
  - `docs/tasks/W17-vis-clear.md`
- Forbidden paths avoided:
  - `crates/rustamp-audio/**` was NOT touched.
  - `crates/rustamp-mediahost/**` and `xtask` were NOT touched.
  - No git write commands executed; no external network requests or dependency additions.
- Local verification commands:
  - `cargo fmt --all -- --check`
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings`
  - `cargo build --workspace`

## 5. What Only the VM or Real Desktop Can Prove

- **Runtime Test Execution:** `cargo test` is reserved for the Hyper-V VM orchestrator suite.
- **Golden Test Re-blessing:** `tests/golden/lookalike/vis_stopped_tail.png` and `tests/golden/rust-theme/vis_stopped_tail.png` were previously generated with decaying spectrum bars under the old non-clearing stop behavior. Updating / re-blessing these golden images via `cargo xtask golden --bless` must be executed inside the VM.
- **Live Presentation & Timing:** Only a real desktop or VM can prove actual visual absence of tearing or flicker when stopping audio playback during real audio device output.
