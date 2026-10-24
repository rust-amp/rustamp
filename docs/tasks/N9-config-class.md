# Task card N9 — Configuration class closure (wave 67)

**Goal**
Close the whole class of bug in Rustamp configuration persistence where the
writer hand-writes fields that the loader reads through Serde, causing dropped
or misformatted fields to silently fall back to defaults or fail deserialization.

**Lane** N — application.

**Owned paths**
- `crates/rustamp-app/**`
- `crates/rustamp-core/src/config.rs`
- `docs/tasks/N9-config-class.md`

Do **not** touch `rustamp-ui`, `rustamp-playlist` or any other crate.

**Allowed sources**
- `docs/tasks/N8-app-red.md` (the lane that fixed one instance and identified the class).
- `docs/tasks/N6-config.md` (honest surface and unhonoured settings).
- `docs/tasks/W10-batch-review.md` (finding F1, the first instance).
- `crates/rustamp-app/src/config.rs` (the writer).
- `crates/rustamp-core/src/config.rs` (the loader types).
- Clean-room Rustamp repository code.

**Forbidden sources**
- Winamp source code, SDK headers, disassembly, dirty room
  (`D:\code\rustamp-research`, `D:\code\rustamp-refbin`, `D:\code\Rustamp`
  as a write target — work only in assigned worktree).

---

## 1. The Bug Class: History and Mechanism

The configuration system previously exhibited two defects of the exact same shape:
1. **W10 finding F1:** `serialize_honest` hand-rolled `WindowId` as a string
   `"Gen(3)"`. The loader deserializes `WindowId` through Serde, where `Gen(u32)`
   is an externally tagged enum representation `{ Gen = 3 }`. The clean-exit save
   wrote a file that the next launch aborted on.
2. **N8 test failure:** `serialize_honest` hand-wrote `WindowLayout` fields (`x`,
   `y`, `visible`, `shaded`) and completely omitted `width` and `height`.
   When loaded back through Serde, `width` and `height` silently defaulted to `None`.

**Mechanism of the class:**
When the loader deserializes via `serde` (`toml::from_str::<Config>`) while the
writer constructs `toml::Table` values by hand field-by-field, any field the
writer forgets or misformats is silently defaulted by Serde (if `#[serde(default)]`
or `Option<T>`) or causes a syntax/variant rejection at load time.
Furthermore, previous round-trip tests tested only unit variants and default
`width: None, height: None` values and checked assertions field-by-field rather than
verifying full struct equality (`assert_eq!(loaded.config, config)`), allowing omissions
to pass tests undetected.

---

## 2. Field Audit Table (The Whole Class)

Every field of every configuration type across `rustamp-core` and `rustamp-proto`:

### `Config` (`crates/rustamp-core/src/config.rs:18-33`)
| Field | Type | Emitted by Writer? | Read by Loader? | Missing Value Becomes | Status | File:Line |
|---|---|---|---|---|---|---|
| `version` | `u32` | Yes | Yes | `1` (`CONFIG_VERSION`) | Persisted | `crates/rustamp-app/src/config.rs:255` |
| `audio` | `AudioConfig` | Yes (table) | Yes | `AudioConfig::default()` | Persisted (honoured subset) | `crates/rustamp-app/src/config.rs:260` |
| `ui` | `UiConfig` | Yes (table) | Yes | `UiConfig::default()` | Persisted (honoured subset) | `crates/rustamp-app/src/config.rs:271` |
| `eq` | `EqConfig` | Yes (table) | Yes | `EqConfig::default()` | Persisted (honoured subset) | `crates/rustamp-app/src/config.rs:325` |
| `playlist` | `PlaylistConfig` | No (dropped) | Yes | `PlaylistConfig::default()` | Deliberately not persisted (unhonoured) | `crates/rustamp-app/src/config.rs:253-351` |
| `plugins` | `PluginsConfig` | No (dropped) | Yes | `PluginsConfig::default()` | Deliberately not persisted (unhonoured) | `crates/rustamp-app/src/config.rs:253-351` |

