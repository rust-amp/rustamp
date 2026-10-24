# Task card B2 — Decoder robustness: hostile input, error kinds, resources, registry (lane B)

**Goal**
Make the symphonia-backed decoder (`rustamp-decoders`) fail loudly and safely on
hostile input. Every hostile case must end in a defined error or a defined
degradation — never a panic, never a hang — and the error kinds must tell "not my
format" apart from "mine but damaged". Also: release resources on a half-way
failure, and state (not guess) what the registry does when a file is claimed
twice.

**Lane** B — decoders (`rustamp-decoders`).

**Owned paths**
- `crates/rustamp-decoders/**`
- This card (`docs/tasks/B2-decoder-robustness.md`) only.
- Do **not** edit `rustamp-core`, `rustamp-proto`, `rustamp-audio`, `xtask`, the
  root `Cargo.toml`, or any other crate. A core contract gap is a *finding*
  (RFC), not an edit. No new dependencies.

**Allowed sources**
- `docs/rfc/0002-audio-contracts.md` (accepted): C1 (`skipped_packets`), C2
  (`ReadSeek: Send + Sync`), C3 (`FormatChanged`), C4 (`ErrorKind::FormatChanged`).
- `crates/rustamp-core/src/audio.rs` and `error.rs` (the frozen contract,
  read-only in effect), `crates/rustamp-audio/src/decode.rs` (proves the engine
  handles `FormatChanged` and continues), the vendored symphonia 0.5.5 sources.
- `crates/rustamp-decoders/CONTRACT_NOTES.md`, `tests/fixtures/FIXTURES.txt`.

**Forbidden sources**
- Winamp 5.x source, SDK headers or disassembly, the Winamp 2.95 binary,
  `D:\code\rustamp-research`, `D:\code\rustamp-refbin`. Nothing from them.
- No downloads, no `cargo add`, no new dependencies, no window, no audio.
- **No test execution on this host** (the suite runs in a VM); no commits.

**Hostile cases and what changed**

| Case | Before | After |
| --- | --- | --- |
| Zero-length source | probe rejected it as`UnsupportedFormat("core (probe): no suitable format reader found")` — names the wrong problem | `Error::Decode("empty source: 0 bytes to decode")` before probing (`symphonia_dec.rs:297-303`) |
| Header lying about the format (ID3 with a bogus tag size, RIFF claiming 4 GiB, `fLaC`/`OggS`/ADTS/MPEG magic over noise) | probe failure surfaced as `UnsupportedFormat` (signature matched, body rejected) — a *named collapse*, still so, since nothing is open yet | unchanged, but now a defined outcome on every case and pinned by a table test; the probe-time collapse is documented in `CONTRACT_NOTES.md` |
| Pure noise (no signature) | `probe` returns `NONE`; the registry refuses | same; pinned (`probe` is content-only, no 0xFF false sync in the generated pattern) |
| Truncated WAV / FLAC | short reads degrade to a shorter stream ending in `Ok(0)` | unchanged (contract-conformant), but bounded and never more frames than the bytes hold; truncation-is-silent-EOF is a *named collapse* |
| Format change mid-stream (sample rate) | `convert` returned `false` (`:474-475`), `fill_pending` turned it into `finished = true; Ok(false)` (`:351-354`) — a **silent end of stream** | `Converted::RateChanged` queues the new spec (`:467-473`); `read` reports `Error::FormatChanged` from a call that writes no frames and switches `spec` (`:531-540`), per RFC 0002 C3 |
| Container reset (chained Ogg) | `finished = true; Ok(false)` | unchanged: C3.4 says a container that cannot be continued ends the stream |
| Seek past the end / hostile `Duration` with no known duration | `target.as_secs() * rate` (`:406-407`) overflowed: debug panic / release wrap | `frames_at` saturates and the target is capped at `MAX_SEEK_FRAMES` (2^53) (`:217-221`, `:67`, `:553`) |
| A source that keeps handing out unusable packets | the two bare `continue` arms (`:313`, `:329`) could spin with no bound | counted and failed after `MAX_NO_PROGRESS_PACKETS` (65536) (`:419-420`, `:437-438`, `:495-502`) |
| Post-open `Unsupported` (container/codec mid-stream) | `dec_err` mapped it to `UnsupportedFormat` (`:152-158`): a mid-stream failure was reported as "not my format" | `stream_err` maps it to `Error::Decode`, keeping the message (`:202-208`); `open_err` keeps the open-time mapping (`:188-194`) |
| Zero channel count | `chunks_exact(0)` risk / an empty `pending` could re-loop | `.max(1)` in `read` and `convert` (`:518`, `:660`) |

**Tests**
- Unit (`src/symphonia_dec.rs`): `frames_at_is_exact_and_saturates_at_the_extremes`,
  `unsupported_input_is_classified_by_the_phase_it_happened_in`,
  `a_queued_format_change_is_reported_exactly_once_and_switches_the_spec`,
  `convert_reports_a_sample_rate_change_instead_of_ending_the_stream`,
  `convert_remaps_the_channel_count_to_the_opened_spec`,
  `convert_drops_the_requested_lead_in_frames`,
  `convert_learns_the_spec_from_the_first_buffer`.
- Integration (`tests/robustness.rs`, all inputs generated in-test or a
  truncated/corrupted copy of a generated repo fixture): zero-length reader and
  file, truncated WAV/FLAC, signature-over-garbage table, pure noise, content
  beats a lying extension, bounded drain (a hang becomes a failure),
  reader-release (drop-counting reader; a refused open, a decoder dropped
  mid-stream, 200 open/drain/drop cycles), seek clamping and the unclamped seek,
  registry tie-break (guesses by registration order) and registry refusal.

**Findings for core (not edited here)**
- `DecoderRegistry::probe` breaks a tie by registration order and never refuses;
  a second factory claiming the same file makes a wrong pick possible. Needs an
  RFC (or a caller-side policy).
- `AudioSpec::duration_of` (`rustamp-core`) uses `Duration::from_secs_f64`, which
  panics for an absurd `n_frames` at a low rate. Not reachable through the
  hand-buildable containers here (WAV/AIFF data size ≤ u32, FLAC 36-bit), so the
  decoder caps the frame count instead (`MAX_SEEK_FRAMES`).

**Acceptance gates (this worktree)**
- [x] `cargo fmt --all -- --check` clean.
- [x] `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean.
- [x] `cargo build --workspace --offline` clean.
- [ ] `cargo test --workspace --all-features` — **not run on this host** (VM job);
      the new tests are reasoned-only.
- [x] No test opens a window, plays audio, or downloads; only this worktree was
      written; nothing was committed.
