# RFC 0011: Move `eq_classic_from_db` to `rustamp-proto`

Status: **Proposed**
Owner: Lane I (contracts / architecture).
Depends on: RFC 0001 (core contracts), RFC 0004 (licensing and permissive wire crates), RFC 0008 (core re-export pattern).

## 1. Why

The 10-band equalizer stage (`rustamp-dsp`) operates entirely on values in decibels (`dB`), nominally in `-12.0..=12.0` (`EqState::MAX_GAIN_DB`). In unit tests validating the S09 anchor points (`0 = +12 dB`, `31 = 0 dB`, `63 = -12 dB`), `rustamp-dsp` tested that its gain factors corresponded with the classic 0..=63 integer domain used by the Winamp IPC surface.

To avoid duplicating the measured mapping formula:
```rust
pub fn eq_classic_from_db(db: f32) -> i32 {
    if !db.is_finite() {
        return 31;
    }
    ((12.0_f32 - db) * 63.0_f32 / 24.0_f32)
        .floor()
        .clamp(0.0, 63.0) as i32
}
```
`rustamp-dsp` originally imported this function from `rustamp_plughost::state_bridge::eq_classic_from_db`, adding `rustamp-plughost` to `rustamp-dsp`'s `[dev-dependencies]`.

This represented a layering inversion:
1. `rustamp-dsp` is an internal audio DSP engine component; depending on `rustamp-plughost` (the out-of-process 32-bit plugin host library) solely for an audited arithmetic conversion of an `EqState` value introduces unnecessary coupling between pure DSP and plugin hosting.
2. The conversion function maps protocol/state domain values (`EqState` gains in dB) to classic surface units. It conceptually belongs with `EqState`, `EQ_BANDS`, and `MAX_GAIN_DB`.
3. Licensing constraints: per RFC 0004, `rustamp-proto` is permissive (`MIT OR Apache-2.0`), whereas `rustamp-core` and `rustamp-dsp` are `GPL-3.0-or-later`. `rustamp-plughost` is permissive and must NOT depend on GPL crates. Placing `eq_classic_from_db` in `rustamp-proto` allows both `rustamp-plughost` and GPL-licensed engine crates (`rustamp-core`, `rustamp-dsp`) to consume it without licensing violations.

## 2. What changes

1. **Move function to `rustamp-proto`**:
   `eq_classic_from_db` is defined in `crates/rustamp-proto/src/state.rs` right next to `EqState`:
   ```rust
   pub fn eq_classic_from_db(db: f32) -> i32;
   ```
2. **Re-export in `rustamp-plughost`**:
   `crates/rustamp-plughost/src/state_bridge.rs` replaces its local definition with:
   ```rust
   pub use rustamp_proto::state::eq_classic_from_db;
   ```
   All existing callers inside `rustamp-plughost` continue to resolve either locally or via `rustamp_proto`.
3. **Re-export in `rustamp-core`**:
   Following the house rule established in RFC 0008, `rustamp_core::state` re-exports `eq_classic_from_db` along with `EqState` and `EQ_BANDS`.
4. **Clean up `rustamp-dsp` dependencies**:
   - `rustamp-plughost` is completely removed from `crates/rustamp-dsp/Cargo.toml` (`[dev-dependencies]` section dropped).
   - `rustamp-proto` is added to `crates/rustamp-dsp/Cargo.toml` dependencies (`rustamp-proto.workspace = true`).
   - `crates/rustamp-dsp/src/eq.rs` updates its test import to `use rustamp_proto::state::eq_classic_from_db;`.

## 3. The rule

Pure mathematical and conversion functions between protocol state domain values and classic Winamp representation that are shared across permissive hosts (`rustamp-plughost`) and GPL core/DSP crates belong in `rustamp-proto` alongside the corresponding state definitions, and must be re-exported by `rustamp-core::state`.

## 4. What does not change

- **No wire representation change**: The function is a pure calculation helper. No serialized types, struct layouts, enum variants, or binary encodings are altered.
- **No protocol bump**: `PROTOCOL_VERSION` remains untouched.
- **Exact arithmetic preserved**: The formula `((12.0_f32 - db) * 63.0_f32 / 24.0_f32).floor().clamp(0.0, 63.0) as i32` and non-finite fallback `31` are identical down to every bit.
- **No API break for plughost callers**: `rustamp_plughost::state_bridge::eq_classic_from_db` is maintained as a public re-export.

## 5. Acceptance criteria

1. `pub fn eq_classic_from_db(db: f32) -> i32` is public in `rustamp_proto::state`.
2. `crates/rustamp-dsp/Cargo.toml` contains no reference to `rustamp-plughost`.
3. `rustamp-plughost` continues to re-export `eq_classic_from_db` from `rustamp_proto::state`.
4. `rustamp-core::state` re-exports `eq_classic_from_db`.
5. Permissive crates (`rustamp-proto`, `rustamp-plughost`, `rustamp-ipc`, `rustamp-plugin-abi`) maintain a clean license closure free from GPL dependencies.
6. `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `cargo build --workspace --offline` pass cleanly.

## 6. Open questions

| ID | Question | Resolution |
|---|---|---|
| Q1 | Should the inverse conversion (`eq_db_from_classic(classic: i32) -> f32`) also be placed in `rustamp-proto`? | Not yet. No component currently requires the inverse calculation; per project rules, we do not implement speculative unmeasured functions until needed by an RFC or spec. |
| Q2 | Should `rustamp-dsp` import `eq_classic_from_db` through `rustamp-core::state` or directly from `rustamp-proto`? | Both work because `rustamp-core::state` re-exports it. Directly naming `rustamp-proto::state::eq_classic_from_db` explicitly ties the test to the wire type definitions. `rustamp-dsp` includes `rustamp-proto.workspace = true` in dependencies. |