### `AudioConfig` (`crates/rustamp-core/src/config.rs:61-80`)
| Field | Type | Emitted by Writer? | Read by Loader? | Missing Value Becomes | Status | File:Line |
|---|---|---|---|---|---|---|
| `output` | `String` | No (dropped) | Yes | `"auto".into()` | Deliberately not persisted (only default backend supported) | `crates/rustamp-app/src/config.rs:260-269` |
| `device` | `Option<String>` | No (dropped) | Yes | `None` | Deliberately not persisted (device selection unsupported) | `crates/rustamp-app/src/config.rs:260-269` |
| `buffer_ms` | `u32` | No (dropped) | Yes | `200` | Deliberately not persisted (buffer length fixed) | `crates/rustamp-app/src/config.rs:260-269` |
| `fadeout_ms` | `u32` | No (dropped) | Yes | `500` | Deliberately not persisted (fadeout unsupported) | `crates/rustamp-app/src/config.rs:260-269` |
| `gapless` | `bool` | No (dropped) | Yes | `true` | Deliberately not persisted (gapless not configurable) | `crates/rustamp-app/src/config.rs:260-269` |
| `replaygain` | `ReplayGainMode` | No (dropped) | Yes | `ReplayGainMode::Off` | Deliberately not persisted (ReplayGain unsupported) | `crates/rustamp-app/src/config.rs:260-269` |
| `volume` | `u8` | Yes | Yes | `200` | Persisted (honoured) | `crates/rustamp-app/src/config.rs:261` |
| `balance` | `i8` | Yes | Yes | `0` | Persisted (honoured) | `crates/rustamp-app/src/config.rs:265` |

### `UiConfig` (`crates/rustamp-core/src/config.rs:117-132`)
| Field | Type | Emitted by Writer? | Read by Loader? | Missing Value Becomes | Status | File:Line |
|---|---|---|---|---|---|---|
| `skin` | `Option<PathBuf>` | Yes (when Some) | Yes | `None` | Persisted (honoured) | `crates/rustamp-app/src/config.rs:272` |
| `double_size` | `bool` | Yes | Yes | `false` | Persisted (honoured) | `crates/rustamp-app/src/config.rs:278` |
| `always_on_top` | `bool` | Yes | Yes | `false` | Persisted (honoured) | `crates/rustamp-app/src/config.rs:282` |
| `easy_move` | `bool` | Yes | Yes | `true` | Persisted (honoured, wired in N8) | `crates/rustamp-app/src/config.rs:286` |
| `snap_distance` | `u32` | No (dropped) | Yes | `10` | Deliberately not persisted (docking distance fixed) | `crates/rustamp-app/src/config.rs:271-323` |
| `windows` | `Vec<WindowLayout>` | Yes (when non-empty) | Yes | `vec![]` | Persisted (honoured) | `crates/rustamp-app/src/config.rs:291` |

### `WindowLayout` (`crates/rustamp-core/src/config.rs:98-114`)
| Field | Type | Emitted by Writer? | Read by Loader? | Missing Value Becomes | Status | File:Line |
|---|---|---|---|---|---|---|
| `id` | `WindowId` | Yes | Yes | Deserialization error (required field) | Persisted (honoured) | `crates/rustamp-app/src/config.rs:293` |
| `x` | `i32` | Yes | Yes | Deserialization error (required field) | Persisted (honoured) | `crates/rustamp-app/src/config.rs:293` |
| `y` | `i32` | Yes | Yes | Deserialization error (required field) | Persisted (honoured) | `crates/rustamp-app/src/config.rs:293` |
| `visible` | `bool` | Yes | Yes | Deserialization error (required field) | Persisted (honoured) | `crates/rustamp-app/src/config.rs:293` |
| `shaded` | `bool` | Yes | Yes | Deserialization error (required field) | Persisted (honoured) | `crates/rustamp-app/src/config.rs:293` |
| `width` | `Option<u32>` | Yes (when Some) | Yes | `None` | Persisted (honoured) | `crates/rustamp-app/src/config.rs:293` |
| `height` | `Option<u32>` | Yes (when Some) | Yes | `None` | Persisted (honoured) | `crates/rustamp-app/src/config.rs:293` |

