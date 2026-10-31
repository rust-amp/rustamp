# RFC 0023: a crossfade duration in `AudioConfig`

Status: proposed

## Problem

The player can overlap the end of a track with the start of the next, and the
duration of that overlap is a user setting that has to survive a restart. The
frozen `rustamp-core` contract has no place for it.

## Change

`rustamp_core::config::AudioConfig` gets one field:

```rust
/// Crossfade between two tracks, in ms; `0` is the gapless seam.
pub crossfade_ms: u32,
```

- The default is `0`, so behaviour does not change until the user asks for it.
- `AudioConfig` is `#[serde(default)]`, so existing configuration files keep
  loading, and an older player keeps its own view of a newer file.
- No `Command`, `Event` or `PlayerState` change: the setting lives on the engine
  side, like the limiter switch and the ReplayGain settings.

## Not a claim about the reference

Whether the reference crossfades at all is not established (S18 §7, item 9). The
setting and its behaviour are an own choice; `crates/rustamp-audio/src/crossfade.rs`
states the rules.
