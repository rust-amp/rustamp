# RFC 0013: Decoder Registry Selection Rule

Status: **Proposed**  
Owner: Lane B (decoders) & Lane I (contracts / architecture).  
Depends on: RFC 0001 (core contracts), RFC 0002 (audio contracts).

---

## 1. Why & Problem Statement

### 1.1 The Problem
In Rustamp, media playback begins by resolving a `MediaSource` to a concrete `Decoder` via the `DecoderRegistry`. The registry queries registered `DecoderFactory` implementations using `DecoderFactory::probe(hint)` and selects a factory based on the returned `Confidence`.

In the current implementation:
1. `default_registry()` in `rustamp-decoders` registers exactly one factory (`SymphoniaDecoderFactory`). As a result, multi-decoder selection and tie-breaking scenarios have never been exercised in production.
2. An audit in Task B2 (`docs/tasks/B2-decoder-robustness.md`) identified that `DecoderRegistry::probe` resolves ties by **registration order** and **never refuses** when two factories claim the same media with identical confidence.
3. Furthermore, when content sniffing fails or is absent, the selection falls back to the file extension hint (`Confidence::EXTENSION`), which can lead to misrouting or unexpected failures the moment a second decoder factory (such as an FFI plugin for chiptunes, module music, or classic Winamp `in_*.dll` decoders) is added.

This rule was neither documented in a dedicated specification nor tested with competing factories. Without an explicit contract, introducing additional decoders risks silent format misdetection, nondeterministic playback behavior, or uninformative errors.

### 1.2 Scope & Contract Layering
**This is a contract change to `rustamp-core`.**

Although `rustamp-decoders` provides concrete factories (like Symphonia), the core contracts governing discovery and selection reside entirely in `rustamp-core`:
- `DecoderRegistry`, `DecoderFactory`, `Confidence`, `ProbeHint`, and `MediaSource` are defined in `crates/rustamp-core/src/audio.rs`.
- The selection algorithm (`DecoderRegistry::probe`) and stream dispatch (`DecoderRegistry::open`) are core methods.
- The error returned when nothing claims a stream (`Error::UnsupportedFormat`) is a `rustamp-core` error enum variant.

Therefore, this RFC establishes binding guarantees for `rustamp-core::audio::DecoderRegistry`, and updates `rustamp-decoders` to conform to and verify these guarantees.

---

## 2. Registry Guarantees

The `DecoderRegistry` guarantees four fundamental invariants:

### Guarantee 1: Content Decides, Not the Extension
*Content signature verification strictly outranks file extension matching.*

- `Confidence` values form a total order:
  - `Confidence::NONE` (`0`): Cannot decode.
  - `Confidence::EXTENSION` (`40`): Extension matches, content not verified.
  - `Confidence::CONTENT` (`90`): Content signature matches.
- Any factory returning `Confidence::CONTENT` (or any score $\ge 90$) strictly defeats any factory returning `Confidence::EXTENSION` (or any score $< 90$), **regardless of registration order**.
- If a file has a misleading or incorrect extension (for example, a FLAC file named `track.mp3`), the factory that identifies the content signature (`fLaC`) is chosen over any factory claiming the `.mp3` extension.

### Guarantee 2: Deterministic Tie-Breaking
*When multiple factories report identical maximum confidence, the earlier registration wins.*

- If two or more factories report the same confidence score ($c > \text{Confidence::NONE}$):
  - The registry selects the factory that was registered first via `DecoderRegistry::register`.
  - The tie-break is completely deterministic and stable across identical runs.
  - The registry does not panic, does not choose at random, and under this baseline rule does not refuse to play.
- Note: Whether ties between equal claimants should instead be refused outright is submitted to the owner as an open question (see §6, Q1).

### Guarantee 3: Deterministic Refusal on Unclaimed Input
*When no factory claims the media, the registry refuses cleanly with `Error::UnsupportedFormat`.*

- If every registered factory reports `Confidence::NONE` for the given `ProbeHint`:
  - `DecoderRegistry::probe` returns `None`.
  - `DecoderRegistry::open` returns `Err(Error::UnsupportedFormat(ext))`, where `ext` is the lowercase extension hint if present, or `"unknown"` if no extension was provided.
  - No factory's `open()` method is invoked, and no partial resources are allocated.

### Guarantee 4: Caller Reliance & Dispatch Semantics
*Callers can predict selection via `probe` and rely on single-winner dispatch.*

- `DecoderRegistry::probe(&hint)` is pure and non-destructive. Callers can inspect which factory will be chosen and its confidence score before attempting to open the stream.
- `DecoderRegistry::open(source)` sniffs up to `DecoderRegistry::SNIFF_BYTES` (4096 bytes), constructs the `ProbeHint`, calls `self.probe(&hint)`, and delegates directly to the winning factory's `open(source)`.
- `open` does not perform speculative backtracking: if the winning factory fails to open the source, the error from that factory is returned immediately. (Backtracking would require buffering or re-opening the `MediaSource`, which is explored in §6, Q4).

