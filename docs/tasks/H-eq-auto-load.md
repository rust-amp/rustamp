# H — S07 EQ auto-load per track

**Status:** implemented in `rustamp-app`; no spec change, no core/proto change.
**Date:** 2026-10-19.
**Write boundary:** only the lane worktree
`C:\Users\micro\.local\share\opencode\worktree\26de0e\wave162-eq-autoload`.
Owned source paths: `crates/rustamp-app/**`, `THIRD_PARTY.md`, this card.
Nothing under `D:\code\Rustamp` was written.

## 1. What S07 settles — and what it does not

S07 §12 item 13 ("Engagement setter") is the whole of the specification for
this task. It settles **one** fact:

- `autoload_eq` is the saved-state key behind the AUTO button, and it toggles
  the AUTO indicator cell exactly (S07 §2.1, S01 §3.1). S07 §1 lists it among
  the equalizer's saved-state keys (`preamp`, `eq_data`, `autoload_eq`,
  `use_eq`).

That is the entire settlement: **the specification names the flag, not the
behaviour behind it.** Everything the task brief asks about — the trigger,
the key that decides the preset, the fallback, the granularity
(per-track/genre/album), the persistence and the precedence against a manual
change — is **open in S07** and is therefore a Rustamp decision.

Two neighbouring open items frame the boundary and were read before the
implementation:

- S07 §12 item 12 (preset load semantics): whether loading a preset changes
  `preamp`, `eq_data`, `autoload_eq` or `use_eq` is open. S04 §10 item 10
  defers the same question to S07.
- S03 §3 row "A | 40245 | Toggle EQ autoloading" settles only that the
  shortcut exists and toggles the flag.

No measurement of the reference player's auto-load behaviour exists, so
nothing here may be cited as a Winamp fact.

## 2. The decisions (Rustamp's own)

| Question | Decision | Why |
|---|---|---|
| Trigger | The lookup runs **once per track start**, in the app's track-change path (`App::handle_event`, the branch that already drives ReplayGain from the track's tags), and only while the AUTO flag is set. | The flag is the settled part; the track start is the only moment the metadata is known. |
| Key | A rule matches a track's **genre, artist or album** tag. | The metadata carries exactly these three as free text (`rustamp_core::meta::TrackMetadata`); the decoders fill them from the file's tags. |
| Order | The rules are tried **in the order the file lists them**; the first match decides. | The file is hand-written, so the user owns the priority. No fixed key precedence is imposed. |
| Match | ASCII case-insensitive, surrounding space ignored, whole value (no splitting of `Rock/Pop`). | Simple and predictable. |
| Fallback | A track that matches no rule gets the file's `default` preset; a file **without** a default leaves the current curve alone. | "Do no harm" is the default; a default is offered for a deterministic auto mode. |
| Precedence | The auto-load runs **at the track start only**. A manual change during the track is never overwritten; the next track start decides again and then the auto-load wins over the manual curve. | The auto-load must not fight the user mid-track, and "auto" means the next track re-decides. Turning AUTO off stops it for good. |
| What a load changes | Only the curve: the ten bands and, for a saved preset, its preamp. The **ON and AUTO flags are left alone** (S07 §12 item 12 leaves this open). | Loading a curve is not a change of the ON/AUTO state, and the built-in presets carry no preamp of their own (same rule as the Presets popup's Load). |
| Granularity | Per track, decided from the track's tags. There is no per-album or per-artist *state*: a rule that names an artist applies to every track of that artist. | "Per album/genre" is a rule *key*, not a stored per-album mode. |
| Persistence | The rules live in **`eq-autoload.toml`**, beside the configuration file and next to the user's `presets.eqf`; read once at start-up, a change takes effect at the next start. | The task brief asks for a `presets.eqf`-adjacent file. A separate file avoids a `rustamp-core` config change, which would need an RFC. |
| Unknown preset name | The rule is noted and changes nothing. | A rule may name a preset the user deleted. |

## 3. The implementation

- `crates/rustamp-app/src/eq_autoload.rs` (new): the rule table
  (`RuleTag`, `AutoLoadRule`, `AutoLoadRules`), the pure decision
  (`AutoLoadRules::decide`), the file parser (`parse`) and the app wiring
  (`load_eq_auto_rules`, `auto_load_eq_preset`).
- `crates/rustamp-app/src/eq_presets.rs`: `named_curve` resolves a preset name
  (built-in first, then the user's saved presets) to its bands and its
  optional preamp.
- `crates/rustamp-app/src/app.rs`: the `eq_auto_rules` field, the load in
  `set_config_path` (next to the `presets.eqf` load) and the call in the
  track-change branch of `handle_event`.
- `crates/rustamp-app/Cargo.toml` + `THIRD_PARTY.md`: `serde` (derive) is now
  a direct dependency of the app, for the hand-written rules file. The crate
  was already in the workspace and in the license inventory; no download.

**No `rustamp-dsp` change was needed.** The engine already owns the
equalizer stage and its `set_eq`; the auto-load only decides a curve and hands
it over. `rustamp-audio`, `rustamp-output` and `rustamp-ui` were not touched.

## 4. Tests (headless, `cargo test -p rustamp-app --all-features`)

Twenty new tests in `eq_autoload.rs`:

- the decision table: the first matching rule decides, every tag selects its
  own metadata field, a value matches ignoring case and space, a track
  without the tag does not match, an unmatched track gets the default, a
  table without a default selects nothing, an empty value or default selects
  nothing;
- the file: it is read into the table, an unknown tag is reported, an unknown
  key and malformed text are reported, a missing file leaves no rules and no
  note, a malformed file is noted;
- the track-start path: a matching rule applies the preset, an artist rule
  selects the preset from the track's artist, the AUTO flag gates the
  lookup, a track without a rule keeps the current curve, the default is
  applied when no rule matches, **a manual change during the track survives
  until the next track starts**, the ON and AUTO flags survive the auto-load,
  and a rule naming an unknown preset is noted and changes nothing.

## 5. Gates

Run in this worktree, `CARGO_NET_OFFLINE` not needed (no new crate was
fetched):

- `cargo fmt --all -- --check`: **0**
- `cargo clippy -p rustamp-app --all-targets --all-features -- -D warnings`: **0**
- `cargo test -p rustamp-app --all-features`: **0** (283 lib tests + the
  integration binaries; the VM-only window tests stay `ignored`)
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: **0**
  (the workspace still compiles clean)

## 6. Not done, and not claimed

- No user interface for editing the rules file. The file is hand-written;
  a preferences page would need a spec (S16 is not started).
- No per-album or per-artist *state*, only rules that name those tags.
- The reference player's auto-load behaviour is still unmeasured: nothing here
  is evidence about it.
- No VM/window acceptance: the feature has no skinned surface of its own
  (the AUTO cell already lights, see S07 §12 item 13).
