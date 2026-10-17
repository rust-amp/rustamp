## Summary

<!-- What does this PR change, and why? Two or three sentences. -->

## Linked work

- Task card: <!-- link or ID -->
- Specs: <!-- S-xx, D-xx IDs -->
- Related RFCs: <!-- docs/rfc/... or none -->

## Test evidence

<!-- Commands run and their results, e.g. cargo test, golden cases, manual checks. -->

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- [ ] `cargo test --workspace --all-features`

## Provenance checklist

- [ ] I did not view Winamp source, the Winamp SDK, or Winamp disassembly while writing this change.
- [ ] I license this contribution under the target crate's license (**inbound = outbound**): GPL-3.0-or-later, or MIT OR Apache-2.0 for `rustamp-proto`, `rustamp-plugin-abi`, `rustamp-ipc`, `rustamp-plughost` and `rustamp-mediahost` (RFC 0004 §7 erratum).
- [ ] Dependency changes include the lockfile, THIRD_PARTY inventory and purpose, both-target/all-feature license-check evidence, and a review of build/native/copied-code and plugin-exception obligations (CONTRIBUTING.md §Dependencies).
- [ ] Specs used: <!-- list the spec files, e.g. specs/S-07-equalizer.md -->
- [ ] Third-party code copied: <!-- none, or list: source, license, and file path -->
  - [ ] Original license header kept
  - [ ] Attribution added (THIRD_PARTY notice updated)
- [ ] No source code, pseudo-code or comments from dirty sources appear in this change.
