# Task card N6 — Configuration persistence, honest surface, and loud diagnostics

**Goal**
Resolve defect BETA-12: the configuration was read-only in practice, ignored unhonoured
settings silently, and omitted state serialization on exit. Make the settings the player
can honour actually take effect at launch and persist on clean exit, omit unhonoured
settings from serialization so they are not advertised, report unapplied fields explicitly,
and ensure malformed config files fail loudly with clear diagnostics.

**Lane** N — application.

**Owned paths**
- `crates/rustamp-app/src/**`
- `crates/rustamp-app/tests/**`
- `crates/rustamp-prefs/**` (if model needs it)
- `docs/tasks/N6-config.md`
- Do **not** touch `rustamp-skin`, `rustamp-ui`, `rustamp-audio`, `rustamp-output` or `rustamp-ipc`.

**Allowed sources**
- `docs/tasks/BETA-readiness.md` (finding BETA-12).
- `docs/rfc/0001-core-contracts.md` (Config types and defaults).
- `specs/README.md` (S12 is not started; S01/S02/S03 audited).
- Clean-room Rustamp repository code.

**Forbidden sources**
- Winamp source code, SDK headers, disassembly, dirty room (`D:\code\rustamp-research`, `D:\code\rustamp-refbin`, `D:\code\Rustamp`).

**Scope**
- **In scope:**
  1. Auditing all `Config` fields (`field`, `read`, `written`, `honoured`, `file:line`).
  2. Applying settings the player can honour:
     - `audio.volume` and `audio.balance` (engine gain, UI sliders, persisted on clean exit).
     - `ui.skin` (skin path loaded if present; updated on skin change; persisted on clean exit).
     - `ui.double_size` (applied to main and EQ windows at startup; persisted on clean exit).
     - `ui.always_on_top` (applied to window level; persisted on clean exit).
     - `ui.easy_move` (honoured in drag handling; persisted on clean exit).
     - `ui.windows` (screen positions, visibility, and windowshade for Main, Equalizer, Playlist applied at startup and captured on clean exit).
     - `eq.state` (applied to audio engine and equalizer window at startup; updated on EQ changes; persisted on clean exit).
  3. Non-honoured settings:
     - Omitted from serialized configuration (`serialize_honest`), so they are not advertised.
     - When present in an input TOML file, explicit warnings are emitted rather than silently ignored.
     - In the Preferences dialog, pages state that settings are not applied yet.
  4. File lifecycle:
     - Missing config file uses defaults cleanly.
     - Malformed config file fails loudly with `ConfigError::Malformed` and aborts startup with an informative message.
     - Saving configuration on clean exit in both headless (`--null`) and windowed modes.
  5. Headless tests for round-trip, unadvertised ignored fields, loud error reporting, and runtime application.
- **Out of scope:**
  - Modifying `rustamp-core` contracts (RFC required).
  - Editing `rustamp-output`, `rustamp-audio`, `rustamp-skin`, `rustamp-ui`, or `rustamp-ipc`.

**Acceptance tests**
- [ ] `cargo fmt --all -- --check` clean.
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean.
- [ ] `cargo build --workspace` clean.
- [ ] Headless tests:
  - Config round trip (saving and loading preserves all honoured settings).
  - An ignored field is not advertised (unhonoured fields omitted from serialization; warnings emitted if present in input).
  - A malformed file is reported (fails loudly with file path and error description).
  - Clean exit writes config in headless mode.
  - Startup restores EQ state and window layout flags.
- [ ] No host windows opened, no audio played, no `cargo test` run on host.