---

## 3. Exact API

The public API in `crates/rustamp-core/src/audio.rs` remains structurally backward-compatible, with doc comments and contracts updated to guarantee the selection invariants:

```rust
impl DecoderRegistry {
    /// Number of header bytes sniffed before probing (4096 bytes).
    pub const SNIFF_BYTES: usize = 4096;

    /// Empty registry.
    pub fn new() -> Self;

    /// Register a factory.
    ///
    /// On equal probe confidence, earlier registrations win (RFC 0013).
    pub fn register(&mut self, factory: Arc<dyn DecoderFactory>);

    /// Best factory for `hint`, with its confidence, if any scores above zero.
    ///
    /// Follows the RFC 0013 selection guarantees:
    /// 1. Content decides: `Confidence::CONTENT` strictly beats `Confidence::EXTENSION`.
    /// 2. Deterministic tie-break: on equal confidence, earlier registrations win.
    /// 3. Clear refusal: returns `None` if no factory scores above `Confidence::NONE`.
    pub fn probe(&self, hint: &ProbeHint<'_>) -> Option<(Arc<dyn DecoderFactory>, Confidence)>;

    /// Sniff `source`, pick the best factory, and open a decoder.
    ///
    /// Returns `Err(Error::UnsupportedFormat)` if no factory claims the source (RFC 0013).
    pub fn open(&self, mut source: MediaSource) -> Result<Box<dyn Decoder>>;
}
```

No wire formats or IPC types are altered.

---

## 4. Compatibility Impact

- **`rustamp-audio`**: The audio engine's decode thread relies on `DecoderRegistry::open`. Because the error contract (`Error::UnsupportedFormat`) and return type (`Result<Box<dyn Decoder>>`) are unchanged, the engine continues to handle unplayable files cleanly without modification.
- **`rustamp-app`**: Application wiring uses `DecoderRegistry::new()` and `default_registry()`. Behavior with the bundled Symphonia factory is 100% identical.
- **Mocks & Testing**: Existing tests in `rustamp-core`, `rustamp-audio`, and `rustamp-decoders` pass without breaking changes.

---

## 5. Acceptance Criteria

1. **Content Beats Extension**: In a registry containing both a fake extension claimant registered first and Symphonia (or another content claimant) registered second, real audio content with a mismatched extension is selected by content signature and decodes cleanly.
2. **Deterministic Tie-Break**: Two test factories claiming identical confidence (both at `CONTENT` or both at `EXTENSION`) resolve strictly to the earlier-registered factory in both registration orders, without refusing or panicking.
3. **Refusal on Unclaimed Input**: Media that matches neither content nor extension returns `None` from `probe` and `Err(Error::UnsupportedFormat(...))` from `open`.
4. **Repo Fixtures Open Cleanly**: All repository test fixtures (`sine.flac`, `sine.mp3`, `sine.ogg`, `sine_aac.m4a`, `sine_alac.m4a`) open and decode to completion in the presence of competing extension claimants.
5. **Clean Verification**: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, and `cargo build --workspace` succeed without warnings.

---

## 6. Open Questions for the Owner

The following policy choices are left for the project owner to decide:

| ID | Question | Options & Considerations |
|---|---|---|
| **Q1** | **Should ties be refused outright rather than broken by registration order?** | **Option A (Current / Baseline)**: Earlier registration wins. Permissive and allows default fallbacks (e.g. built-in Symphonia vs external plugin), but may mask configuration ambiguities.<br>**Option B (Strict Refusal)**: If two factories return equal top confidence (especially at `Confidence::CONTENT`), return `Err(Error::AmbiguousFormat)`. Prevents guessing, but makes overlapping format claims unplayable unless disambiguated. |
| **Q2** | **Should decoders have configurable user priority?** | Classic Winamp allowed users to order input plugins in *Preferences -> Plug-ins -> Input*. Should `DecoderRegistry` support explicit numeric priorities or reordering, superseding registration order? |
| **Q3** | **Should non-empty sniffed headers suppress extension matching?** | If `hint.header` contains 4096 bytes from a local file, but no factory recognizes any signature, should extension matching (`Confidence::EXTENSION`) still be allowed to attempt opening the file, or should non-matching bytes suppress extension claims to fail faster? |
| **Q4** | **Should `open()` backtrack on decoder initialization failure?** | If factory A wins `probe()` but its `open()` fails (e.g. invalid header parameters mid-stream), should the registry rewind the `MediaSource` and attempt factory B? (Note: requires `MediaSource` to be seekable/re-openable). |
