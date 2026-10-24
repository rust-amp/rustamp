# RFC 0019: a configuration key for the output mode

Status: **Proposed** (2026-10-19). Acceptance belongs to the integration lead.

## Goal

The player must be able to remember that the user asked for an **exclusive**
audio device instead of the default shared one, so that the choice survives a
restart and the preferences dialog can offer it.

The backend already knows both modes: `rustamp_output::WasapiMode` has
`Shared` and `Exclusive` (wave 142, `crates/rustamp-output/src/wasapi_out.rs`),
and `WasapiOutput::with_mode` takes it. The **configuration** had no key for
it, so there was nowhere to put the choice: `AudioConfig` held `output`,
`device`, `buffer_ms`, `fadeout_ms`, `gapless`, `replaygain`, `volume` and
`balance` — and no mode.

This RFC adds that one key and nothing else.

## Decision

- A new type in `rustamp_core::config`:

  ```rust
  /// Output mode of the audio device.
  #[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
  pub enum OutputMode {
      /// Shared mode: the endpoint mixes the player's stream with other clients.
      #[default]
      Shared,
      /// Exclusive mode: the endpoint belongs to the player alone.
      Exclusive,
  }
  ```

- A new field in `AudioConfig`:

  ```rust
  /// Output mode of the device (shared or exclusive).
  pub output_mode: OutputMode,
  ```

  It defaults to `OutputMode::Shared`, which is what the player uses today.

- The mode is a **core** type of its own, not `rustamp_output::WasapiMode`:
  RFC 0001 fixes the dependency direction (`rustamp-core` depends on no
  internal crate), and `rustamp-output` depends on `rustamp-core`, not the
  other way round. The player maps between the two when it opens the device.

## What this RFC does not do

It does **not** make the player open the device in exclusive mode. The app owns
the device factory (`create_output` → `default_output()`), and only the app can
decide which backend is opened and with which mode.

Until the app applies the mode, the field is a **stored but unhonoured**
setting, exactly like `audio.output` and `audio.device` before it: the app's
honest serializer must not write it to the configuration file, and the app's
loader must report it as not applied. The preferences dialog may still show
and change it; the app stores the choice so that the wiring can pick it up.

## Backwards compatibility

- The field is **additive**. `AudioConfig` is `#[serde(default)]`, so a
  configuration file written before this RFC loads unchanged and the missing
  field takes the default (`Shared`).
- The wire is untouched: the configuration is not part of `rustamp-proto` and
  travels only through the TOML file.
- No existing signature changes; the new type and the new field are additions.

## RFC acceptance

- `rustamp_core::config::OutputMode` exists with `Shared` (the default) and
  `Exclusive`.
- `AudioConfig::output_mode` exists, defaults to `Shared` and survives a
  TOML write/read cycle for every mode.
- A file without the key loads and keeps the default.
- `cargo fmt --all -- --check` and
  `cargo clippy --workspace --all-targets --all-features -- -D warnings` are
  clean.