### `EqConfig` (`crates/rustamp-core/src/config.rs:148-153`)
| Field | Type | Emitted by Writer? | Read by Loader? | Missing Value Becomes | Status | File:Line |
|---|---|---|---|---|---|---|
| `state` | `EqState` | Yes | Yes | `EqConfig::default().state` (`EqState { enabled: true, .. }`) | Persisted (honoured) | `crates/rustamp-app/src/config.rs:326` |

### `EqState` (`crates/rustamp-proto/src/state.rs:60-70`)
| Field | Type | Emitted by Writer? | Read by Loader? | Missing Value Becomes | Status | File:Line |
|---|---|---|---|---|---|---|
| `bands` | `[f32; 10]` | Yes | Yes | Deserialization error (required field) | Persisted (honoured) | `crates/rustamp-app/src/config.rs:326` |
| `preamp` | `f32` | Yes | Yes | Deserialization error (required field) | Persisted (honoured) | `crates/rustamp-app/src/config.rs:326` |
| `enabled` | `bool` | Yes | Yes | Deserialization error (required field) | Persisted (honoured) | `crates/rustamp-app/src/config.rs:326` |
| `auto` | `bool` | Yes | Yes | Deserialization error (required field) | Persisted (honoured) | `crates/rustamp-app/src/config.rs:326` |

### `PlaylistConfig` (`crates/rustamp-core/src/config.rs:175-186`)
| Field | Type | Emitted by Writer? | Read by Loader? | Missing Value Becomes | Status | File:Line |
|---|---|---|---|---|---|---|
| `shuffle` | `bool` | No (dropped) | Yes | `false` | Deliberately not persisted (engine does not implement shuffle) | `crates/rustamp-app/src/config.rs:253-351` |
| `repeat` | `bool` | No (dropped) | Yes | `false` | Deliberately not persisted (engine does not implement repeat) | `crates/rustamp-app/src/config.rs:253-351` |
| `manual_advance` | `bool` | No (dropped) | Yes | `false` | Deliberately not persisted (manual advance unsupported) | `crates/rustamp-app/src/config.rs:253-351` |
| `last_playlist` | `Option<PathBuf>` | No (dropped) | Yes | `None` | Deliberately not persisted (restoring playlist unsupported) | `crates/rustamp-app/src/config.rs:253-351` |

### `PluginsConfig` (`crates/rustamp-core/src/config.rs:189-200`)
| Field | Type | Emitted by Writer? | Read by Loader? | Missing Value Becomes | Status | File:Line |
|---|---|---|---|---|---|---|
| `plugin_dir` | `Option<PathBuf>` | No (dropped) | Yes | `None` | Deliberately not persisted (plugin directory fixed) | `crates/rustamp-app/src/config.rs:253-351` |
| `vis_plugin` | `Option<String>` | No (dropped) | Yes | `None` | Deliberately not persisted (vis plugin loading unsupported) | `crates/rustamp-app/src/config.rs:253-351` |
| `vis_preset` | `Option<String>` | No (dropped) | Yes | `None` | Deliberately not persisted (vis preset unsupported) | `crates/rustamp-app/src/config.rs:253-351` |
| `disabled` | `Vec<String>` | No (dropped) | Yes | `vec![]` | Deliberately not persisted (disabling plugins unsupported) | `crates/rustamp-app/src/config.rs:253-351` |

### Summary of Dropped Fields (15 total)
The writer drops exactly 15 fields today, all of which are **deliberately not persisted**:
1. `config.audio.output` (`crates/rustamp-app/src/config.rs:260-269`)
2. `config.audio.device` (`crates/rustamp-app/src/config.rs:260-269`)
3. `config.audio.buffer_ms` (`crates/rustamp-app/src/config.rs:260-269`)
4. `config.audio.fadeout_ms` (`crates/rustamp-app/src/config.rs:260-269`)
5. `config.audio.gapless` (`crates/rustamp-app/src/config.rs:260-269`)
6. `config.audio.replaygain` (`crates/rustamp-app/src/config.rs:260-269`)
7. `config.ui.snap_distance` (`crates/rustamp-app/src/config.rs:271-323`)
8. `config.playlist.shuffle` (`crates/rustamp-app/src/config.rs:253-351`)
9. `config.playlist.repeat` (`crates/rustamp-app/src/config.rs:253-351`)
10. `config.playlist.manual_advance` (`crates/rustamp-app/src/config.rs:253-351`)
11. `config.playlist.last_playlist` (`crates/rustamp-app/src/config.rs:253-351`)
12. `config.plugins.plugin_dir` (`crates/rustamp-app/src/config.rs:253-351`)
13. `config.plugins.vis_plugin` (`crates/rustamp-app/src/config.rs:253-351`)
14. `config.plugins.vis_preset` (`crates/rustamp-app/src/config.rs:253-351`)
15. `config.plugins.disabled` (`crates/rustamp-app/src/config.rs:253-351`)

---

## 3. The Fix

1. **Serde delegation for child structs:**
   In `crates/rustamp-app/src/config.rs`, `serialize_honest` now serializes `WindowLayout`
   and `EqState` directly through Serde (`toml::Value::try_from(w.clone())` and
   `toml::Value::try_from(config.eq.state)`). This eliminates hand-written table field
   construction for child structs, preventing future field additions or format changes
   from ever drifting between writer and loader.
2. **Explicit in-code documentation of non-persisted fields:**
   The code explicitly annotates each non-persisted field and subsection (`audio.*`,
   `ui.snap_distance`, `playlist.*`, `plugins.*`) with the design rationale: they are
   omitted per task card N6 because the player does not support runtime configuration
   of them, and serializing them would falsely advertise supported functionality.
3. **Compile-time and exhaustive testing:**
   - In `crates/rustamp-core/src/config.rs`, `toml_roundtrip` initializes `Config`
     using exhaustive struct literals where **every single field** of every struct is
     explicitly assigned a non-default value. If a new field is added to any config
     struct without updating the round-trip test, the compiler rejects it.
   - In `crates/rustamp-app/src/config.rs` and `crates/rustamp-app/tests/config.rs`,
     `config_round_trip_honours_all_supported_fields` sets all honoured fields to
     non-default values (including all window ID variants, window width/height, and
     `eq.state.enabled = false`) and asserts `assert_eq!(loaded.config, config)`.
   - `unpersisted_fields_deliberately_fall_back_to_defaults` verifies that non-default
     values for unpersisted fields cleanly revert to documented defaults upon round-trip
     and are never written to disk.

---

## 4. What a Serialization Test Cannot Prove

A serialization test proves that in-memory data structures map to and from TOML text
without loss or corruption (`assert_eq!(loaded, written)`). However, it cannot prove:

1. **Runtime application / Honouredness:**
   A serialization test proves that a value round-trips to disk, but cannot prove that the
   player runtime honours it. As proven by W10 finding F5, `ui.easy_move` round-tripped
   flawlessly in N6 serialization tests while having no drag call site at all. Similarly,
   `eq.state.auto` round-trips today without any auto-preset loading logic implemented.
   Testing honouredness requires behavioral tests asserting actual audio engine gain changes,
   window movement, skin texture loading, or dialog interaction.
2. **Filesystem and OS boundary conditions:**
   A unit or integration test using temporary directories cannot prove resilience under
   disk-full errors, permission denials, read-only filesystems, concurrent process locking,
   UNC network paths, or Windows MAX_PATH length limitations.
3. **Schema migrations and version compatibility:**
   A test serializing and deserializing with the same binary cannot prove backward or
   forward compatibility across software versions (e.g. older configs from v1 missing fields,
   or future configs with unknown keys).
4. **Human ergonomics and formatting:**
   A serialization test cannot prove that the generated TOML is human-readable, cleanly
   commented, logically organized, or formatted pleasantly when inspected by a user in a
   text editor.

---

## 5. Verification Receipt

- `cargo fmt --all -- --check`: clean (exit 0).
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: clean (exit 0).
- `cargo build --workspace`: clean (exit 0).
- `cargo test`: **not run** (task constraint; the test suite runs in the Hyper-V VM only).
- No downloads, no `cargo add`, no new dependencies, no git commit created.
- Files changed:
  - `crates/rustamp-core/src/config.rs`
  - `crates/rustamp-app/src/config.rs`
  - `crates/rustamp-app/tests/config.rs`
  - `docs/tasks/N9-config-class.md` (this card)
